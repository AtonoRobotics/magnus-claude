//! The console's sign-in flow and its guard on every API route (see
//! `auth`). Calls to the provider go to its internal address when one is
//! configured; the issuer the tokens name stays the public one.

use super::auth::{self, Pending, Session, COOKIE, ROLE};
use super::Console;
use anyhow::{anyhow, bail, Context, Result};
use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// The browser's half of a waiting sign-in: it must come back in the same
/// browser that started it.
const LOGIN_COOKIE: &str = "__Host-magnus_admin_login";

/// The provider, as the console reaches it.
pub struct Idp {
    pub issuer: String,
    pub client_id: String,
    pub roles_claim: String,
    /// Where the provider answers the console, when not at its issuer.
    pub internal: Option<String>,
    pub http: reqwest::Client,
    discovery: tokio::sync::OnceCell<Value>,
    jwks: tokio::sync::Mutex<Option<Value>>,
}

impl Idp {
    pub fn new(issuer: String, client_id: String, roles_claim: String, internal: Option<String>) -> Idp {
        Idp { issuer, client_id, roles_claim, internal, http: reqwest::Client::new(), discovery: Default::default(), jwks: Default::default() }
    }

    /// A public address of the provider, as the console reaches it.
    pub fn inside(&self, url: &str) -> String {
        match &self.internal {
            Some(i) => {
                let public = url::Url::parse(&self.issuer).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
                url.strip_prefix(&public).map(|rest| format!("{}{rest}", i.trim_end_matches('/'))).unwrap_or_else(|| url.to_string())
            }
            None => url.to_string(),
        }
    }

    async fn discovery(&self) -> Result<&Value> {
        self.discovery
            .get_or_try_init(|| async {
                let url = self.inside(&format!("{}/.well-known/openid-configuration", self.issuer.trim_end_matches('/')));
                let d: Value = self.http.get(&url).send().await?.error_for_status()?.json().await.with_context(|| format!("read {url}"))?;
                if d["issuer"].as_str() != Some(self.issuer.as_str()) {
                    bail!("{url} names issuer {}, not {}", d["issuer"], self.issuer);
                }
                Ok(d)
            })
            .await
    }

    fn endpoint<'a>(d: &'a Value, name: &str) -> Result<&'a str> {
        d[name].as_str().ok_or_else(|| anyhow!("the provider names no {name}"))
    }

    /// The realm's signing keys, fetched again when a token names one it
    /// has not seen (the realm rotated them).
    async fn keys(&self, fresh: bool) -> Result<Value> {
        let mut g = self.jwks.lock().await;
        if fresh || g.is_none() {
            let d = self.discovery().await?;
            let url = self.inside(Self::endpoint(d, "jwks_uri")?);
            *g = Some(self.http.get(&url).send().await?.error_for_status()?.json().await?);
        }
        Ok(g.clone().unwrap_or_default())
    }

    async fn verified(&self, jwt: &str) -> Result<Value> {
        match auth::verify(jwt, &self.keys(false).await?) {
            Ok(c) => Ok(c),
            Err(_) => auth::verify(jwt, &self.keys(true).await?),
        }
    }

    pub async fn authorize_url(&self, redirect_uri: &str, state: &str, nonce: &str, verifier: &str, step_up: bool) -> Result<String> {
        let d = self.discovery().await?;
        let mut u = url::Url::parse(Self::endpoint(d, "authorization_endpoint")?)?;
        u.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", "openid")
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", &crate::connectors::challenge(verifier))
            .append_pair("code_challenge_method", "S256");
        if step_up {
            // A fresh password (and code) now, whatever session the provider has.
            u.query_pairs_mut().append_pair("prompt", "login").append_pair("max_age", "0");
        }
        Ok(u.into())
    }

    /// A call to the token endpoint, the console proving itself with its key.
    async fn token(&self, key: &auth::Key, form: &[(&str, &str)]) -> Result<Value> {
        self.token_as(key, &self.client_id, form).await
    }

    /// A call to the token endpoint as `client_id`, proven with the
    /// console's key (`private_key_jwt`).
    pub async fn token_as(&self, key: &auth::Key, client_id: &str, form: &[(&str, &str)]) -> Result<Value> {
        let d = self.discovery().await?;
        let endpoint = Self::endpoint(d, "token_endpoint")?;
        let assertion = key.assertion(client_id, endpoint)?;
        let mut f: Vec<(&str, &str)> = form.to_vec();
        f.extend([("client_id", client_id), ("client_assertion_type", "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"), ("client_assertion", assertion.as_str())]);
        let r = self.http.post(self.inside(endpoint)).form(&f).send().await?;
        let status = r.status();
        let b: Value = r.json().await.unwrap_or_default();
        if !status.is_success() {
            bail!("the provider refused: {}", b["error_description"].as_str().or(b["error"].as_str()).unwrap_or(status.as_str()));
        }
        Ok(b)
    }

    /// The admin an ID token names, checked, with whether they hold `admin`.
    async fn admin_of(&self, b: &Value, nonce: Option<&str>) -> Result<(String, String, bool, Option<i64>)> {
        let jwt = b["id_token"].as_str().ok_or_else(|| anyhow!("the provider sent no ID token"))?;
        let c = self.verified(jwt).await?;
        let sub = match nonce {
            Some(n) => auth::check_claims(&c, &self.issuer, &self.client_id, Some(n), Utc::now())?,
            // A refresh's ID token answers no nonce; the rest still holds.
            None => auth::check_claims(&c, &self.issuer, &self.client_id, None, Utc::now())?,
        };
        let name = c["name"].as_str().or(c["preferred_username"].as_str()).unwrap_or(&sub).to_string();
        Ok((sub, name, auth::roles(&c, &self.roles_claim).iter().any(|r| r == ROLE), c["auth_time"].as_i64()))
    }

    async fn logout(&self, key: &auth::Key, refresh_token: &str) {
        let r = async {
            let d = self.discovery().await?;
            let endpoint = Self::endpoint(d, "end_session_endpoint")?;
            let assertion = key.assertion(&self.client_id, Self::endpoint(d, "token_endpoint")?)?;
            self.http
                .post(self.inside(endpoint))
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("refresh_token", refresh_token),
                    ("client_assertion_type", "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"),
                    ("client_assertion", assertion.as_str()),
                ])
                .send()
                .await?
                .error_for_status()?;
            anyhow::Ok(())
        };
        if let Err(e) = r.await {
            tracing::warn!("admin console: ending the provider's session: {e:#}");
        }
    }
}

