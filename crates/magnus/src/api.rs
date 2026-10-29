//! The HTTP API, for people's apps. Every request names a person: by a
//! session from sign-in (the desktop app sends it as a bearer token,
//! a browser as an HttpOnly cookie) or by a configured API token. Approvals,
//! answers and policy arrive only here, a channel the agent cannot reach (its
//! sandbox has no network and no tool calls this API).

use crate::config::{Adapter, Person};
use crate::events::Ev;
use crate::inbox::{self, Answer};
use crate::kernel::{new_id, Kernel, HARNESS_VERSION};
use crate::state::{Run, Status};
use axum::{
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use std::net::SocketAddr;
use serde_json::{json, Value};
use std::sync::Arc;

type K = State<Arc<Kernel>>;

pub(crate) fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

pub(crate) fn internal(e: anyhow::Error) -> Response {
    tracing::error!("api: {e:#}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

/// Which listener a request came in on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Plain,
    Tls,
}

pub const SESSION_COOKIE: &str = "magnus_session";

fn bearer(h: &HeaderMap) -> Option<&str> {
    h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "))
}

fn cookie<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get_all("cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
}

fn credential(h: &HeaderMap) -> Option<&str> {
    bearer(h).or_else(|| cookie(h, SESSION_COOKIE))
}

pub(crate) fn who(k: &Kernel, h: &HeaderMap) -> Result<Person, Response> {
    let token = credential(h).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "sign in"))?;
    if let Some(p) = k.cfg.person_by_token(token) {
        return Ok(p.clone());
    }
    k.accounts
        .session(token)
        .and_then(|s| k.cfg.person(&s.person).cloned())
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "sign in again"))
}

pub(crate) fn developer(k: &Kernel, h: &HeaderMap) -> Result<Person, Response> {
    let p = who(k, h)?;
    if !p.is_developer() {
        return Err(err(StatusCode::FORBIDDEN, "this needs the developer role"));
    }
    Ok(p)
}

/// A browser sends the session cookie on its own, so a change made with the
/// cookie must also carry a header another site's page cannot set.
async fn cookie_guard(req: Request, next: Next) -> Response {
    let h = req.headers();
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && bearer(h).is_none() && cookie(h, SESSION_COOKIE).is_some() && h.get("x-magnus-client").is_none() {
        return err(StatusCode::FORBIDDEN, "missing x-magnus-client header");
    }
    next.run(req).await
}

fn owner(k: &Kernel, h: &HeaderMap) -> Result<Person, Response> {
    let p = who(k, h)?;
    if !p.is_owner() {
        return Err(err(StatusCode::FORBIDDEN, "this needs the owner role"));
    }
    Ok(p)
}

pub fn router(k: Arc<Kernel>, transport: Transport) -> Router {
    Router::new()
        .route("/", get(|| ui_file(Path("index.html".into()))))
        .route("/{file}", get(ui_file))
        .route("/fonts/{file}", get(ui_font))
        .route("/v1/health", get(health))
        .route("/v1/sign-in", get(sign_in_method).post(sign_in))
        .route("/v1/sign-in/device", post(device_start))
        .route("/v1/sign-in/device/{flow}", post(device_poll))
        .route("/v1/sign-out", post(sign_out))
        .route("/v1/tasks/{id}/live", get(live))
        .route("/v1/tasks/{id}/outputs/{out}", get(output))
        .route("/v1/transcribe", post(transcribe))
        .route("/v1/me", get(me))
        .route("/v1/versions", get(versions))
        .route("/v1/tasks", post(submit).get(list_tasks))
        .route("/v1/chat", post(chat_post).get(chat_threads))
        .route("/v1/chat/{thread}", get(chat_thread))
        .route("/v1/chat/cards/{card}", post(chat_card))
        .route("/v1/tasks/{id}", get(task))
        .route("/v1/tasks/{id}/timeline", get(timeline))
        .route("/v1/tasks/{id}/resume", post(resume))
        .route("/v1/tasks/{id}/cancel", post(cancel))
        .route("/v1/inbox", get(inbox_list))
        .route("/v1/inbox/{id}", post(respond))
        .route("/v1/policies", get(policies))
        .route("/v1/policies/{*layer}", put(put_policy))
        .route("/v1/secrets", get(secret_names))
        .route("/v1/secrets/{name}", put(put_secret))
        .route("/v1/kill-switch", post(kill_switch))
        .route("/v1/adapters", get(adapters))
        .route("/v1/model", get(get_model).put(put_model))
        .route("/v1/models", get(models))
        .route("/v1/adapters/{id}", put(put_adapter))
        .route("/v1/adapters/{id}/catalog", get(catalog))
        .route("/v1/adapters/{id}/conformance", post(conformance))
        .route("/v1/adapters/{id}/sign-in", post(sign_in_begin))
        .route("/v1/adapters/{id}/sign-in/complete", post(sign_in_complete))
        .route("/v1/memory", get(memory))
        .route("/v1/memory/{id}", axum::routing::delete(forget))
        .route("/v1/schedules", get(schedules).post(add_schedule))
        .route("/v1/schedules/{id}", axum::routing::delete(remove_schedule))
        .merge(crate::dev::router())
        .merge(crate::library::router())
        .layer(axum::middleware::from_fn(cookie_guard))
        .layer(Extension(transport))
        .with_state(k)
}

/// Sign-in hands out a session, so over plain HTTP only from this machine:
/// neither a password nor a session crosses the network unencrypted.
fn over_tls(k: &Kernel, t: Transport, peer: SocketAddr) -> Result<(), Response> {
    if t == Transport::Plain && !peer.ip().is_loopback() {
        let hint = match &k.cfg.appliance.tls_listen {
            Some(l) => format!("sign in over https (port {})", l.rsplit(':').next().unwrap_or_default()),
            None => "sign in over https; the owner has not turned it on (appliance.tls_listen)".into(),
        };
        return Err(err(StatusCode::FORBIDDEN, hint));
    }
    Ok(())
}

