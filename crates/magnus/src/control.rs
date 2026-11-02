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
        .route("/v1/people/{person}/turn-off", post(turn_off)).with_state(k);
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
    Json(k.read(|s| {
        let halted = s.runs.values().filter(|r| r.status == crate::state::Status::Suspended && r.reason.as_deref() == Some("kill_switch")).count();
        let held = if s.kill_switch { s.schedules.values().filter(|x| x.next_due().is_some()).count() } else { 0 };
        json!({
            "engaged": s.kill_switch,
            "by": s.kill_switch_by.as_ref().map(|(who, _)| who), "at": s.kill_switch_by.as_ref().map(|(_, at)| at),
            "tasks_halted": halted, "schedules_held": held,
            "sign_ins_revoked": s.revoked.len(),
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
            "signed_in": signed_in,
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