fn cookie_of(h: &HeaderMap, name: &str) -> Option<String> {
    h.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).filter_map(|p| p.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
}

fn set_cookie(name: &str, value: &str, max_age: i64) -> String {
    format!("{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}")
}

/// Only a path on the console itself.
fn local_path(then: Option<&String>) -> String {
    then.filter(|t| t.starts_with('/') && !t.starts_with("//") && !t.contains('\\')).cloned().unwrap_or_else(|| "/".into())
}

fn redirect_uri(c: &Console, h: &HeaderMap) -> String {
    // The host is the console's own (checked before any route runs).
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or(&c.hostname);
    format!("https://{host}/auth/callback")
}

fn page(status: StatusCode, title: &str, said: &str) -> Response {
    let body = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>{title}</title><link rel=stylesheet href=/assets/app.css><link rel=stylesheet href=/assets/admin.css><script src=/assets/theme.js></script><main class=\"ad-main\"><section class=\"card ad-sign-in\"><h1>{title}</h1><p class=muted>{said}</p><div class=row><a class=\"button primary\" href=\"/auth/login\">Sign in again</a></div></section></main>"
    );
    (status, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

pub async fn login(State(c): State<Arc<Console>>, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    begin(&c, &h, None, local_path(q.get("then"))).await
}

/// A fresh password check, for the actions that need one in the last five
/// minutes; it comes back to `then`.
pub async fn step_up(State(c): State<Arc<Console>>, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some((sid, _)) = current(&c, &h) else { return Redirect::to("/auth/login").into_response() };
    begin(&c, &h, Some(sid), local_path(q.get("then"))).await
}

async fn begin(c: &Console, h: &HeaderMap, step_up: Option<String>, then: String) -> Response {
    let (state, nonce, verifier) = (crate::connectors::random(), crate::connectors::random(), crate::connectors::random());
    let redirect = redirect_uri(c, h);
    let url = match c.idp.authorize_url(&redirect, &state, &nonce, &verifier, step_up.is_some()).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("admin console: the provider: {e:#}");
            return page(StatusCode::BAD_GATEWAY, "Sign-in is unavailable", "The identity provider did not answer. Try again in a moment.");
        }
    };
    if let Err(e) = c.store.add_pending(state.clone(), Pending { verifier, nonce, step_up, then, redirect_uri: redirect, started: Utc::now() }) {
        return page(StatusCode::TOO_MANY_REQUESTS, "Try again shortly", &e.to_string());
    }
    ([(header::SET_COOKIE, set_cookie(LOGIN_COOKIE, &state, 600))], Redirect::to(&url)).into_response()
}