/// Open a session for a person and hand it over: to the app as a token,
/// to a browser as an HttpOnly cookie the page cannot read.
async fn issue(k: &Kernel, t: Transport, person: &str, app: bool) -> Response {
    let Some(p) = k.cfg.person(person).cloned() else { return err(StatusCode::FORBIDDEN, format!("{person} is not a person on this appliance.")) };
    let (session, token) = match k.accounts.open_session(person) {
        Ok(s) => s,
        Err(e) => return internal(e),
    };
    let ev = Ev::SignedIn { person: person.to_string(), session: session.id.clone(), client: if app { "app".into() } else { "browser".into() } };
    if let Err(e) = k.emit(&format!("person:{person}"), ev).await {
        return internal(e);
    }
    let mut body = json!({
        "person": { "id": p.id, "name": p.name, "roles": p.roles, "groups": p.groups },
        "session": { "id": session.id, "expires": session.expires },
    });
    if app {
        body["token"] = json!(token);
        return Json(body).into_response();
    }
    let secure = if t == Transport::Tls { "; Secure" } else { "" };
    let max_age = (session.expires - session.created).num_seconds();
    let mut r = Json(body).into_response();
    if let Ok(v) = format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}").parse() {
        r.headers_mut().insert("set-cookie", v);
    }
    r
}

/// How people sign in here: through the identity provider when one is
/// configured, with a password always open as the way in when it is down.
async fn sign_in_method(State(k): K) -> Response {
    match &k.oidc {
        Some(o) => Json(json!({ "method": "oidc", "label": o.label(), "password": true })).into_response(),
        None => Json(json!({ "method": "password", "password": true })).into_response(),
    }
}

