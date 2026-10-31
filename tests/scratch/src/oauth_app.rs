//! A fake app behind MCP's authorization spec: an MCP server that answers
//! 401 without a valid token and names its authorization server, which
//! registers clients, signs in with the authorization code and PKCE (it
//! approves at once and redirects), issues expiring access tokens with
//! rotating refresh tokens, and revokes.

use axum::{
    body::Body,
    extract::{Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct AppState {
    clients: HashMap<String, Vec<String>>,
    /// code -> (client, redirect, challenge, resource)
    codes: HashMap<String, (String, String, String, String)>,
    access: HashSet<String>,
    refresh: HashSet<String>,
    pub revoked: Vec<String>,
    pub refreshes: usize,
    pub calls: Vec<(String, Value)>,
    /// Seconds each access token lasts.
    pub expires_in: i64,
    /// list_events' description as the app now advertises it, if changed.
    pub changed: Option<String>,
    /// Answer refreshes without `expires_in` (it is only recommended).
    pub refresh_without_expiry: bool,
    n: usize,
}

#[derive(Clone)]
pub struct FakeOAuthApp {
    pub base: String,
    /// The MCP server's address.
    pub url: String,
    pub state: Arc<Mutex<AppState>>,
}

type S = State<(String, Arc<Mutex<AppState>>)>;

fn oauth_error(status: u16, code: &str) -> Response {
    Response::builder().status(status).header("content-type", "application/json").body(Body::from(json!({ "error": code }).to_string())).unwrap()
}

fn tools(changed: Option<String>) -> Value {
    json!([
        { "name": "list_events", "description": changed.unwrap_or_else(|| "List calendar events.".into()), "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } },
        { "name": "create_event", "description": "Create a calendar event.", "inputSchema": { "type": "object", "properties": { "title": { "type": "string" } } }, "annotations": { "readOnlyHint": false, "destructiveHint": false } },
        { "name": "delete_all", "description": "Delete every event.", "inputSchema": { "type": "object", "properties": {} } }
    ])
}

async fn mcp(State((base, st)): S, h: HeaderMap, Json(req): Json<Value>) -> Response {
    let token = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default();
    if !st.lock().unwrap().access.contains(token) {
        return Response::builder()
            .status(401)
            .header("www-authenticate", format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\""))
            .body(Body::empty())
            .unwrap();
    }
    let id = req["id"].clone();
    let result = match req["method"].as_str().unwrap_or_default() {
        "initialize" => json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "calendar", "version": "1" } }),
        "tools/list" => json!({ "tools": tools(st.lock().unwrap().changed.clone()) }),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or_default().to_string();
            let mut s = st.lock().unwrap();
            s.calls.push((name.clone(), req["params"]["arguments"].clone()));
            let events: Vec<Value> = s.calls.iter().filter(|(n, _)| n == "create_event").map(|(_, a)| a.clone()).collect();
            json!({ "content": [{ "type": "text", "text": if name == "list_events" { json!(events).to_string() } else { "done".into() } }] })
        }
        _ => return (axum::http::StatusCode::ACCEPTED, "").into_response(),
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

async fn resource(State((base, _)): S) -> Json<Value> {
    Json(json!({ "resource": format!("{base}/mcp"), "authorization_servers": [base] }))
}

async fn metadata(State((base, _)): S) -> Json<Value> {
    Json(json!({
        "issuer": base, "authorization_endpoint": format!("{base}/authorize"), "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"), "revocation_endpoint": format!("{base}/revoke"),
        "code_challenge_methods_supported": ["S256"], "grant_types_supported": ["authorization_code", "refresh_token"]
    }))
}

async fn register(State((_, st)): S, Json(b): Json<Value>) -> Response {
    let mut s = st.lock().unwrap();
    s.n += 1;
    let id = format!("client-{}", s.n);
    let redirects = b["redirect_uris"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(String::from)).collect();
    s.clients.insert(id.clone(), redirects);
    (axum::http::StatusCode::CREATED, Json(json!({ "client_id": id }))).into_response()
}