pub async fn callback(State(c): State<Arc<Console>>, axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (Some(state), Some(code)) = (q.get("state"), q.get("code")) else {
        return page(StatusCode::BAD_REQUEST, "Not signed in", "The sign-in did not finish.");
    };
    // Started in this browser, and not answered twice.
    if cookie_of(&h, LOGIN_COOKIE).as_deref() != Some(state.as_str()) {
        return page(StatusCode::BAD_REQUEST, "Not signed in", "This sign-in was started somewhere else. Start again here.");
    }
    let Some(p) = c.store.take_pending(state) else { return page(StatusCode::BAD_REQUEST, "Not signed in", "This sign-in is over. Start again.") };
    if q.get("iss").is_some_and(|i| *i != c.idp.issuer) {
        return page(StatusCode::BAD_REQUEST, "Not signed in", "The sign-in came back from another provider.");
    }
    let b = match c.idp.token(&c.key, &[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &p.redirect_uri), ("code_verifier", &p.verifier)]).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("admin console: code exchange: {e:#}");
            return page(StatusCode::UNAUTHORIZED, "Not signed in", "The identity provider did not accept the sign-in.");
        }
    };
    let (sub, name, admin, auth_time) = match c.idp.admin_of(&b, Some(&p.nonce)).await {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!("admin console: refused an ID token: {e:#}");
            return page(StatusCode::UNAUTHORIZED, "Not signed in", "The identity provider answered, but not with a sign-in for this console.");
        }
    };
    if !admin {
        tracing::warn!("admin console: {sub} signed in without the admin role");
        if let Err(e) = c.audit((&sub, &name), "sign_in_refused", None, peer, None, None).await {
            tracing::error!("admin console: {e:#}");
        }
        return page(StatusCode::FORBIDDEN, "Only admins can enter", "Your account does not hold the admin role. Ask an admin of your organisation.");
    }
    // Recorded before it takes effect: an action the log does not hold
    // does not happen.
    let action = if p.step_up.is_some() { "step_up" } else { "sign_in" };
    if let Err(e) = c.audit((&sub, &name), action, None, peer, None, None).await {
        tracing::error!("admin console: {e:#}");
        return page(StatusCode::SERVICE_UNAVAILABLE, "Not signed in", "The console could not record this sign-in in its audit log, so it did not let it through. Try again in a moment.");
    }
    let refresh = b["refresh_token"].as_str().unwrap_or_default().to_string();
    let now = Utc::now();
    let clear_login = set_cookie(LOGIN_COOKIE, "", 0);
    match p.step_up {
        Some(sid) => {
            // The same admin, with a password just typed.
            let fresh = auth_time.is_some_and(|t| t >= p.started.timestamp() - 1);
            let mut sessions = c.store.sessions.lock().unwrap();
            match sessions.get_mut(&sid) {
                Some(s) if s.sub == sub && fresh => {
                    s.stepped_up = Some(now);
                    if !refresh.is_empty() {
                        s.refresh_token = refresh;
                    }
                    s.refreshed = now;
                }
                _ => return page(StatusCode::FORBIDDEN, "Not confirmed", "That was not a fresh sign-in by the admin of this session."),
            }
            ([(header::SET_COOKIE, clear_login)], Redirect::to(&p.then)).into_response()
        }
        None => {
            let sid = crate::connectors::random();
            let s = Session { sub, name, csrf: crate::connectors::random(), refresh_token: refresh, started: now, seen: now, refreshed: now, stepped_up: Some(now) };
            c.store.sessions.lock().unwrap().insert(sid.clone(), s);
            let session = format!("{COOKIE}={sid}; Path=/; Secure; HttpOnly; SameSite=Strict");
            let mut r = Redirect::to(&p.then).into_response();
            r.headers_mut().append(header::SET_COOKIE, session.parse().unwrap());
            r.headers_mut().append(header::SET_COOKIE, clear_login.parse().unwrap());
            r
        }
    }
}

/// The session the browser's cookie names, if it is still open: neither
/// idle too long nor older than the longest a session lasts.
fn current(c: &Console, h: &HeaderMap) -> Option<(String, Session)> {
    let sid = cookie_of(h, COOKIE)?;
    let mut sessions = c.store.sessions.lock().unwrap();
    let s = sessions.get(&sid)?.clone();
    let now = Utc::now();
    if now - s.seen > Duration::minutes(c.idle_minutes) || now - s.started > Duration::hours(c.session_hours) {
        sessions.remove(&sid);
        return None;
    }
    Some((sid, s))
}

fn refused(status: StatusCode, error: &str, message: &str) -> Response {
    (status, Json(json!({ "error": error, "message": message }))).into_response()
}

