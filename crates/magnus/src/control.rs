//! The kernel's control socket (admin console spec, Server and API): how
//! the admin console reaches the kernel for what only the kernel may do,
//! a local Unix socket in the appliance's state, readable and writable by
//! the appliance's own user alone. Nothing reaches it over the network.
//!
//! Its first use is the audit log: every console action is written to the
//! event log by the kernel, its one writer, before the console answers.

use crate::events::Ev;
use crate::kernel::Kernel;
use anyhow::{Context, Result};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn socket_path(state_dir: &Path) -> PathBuf {
    state_dir.join("control.sock")
}

/// Serve the control socket until the kernel stops.
pub async fn serve(k: Arc<Kernel>, state_dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = socket_path(state_dir);
    let _ = std::fs::remove_file(&path);
    let l = tokio::net::UnixListener::bind(&path).with_context(|| format!("listen on {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!("magnus control socket at {}", path.display());
    let app = Router::new().route("/v1/audit", post(record).get(list)).route("/v1/kill-switch", post(switch).get(switch_state)).route("/v1/overview", axum::routing::get(overview))
        .route("/v1/people", axum::routing::get(people))
        .route("/v1/people/{person}/sessions", axum::routing::get(sessions).delete(end_sessions))
        .route("/v1/people/{person}/sessions/{id}", axum::routing::delete(end_session))
        .route("/v1/people/{person}/turn-off", post(turn_off))
        .route("/v1/people/{person}/schedules", axum::routing::get(person_schedules))
        .route("/v1/people/{person}/schedules/{id}/{change}", post(person_schedule))
        .route("/v1/people/{person}/data/{what}", post(person_data))
        .route("/v1/people/{person}/erase", post(erase_person))
        .route("/v1/admin/policy", axum::routing::get(policy_sets))
        .route("/v1/admin/policy/draft", post(policy_draft))
        .route("/v1/admin/policy/apply", post(policy_apply))
        .route("/v1/admin/policy/test", post(policy_test))
        .route("/v1/admin/policy/history", axum::routing::get(policy_history))
        .route("/v1/admin/console-settings", axum::routing::get(console_settings).put(set_console_settings))
        .route("/v1/admin/modules", axum::routing::get(modules))
        .route("/v1/admin/modules/{name}", post(set_module))
        .route("/v1/admin/providers", axum::routing::get(providers))
        .route("/v1/admin/providers/{id}", axum::routing::patch(set_provider))
        .route("/v1/admin/providers/{id}/test", post(test_provider))
        .route("/v1/admin/updates", axum::routing::get(updates))
        .route("/v1/admin/updates/install", post(install_update))
        .route("/v1/admin/updates/rollback", post(roll_back_update))
        .route("/v1/admin/updates/desktop-channel", axum::routing::put(desktop_channel))
        .route("/v1/admin/connectors", axum::routing::get(connectors).post(add_connector))
        .route("/v1/admin/connectors/{name}", axum::routing::patch(connector_settings).delete(remove_connector))
        .route("/v1/admin/connectors/{name}/credential", axum::routing::put(set_credential).delete(revoke_credential))
        .with_state(k);
    axum::serve(l, app).await?;
    Ok(())
}

fn text(b: &Value, f: &str) -> Option<String> {
    b[f].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// Record a console action; its seq once it is in the log.
async fn record(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let (Some(admin), Some(action)) = (text(&b, "admin"), text(&b, "action")) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "an audit entry names its admin and action" }))).into_response();
    };
    let ev = Ev::AdminAction {
        admin: admin.clone(),
        name: text(&b, "name").unwrap_or_default(),
        action,
        target: text(&b, "target"),
        source: text(&b, "source"),
        before: b.get("before").filter(|v| !v.is_null()).cloned(),
        after: b.get("after").filter(|v| !v.is_null()).cloned(),
    };
    match k.emit(&format!("admin:{admin}"), ev).await {
        Ok(e) => Json(json!({ "seq": e.seq })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e:#}") }))).into_response(),
    }
}

/// The audit, newest first, filtered by actor, action, target and time.
async fn list(State(k): State<Arc<Kernel>>, Query(q): Query<HashMap<String, String>>) -> Response {
    let since = q.get("since").and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok());
    let until = q.get("until").and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok());
    let has = |v: &Value, f: &str, want: Option<&String>| want.is_none_or(|w| v[f].as_str().is_some_and(|x| x.to_lowercase().contains(&w.to_lowercase())));
    let rows: Vec<Value> = k.read(|s| {
        s.audit
            .iter()
            .rev()
            .filter(|(_, at, _)| since.is_none_or(|t| *at >= t) && until.is_none_or(|t| *at <= t))
            .filter(|(_, _, v)| (has(v, "admin", q.get("actor")) || has(v, "name", q.get("actor"))) && has(v, "action", q.get("action")) && has(v, "target", q.get("target")))
            .take(1000)
            .map(|(seq, at, v)| {
                let mut r = v.clone();
                r["seq"] = json!(seq);
                r["at"] = json!(at);
                r
            })
            .collect()
    });
    Json(json!({ "entries": rows })).into_response()
}