/// The person approves at once: straight back to the redirect with a code.
async fn authorize(State((_, st)): S, Query(q): Query<HashMap<String, String>>) -> Response {
    let g = |k: &str| q.get(k).cloned().unwrap_or_default();
    let mut s = st.lock().unwrap();
    if !s.clients.get(&g("client_id")).is_some_and(|r| r.contains(&g("redirect_uri"))) || g("code_challenge_method") != "S256" || g("response_type") != "code" {
        return oauth_error(400, "invalid_request");
    }
    s.n += 1;
    let code = format!("code-{}", s.n);
    s.codes.insert(code.clone(), (g("client_id"), g("redirect_uri"), g("code_challenge"), g("resource")));
    let mut to = url::Url::parse(&g("redirect_uri")).unwrap();
    to.query_pairs_mut().append_pair("code", &code).append_pair("state", &g("state"));
    Response::builder().status(302).header("location", to.to_string()).body(Body::empty()).unwrap()
}

fn issue(s: &mut AppState) -> Value {
    s.n += 1;
    let (a, r) = (format!("at-{}", s.n), format!("rt-{}", s.n));
    s.access.insert(a.clone());
    s.refresh.insert(r.clone());
    json!({ "access_token": a, "refresh_token": r, "token_type": "Bearer", "expires_in": s.expires_in })
}

async fn token(State((base, st)): S, Form(f): Form<HashMap<String, String>>) -> Response {
    let g = |k: &str| f.get(k).cloned().unwrap_or_default();
    let mut s = st.lock().unwrap();
    if g("resource") != format!("{base}/mcp") {
        return oauth_error(400, "invalid_target");
    }
    match g("grant_type").as_str() {
        "authorization_code" => {
            let Some((client, redirect, challenge, _)) = s.codes.remove(&g("code")) else { return oauth_error(400, "invalid_grant") };
            use sha2::Digest;
            let proof = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(g("code_verifier").as_bytes()));
            if client != g("client_id") || redirect != g("redirect_uri") || proof != challenge {
                return oauth_error(400, "invalid_grant");
            }
            Json(issue(&mut s)).into_response()
        }
        // Rotation: a refresh token is good once.
        "refresh_token" => {
            if !s.refresh.remove(&g("refresh_token")) {
                return oauth_error(400, "invalid_grant");
            }
            s.refreshes += 1;
            let mut t = issue(&mut s);
            if s.refresh_without_expiry {
                t.as_object_mut().unwrap().remove("expires_in");
            }
            Json(t).into_response()
        }
        _ => oauth_error(400, "unsupported_grant_type"),
    }
}

async fn revoke(State((_, st)): S, Form(f): Form<HashMap<String, String>>) -> Response {
    let t = f.get("token").cloned().unwrap_or_default();
    let mut s = st.lock().unwrap();
    s.access.remove(&t);
    s.refresh.remove(&t);
    s.revoked.push(t);
    axum::http::StatusCode::OK.into_response()
}

impl FakeOAuthApp {
    pub async fn start(expires_in: i64) -> FakeOAuthApp {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let state = Arc::new(Mutex::new(AppState { expires_in, ..Default::default() }));
        let app = Router::new()
            .route("/mcp", post(mcp))
            .route("/.well-known/oauth-protected-resource/mcp", get(resource))
            .route("/.well-known/oauth-authorization-server", get(metadata))
            .route("/register", post(register))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/revoke", post(revoke))
            .with_state((base.clone(), state.clone()));
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        FakeOAuthApp { url: format!("{base}/mcp"), base, state }
    }

    /// What the person's browser does with the sign-in address: the app
    /// approves and redirects to the loopback address with the code.
    pub async fn browse(authorize_url: &str) -> (String, String) {
        let c = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let r = c.get(authorize_url).send().await.unwrap();
        assert_eq!(r.status(), 302, "the app did not approve: {}", r.text().await.unwrap_or_default());
        let to = url::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
        let q: HashMap<String, String> = to.query_pairs().into_owned().collect();
        (q["code"].clone(), q["state"].clone())
    }

    pub fn calls(&self, tool: &str) -> usize {
        self.state.lock().unwrap().calls.iter().filter(|(n, _)| n == tool).count()
    }
}