/// Password sign-in: the only way without an identity provider, and the
/// backup with one.
async fn sign_in(State(k): K, Extension(t): Extension<Transport>, ConnectInfo(peer): ConnectInfo<SocketAddr>, Json(b): Json<Value>) -> Response {
    if let Err(r) = over_tls(&k, t, peer) {
        return r;
    }
    let person = b["person"].as_str().unwrap_or_default().trim().to_lowercase();
    let password = b["password"].as_str().unwrap_or_default();
    let app = b["client"].as_str() == Some("app");
    if k.cfg.person(&person).is_none() {
        // Same answer and the same work as a wrong password.
        let _ = k.accounts.sign_in(&person, password);
        return err(StatusCode::UNAUTHORIZED, "The name or password is wrong.");
    }
    match k.accounts.sign_in(&person, password) {
        Ok(Ok(_)) => issue(&k, t, &person, app).await,
        Ok(Err(crate::accounts::SignInError::Rejected)) => err(StatusCode::UNAUTHORIZED, "The name or password is wrong."),
        Ok(Err(crate::accounts::SignInError::Locked(until))) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": format!("Too many wrong passwords. Try again after {}.", until.format("%H:%M UTC")), "retry_at": until })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

/// Start a sign-in through the identity provider: the person approves the
/// code on the provider's page, then polls `/v1/sign-in/device/{flow}`.
async fn device_start(State(k): K, Extension(t): Extension<Transport>, ConnectInfo(peer): ConnectInfo<SocketAddr>, Json(b): Json<Value>) -> Response {
    if let Err(r) = over_tls(&k, t, peer) {
        return r;
    }
    let Some(o) = &k.oidc else { return err(StatusCode::NOT_FOUND, "this appliance signs in with a password") };
    match o.start(b["client"].as_str() == Some("app")).await {
        Ok(s) => Json(json!({
            "flow": s.flow,
            "user_code": s.user_code,
            "verification_uri": s.verification_uri,
            "verification_uri_complete": s.verification_uri_complete,
            "expires_in": s.expires_in,
            "interval": s.interval,
            "label": o.label(),
        }))
        .into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

async fn device_poll(State(k): K, Extension(t): Extension<Transport>, ConnectInfo(peer): ConnectInfo<SocketAddr>, Path(flow): Path<String>) -> Response {
    if let Err(r) = over_tls(&k, t, peer) {
        return r;
    }
    let Some(o) = &k.oidc else { return err(StatusCode::NOT_FOUND, "this appliance signs in with a password") };
    match o.poll(&flow).await {
        Ok(crate::oidc::Polled::Pending) => (StatusCode::ACCEPTED, Json(json!({ "pending": true }))).into_response(),
        Ok(crate::oidc::Polled::Done { person, app }) => {
            if k.cfg.person(&person).is_none() {
                tracing::warn!("oidc: {person} signed in at {} but is not a person on this appliance", o.label());
                return err(StatusCode::FORBIDDEN, format!("You signed in as {person}, who is not a person on this appliance. Ask the owner to add you."));
            }
            issue(&k, t, &person, app).await
        }
        Ok(crate::oidc::Polled::Failed(m)) => err(StatusCode::GONE, m),
        Err(e) => err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

async fn sign_out(State(k): K, h: HeaderMap) -> Response {
    let Some(token) = credential(&h) else { return err(StatusCode::UNAUTHORIZED, "not signed in") };
    match k.accounts.sign_out(token) {
        Ok(Some(s)) => {
            if let Err(e) = k.emit(&format!("person:{}", s.person), Ev::SignedOut { person: s.person.clone(), session: s.id }).await {
                return internal(e);
            }
        }
        Ok(None) => {}
        Err(e) => return internal(e),
    }
    let mut r = Json(json!({ "ok": true })).into_response();
    if let Ok(v) = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0").parse() {
        r.headers_mut().insert("set-cookie", v);
    }
    r
}

async fn me(State(k): K, h: HeaderMap) -> Response {
    match who(&k, &h) {
        Ok(p) => Json(json!({ "id": p.id, "name": p.name, "groups": p.groups, "roles": p.roles, "voice": k.cfg.speech.is_some() })).into_response(),
        Err(r) => r,
    }
}

/// What the appliance is running now, and every change to it, so apps can
/// tell people when the agent they work with has changed (U5).
async fn versions(State(k): K, h: HeaderMap) -> Response {
    if let Err(r) = who(&k, &h) {
        return r;
    }
    let v = k.read(|s| {
        let changes: Vec<Value> = s.version_changes.iter().map(|(seq, at, what, value)| json!({ "seq": seq, "at": at, "what": what, "value": value })).collect();
        json!({
            "harness": crate::kernel::HARNESS_VERSION,
            "instructions": crate::run::instructions_version(),
            "chat_instructions": crate::run::chat_instructions_version(),
            "model": s.model.as_ref().map(|(a, m)| format!("{a}/{m}")),
            "playbook": s.playbook_version,
            "latest_seq": s.version_changes.last().map(|c| c.0),
            "changes": changes,
        })
    });
    Json(v).into_response()
}

/// The people's web app: the desktop app's own frontend, served as is.
const UI: &[(&str, &str, &[u8])] = &[
    ("index.html", "text/html; charset=utf-8", include_bytes!("../../../desktop/ui/index.html")),
    ("app.css", "text/css; charset=utf-8", include_bytes!("../../../desktop/ui/app.css")),
    ("bridge.js", "text/javascript; charset=utf-8", include_bytes!("../../../desktop/ui/bridge.js")),
    ("compose.js", "text/javascript; charset=utf-8", include_bytes!("../../../desktop/ui/compose.js")),
    ("app.js", "text/javascript; charset=utf-8", include_bytes!("../../../desktop/ui/app.js")),
    ("dev.js", "text/javascript; charset=utf-8", include_bytes!("../../../desktop/ui/dev.js")),
];
const FONTS: &[(&str, &[u8])] = &[
    ("inter-latin-wght-normal.woff2", include_bytes!("../../../desktop/ui/fonts/inter-latin-wght-normal.woff2")),
    ("inter-latin-ext-wght-normal.woff2", include_bytes!("../../../desktop/ui/fonts/inter-latin-ext-wght-normal.woff2")),
    ("ibm-plex-mono-latin-400-normal.woff2", include_bytes!("../../../desktop/ui/fonts/ibm-plex-mono-latin-400-normal.woff2")),
    ("ibm-plex-mono-latin-500-normal.woff2", include_bytes!("../../../desktop/ui/fonts/ibm-plex-mono-latin-500-normal.woff2")),
];
const UI_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-src 'self' about:; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

fn static_file(media: &str, body: &'static [u8]) -> Response {
    Response::builder()
        .header("content-type", media)
        .header("content-security-policy", UI_CSP)
        .header("x-content-type-options", "nosniff")
        .header("cache-control", "no-cache")
        .body(axum::body::Body::from(body))
        .unwrap()
}

async fn ui_file(Path(file): Path<String>) -> Response {
    match UI.iter().find(|(n, _, _)| *n == file) {
        Some((_, media, body)) => static_file(media, body),
        None => err(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn ui_font(Path(file): Path<String>) -> Response {
    match FONTS.iter().find(|(n, _)| *n == file) {
        Some((_, body)) => static_file("font/woff2", body),
        None => err(StatusCode::NOT_FOUND, "not found"),
    }
}

pub(crate) fn status_str(s: Status) -> &'static str {
    match s {
        Status::Running => "running",
        Status::Suspended => "suspended",
        Status::Completed => "completed",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
    }
}

/// The short title the model gave the task (when starting it, or in its
/// contract), else the start of what the person wrote.
pub(crate) fn title_of(task: &crate::state::Task, run: &Run) -> String {
    task.title
        .clone()
        .or_else(|| run.contract.as_ref().and_then(|c| c["title"].as_str()).map(str::trim).filter(|t| !t.is_empty()).map(String::from))
        .unwrap_or_else(|| task.text.chars().take(80).collect())
}

/// Whether the task waits on an inbox item this person can answer.
fn needs(s: &crate::state::State, p: &Person, run: &Run) -> bool {
    run.status == Status::Suspended && s.items.values().any(|i| i.run.as_deref() == Some(run.id.as_str()) && i.resolution.is_none() && inbox::may_respond(p, i))
}

/// A task in a list: what it is, where it stands, whether it needs this
/// person, and the request it came from (a chat message or a schedule).
pub(crate) fn task_row(k: &Kernel, s: &crate::state::State, p: &Person, t: &crate::state::Task, r: &Run) -> Value {
    let mut v = task_json(k, t, r, false);
    v["needs_you"] = json!(needs(s, p, r));
    v["project"] = crate::library::project_ref(s, &t.task_id);
    v["waiting_item"] = json!(r.waiting_item);
    v["ended_at"] = json!(r.ended_at);
    v["group"] = if let Some(sc) = t.schedule.as_ref().and_then(|id| s.schedules.get(id)) {
        json!({ "kind": "schedule", "id": sc.id, "title": sc.title.clone().unwrap_or_else(|| sc.text.clone()) })
    } else if let Some((c, turn)) = t.chat.as_ref().and_then(|c| c.from_run.as_ref().and_then(|f| s.runs.get(f)).and_then(|r| s.tasks.get(&r.task)).map(|turn| (c, turn))) {
        json!({ "kind": "chat", "id": turn.task_id, "title": turn.text, "thread": c.thread })
    } else {
        Value::Null
    };
    v
}

fn task_json(k: &Kernel, task: &crate::state::Task, run: &Run, full: bool) -> Value {
    let mut v = json!({
        "task_id": task.task_id,
        "run_id": run.id,
        "person": task.person,
        "text": task.text,
        "title": title_of(task, run),
        "created_at": task.created_at,
        "status": status_str(run.status),
        "waiting_on": run.reason,
        "summary": run.summary,
        "thread": task.chat.as_ref().map(|c| c.thread.clone()),
    });
    if full {
        let criteria: Vec<Value> = run
            .contract
            .as_ref()
            .and_then(|c| c["criteria"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|c| {
                let id = c["id"].as_str().unwrap_or_default().to_string();
                let ver = run.verified.get(&id);
                json!({
                    "id": id,
                    "description": c["description"],
                    "check": c.get("check"),
                    "assurance": c.get("assurance"),
                    "assurance_why": c.get("assurance_why"),
                    "verified": ver.map(|v| v.0),
                    "evidence": ver.map(|v| v.1.clone()),
                    "verified_by": ver.map(|v| v.2.clone()),
                })
            })
            .collect();
        v["contract"] = run.contract.clone().unwrap_or(Value::Null);
        v["contract_version"] = json!(run.contract_version);
        v["contract_frozen"] = json!(run.contract_frozen.is_some());
        v["contract_approved_by"] = json!(run.contract_approved_by);
        // Where each criterion that holds got its verdict: the appliance's
        // own tools, a check that ran workspace code, or a person.
        let held = |label: &str| criteria.iter().filter(|c| c["verified"] == json!(true) && !c["verified_by"].as_str().unwrap_or_default().starts_with("person:") && c["assurance"] == label).count();
        let by_person = criteria.iter().filter(|c| c["verified"] == json!(true) && c["verified_by"].as_str().unwrap_or_default().starts_with("person:")).count();
        v["assurance"] = json!({ "appliance_verified": held("appliance-verified"), "self_checked": held("self-checked"), "by_person": by_person });
        v["criteria"] = json!(criteria);
        v["model"] = json!({ "adapter": run.config.adapter, "name": run.config.model });
        v["usage"] = json!({ "model_calls": run.model_calls, "actions": run.actions, "cost_usd": run.usd });
        v["taint"] = json!({ "untrusted": run.untrusted, "private": run.private });
        v["outputs"] = json!(run.outputs.iter().map(|(id, name, mt, size)| json!({ "id": id, "name": name, "media_type": mt, "size": size })).collect::<Vec<_>>());
        v["plan"] = run.plan.clone().unwrap_or(Value::Null);
        v["plan_version"] = json!(run.plan_version);
        v["outcome_view"] = run.outcome_view.clone().unwrap_or(Value::Null);
        // Only here, never in a list's rows: those are built under the state lock.
        v["files"] = json!(k.read(|s| task.files.iter().filter_map(|id| s.files.get(id)).map(|f| crate::library::file_json(s, f)).collect::<Vec<_>>()));
        v["project"] = k.read(|s| crate::library::project_ref(s, &task.task_id));
        v["preview"] = json!({ "chars": crate::compose::PREVIEW_CHARS, "rows": crate::compose::MAX_ROWS });
        v["spoken"] = json!(run.spoken);
        v["ended_at"] = json!(run.ended_at);
        v["waiting_item"] = json!(run.waiting_item);
        v["config"] = json!({ "harness": run.config.harness, "instructions": run.config.instructions, "playbook": run.config.playbook });
        v["open_items"] = json!(k.read(|s| s
            .items
            .values()
            .filter(|i| i.run.as_deref() == Some(&run.id) && i.resolution.is_none())
            .map(|i| i.id.clone())
            .collect::<Vec<_>>()));
    }
    v
}

async fn submit(State(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let text = b["text"].as_str().unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return err(StatusCode::BAD_REQUEST, "the task has no text");
    }
    let mut files = match crate::library::given_files(&k, &p, &b) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let project = match crate::library::chosen_project(&k, &p, &b) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let model = match chosen_model(&k, &b).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    // Files sent with the task are uploads like any other: they go to the
    // person's library, linked to this task.
    for a in b["attachments"].as_array().into_iter().flatten() {
        match crate::library::add_file(&k, &p, a, None).await {
            Ok(Ok(id)) => files.push(id),
            Ok(Err(r)) => return r,
            Err(e) => return internal(e),
        }
    }
    let o = crate::kernel::NewRun { model, files, project, ..Default::default() };
    match k.create_run(&format!("person:{}", p.id), &p.id, text, vec![], o).await {
        Ok((task, run)) => (StatusCode::CREATED, Json(json!({ "task_id": task, "run_id": run }))).into_response(),
        Err(e) => internal(e),
    }
}

/// The model a request chose (`"model": {"adapter", "model"}`), if any,
/// once its adapter confirms it offers it.
pub(crate) async fn chosen_model(k: &Kernel, b: &Value) -> Result<Option<(String, String)>, Response> {
    if b["model"].is_null() {
        return Ok(None);
    }
    let (Some(adapter), Some(model)) = (b["model"]["adapter"].as_str(), b["model"]["model"].as_str()) else {
        return Err(err(StatusCode::BAD_REQUEST, "model needs adapter and model"));
    };
    match k.check_model(adapter, model).await {
        Ok(()) => Ok(Some((adapter.into(), model.into()))),
        Err(m) => Err(err(StatusCode::BAD_REQUEST, m)),
    }
}

fn visible_task(p: &Person, t: &crate::state::Task) -> bool {
    t.person == p.id || p.is_owner()
}

async fn list_tasks(State(k): K, h: HeaderMap) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let tasks: Vec<Value> = k.read(|s| {
        s.tasks
            .values()
            .filter(|t| visible_task(&p, t) && !t.is_chat_turn())
            .filter_map(|t| s.runs.get(&t.run_id).map(|r| task_row(&k, s, &p, t, r)))
            .collect()
    });
    Json(json!({ "tasks": tasks })).into_response()
}

/// The person writes in chat: a new thread, or one of theirs.
async fn chat_post(State(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let text = b["text"].as_str().unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return err(StatusCode::BAD_REQUEST, "the message has no text");
    }
    let model = match chosen_model(&k, &b).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let files = match crate::library::given_files(&k, &p, &b) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let project = match crate::library::chosen_project(&k, &p, &b) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match k.chat_say(&p.id, text, b["thread"].as_str().map(String::from), model, None, files, project).await {
        Ok(Some((thread, task))) => (StatusCode::CREATED, Json(json!({ "thread": thread, "task_id": task }))).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "no such thread"),
        Err(e) => internal(e),
    }
}

async fn chat_threads(State(k): K, h: HeaderMap) -> Response {
    match who(&k, &h) {
        Ok(p) => Json(json!({ "threads": k.threads(&p.id) })).into_response(),
        Err(r) => r,
    }
}

/// A thread is its starter's alone.
async fn chat_thread(State(k): K, h: HeaderMap, Path(thread): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match k.thread_entries(&p.id, &thread) {
        Some(entries) => Json(json!({ "thread": thread, "entries": entries })).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such thread"),
    }
}

/// Confirm or decline a schedule card; only the person it was proposed to.
async fn chat_card(State(k): K, h: HeaderMap, Path(card): Path<String>, Json(b): Json<Value>) -> Response {
    use crate::chat::CardRefusal;
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match k.decide_card(&p.id, &card, b["decision"].as_str().unwrap_or_default()).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(CardRefusal::NotFound)) => err(StatusCode::NOT_FOUND, "no such card"),
        Ok(Err(CardRefusal::Decided)) => err(StatusCode::CONFLICT, "this card has been decided"),
        Ok(Err(CardRefusal::Invalid(m))) => err(StatusCode::BAD_REQUEST, &m),
        Err(e) => internal(e),
    }
}

fn find(k: &Kernel, p: &Person, id: &str) -> Option<(crate::state::Task, Run)> {
    k.read(|s| {
        let t = s.tasks.get(id)?;
        if !visible_task(p, t) {
            return None;
        }
        Some((t.clone(), s.runs.get(&t.run_id)?.clone()))
    })
}

async fn task(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match find(&k, &p, &id) {
        Some((t, r)) => Json(task_json(&k, &t, &r, true)).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such task"),
    }
}

/// The task view's evidence: each step's tool calls, boundary decisions and
/// results, as derived from the log (K45).
async fn timeline(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some((_, run)) = find(&k, &p, &id) else { return err(StatusCode::NOT_FOUND, "no such task") };
    let mut steps = vec![];
    for reply in &run.replies {
        let mut calls = vec![];
        let mut said = String::new();
        for b in &reply.blocks {
            match b {
                magnus_adapter_proto::Block::Text { text } => said.push_str(text),
                magnus_adapter_proto::Block::ToolCall { id, name, input } => {
                    let r = run.results.get(id);
                    let decisions: Vec<Value> = run
                        .decisions
                        .iter()
                        .filter(|d| &d.call_id == id)
                        .map(|d| json!({ "stage": d.stage, "outcome": d.outcome, "rule": d.rule }))
                        .collect();
                    calls.push(json!({
                        "call_id": id, "tool": name, "input": input,
                        "issuer": run.intents.get(id).map(|i| i.issuer.clone()).unwrap_or_else(|| "model".into()),
                        "decisions": decisions,
                        "result": r.map(|r| r.text.chars().take(4000).collect::<String>()),
                        "is_error": r.map(|r| r.is_error), "status": r.map(|r| r.status.clone()),
                    }));
                }
                _ => {}
            }
        }
        steps.push(json!({ "step": reply.step, "text": said, "calls": calls }));
    }
    let notices: Vec<Value> = run.notices.iter().map(|(step, seq, text)| json!({ "step": step, "seq": seq, "text": text })).collect();
    Json(json!({ "steps": steps, "notices": notices, "verifications": run.verified.iter().map(|(c, v)| json!({ "criterion": c, "passed": v.0, "evidence": v.1, "by": v.2 })).collect::<Vec<_>>() })).into_response()
}

async fn resume(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some((_, run)) = find(&k, &p, &id) else { return err(StatusCode::NOT_FOUND, "no such task") };
    if run.status != Status::Suspended {
        return err(StatusCode::CONFLICT, "the task is not paused");
    }
    if k.read(|s| s.kill_switch) {
        return err(StatusCode::CONFLICT, "the kill switch is engaged");
    }
    if let Err(e) = k.emit(&format!("person:{}", p.id), Ev::Resumed { run: run.id.clone() }).await {
        return internal(e);
    }
    k.wake(&run.id);
    Json(json!({ "ok": true })).into_response()
}

async fn cancel(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some((_, run)) = find(&k, &p, &id) else { return err(StatusCode::NOT_FOUND, "no such task") };
    if run.status.terminal() {
        return err(StatusCode::CONFLICT, "the task has ended");
    }
    if let Err(e) = k.emit(&format!("person:{}", p.id), Ev::Cancelled { run: run.id.clone() }).await {
        return internal(e);
    }
    k.on_stop(&run.id, true).await;
    Json(json!({ "ok": true })).into_response()
}

async fn inbox_list(State(k): K, h: HeaderMap) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let items: Vec<Value> = k.read(|s| s.items.values().filter(|i| inbox::visible(&p, i)).map(|i| inbox::render(s, &p, i)).collect());
    Json(json!({ "items": items })).into_response()
}

async fn respond(State(k): K, h: HeaderMap, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match k.respond(&p, &id, &b).await {
        Ok(Answer::Ok) => Json(json!({ "ok": true })).into_response(),
        Ok(Answer::Forbidden(m)) => err(StatusCode::FORBIDDEN, m),
        Ok(Answer::NotFound) => err(StatusCode::NOT_FOUND, "no such item"),
        Ok(Answer::Conflict(m)) => err(StatusCode::CONFLICT, m),
        Ok(Answer::Invalid(m)) => err(StatusCode::BAD_REQUEST, m),
        Err(e) => internal(e),
    }
}

async fn policies(State(k): K, h: HeaderMap) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let all = k.read(|s| s.policies.clone());
    let mine: serde_json::Map<String, Value> = all
        .into_iter()
        .filter(|(l, _)| inbox::may_set_policy(&p, l) || l == "org" || l == &format!("user/{}", p.id) || p.groups.iter().any(|g| l == &format!("group/{g}")))
        .map(|(l, c)| (l, Value::String(c)))
        .collect();
    Json(json!({ "layers": mine, "default_org": crate::policy::DEFAULT_ORG })).into_response()
}

async fn put_policy(State(k): K, h: HeaderMap, Path(layer): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let layer = layer.trim_matches('/').to_string();
    let valid = layer == "org"
        || layer.strip_prefix("user/").is_some_and(|u| k.cfg.person(u).is_some())
        || layer.strip_prefix("group/").is_some_and(|g| !g.is_empty() && !g.contains('/'));
    if !valid {
        return err(StatusCode::NOT_FOUND, "no such policy layer");
    }
    if !inbox::may_set_policy(&p, &layer) {
        return err(StatusCode::FORBIDDEN, "you may not set this layer's policy");
    }
    let cedar = b["cedar"].as_str().unwrap_or_default().to_string();
    if let Err(e) = crate::policy::validate(&cedar) {
        return err(StatusCode::BAD_REQUEST, format!("the policy does not parse: {e}"));
    }
    match k.emit(&format!("person:{}", p.id), Ev::PolicySet { layer, cedar }).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => internal(e),
    }
}

async fn secret_names(State(k): K, h: HeaderMap) -> Response {
    if let Err(r) = owner(&k, &h) {
        return r;
    }
    match k.vault.names() {
        Ok(n) => Json(json!({ "names": n })).into_response(),
        Err(e) => internal(e),
    }
}

async fn put_secret(State(k): K, h: HeaderMap, Path(name): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(value) = b["value"].as_str() else { return err(StatusCode::BAD_REQUEST, "no value") };
    if let Err(e) = k.vault.put(&name, value) {
        return internal(e);
    }
    match k.emit(&format!("person:{}", p.id), Ev::SecretSet { name }).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e),
    }
}