/// The kill switch as the console shows it: on or off, who moved it and
/// when, and what it holds stopped (the runs it suspended, the schedules
/// that do not fire while it is on).
async fn switch_state(State(k): State<Arc<Kernel>>) -> Response {
    let stopped: Vec<String> = k.read(|s| s.runs.values().filter(|r| r.status == crate::state::Status::Suspended && r.reason.as_deref() == Some("kill_switch")).map(|r| r.id.clone()).collect());
    let frozen = stopped.iter().filter(|r| k.sandboxes.kept(r)).count();
    let running = k.sandboxes.running().await;
    Json(k.read(|s| {
        let halted = s.runs.values().filter(|r| r.status == crate::state::Status::Suspended && r.reason.as_deref() == Some("kill_switch")).count();
        let held = if s.kill_switch { s.schedules.values().filter(|x| x.next_due().is_some()).count() } else { 0 };
        json!({
            "engaged": s.kill_switch,
            "by": s.kill_switch_by.as_ref().map(|(who, _)| who), "at": s.kill_switch_by.as_ref().map(|(_, at)| at),
            "tasks_halted": halted, "schedules_held": held,
            "sign_ins_revoked": s.revoked.len(),
            "microvms_frozen": frozen, "microvms_running": running,
        })
    }))
    .into_response()
}

/// Engage or release it, as `actor`: the one switch, the app's own.
async fn switch(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let Some(actor) = text(&b, "actor") else { return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name who moves the switch" }))).into_response() };
    match k.kill_switch(&actor, b["engaged"].as_bool().unwrap_or(true)).await {
        Ok(v) => Json(v).into_response(),
        Err((st, v)) => (st, Json(v)).into_response(),
    }
}

/// What the console's Overview and Health show of the kernel: its version,
/// whether the log answers, the kill switch, tasks running, people signed
/// in, what agents changed in the organisation's knowledge and memory in
/// the last day, and the runs that failed most recently.
async fn overview(State(k): State<Arc<Kernel>>) -> Response {
    use crate::state::Status;
    let log_up = k.log.up().await;
    let signed_in = k.accounts.signed_in().len();
    let vms = k.sandboxes.running().await;
    let day = chrono::Utc::now() - chrono::Duration::hours(24);
    let v = k.read(|s| {
        let mut failed: Vec<&crate::state::Run> = s.runs.values().filter(|r| r.status == Status::Failed).collect();
        failed.sort_by(|a, b| b.ended_at.cmp(&a.ended_at));
        let knowledge = s.knowledge.values().filter(|e| e.at > day && !e.author.starts_with("person:")).count();
        let memory = s.memory.iter().filter(|m| m.at > day).count();
        json!({
            "version": crate::update::version(), "log_up": log_up, "kill_switch": s.kill_switch,
            "running_tasks": s.runs.values().filter(|r| r.status == Status::Running).count(),
            "waiting_tasks": s.runs.values().filter(|r| r.status == Status::Suspended).count(),
            "signed_in": signed_in, "microvms_running": vms,
            "knowledge_changes_24h": knowledge + memory,
            "recent_errors": failed.iter().take(10).map(|r| json!({ "run": r.id, "task": r.task, "person": r.person, "reason": r.reason, "at": r.ended_at })).collect::<Vec<_>>(),
        })
    });
    Json(v).into_response()
}

/// The person here whom the identity provider knows as `sub` (owner's
/// ruling 3: people are bound to their provider's subject at their first
/// sign-in through it). The console names people by their `sub`.
fn person_of(k: &Kernel, sub: &str) -> Option<String> {
    k.read(|s| s.subjects.iter().find(|(_, v)| v.as_str() == sub).map(|(p, _)| p.clone()))
}

/// Each person known here, by their provider `sub`: when they last signed
/// in, and how many sessions they hold open (the console's Users list; the
/// people themselves are the identity provider's).
async fn people(State(k): State<Arc<Kernel>>) -> Response {
    let bound: Vec<(String, String, Option<chrono::DateTime<chrono::Utc>>)> = k.read(|s| s.subjects.iter().map(|(p, sub)| (p.clone(), sub.clone(), s.last_sign_in.get(p).copied())).collect());
    let out: serde_json::Map<String, Value> = bound.into_iter().map(|(p, sub, at)| (sub, json!({ "last_sign_in": at, "sessions": k.accounts.sessions_of(&p, "").len() }))).collect();
    Json(json!({ "people": out })).into_response()
}

/// A person's open sessions here, newest first: each a device signed in.
async fn sessions(State(k): State<Arc<Kernel>>, axum::extract::Path(sub): axum::extract::Path<String>) -> Response {
    let list: Vec<Value> = person_of(&k, &sub).map(|p| k.accounts.sessions_of(&p, "")).unwrap_or_default().into_iter().map(|(s, _)| json!({ "id": s.id, "client": s.client, "created": s.created, "expires": s.expires })).collect();
    Json(json!({ "sessions": list })).into_response()
}

fn actor(b: &Value) -> Option<String> {
    text(b, "actor")
}

async fn signed_out(k: &Kernel, actor: &str, person: &str, ended: Vec<String>) -> Response {
    let n = ended.len();
    let evs: Vec<(&str, Ev)> = ended.into_iter().map(|session| (actor, Ev::SignedOut { person: person.to_string(), session })).collect();
    if !evs.is_empty() {
        if let Err(e) = k.emit_all(evs).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e:#}") }))).into_response();
        }
    }
    Json(json!({ "ended": n })).into_response()
}

