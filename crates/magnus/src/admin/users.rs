//! Users in the admin console (admin console spec, Users): everyone in the
//! organisation's realm, invited from here, their role, whether they are
//! turned on, and the sessions they hold on the appliance. Changing a role
//! needs a fresh password, and so does inviting an admin; the last admin
//! cannot be demoted or turned off. Every change is recorded before it is
//! made.

use super::directory::{self, Refused};
use super::session::Admin;
use super::Console;
use axum::extract::{Path, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn failed(e: anyhow::Error) -> Response {
    if let Some(r) = e.downcast_ref::<super::kernel::Refused>() {
        let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::BAD_REQUEST);
        return (status, Json(json!({ "error": "refused", "message": r.message }))).into_response();
    }
    if let Some(r) = e.downcast_ref::<Refused>() {
        let status = match r.status {
            404 => StatusCode::NOT_FOUND,
            409 => StatusCode::CONFLICT,
            400 => StatusCode::BAD_REQUEST,
            _ => StatusCode::BAD_GATEWAY,
        };
        return (status, Json(json!({ "error": "identity_provider", "message": r.to_string() }))).into_response();
    }
    (StatusCode::BAD_GATEWAY, Json(json!({ "error": "unavailable", "message": format!("{e:#}") }))).into_response()
}

pub(super) fn refuse(status: StatusCode, error: &str, message: &str) -> Response {
    (status, Json(json!({ "error": error, "message": message }))).into_response()
}

/// Record the change, then make it: nothing changes that the audit log
/// does not hold. A change that then fails is recorded as failed too.
pub(super) async fn recorded<F>(c: &Console, a: &Admin, action: &str, target: &str, before: Option<Value>, after: Option<Value>, change: F) -> Result<Value, Response>
where
    F: std::future::Future<Output = anyhow::Result<Value>>,
{
    if let Err(e) = c.audit((&a.session.sub, &a.session.name), action, Some(target), a.peer, before, after).await {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "audit", &format!("{e:#}")));
    }
    match change.await {
        Ok(v) => Ok(v),
        Err(e) => {
            let _ = c.audit((&a.session.sub, &a.session.name), &format!("{action}_failed"), Some(target), a.peer, None, Some(json!({ "error": format!("{e:#}") }))).await;
            Err(failed(e))
        }
    }
}

/// An id as the provider and the kernel make them: nothing that could
/// become another path.
pub(super) fn plain(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(super) fn actor(a: &Admin) -> String {
    format!("admin:{}", a.session.sub)
}

/// Everyone in the realm, with their role, status, last sign-in here and
/// open sessions here.
pub async fn list(State(c): State<Arc<Console>>) -> Response {
    let (users, admins, here) = tokio::join!(c.directory.users(&c.idp, &c.key), c.directory.admins(&c.idp, &c.key, true), c.kernel.call(Method::GET, "/v1/people", None));
    let (users, admins) = match (users, admins) {
        (Ok(u), Ok(a)) => (u, a),
        (Err(e), _) | (_, Err(e)) => return failed(e),
    };
    let here = here.unwrap_or_default();
    let rows: Vec<Value> = users
        .iter()
        .map(|u| {
            let mut p = directory::person(u, &admins);
            let h = &here["people"][p["id"].as_str().unwrap_or_default()];
            p["last_sign_in"] = h["last_sign_in"].clone();
            p["sessions"] = json!(h["sessions"].as_u64().unwrap_or(0));
            p
        })
        .collect();
    Json(json!({ "users": rows })).into_response()
}

async fn one(c: &Console, id: &str) -> Result<(Value, bool), Response> {
    if !plain(id) {
        return Err(refuse(StatusCode::NOT_FOUND, "not_found", "No such person."));
    }
    let (u, admins) = tokio::join!(c.directory.user(&c.idp, &c.key, id), c.directory.admins(&c.idp, &c.key, true));
    let (u, admins) = (u.map_err(failed)?, admins.map_err(failed)?);
    let p = directory::person(&u, &admins);
    let admin = p["role"] == "admin";
    Ok((p, admin))
}

/// One person, with each session they hold here.
pub async fn detail(State(c): State<Arc<Console>>, Path(id): Path<String>) -> Response {
    let (mut p, _) = match one(&c, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let s = c.kernel.call(Method::GET, &format!("/v1/people/{id}/sessions"), None).await;
    p["sessions"] = s.map(|v| v["sessions"].clone()).unwrap_or(Value::Null);
    Json(p).into_response()
}

/// The last admin turned on cannot lose the role, be turned off or go.
/// The check takes the console's one guard for changes that can take an
/// admin away and reads the admins from the provider under it; the caller
/// keeps the guard it returns until the provider has made the change, so
/// two such changes never both see an admin to spare (#192).
async fn last_admin<'a>(c: &'a Console, id: &str) -> Result<tokio::sync::MutexGuard<'a, ()>, Response> {
    let held = c.admin_removals.lock().await;
    let on = c.directory.admins(&c.idp, &c.key, false).await.map_err(failed)?;
    if on.iter().any(|a| a == id) && on.len() <= 1 {
        return Err(refuse(StatusCode::CONFLICT, "last_admin", "This is the organisation's last admin: make someone else an admin first."));
    }
    Ok(held)
}