/// Every API route but health: an open admin session, checked with the
/// provider at least every few minutes, and on a change, the session's
/// CSRF token. The admin rides along to the route.
pub async fn guard(State(c): State<Arc<Console>>, axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>, mut req: Request, next: Next) -> Response {
    let Some((sid, mut s)) = current(&c, req.headers()) else { return refused(StatusCode::UNAUTHORIZED, "sign_in_required", "Sign in to the admin console.") };
    let now = Utc::now();
    if now - s.refreshed > Duration::seconds(c.refresh_seconds) {
        // Still an admin, still turned on, at the provider.
        let still = match c.idp.token(&c.key, &[("grant_type", "refresh_token"), ("refresh_token", &s.refresh_token)]).await {
            Ok(b) => match c.idp.admin_of(&b, None).await {
                Ok((sub, _, true, _)) if sub == s.sub => {
                    if let Some(r) = b["refresh_token"].as_str() {
                        s.refresh_token = r.to_string();
                    }
                    true
                }
                Ok((sub, _, admin, _)) => {
                    tracing::warn!("admin console: {}'s session ends: the provider now names {sub}, admin {admin}", s.sub);
                    false
                }
                Err(e) => {
                    tracing::warn!("admin console: {}'s session ends: the provider's new ID token was refused: {e:#}", s.sub);
                    false
                }
            },
            Err(e) => {
                tracing::warn!("admin console: {}'s session ends: {e:#}", s.sub);
                false
            }
        };
        if !still {
            c.store.sessions.lock().unwrap().remove(&sid);
            if let Err(e) = c.audit((&s.sub, &s.name), "session_ended_by_provider", None, peer, None, None).await {
                tracing::error!("admin console: {e:#}");
            }
            return refused(StatusCode::UNAUTHORIZED, "sign_in_required", "Your console session ended: the identity provider no longer lets you in as an admin.");
        }
        s.refreshed = now;
    }
    if req.method() != Method::GET && req.method() != Method::HEAD && req.headers().get("x-csrf-token").and_then(|v| v.to_str().ok()) != Some(s.csrf.as_str()) {
        return refused(StatusCode::FORBIDDEN, "csrf", "This change did not come from the console's own page.");
    }
    s.seen = now;
    c.store.sessions.lock().unwrap().insert(sid.clone(), s.clone());
    req.extensions_mut().insert(Admin { sid, session: s, step_up_seconds: c.step_up_seconds, peer });
    next.run(req).await
}

/// The signed-in admin, for a route.
#[derive(Clone)]
pub struct Admin {
    pub sid: String,
    pub session: Session,
    pub step_up_seconds: i64,
    /// Where the request came from, for the audit log.
    pub peer: std::net::SocketAddr,
}

impl Admin {
    /// Refuse an action that needs a fresh password, unless one was typed
    /// in the last five minutes.
    pub fn step_up(&self) -> Result<(), Response> {
        match self.session.stepped_up(Utc::now(), self.step_up_seconds) {
            true => Ok(()),
            false => Err(refused(StatusCode::UNAUTHORIZED, "step_up_required", "Confirm it's you: sign in again to do this.")),
        }
    }
}

/// Who is signed in, the token every change carries, and until when a
/// step-up counts.
pub async fn me(axum::Extension(a): axum::Extension<Admin>) -> Response {
    let until = a.session.stepped_up.map(|t| t + Duration::seconds(a.step_up_seconds)).filter(|t| *t > Utc::now());
    Json(json!({ "name": a.session.name, "sub": a.session.sub, "csrf": a.session.csrf, "stepped_up_until": until })).into_response()
}

/// Sign out of the console and of the provider.
pub async fn logout(State(c): State<Arc<Console>>, axum::Extension(a): axum::Extension<Admin>) -> Response {
    if let Err(e) = c.audit((&a.session.sub, &a.session.name), "sign_out", None, a.peer, None, None).await {
        return refused(StatusCode::SERVICE_UNAVAILABLE, "audit", &format!("{e:#}"));
    }
    c.store.sessions.lock().unwrap().remove(&a.sid);
    c.idp.logout(&c.key, &a.session.refresh_token).await;
    ([(header::SET_COOKIE, format!("{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"))], StatusCode::NO_CONTENT).into_response()
}

/// An action that needs a fresh password: refused without one in the last
/// five minutes. The first such action in the console's own sections will
/// call `Admin::step_up` the same way.
pub async fn confirm(axum::Extension(a): axum::Extension<Admin>) -> Response {
    match a.step_up() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => r,
    }
}
