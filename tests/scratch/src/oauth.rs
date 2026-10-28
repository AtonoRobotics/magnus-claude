//! A fake OAuth provider for connector sign-in: the device-code grant, the
//! authorization-code grant with PKCE (its authorize endpoint redirects at
//! once, as if the person approved), and refresh tokens that rotate.

use axum::{
    extract::{Form, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct S {
    /// device_code -> (user_code, approved)
    devices: HashMap<String, (String, Option<bool>)>,
    /// code -> (code_challenge, redirect_uri)
    codes: HashMap<String, (String, String)>,
    /// live refresh tokens
    refresh: Vec<String>,
    issued: Vec<String>,
    refresh_grants: usize,
    expires_in: i64,
    n: usize,
}

#[derive(Clone)]
pub struct FakeOAuth {
    pub url: String,
    s: Arc<Mutex<S>>,
}

fn challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

fn issue(s: &mut S) -> Value {
    s.n += 1;
    let (access, refresh) = (format!("at-{}-{}", s.n, uuid::Uuid::new_v4().simple()), format!("rt-{}-{}", s.n, uuid::Uuid::new_v4().simple()));
    s.issued.push(access.clone());
    s.refresh.push(refresh.clone());
    json!({ "access_token": access, "refresh_token": refresh, "token_type": "Bearer", "expires_in": s.expires_in })
}

fn oauth_err(code: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": code, "error_description": format!("fake provider: {code}") }))).into_response()
}

async fn device(State(st): State<FakeOAuth>, Form(_f): Form<HashMap<String, String>>) -> Response {
    let mut s = st.s.lock().unwrap();
    let (dc, uc) = (uuid::Uuid::new_v4().simple().to_string(), format!("CODE-{}", s.devices.len() + 1));
    s.devices.insert(dc.clone(), (uc.clone(), None));
    Json(json!({ "device_code": dc, "user_code": uc, "verification_uri": format!("{}/activate", st.url), "expires_in": 600, "interval": 0 })).into_response()
}

/// The provider's page where the person enters the code and approves.
async fn activate(State(st): State<FakeOAuth>, Query(q): Query<HashMap<String, String>>) -> Response {
    let code = q.get("user_code").cloned().unwrap_or_default();
    st.decide(&code, true);
    axum::response::Html(format!("<h1>Approved</h1><p>Magnus may use your account ({code}).</p>")).into_response()
}

async fn authorize(State(st): State<FakeOAuth>, Query(q): Query<HashMap<String, String>>) -> Response {
    let mut s = st.s.lock().unwrap();
    if q.get("code_challenge_method").map(String::as_str) != Some("S256") {
        return oauth_err("invalid_request");
    }
    let code = uuid::Uuid::new_v4().simple().to_string();
    let redirect = q.get("redirect_uri").cloned().unwrap_or_default();
    s.codes.insert(code.clone(), (q.get("code_challenge").cloned().unwrap_or_default(), redirect.clone()));
    let sep = if redirect.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{redirect}{sep}code={code}&state={}", q.get("state").cloned().unwrap_or_default())).into_response()
}

async fn token(State(st): State<FakeOAuth>, Form(f): Form<HashMap<String, String>>) -> Response {
    let mut s = st.s.lock().unwrap();
    match f.get("grant_type").map(String::as_str) {
        Some("urn:ietf:params:oauth:grant-type:device_code") => {
            let dc = f.get("device_code").cloned().unwrap_or_default();
            match s.devices.get(&dc).map(|d| d.1) {
                None => oauth_err("invalid_grant"),
                Some(None) => oauth_err("authorization_pending"),
                Some(Some(false)) => oauth_err("access_denied"),
                Some(Some(true)) => {
                    s.devices.remove(&dc);
                    Json(issue(&mut s)).into_response()
                }
            }
        }
        Some("authorization_code") => {
            let Some((ch, redirect)) = s.codes.remove(f.get("code").map(String::as_str).unwrap_or_default()) else { return oauth_err("invalid_grant") };
            if f.get("redirect_uri") != Some(&redirect) || challenge(f.get("code_verifier").map(String::as_str).unwrap_or_default()) != ch {
                return oauth_err("invalid_grant");
            }
            Json(issue(&mut s)).into_response()
        }
        Some("refresh_token") => {
            let rt = f.get("refresh_token").cloned().unwrap_or_default();
            let Some(i) = s.refresh.iter().position(|r| *r == rt) else { return oauth_err("invalid_grant") };
            s.refresh.remove(i);
            s.refresh_grants += 1;
            Json(issue(&mut s)).into_response()
        }
        _ => oauth_err("unsupported_grant_type"),
    }
}

impl FakeOAuth {
    pub async fn start() -> FakeOAuth {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let me = FakeOAuth { url, s: Arc::new(Mutex::new(S { expires_in: 3600, ..Default::default() })) };
        let app = Router::new().route("/device", post(device)).route("/activate", get(activate)).route("/authorize", get(authorize)).route("/token", post(token)).with_state(me.clone());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        me
    }

    /// The person approves (or declines) the code on the provider's page.
    pub fn decide(&self, user_code: &str, approve: bool) {
        let mut s = self.s.lock().unwrap();
        for d in s.devices.values_mut() {
            if d.0 == user_code {
                d.1 = Some(approve);
            }
        }
    }

    /// How long tokens issued from now on last.
    pub fn set_expires_in(&self, seconds: i64) {
        self.s.lock().unwrap().expires_in = seconds;
    }

    pub fn refresh_grants(&self) -> usize {
        self.s.lock().unwrap().refresh_grants
    }

    pub fn issued(&self) -> Vec<String> {
        self.s.lock().unwrap().issued.clone()
    }

    /// The provider forgets every refresh token (a revoked grant).
    pub fn revoke(&self) {
        self.s.lock().unwrap().refresh.clear();
    }

    /// Connector sign-in settings for this provider.
    pub fn auth(&self, kind: &str) -> Value {
        json!({ "kind": kind, "client_id": "magnus-test", "device_authorization_endpoint": format!("{}/device", self.url),
            "authorization_endpoint": format!("{}/authorize", self.url), "token_endpoint": format!("{}/token", self.url), "scopes": ["mail"] })
    }
}
