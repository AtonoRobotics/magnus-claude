//! Security › Sign-in in the admin console (admin console spec; owner's
//! ruling 1: no passkeys, people sign in with their password, and their
//! code when they use one): how members sign in (a code optional or
//! required), how admins sign in (shown, fixed), how long sessions last for
//! members (the realm's) and for the console (its own), and when repeated
//! failures lock an account. Changes go to the identity provider through its
//! admin API, as the console's service account, with a fresh password.

use super::session::Admin;
use super::users::{failed, recorded, refuse};
use super::Console;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The console takes lifetimes an admin set earlier (`ConsoleSettings` in
/// the log), when it starts.
pub fn adopt(c: &Console, v: &Value) {
    if let Some(m) = v["idle_minutes"].as_i64().filter(|m| (5..=240).contains(m)) {
        c.idle_minutes.store(m, Ordering::Relaxed);
    }
    if let Some(h) = v["session_hours"].as_i64().filter(|h| (1..=24).contains(h)) {
        c.session_hours.store(h, Ordering::Relaxed);
    }
}

/// The login flows' code steps: each flow's "Conditional OTP" subflow, its
/// id and requirement. CONDITIONAL asks for a code of whoever set one up;
/// REQUIRED asks everyone, and has them set one up first.
async fn code_steps(c: &Console, realm: &Value) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut out = vec![];
    for flow in [realm["browserFlow"].as_str().unwrap_or("browser"), realm["directGrantFlow"].as_str().unwrap_or("direct grant")] {
        let path = format!("/authentication/flows/{}/executions", urlencoding(flow));
        let ex = c.directory.call(&c.idp, &c.key, reqwest::Method::GET, &path, None).await?;
        for e in ex.as_array().into_iter().flatten() {
            if e["authenticationFlow"] == true && e["displayName"].as_str().is_some_and(|n| n.contains("OTP")) {
                out.push((flow.to_string(), e["id"].as_str().unwrap_or_default().to_string(), e["requirement"].as_str().unwrap_or_default().to_string()));
            }
        }
    }
    Ok(out)
}

fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

async fn read(c: &Console) -> anyhow::Result<Value> {
    let realm = c.directory.call(&c.idp, &c.key, reqwest::Method::GET, "", None).await?;
    let steps = code_steps(c, &realm).await?;
    let code = if !steps.is_empty() && steps.iter().all(|(_, _, r)| r == "REQUIRED") { "required" } else { "optional" };
    let secs = |f: &str| realm[f].as_i64().unwrap_or(0);
    Ok(json!({
        "members": { "method": "password", "code": code },
        "admins": { "method": "password", "code": "as members", "fixed": true },
        "sessions": {
            "members": { "idle_minutes": secs("ssoSessionIdleTimeout") / 60, "max_hours": secs("ssoSessionMaxLifespan") / 3600 },
            "console": { "idle_minutes": c.idle_minutes.load(Ordering::Relaxed), "session_hours": c.session_hours.load(Ordering::Relaxed) },
        },
        "lockout": {
            "enabled": realm["bruteForceProtected"] == true,
            "failures": realm["failureFactor"].as_i64().unwrap_or(30),
            "wait_minutes": secs("waitIncrementSeconds") / 60,
            "max_wait_minutes": secs("maxFailureWaitSeconds") / 60,
            "permanent": realm["permanentLockout"] == true,
        },
    }))
}

pub async fn get(State(c): State<Arc<Console>>) -> Response {
    match read(&c).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => failed(e),
    }
}

fn int(v: &Value, range: std::ops::RangeInclusive<i64>, what: &str) -> Result<Option<i64>, Response> {
    match v {
        Value::Null => Ok(None),
        x => match x.as_i64().filter(|n| range.contains(n)) {
            Some(n) => Ok(Some(n)),
            None => Err(refuse(StatusCode::BAD_REQUEST, "invalid", &format!("{what} is a whole number from {} to {}.", range.start(), range.end()))),
        },
    }
}