async fn kill_switch(State(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let engaged = b["engaged"].as_bool().unwrap_or(true);
    let actor = format!("person:{}", p.id);
    if let Err(e) = k.emit(&actor, Ev::KillSwitch { engaged }).await {
        return internal(e);
    }
    if engaged {
        let active: Vec<String> = k.read(|s| s.runs.values().filter(|r| r.status == Status::Running).map(|r| r.id.clone()).collect());
        for run in active {
            if let Err(e) = k.suspend(&run, "kill_switch", None).await {
                return internal(e);
            }
        }
    }
    Json(json!({ "engaged": engaged })).into_response()
}

async fn adapters(State(k): K, h: HeaderMap) -> Response {
    if let Err(r) = owner(&k, &h) {
        return r;
    }
    let list: Vec<Value> = k
        .adapters
        .list()
        .await
        .into_iter()
        .map(|(id, cmd, running)| {
            let verified = k.read(|s| s.conformance.get(&id).cloned());
            json!({
                "id": id, "command": cmd,
                "running": running.map(|(a, v)| json!({ "adapter": a, "version": v })),
                "conformance": verified.map(|(v, m, ok, _)| json!({ "version": v, "model": m, "passed": ok })),
            })
        })
        .collect();
    Json(json!({ "adapters": list })).into_response()
}

/// Replace or add an adapter while the kernel runs. The candidate must pass
/// the conformance suite against the live provider before it takes requests,
/// unless the owner explicitly skips the gate.
async fn put_adapter(State(k): K, h: HeaderMap, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let cfg: Adapter = match serde_json::from_value(json!({ "id": id, "command": b["command"], "settings": b.get("settings").cloned().unwrap_or(json!({})) })) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("bad adapter: {e}")),
    };
    let candidate = match k.adapters.launch(&cfg).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    };
    let info = json!({ "adapter": candidate.adapter, "version": candidate.version });
    if b["skip_conformance"].as_bool() != Some(true) {
        let model = match conformance_model(&k, &id, &b["model"]).await {
            Ok(m) => m,
            Err(m) => return err(StatusCode::BAD_GATEWAY, m),
        };
        let report = k.adapters.conformance(&candidate, &model).await;
        let rv = serde_json::to_value(&report).unwrap_or_default();
        let ev = Ev::AdapterVerified {
            adapter_id: id.clone(),
            adapter: report.adapter.clone(),
            version: report.version.clone(),
            model,
            passed: report.passed,
            report: rv.clone(),
        };
        if let Err(e) = k.emit(&format!("person:{}", p.id), ev).await {
            return internal(e);
        }
        if !report.passed {
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "installed": false, "conformance": rv }))).into_response();
        }
    }
    k.adapters.install(candidate).await;
    Json(json!({ "installed": true, "running": info })).into_response()
}