fn role_of(b: &Value) -> Option<bool> {
    match b["role"].as_str() {
        Some("admin") => Some(true),
        Some("member") => Some(false),
        _ => None,
    }
}

/// Invite someone: the provider makes their account and emails them the
/// link to choose their password. Inviting an admin needs a fresh password.
pub async fn invite(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Json(b): Json<Value>) -> Response {
    let email = b["email"].as_str().map(str::trim).unwrap_or_default().to_lowercase();
    let name = b["name"].as_str().map(str::trim).unwrap_or_default().to_string();
    let Some(admin) = role_of(&b) else { return refuse(StatusCode::BAD_REQUEST, "invalid", "The role is admin or member.") };
    if !valid_email(&email) || name.is_empty() || name.len() > 200 {
        return refuse(StatusCode::BAD_REQUEST, "invalid", "An invitation needs an email address and a name.");
    }
    if admin {
        if let Err(r) = a.step_up() {
            return r;
        }
    }
    let after = json!({ "email": email, "name": name, "role": if admin { "admin" } else { "member" } });
    let r = recorded(&c, &a, "user_invited", &email, None, Some(after), async {
        let id = c.directory.create(&c.idp, &c.key, &email, &name).await?;
        if admin {
            c.directory.set_admin(&c.idp, &c.key, &id, true).await?;
        }
        // The account stays when the email cannot be sent: Resend tries again.
        let sent = c.directory.send_invite(&c.idp, &c.key, &id).await.map_err(|e| format!("{e:#}"));
        Ok(json!({ "id": id, "email_sent": sent.is_ok(), "email_error": sent.err() }))
    })
    .await;
    match r {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(r) => r,
    }
}

fn valid_email(e: &str) -> bool {
    let Some((user, host)) = e.split_once('@') else { return false };
    !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.') && e.len() <= 254 && !e.chars().any(|c| c.is_whitespace() || c == '<' || c == '>' || c == ',')
}

/// Change a role: a fresh password, and never the last admin's.
pub async fn update(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    let Some(to_admin) = role_of(&b) else { return refuse(StatusCode::BAD_REQUEST, "invalid", "The role is admin or member.") };
    if let Err(r) = a.step_up() {
        return r;
    }
    let (_, is_admin) = match one(&c, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if is_admin == to_admin {
        return StatusCode::NO_CONTENT.into_response();
    }
    // Held until the role has changed at the provider.
    let _held = if to_admin {
        None
    } else {
        match last_admin(&c, &id).await {
            Ok(g) => Some(g),
            Err(r) => return r,
        }
    };
    let role = |admin: bool| json!({ "role": if admin { "admin" } else { "member" } });
    match recorded(&c, &a, "user_role_changed", &id, Some(role(is_admin)), Some(role(to_admin)), async {
        c.directory.set_admin(&c.idp, &c.key, &id, to_admin).await?;
        Ok(Value::Null)
    })
    .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => r,
    }
}

