//! Overview, Health and the kill switch in the admin console (admin console
//! spec, Overview, Operations; owner's ruling 10: the console's switch is
//! the app's own, through the kernel). Engaging never needs a step-up;
//! releasing does. Each change is in the audit log.

use super::session::Admin;
use super::Console;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

fn kernel_down(e: anyhow::Error) -> Response {
    (StatusCode::BAD_GATEWAY, Json(json!({ "error": "kernel", "message": format!("{e:#}") }))).into_response()
}

pub async fn switch_state(State(c): State<Arc<Console>>) -> Response {
    match c.kernel.call(Method::GET, "/v1/kill-switch", None).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => kernel_down(e),
    }
}

/// Stop all agents: at once, with no step-up. It is recorded by the switch
/// itself even when nothing else can be; the audit entry follows.
pub async fn engage(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>) -> Response {
    let actor = format!("admin:{}", a.session.sub);
    let r = c.kernel.call(Method::POST, "/v1/kill-switch", Some(&json!({ "actor": actor, "engaged": true }))).await;
    let audited = c.audit((&a.session.sub, &a.session.name), "kill_switch_engaged", None, a.peer, None, r.as_ref().ok().cloned()).await;
    match (r, audited) {
        (Ok(v), Ok(_)) => Json(v).into_response(),
        (Ok(v), Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "audit", "message": format!("The kill switch is engaged, but its audit entry could not be written: {e:#}"), "switch": v }))).into_response(),
        (Err(e), _) => kernel_down(e),
    }
}

/// Release it: only with a fresh password, recorded first.
pub async fn release(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>) -> Response {
    if let Err(r) = a.step_up() {
        return r;
    }
    if let Err(e) = c.audit((&a.session.sub, &a.session.name), "kill_switch_released", None, a.peer, Some(json!({ "engaged": true })), Some(json!({ "engaged": false }))).await {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "audit", "message": format!("{e:#}") }))).into_response();
    }
    match c.kernel.call(Method::POST, "/v1/kill-switch", Some(&json!({ "actor": format!("admin:{}", a.session.sub), "engaged": false }))).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => kernel_down(e),
    }
}

/// How full the disk holding the appliance's state is, as a fraction.
fn disk_used(path: &std::path::Path) -> Option<f64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `s` a writable statvfs.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 || s.f_blocks == 0 {
        return None;
    }
    Some(1.0 - (s.f_bavail as f64 / s.f_blocks as f64))
}

/// The machine: load, memory and disk.
fn host(state_dir: &std::path::Path) -> Value {
    let load = std::fs::read_to_string("/proc/loadavg").ok().and_then(|l| l.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()));
    let mem = std::fs::read_to_string("/proc/meminfo").ok().map(|m| {
        let kb = |k: &str| m.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0);
        let (total, avail) = (kb("MemTotal:"), kb("MemAvailable:"));
        if total > 0.0 { 1.0 - avail / total } else { 0.0 }
    });
    json!({ "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1), "load": load, "memory_used": mem, "disk_used": disk_used(state_dir) })
}

async fn provider_up(c: &Console) -> bool {
    let url = c.idp.inside(&format!("{}/.well-known/openid-configuration", c.idp.issuer.trim_end_matches('/')));
    matches!(tokio::time::timeout(std::time::Duration::from_secs(3), c.idp.http.get(&url).send()).await, Ok(Ok(r)) if r.status().is_success())
}

/// The status strip, the cards and the alerts, each alert naming where it
/// is fixed.
pub async fn overview(State(c): State<Arc<Console>>) -> Response {
    let (k, idp) = tokio::join!(c.kernel.call(Method::GET, "/v1/overview", None), provider_up(&c));
    let h = host(&c.state_dir);
    let mut alerts = vec![];
    match &k {
        Err(_) => alerts.push(json!({ "what": "The kernel is not answering.", "fix": "#/operations/health" })),
        Ok(v) if v["log_up"] != true => alerts.push(json!({ "what": "The event log (Postgres) is not answering.", "fix": "#/operations/health" })),
        _ => {}
    }
    if !idp {
        alerts.push(json!({ "what": "The identity provider is not answering: no one can sign in.", "fix": "#/operations/health" }));
    }
    if h["disk_used"].as_f64().is_some_and(|d| d > 0.85) {
        alerts.push(json!({ "what": format!("The disk is {:.0}% full.", h["disk_used"].as_f64().unwrap_or(0.0) * 100.0), "fix": "#/operations/health" }));
    }
    let k = k.unwrap_or(Value::Null);
    Json(json!({
        "status": { "healthy": alerts.is_empty(), "kill_switch": k["kill_switch"], "version": k["version"] },
        "cards": { "running_tasks": k["running_tasks"], "waiting_tasks": k["waiting_tasks"], "signed_in": k["signed_in"], "knowledge_changes_24h": k["knowledge_changes_24h"] },
        "alerts": alerts,
    }))
    .into_response()
}

/// Each service, the machine, and the latest errors.
pub async fn health(State(c): State<Arc<Console>>) -> Response {
    let (k, idp) = tokio::join!(c.kernel.call(Method::GET, "/v1/overview", None), provider_up(&c));
    let kernel_up = k.is_ok();
    let k = k.unwrap_or(Value::Null);
    Json(json!({
        "services": [
            { "name": "Kernel", "up": kernel_up },
            { "name": "Event log (Postgres)", "up": k["log_up"] == true },
            { "name": "Identity provider", "up": idp },
            { "name": "Admin console", "up": true },
        ],
        "host": host(&c.state_dir),
        "running_tasks": k["running_tasks"],
        "recent_errors": k["recent_errors"],
    }))
    .into_response()
}