/// The model a conformance run uses when none is named: the appliance's model
/// if this adapter serves it, otherwise the first model the adapter lists
/// (a fixed version first). Another provider's model would fail every check.
async fn conformance_model(k: &Kernel, id: &str, asked: &Value) -> Result<String, String> {
    if let Some(m) = asked.as_str() {
        return Ok(m.to_string());
    }
    let (adapter, model) = k.read(|s| s.model.clone()).unwrap_or_else(|| (k.cfg.model.adapter.clone(), k.cfg.model.name.clone()));
    if adapter == id {
        return Ok(model);
    }
    match k.adapters.catalog(id).await {
        Ok(magnus_adapter_proto::Event::Catalog { models }) => {
            models.iter().find(|m| m.fixed_version).or(models.first()).map(|m| m.id.clone()).ok_or_else(|| format!("adapter {id} lists no models; name one to test"))
        }
        Ok(magnus_adapter_proto::Event::Error { message, .. }) => Err(format!("could not list adapter {id}'s models: {message}")),
        Ok(other) => Err(format!("unexpected answer to the model list: {other:?}")),
        Err(e) => Err(format!("{e:#}")),
    }
}

async fn conformance(State(k): K, h: HeaderMap, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let running = match k.adapters.get(&id).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    };
    let model = match conformance_model(&k, &id, &b["model"]).await {
        Ok(m) => m,
        Err(m) => return err(StatusCode::BAD_GATEWAY, m),
    };
    let report = k.adapters.conformance(&running, &model).await;
    let rv = serde_json::to_value(&report).unwrap_or_default();
    let ev = Ev::AdapterVerified { adapter_id: id, adapter: report.adapter.clone(), version: report.version.clone(), model, passed: report.passed, report: rv.clone() };
    if let Err(e) = k.emit(&format!("person:{}", p.id), ev).await {
        return internal(e);
    }
    Json(rv).into_response()
}