/// End every session a person holds here, as `actor`.
async fn end_sessions(State(k): State<Arc<Kernel>>, axum::extract::Path(sub): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name who ends them" }))).into_response() };
    let Some(person) = person_of(&k, &sub) else { return Json(json!({ "ended": 0 })).into_response() };
    match k.accounts.end_all(&person) {
        Ok(ended) => signed_out(&k, &actor, &person, ended).await,
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e:#}") }))).into_response(),
    }
}

/// End one of them.
async fn end_session(State(k): State<Arc<Kernel>>, axum::extract::Path((sub, id)): axum::extract::Path<(String, String)>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name who ends it" }))).into_response() };
    let gone = match person_of(&k, &sub) {
        Some(person) => k.accounts.end(&person, &id).map(|s| s.map(|s| (person, s))),
        None => Ok(None),
    };
    match gone {
        Ok(Some((person, s))) => signed_out(&k, &actor, &person, vec![s.id]).await,
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "no such session" }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e:#}") }))).into_response(),
    }
}

/// An admin turned the person off: as when the provider does, their
/// sessions end and their schedules pause (admin console spec, Users:
/// "Disabling also suspends their schedules and triggers").
async fn turn_off(State(k): State<Arc<Kernel>>, axum::extract::Path(sub): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name who turns them off" }))).into_response() };
    let Some(person) = person_of(&k, &sub) else { return Json(json!({ "sessions_ended": 0, "schedules_paused": 0 })).into_response() };
    let due = k.read(|s| s.schedules.values().filter(|x| x.person == person && x.next_due().is_some()).count());
    let ended = k.turn_off(&person, &format!("{actor} turned {person} off in the admin console"), "An administrator has turned your account off").await;
    Json(json!({ "sessions_ended": ended, "schedules_paused": due })).into_response()
}

// The organisation's apps and their credentials (admin console spec,
// Security › Connectors and credentials), as `actor`: the kernel's own
// operations, the same the app's owner routes use. Credentials go in and
// never come out.

fn done(r: Result<Value, crate::connectors::Refusal>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err((st, m)) => (st, Json(json!({ "error": m }))).into_response(),
    }
}

fn no_actor() -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": "name who does this" }))).into_response()
}

async fn connectors(State(k): State<Arc<Kernel>>) -> Response {
    Json(json!({ "connectors": k.admin_connectors() })).into_response()
}

async fn add_connector(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return no_actor() };
    done(k.add_connector(&actor, &b).await)
}

async fn connector_settings(State(k): State<Arc<Kernel>>, axum::extract::Path(name): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return no_actor() };
    let groups = b.get("access_groups").and_then(|g| g.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect());
    done(k.connector_settings(&actor, &name, b["enabled"].as_bool(), groups).await)
}

async fn remove_connector(State(k): State<Arc<Kernel>>, axum::extract::Path(name): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return no_actor() };
    done(k.remove_connector(&actor, &name).await)
}

async fn set_credential(State(k): State<Arc<Kernel>>, axum::extract::Path(name): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return no_actor() };
    done(k.set_connector_credential(&actor, &name, b["value"].as_str().unwrap_or_default()).await)
}