/// Change the sign-in rules: a fresh password first. Only what is given
/// changes.
pub async fn patch(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Json(b): Json<Value>) -> Response {
    if let Err(r) = a.step_up() {
        return r;
    }
    let code = match b["members"]["code"].as_str() {
        None => None,
        Some(x @ ("optional" | "required")) => Some(x.to_string()),
        Some(_) => return refuse(StatusCode::BAD_REQUEST, "invalid", "A code is optional or required."),
    };
    let parsed = (|| -> Result<_, Response> {
        Ok((
            int(&b["sessions"]["members"]["idle_minutes"], 5..=1440, "Members' idle time")?,
            int(&b["sessions"]["members"]["max_hours"], 1..=720, "Members' longest session")?,
            int(&b["sessions"]["console"]["idle_minutes"], 5..=240, "The console's idle time")?,
            int(&b["sessions"]["console"]["session_hours"], 1..=24, "The console's longest session")?,
            int(&b["lockout"]["failures"], 1..=100, "Failures before a lockout")?,
            int(&b["lockout"]["wait_minutes"], 1..=1440, "The first wait")?,
            int(&b["lockout"]["max_wait_minutes"], 1..=10080, "The longest wait")?,
        ))
    })();
    let (m_idle, m_max, c_idle, c_hours, failures, wait, max_wait) = match parsed {
        Ok(x) => x,
        Err(r) => return r,
    };
    let before = match read(&c).await {
        Ok(v) => v,
        Err(e) => return failed(e),
    };
    if wait.zip(max_wait.or(before["lockout"]["max_wait_minutes"].as_i64())).is_some_and(|(w, m)| m < w) || max_wait.zip(wait.or(before["lockout"]["wait_minutes"].as_i64())).is_some_and(|(m, w)| m < w) {
        return refuse(StatusCode::BAD_REQUEST, "invalid", "The longest wait cannot be shorter than the first.");
    }
    // The realm's own settings, given as a partial representation.
    let mut realm = serde_json::Map::new();
    if let Some(m) = m_idle {
        realm.insert("ssoSessionIdleTimeout".into(), json!(m * 60));
    }
    if let Some(h) = m_max {
        realm.insert("ssoSessionMaxLifespan".into(), json!(h * 3600));
    }
    if let Some(on) = b["lockout"]["enabled"].as_bool() {
        realm.insert("bruteForceProtected".into(), json!(on));
    }
    if let Some(n) = failures {
        realm.insert("failureFactor".into(), json!(n));
    }
    if let Some(w) = wait {
        realm.insert("waitIncrementSeconds".into(), json!(w * 60));
        realm.insert("minimumQuickLoginWaitSeconds".into(), json!((w * 60).min(60)));
    }
    if let Some(m) = max_wait {
        realm.insert("maxFailureWaitSeconds".into(), json!(m * 60));
    }
    if let Some(p) = b["lockout"]["permanent"].as_bool() {
        realm.insert("permanentLockout".into(), json!(p));
    }
    let console = c_idle.is_some() || c_hours.is_some();
    let r = recorded(&c, &a, "sign_in_rules_changed", "realm", Some(before.clone()), Some(b.clone()), async {
        if !realm.is_empty() {
            c.directory.call(&c.idp, &c.key, reqwest::Method::PUT, "", Some(&Value::Object(realm))).await?;
        }
        if let Some(code) = &code {
            let want = if code == "required" { "REQUIRED" } else { "CONDITIONAL" };
            let realm = c.directory.call(&c.idp, &c.key, reqwest::Method::GET, "", None).await?;
            for (flow, id, now) in code_steps(&c, &realm).await? {
                if now != want {
                    let path = format!("/authentication/flows/{}/executions", urlencoding(&flow));
                    c.directory.call(&c.idp, &c.key, reqwest::Method::PUT, &path, Some(&json!({ "id": id, "requirement": want }))).await?;
                }
            }
        }
        if console {
            let idle = c_idle.unwrap_or(c.idle_minutes.load(Ordering::Relaxed));
            let hours = c_hours.unwrap_or(c.session_hours.load(Ordering::Relaxed));
            c.kernel.call(Method::PUT, "/v1/admin/console-settings", Some(&json!({ "actor": super::users::actor(&a), "idle_minutes": idle, "session_hours": hours }))).await?;
            c.idle_minutes.store(idle, Ordering::Relaxed);
            c.session_hours.store(hours, Ordering::Relaxed);
        }
        Ok(Value::Null)
    })
    .await;
    if let Err(r) = r {
        return r;
    }
    match read(&c).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => failed(e),
    }
}
