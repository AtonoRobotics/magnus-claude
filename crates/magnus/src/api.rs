//! The local HTTP API, for the desktop app. Every request names a person by
//! token. Approvals, answers and policy arrive only here, a channel the agent
//! cannot reach (its sandbox has no network and no tool calls this API).

use crate::config::{Adapter, Person};
use crate::events::Ev;
use crate::inbox::{self, Answer};
use crate::kernel::{new_id, Kernel, HARNESS_VERSION};
use crate::state::{Run, Status};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

type K = State<Arc<Kernel>>;

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

fn internal(e: anyhow::Error) -> Response {
    tracing::error!("api: {e:#}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

fn who(k: &Kernel, h: &HeaderMap) -> Result<Person, Response> {
    let token = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "sign in"))?;
    k.cfg.person_by_token(token).cloned().ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unknown token"))
}

fn owner(k: &Kernel, h: &HeaderMap) -> Result<Person, Response> {
    let p = who(k, h)?;
    if !p.is_owner() {
        return Err(err(StatusCode::FORBIDDEN, "this needs the owner role"));
    }
    Ok(p)
}

pub fn router(k: Arc<Kernel>) -> Router {
    Router::new()
        .route("/", get(|| async { axum::response::Html(include_str!("../ui/index.html")) }))
        .route("/v1/health", get(health))
        .route("/v1/tasks/{id}/live", get(live))
        .route("/v1/transcribe", post(transcribe))
        .route("/v1/me", get(me))
        .route("/v1/tasks", post(submit).get(list_tasks))
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
        .route("/v1/adapters/{id}", put(put_adapter))
        .route("/v1/adapters/{id}/catalog", get(catalog))
        .route("/v1/adapters/{id}/conformance", post(conformance))
        .route("/v1/adapters/{id}/sign-in", post(sign_in_begin))
        .route("/v1/adapters/{id}/sign-in/complete", post(sign_in_complete))
        .route("/v1/memory", get(memory))
        .route("/v1/schedules", get(schedules).post(add_schedule))
        .route("/v1/schedules/{id}", axum::routing::delete(remove_schedule))
        .with_state(k)
}

async fn me(State(k): K, h: HeaderMap) -> Response {
    match who(&k, &h) {
        Ok(p) => Json(json!({ "id": p.id, "name": p.name, "groups": p.groups, "roles": p.roles })).into_response(),
        Err(r) => r,
    }
}

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Running => "running",
        Status::Suspended => "suspended",
        Status::Completed => "completed",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
    }
}

fn task_json(k: &Kernel, task: &crate::state::Task, run: &Run, full: bool) -> Value {
    let mut v = json!({
        "task_id": task.task_id,
        "run_id": run.id,
        "person": task.person,
        "text": task.text,
        "title": task.text.chars().take(80).collect::<String>(),
        "created_at": task.created_at,
        "status": status_str(run.status),
        "waiting_on": run.reason,
        "summary": run.summary,
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
                    "verified": ver.map(|v| v.0),
                    "evidence": ver.map(|v| v.1.clone()),
                    "verified_by": ver.map(|v| v.2.clone()),
                })
            })
            .collect();
        v["contract"] = run.contract.clone().unwrap_or(Value::Null);
        v["contract_version"] = json!(run.contract_version);
        v["criteria"] = json!(criteria);
        v["model"] = json!({ "adapter": run.config.adapter, "name": run.config.model });
        v["usage"] = json!({ "model_calls": run.model_calls, "actions": run.actions, "cost_usd": run.usd });
        v["taint"] = json!({ "untrusted": run.untrusted, "private": run.private });
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
    let attachments = b["attachments"].as_array().cloned().unwrap_or_default();
    for a in &attachments {
        let size = a["data_b64"].as_str().map(|d| d.len() * 3 / 4).unwrap_or(0);
        if a["name"].as_str().is_none() || a["media_type"].as_str().is_none() || size == 0 {
            return err(StatusCode::BAD_REQUEST, "each attachment needs name, media_type and data_b64");
        }
        if size > 25 * 1024 * 1024 {
            return err(StatusCode::PAYLOAD_TOO_LARGE, "attachments are limited to 25 MB each");
        }
    }
    match k.create_task(&format!("person:{}", p.id), &p.id, text, attachments).await {
        Ok((task, run)) => (StatusCode::CREATED, Json(json!({ "task_id": task, "run_id": run }))).into_response(),
        Err(e) => internal(e),
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
            .filter(|t| visible_task(&p, t))
            .filter_map(|t| s.runs.get(&t.run_id).map(|r| task_json(&k, t, r, false)))
            .collect()
    });
    Json(json!({ "tasks": tasks })).into_response()
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
                    calls.push(json!({
                        "tool": name, "input": input,
                        "result": r.map(|r| r.text.chars().take(4000).collect::<String>()),
                        "is_error": r.map(|r| r.is_error), "status": r.map(|r| r.status.clone()),
                    }));
                }
                _ => {}
            }
        }
        steps.push(json!({ "step": reply.step, "text": said, "calls": calls }));
    }
    Json(json!({ "steps": steps, "verifications": run.verified.iter().map(|(c, v)| json!({ "criterion": c, "passed": v.0, "evidence": v.1, "by": v.2 })).collect::<Vec<_>>() })).into_response()
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
    let items: Vec<Value> = k.read(|s| s.items.values().filter(|i| inbox::visible(&p, i)).map(|i| inbox::render(&p, i)).collect());
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
        let model = b["model"].as_str().map(String::from).unwrap_or_else(|| k.cfg.model.name.clone());
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

async fn conformance(State(k): K, h: HeaderMap, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let running = match k.adapters.get(&id).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    };
    let model = b["model"].as_str().map(String::from).unwrap_or_else(|| k.cfg.model.name.clone());
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
    match crate::speech::transcribe(&k.cfg.appliance.state_dir, data).await {
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
            .map(|x| json!({ "id": x.id, "person": x.person, "text": x.text, "every_minutes": x.every_minutes, "daily_at": x.daily_at, "next": x.next_due() }))
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
    let id = new_id("sched");
    match k.emit(&format!("person:{}", p.id), Ev::ScheduleSet { id: id.clone(), person: p.id.clone(), text, every_minutes, daily_at }).await {
        Ok(_) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Err(e) => internal(e),
    }
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