async fn revoke_credential(State(k): State<Arc<Kernel>>, axum::extract::Path(name): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return no_actor() };
    done(k.revoke_connector_credential(&actor, &name).await)
}

// One person, as root acts on them (admin console spec, Users): their
// schedules suspended and resumed, their data read, their data deleted. The
// console names people by their provider `sub`; one never seen here has
// nothing to show and nothing to delete.

fn person_refused((st, m): crate::person_admin::Refusal) -> Response {
    (st, Json(json!({ "error": m }))).into_response()
}

async fn person_schedules(State(k): State<Arc<Kernel>>, axum::extract::Path(sub): axum::extract::Path<String>) -> Response {
    let Some(person) = person_of(&k, &sub) else { return Json(json!({ "schedules": [] })).into_response() };
    match k.person_schedules(&person).await {
        Ok(v) => Json(json!({ "schedules": v })).into_response(),
        Err(r) => person_refused(r),
    }
}

async fn person_schedule(State(k): State<Arc<Kernel>>, axum::extract::Path((sub, id, change)): axum::extract::Path<(String, String, String)>, Json(b): Json<Value>) -> Response {
    let (Some(actor), Some(person)) = (actor(&b), person_of(&k, &sub)) else { return person_refused((StatusCode::NOT_FOUND, "no such schedule".into())) };
    let on = match change.as_str() {
        "resume" => true,
        "suspend" => false,
        _ => return person_refused((StatusCode::NOT_FOUND, "suspend or resume".into())),
    };
    let by = text(&b, "name").unwrap_or_else(|| "an admin".into());
    match k.admin_schedule(&actor, &by, &person, &id, on).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => person_refused(r),
    }
}

/// A read: a POST, since it is recorded and the person is told.
async fn person_data(State(k): State<Arc<Kernel>>, axum::extract::Path((sub, what)): axum::extract::Path<(String, String)>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return person_refused((StatusCode::BAD_REQUEST, "name who reads it".into())) };
    let Some(target) = person_of(&k, &sub).and_then(|p| k.person(&p)) else {
        return person_refused((StatusCode::NOT_FOUND, "they have never signed in to Magnus: there is no data of theirs here".into()));
    };
    let by = text(&b, "name").unwrap_or_else(|| "An admin".into());
    match k.read_person_data(&actor, &by, &target, &what, b["item"].as_str()).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => person_refused(r),
    }
}

async fn erase_person(State(k): State<Arc<Kernel>>, axum::extract::Path(sub): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return person_refused((StatusCode::BAD_REQUEST, "name who deletes it".into())) };
    let Some(person) = person_of(&k, &sub) else { return Json(json!({ "never_here": true })).into_response() };
    match k.erase_person(&actor, &person).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => person_refused(r),
    }
}

// The agent policy (admin console spec, Security › Agent policy), as the
// kernel keeps it: sets, drafts, versions, tests.

fn policy_refused((st, m): crate::policy_admin::Refusal) -> Response {
    (st, Json(json!({ "error": m }))).into_response()
}

/// The sets, and what a test can name: the appliance's people and the
/// organisation's tools.
async fn policy_sets(State(k): State<Arc<Kernel>>) -> Response {
    let people: Vec<Value> = k.cfg.people.iter().filter_map(|p| k.person(&p.id)).map(|p| json!({ "id": p.id, "name": p.name, "groups": p.groups })).collect();
    let tools: Vec<Value> = k.connectors().into_iter().flat_map(|c| c.tools.iter().map(|(t, cl)| json!({ "name": format!("{}__{t}", c.name), "risk": cl.risk, "audience": c.audience })).collect::<Vec<_>>()).collect();
    Json(json!({ "sets": k.policy_sets(), "people": people, "tools": tools })).into_response()
}

async fn policy_draft(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let (Some(actor), Some(layer)) = (actor(&b), text(&b, "layer")) else { return policy_refused((StatusCode::BAD_REQUEST, "name who drafts which layer".into())) };
    match k.policy_draft(&actor, &layer, b["cedar"].as_str().map(String::from)).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => policy_refused(r),
    }
}

async fn policy_apply(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let (Some(actor), Some(layer)) = (actor(&b), text(&b, "layer")) else { return policy_refused((StatusCode::BAD_REQUEST, "name who applies which layer".into())) };
    match k.policy_apply(&actor, &layer, b["cedar"].as_str().unwrap_or_default()).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => policy_refused(r),
    }
}

async fn policy_test(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    match k.policy_test(&b) {
        Ok(v) => Json(v).into_response(),
        Err(r) => policy_refused(r),
    }
}