async fn catalog(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = owner(&k, &h) {
        return r;
    }
    match k.adapters.catalog(&id).await {
        Ok(ev) => Json(serde_json::to_value(ev).unwrap_or_default()).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

async fn sign_in_begin(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = owner(&k, &h) {
        return r;
    }
    match k.adapters.auth_begin(&id).await {
        Ok(ev) => Json(serde_json::to_value(ev).unwrap_or_default()).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

async fn sign_in_complete(State(k): K, h: HeaderMap, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match k.adapters.auth_complete(&id, b["flow"].clone(), b["input"].clone()).await {
        Ok(Ok(())) => {
            let name = k.cfg.adapters.iter().find(|a| a.id == id).and_then(|a| a.settings.get("credential")).and_then(|v| v.as_str()).unwrap_or(&id).to_string();
            let _ = k.emit(&format!("person:{}", p.id), Ev::SecretSet { name }).await;
            Json(json!({ "ok": true })).into_response()
        }
        Ok(Err(m)) => err(StatusCode::BAD_REQUEST, m),
        Err(e) => err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

async fn memory(State(k): K, h: HeaderMap) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let items: Vec<Value> = k.read(|s| {
        s.memory
            .iter()
            .filter(|m| m.person == p.id)
            .map(|m| json!({ "id": m.id, "fact": m.fact, "topic": m.topic, "label": m.label, "run": m.run, "at": m.at }))
            .collect()
    });
    Json(json!({ "memory": items })).into_response()
}

/// Forget one of the person's own memories.
async fn forget(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !k.read(|s| s.memory.iter().any(|m| m.id == id && m.person == p.id)) {
        return err(StatusCode::NOT_FOUND, "no such memory");
    }
    match k.emit(&format!("person:{}", p.id), Ev::MemoryForgotten { id }).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e),
    }
}

async fn health(State(k): K) -> Response {
    Json(json!({ "ok": true, "version": HARNESS_VERSION, "kill_switch": k.read(|s| s.kill_switch) })).into_response()
}

/// Model text for a task as it streams (server-sent events), before the
/// turn is appended to the log (K6).
async fn live(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some((_, run)) = find(&k, &p, &id) else { return err(StatusCode::NOT_FOUND, "no such task") };
    let mut rx = k.live_text.subscribe();
    let stream = async_stream(move |tx| async move {
        loop {
            match rx.recv().await {
                Ok((r, text)) if r == run.id => {
                    if tx.send(format!("data: {}\n\n", json!({ "text": text }))).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
        }
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

fn async_stream<F, Fut>(f: F) -> impl futures::Stream<Item = Result<String, std::convert::Infallible>>
where
    F: FnOnce(tokio::sync::mpsc::Sender<String>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(f(tx));
    futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|s| (Ok(s), rx)) })
}

/// Speech to text on the appliance. The person reviews the text before sending it.
async fn transcribe(State(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    if let Err(r) = who(&k, &h) {
        return r;
    }
    let Some(data) = b["data_b64"].as_str() else { return err(StatusCode::BAD_REQUEST, "no audio") };
    match crate::speech::transcribe(k.cfg.speech.as_ref(), &k.cfg.appliance.state_dir, data).await {
        Ok(text) => Json(json!({ "text": text })).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")),
    }
}

async fn get_model(State(k): K, h: HeaderMap) -> Response {
    if let Err(r) = who(&k, &h) {
        return r;
    }
    let (adapter, model) = k.read(|s| s.model.clone()).unwrap_or_else(|| (k.cfg.model.adapter.clone(), k.cfg.model.name.clone()));
    Json(json!({ "adapter": adapter, "model": model })).into_response()
}

/// The models anyone signed in may choose for a task or a chat message,
/// and the one used when they choose none.
async fn models(State(k): K, h: HeaderMap) -> Response {
    if let Err(r) = who(&k, &h) {
        return r;
    }
    let (adapter, model) = k.default_model();
    let list: Vec<Value> = k.models().await.into_iter().map(|(id, models)| json!({ "adapter": id, "models": models })).collect();
    Json(json!({ "default": { "adapter": adapter, "model": model }, "adapters": list })).into_response()
}

/// Choose the model new tasks run on. Running tasks keep their pinned model (K9).
async fn put_model(State(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let (Some(adapter), Some(model)) = (b["adapter"].as_str(), b["model"].as_str()) else {
        return err(StatusCode::BAD_REQUEST, "adapter and model are required");
    };
    if k.adapters.list().await.iter().all(|(id, _, _)| id != adapter) {
        return err(StatusCode::NOT_FOUND, "no such adapter");
    }
    match k.emit(&format!("person:{}", p.id), Ev::ModelSet { adapter: adapter.into(), model: model.into() }).await {
        Ok(_) => Json(json!({ "adapter": adapter, "model": model })).into_response(),
        Err(e) => internal(e),
    }
}

async fn schedules(State(k): K, h: HeaderMap) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let list: Vec<Value> = k.read(|s| {
        s.schedules
            .values()
            .filter(|x| x.person == p.id || p.is_owner())
            .map(|x| {
                // Its runs, newest first; the last result is the newest that ended.
                let mut runs: Vec<(&crate::state::Task, &Run)> =
                    s.tasks.values().filter(|t| t.schedule.as_deref() == Some(x.id.as_str())).filter_map(|t| s.runs.get(&t.run_id).map(|r| (t, r))).collect();
                runs.sort_by(|a, b| b.0.created_at.cmp(&a.0.created_at));
                let rows: Vec<Value> = runs.iter().map(|(t, r)| task_row(&k, s, &p, t, r)).collect();
                let last = rows.iter().find(|r| !r["ended_at"].is_null()).cloned();
                json!({
                    "id": x.id, "person": x.person, "text": x.text, "title": x.title.clone().unwrap_or_else(|| x.text.clone()),
                    "every_minutes": x.every_minutes, "daily_at": x.daily_at, "next": x.next_due(), "last_fired": x.last_fired,
                    "last": last, "runs": rows,
                    "project": s.project_of(&x.id).and_then(|id| s.projects.get(&id)).map(|p| json!({ "id": p.id, "name": p.name })),
                })
            })
            .collect()
    });
    Json(json!({ "schedules": list })).into_response()
}

fn schedule_rule(b: &Value) -> Result<(Option<u32>, Option<String>), String> {
    let every = b["every_minutes"].as_u64().map(|m| m as u32);
    let daily = b["daily_at"].as_str().map(String::from);
    match (every, &daily) {
        (Some(m), None) if m >= 5 => Ok((Some(m), None)),
        (Some(_), None) => Err("schedules run at most every 5 minutes".into()),
        (None, Some(t)) if t.split_once(':').is_some_and(|(h, m)| h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60)) => Ok((None, daily)),
        _ => Err("give either every_minutes or daily_at as HH:MM".into()),
    }
}

async fn add_schedule(State(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let text = b["text"].as_str().unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return err(StatusCode::BAD_REQUEST, "the schedule has no task text");
    }
    let (every_minutes, daily_at) = match schedule_rule(&b) {
        Ok(r) => r,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    let project = match crate::library::chosen_project(&k, &p, &b) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let id = new_id("sched");
    let actor = format!("person:{}", p.id);
    if let Err(e) = k.emit(&actor, Ev::ScheduleSet { id: id.clone(), person: p.id.clone(), text, every_minutes, daily_at, budget: None, card: None, label: None, title: None }).await {
        return internal(e);
    }
    if project.is_some() {
        if let Err(e) = k.emit(&actor, Ev::Moved { item: id.clone(), project }).await {
            return internal(e);
        }
    }
    (StatusCode::CREATED, Json(json!({ "id": id }))).into_response()
}

async fn remove_schedule(State(k): K, h: HeaderMap, Path(id): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(owner_id) = k.read(|s| s.schedules.get(&id).map(|x| x.person.clone())) else { return err(StatusCode::NOT_FOUND, "no such schedule") };
    if owner_id != p.id && !p.is_owner() {
        return err(StatusCode::NOT_FOUND, "no such schedule");
    }
    match k.emit(&format!("person:{}", p.id), Ev::ScheduleRemoved { id }).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e),
    }
}

pub(crate) fn rule_for(b: &Value) -> Result<(Option<u32>, Option<String>), String> {
    schedule_rule(b)
}

/// Download a delivered file. Read from the log, the source of truth.
async fn output(State(k): K, h: HeaderMap, Path((id, out)): Path<(String, String)>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some((_, run)) = find(&k, &p, &id) else { return err(StatusCode::NOT_FOUND, "no such task") };
    if !run.outputs.iter().any(|(o, ..)| o == &out) {
        return err(StatusCode::NOT_FOUND, "no such file");
    }
    match k.output_bytes(&run.id, &out).await {
        Ok(Some((name, mt, bytes))) => Response::builder()
            .header("content-type", mt)
            .header("content-disposition", format!("attachment; filename=\"{}\"", name.replace('"', "")))
            .body(axum::body::Body::from(bytes))
            .unwrap(),
        Ok(None) => err(StatusCode::NOT_FOUND, "no such file"),
        Err(e) => internal(e),
    }
}