/// Turn someone off: the provider refuses them, their sessions end there
/// and here, and their schedules pause. Never the last admin.
pub async fn disable(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(id): Path<String>) -> Response {
    if let Err(r) = one(&c, &id).await {
        return r;
    }
    // Held until they are turned off at the provider.
    let _held = match last_admin(&c, &id).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    let r = recorded(&c, &a, "user_disabled", &id, Some(json!({ "enabled": true })), Some(json!({ "enabled": false })), async {
        c.directory.set_enabled(&c.idp, &c.key, &id, false).await?;
        c.directory.sign_out(&c.idp, &c.key, &id).await?;
        c.kernel.call(Method::POST, &format!("/v1/people/{id}/turn-off"), Some(&json!({ "actor": actor(&a) }))).await
    })
    .await;
    match r {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// Turn them back on. Their paused schedules stay paused until they resume
/// them, as when the provider turns someone back on.
pub async fn enable(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(id): Path<String>) -> Response {
    if let Err(r) = one(&c, &id).await {
        return r;
    }
    let r = recorded(&c, &a, "user_enabled", &id, Some(json!({ "enabled": false })), Some(json!({ "enabled": true })), async {
        c.directory.set_enabled(&c.idp, &c.key, &id, true).await?;
        Ok(Value::Null)
    })
    .await;
    match r {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => r,
    }
}

/// End all their sessions: at the provider, and every device signed in here.
pub async fn end_sessions(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(id): Path<String>) -> Response {
    if let Err(r) = one(&c, &id).await {
        return r;
    }
    let r = recorded(&c, &a, "user_sessions_ended", &id, None, None, async {
        c.directory.sign_out(&c.idp, &c.key, &id).await?;
        c.kernel.call(Method::DELETE, &format!("/v1/people/{id}/sessions"), Some(&json!({ "actor": actor(&a) }))).await
    })
    .await;
    match r {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// End one session here: that device is signed out.
pub async fn end_session(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path((id, sid)): Path<(String, String)>) -> Response {
    if !plain(&id) || !plain(&sid) {
        return refuse(StatusCode::NOT_FOUND, "not_found", "No such session.");
    }
    let target = format!("{id}/{sid}");
    let r = recorded(&c, &a, "user_session_ended", &target, None, None, async { c.kernel.call(Method::DELETE, &format!("/v1/people/{id}/sessions/{sid}"), Some(&json!({ "actor": actor(&a) }))).await }).await;
    match r {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

async fn invited(c: &Console, id: &str) -> Result<Value, Response> {
    let (p, _) = one(c, id).await?;
    if p["status"] != "invited" {
        return Err(refuse(StatusCode::CONFLICT, "not_invited", "They have already chosen their password: this is an account, not an invitation."));
    }
    Ok(p)
}

/// Send the invitation again.
pub async fn resend(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(id): Path<String>) -> Response {
    if let Err(r) = invited(&c, &id).await {
        return r;
    }
    match recorded(&c, &a, "user_invite_resent", &id, None, None, async {
        c.directory.send_invite(&c.idp, &c.key, &id).await?;
        Ok(Value::Null)
    })
    .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => r,
    }
}

/// Withdraw an invitation nobody has taken up: the account goes. An account
/// in use is removed only by Delete.
pub async fn cancel(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(id): Path<String>) -> Response {
    let p = match invited(&c, &id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    // An admin's invitation counts as an admin turned on, and the role is
    // read again under the guard: held until the account has gone.
    let _held = match last_admin(&c, &id).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    match recorded(&c, &a, "user_invite_cancelled", &id, Some(json!({ "email": p["email"], "role": p["role"] })), None, async {
        c.directory.delete(&c.idp, &c.key, &id).await?;
        Ok(Value::Null)
    })
    .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => r,
    }
}

/// Their schedules (admin console spec, Users › Schedules and triggers).
pub async fn schedules(State(c): State<Arc<Console>>, Path(id): Path<String>) -> Response {
    if let Err(r) = one(&c, &id).await {
        return r;
    }
    match c.kernel.call(Method::GET, &format!("/v1/people/{id}/schedules"), None).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => failed(e),
    }
}

/// Suspend or resume one of their schedules.
pub async fn schedule(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path((id, sid, change)): Path<(String, String, String)>) -> Response {
    if !plain(&sid) || !matches!(change.as_str(), "suspend" | "resume") {
        return refuse(StatusCode::NOT_FOUND, "not_found", "No such schedule.");
    }
    if let Err(r) = one(&c, &id).await {
        return r;
    }
    let on = change == "resume";
    let action = if on { "user_schedule_resumed" } else { "user_schedule_suspended" };
    let body = json!({ "actor": actor(&a), "name": a.session.name });
    match recorded(&c, &a, action, &format!("{id}/{sid}"), Some(json!({ "suspended": on })), Some(json!({ "suspended": !on })), c.kernel.call(Method::POST, &format!("/v1/people/{id}/schedules/{sid}/{change}"), Some(&body))).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// Root's reading of their tasks, conversations, files or memory, or one of
/// them (admin console spec, Users › Data; owner's ruling 14). Each read is
/// in the audit log, with the admin, the person, what and when, before it
/// is made; the kernel records it too and tells the person.
pub async fn data(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(p): Path<Vec<(String, String)>>) -> Response {
    let get = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let (Some(id), Some(kind)) = (get("id"), get("kind")) else { return refuse(StatusCode::NOT_FOUND, "not_found", "Nothing here.") };
    let item = get("item");
    if !crate::person_admin::KINDS.contains(&kind.as_str()) || item.as_deref().is_some_and(|i| !plain(i)) {
        return refuse(StatusCode::NOT_FOUND, "not_found", "Their tasks, chats, files or memory.");
    }
    if let Err(r) = one(&c, &id).await {
        return r;
    }
    let body = json!({ "actor": actor(&a), "name": a.session.name, "item": item });
    let read = json!({ "resource": kind, "item": item });
    match recorded(&c, &a, "user_data_read", &id, None, Some(read), c.kernel.call(Method::POST, &format!("/v1/people/{id}/data/{kind}"), Some(&body))).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// Delete them (admin console spec, Users › Delete): their data first (the
/// kernel stops their runs, ends their schedules, sessions and sign-ins,
/// and removes their content from the store and the backups), then their
/// account at the provider. A fresh password, and their email typed.
pub async fn delete(State(c): State<Arc<Console>>, Extension(a): Extension<Admin>, Path(id): Path<String>, Json(b): Json<Value>) -> Response {
    if let Err(r) = a.step_up() {
        return r;
    }
    let (p, _) = match one(&c, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let typed = b["email"].as_str().map(|e| e.trim().to_lowercase()).unwrap_or_default();
    if p["email"].as_str().is_none_or(|e| e.to_lowercase() != typed) {
        return refuse(StatusCode::BAD_REQUEST, "confirm", "Type their email address to delete them.");
    }
    if id == a.session.sub {
        return refuse(StatusCode::CONFLICT, "self", "You cannot delete yourself: another admin can.");
    }
    // Held until their account has gone at the provider.
    let _held = match last_admin(&c, &id).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    let before = json!({ "name": p["name"], "email": p["email"], "role": p["role"] });
    let body = json!({ "actor": actor(&a) });
    let r = recorded(&c, &a, "user_deleted", &id, Some(before), None, async {
        let erased = c.kernel.call(Method::POST, &format!("/v1/people/{id}/erase"), Some(&body)).await?;
        c.directory.sign_out(&c.idp, &c.key, &id).await?;
        c.directory.delete(&c.idp, &c.key, &id).await?;
        Ok(erased)
    })
    .await;
    match r {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_id_never_becomes_another_path() {
        assert!(super::plain("0b6c1f2e-6c1d-4a1e-9b1a-3f2c1d0e9a8b") && super::plain("Zx_9-a"));
        for bad in ["", "..", "a/b", "a%2Fb", "x/../../audit", "a?b", &"a".repeat(101)] {
            assert!(!super::plain(bad), "{bad}");
        }
    }

    #[test]
    fn an_invitation_goes_to_an_email_address() {
        for ok in ["al@acme.test", "a.b+c@mail.acme.test"] {
            assert!(super::valid_email(ok), "{ok}");
        }
        for bad in ["", "al", "@acme.test", "al@acme", "al@.acme.test", "al @acme.test", "al@acme.test, bo@acme.test", "<al@acme.test>"] {
            assert!(!super::valid_email(bad), "{bad}");
        }
    }
}