async fn policy_history(State(k): State<Arc<Kernel>>, Query(q): Query<HashMap<String, String>>) -> Response {
    Json(json!({ "versions": k.policy_history(q.get("layer").map(String::as_str).unwrap_or("org")) })).into_response()
}

/// The console's own session lifetimes, as an admin last set them.
async fn console_settings(State(k): State<Arc<Kernel>>) -> Response {
    Json(k.read(|s| s.console_settings.clone()).unwrap_or(json!({}))).into_response()
}

async fn set_console_settings(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return policy_refused((StatusCode::BAD_REQUEST, "name who sets them".into())) };
    let now = k.read(|s| s.console_settings.clone()).unwrap_or(json!({}));
    let idle = b["idle_minutes"].as_i64().or(now["idle_minutes"].as_i64()).unwrap_or(30);
    let hours = b["session_hours"].as_i64().or(now["session_hours"].as_i64()).unwrap_or(12);
    let allow: Option<Vec<String>> = b.get("allow").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).or_else(|| serde_json::from_value(now["allow"].clone()).ok());
    match k.emit(&actor, Ev::ConsoleSettings { idle_minutes: idle, session_hours: hours, allow: allow.clone() }).await {
        Ok(_) => Json(json!({ "idle_minutes": idle, "session_hours": hours, "allow": allow })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e:#}") }))).into_response(),
    }
}

// Operations (admin console spec): the run modules, the model providers and
// the updates, as the kernel's own operations.

fn ops_refused((st, m): crate::ops_admin::Refusal) -> Response {
    (st, Json(json!({ "error": m }))).into_response()
}

fn ops(r: Result<Value, crate::ops_admin::Refusal>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => ops_refused(e),
    }
}

async fn modules(State(k): State<Arc<Kernel>>) -> Response {
    Json(json!({ "modules": k.admin_modules() })).into_response()
}

async fn set_module(State(k): State<Arc<Kernel>>, axum::extract::Path(name): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return ops_refused((StatusCode::BAD_REQUEST, "name who sets it".into())) };
    ops(k.set_module(&actor, &name, b["on"].as_bool()).await)
}

async fn providers(State(k): State<Arc<Kernel>>) -> Response {
    Json(json!({ "providers": k.admin_providers().await })).into_response()
}

async fn test_provider(State(k): State<Arc<Kernel>>, axum::extract::Path(id): axum::extract::Path<String>) -> Response {
    ops(k.test_provider(&id).await)
}

async fn set_provider(State(k): State<Arc<Kernel>>, axum::extract::Path(id): axum::extract::Path<String>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return ops_refused((StatusCode::BAD_REQUEST, "name who sets it".into())) };
    ops(k.set_provider(&actor, &id, b["enabled"].as_bool(), b["default_model"].as_str()).await)
}

async fn updates(State(k): State<Arc<Kernel>>) -> Response {
    Json(k.admin_updates()).into_response()
}

/// Install the latest release now; the switch happens after the answer.
async fn install_update(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return ops_refused((StatusCode::BAD_REQUEST, "name who installs it".into())) };
    let current = crate::update::version();
    let to = k.latest_release().and_then(|m| m["version"].as_str().map(String::from));
    match to {
        Some(to) if crate::update::newer(&to, &current) && !k.read(|s| s.skipped_releases.contains(&to)) => {
            tokio::spawn(async move {
                if let Err(e) = k.update(&actor).await {
                    tracing::error!("update: {e:#}");
                }
            });
            (StatusCode::ACCEPTED, Json(json!({ "status": "installing", "version": current, "latest": to }))).into_response()
        }
        to => Json(json!({ "status": "up_to_date", "version": current, "latest": to })).into_response(),
    }
}

/// Go back to the release kept; the switch happens after the answer.
async fn roll_back_update(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return ops_refused((StatusCode::BAD_REQUEST, "name who rolls it back".into())) };
    let Some(to) = crate::update::previous_version() else { return ops_refused((StatusCode::CONFLICT, "no earlier release is kept on this appliance".into())) };
    tokio::spawn(async move {
        if let Err(e) = k.roll_back(&actor).await {
            tracing::error!("roll back: {e:#}");
        }
    });
    (StatusCode::ACCEPTED, Json(json!({ "status": "rolling_back", "version": crate::update::version(), "to": to }))).into_response()
}

async fn desktop_channel(State(k): State<Arc<Kernel>>, Json(b): Json<Value>) -> Response {
    let Some(actor) = actor(&b) else { return ops_refused((StatusCode::BAD_REQUEST, "name who sets it".into())) };
    ops(k.set_desktop_channel(&actor, b["channel"].as_str().unwrap_or_default()).await)
}

