//! Shared setup for the desktop app's flow tests: an appliance with a fake
//! Anthropic API, a policy that asks before a file leaves a task, a person
//! with a password, and a fake identity provider for sign-in through one.

use axum::{body::Body, extract::State, http::HeaderMap, response::{IntoResponse, Response}, routing::{get, post}, Json, Router};
use magnus_invariants::appliance::bin;

mod oauth_app;
pub use oauth_app::FakeOAuthApp;
use magnus_invariants::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const PERSON: &str = "alice";
pub const PASSWORD: &str = "correct horse battery";
pub const CHAT_QUESTION: &str = "What was Q3 revenue?";
pub const CHAT_ANSWER: &str = "Q3 revenue was 41.7M.";
pub const TASK: &str = "Write me a one-line summary of Q3 revenue (41.7M, Q2 was 38.2M) and send me the file.";

/// A scripted reply's text beside its tool calls: the scripted model writes
/// either text or tool calls, so a call to `say` is passed on as text.
pub fn say(text: &str) -> (&'static str, Value) {
    (SAY, json!({ "text": text }))
}

const SAY: &str = "say";

/// In a stream, the pause between a `say` text and the tool calls after it,
/// as a model takes time to write them: longer than a chat view's 2-second
/// refresh, so a view sees the text streaming before anything else.
pub const SAID_PAUSE: std::time::Duration = std::time::Duration::from_secs(3);

/// A model response with each `say` call turned into the text it carries,
/// as one JSON message or as its stream of events, each marked if it comes
/// after a pause.
fn said(body: &str, stream: bool) -> Vec<(bool, String)> {
    let is_say = |b: &Value| b["type"] == "tool_use" && b["name"] == SAY;
    if !stream {
        let Ok(mut msg) = serde_json::from_str::<Value>(body) else { return vec![(false, body.to_string())] };
        let Some(content) = msg["content"].as_array().cloned() else { return vec![(false, body.to_string())] };
        let content: Vec<Value> = content.into_iter().map(|b| if is_say(&b) { json!({ "type": "text", "text": b["input"]["text"] }) } else { b }).collect();
        if !content.iter().any(|b| b["type"] == "tool_use") {
            msg["stop_reason"] = json!("end_turn");
        }
        msg["content"] = json!(content);
        return vec![(false, msg.to_string())];
    }
    let (mut says, mut calls) = (std::collections::HashSet::new(), 0);
    let mut out = vec![];
    for event in body.split("\n\n").filter(|e| !e.trim().is_empty()) {
        let (name, data) = event.split_once("\ndata: ").unwrap_or((event, ""));
        let Ok(mut d) = serde_json::from_str::<Value>(data) else {
            out.push((false, format!("{event}\n\n")));
            continue;
        };
        let mut pause = false;
        match d["type"].as_str() {
            Some("content_block_start") if is_say(&d["content_block"]) => {
                says.insert(d["index"].as_u64());
                d["content_block"] = json!({ "type": "text", "text": "" });
            }
            Some("content_block_start") if d["content_block"]["type"] == "tool_use" => {
                pause = !says.is_empty();
                calls += 1
            }
            Some("content_block_delta") if says.contains(&d["index"].as_u64()) => {
                let input: Value = serde_json::from_str(d["delta"]["partial_json"].as_str().unwrap_or("{}")).unwrap_or_default();
                d["delta"] = json!({ "type": "text_delta", "text": input["text"] });
            }
            Some("message_delta") if calls == 0 && !says.is_empty() => d["delta"]["stop_reason"] = json!("end_turn"),
            _ => {}
        }
        out.push((pause, format!("{name}\ndata: {d}\n\n")));
    }
    out
}

/// `/v1/models` answered here, since connecting a key checks it there;
/// everything else goes to the scripted model.
pub async fn provider(model: &FakeModel) -> String {
    provider_with(model, &["claude-test"]).await
}

/// The same, listing these model ids.
pub async fn provider_with(model: &FakeModel, ids: &[&str]) -> String {
    let data: Vec<Value> = ids.iter().map(|id| json!({ "id": id, "display_name": id, "type": "model" })).collect();
    let listing = json!({ "data": data, "has_more": false });
    #[derive(Clone)]
    struct Up(String, reqwest::Client);
    async fn forward(State(up): State<Up>, headers: HeaderMap, body: String) -> Response {
        let mut req = up.1.post(format!("{}/v1/messages", up.0)).body(body);
        for (k, v) in headers.iter() {
            if k != "host" && k != "content-length" {
                req = req.header(k, v);
            }
        }
        let r = req.send().await.unwrap();
        let mut resp = Response::builder().status(r.status());
        if let Some(ct) = r.headers().get("content-type").cloned() {
            resp = resp.header("content-type", ct);
        }
        let stream = resp.headers_ref().and_then(|h| h.get("content-type")).is_some_and(|c| c.as_bytes().starts_with(b"text/event-stream"));
        let body = String::from_utf8_lossy(&r.bytes().await.unwrap()).to_string();
        let chunks = futures::StreamExt::then(futures::stream::iter(said(&body, stream)), |(pause, chunk)| async move {
            if pause {
                tokio::time::sleep(SAID_PAUSE).await;
            }
            Ok::<_, std::convert::Infallible>(chunk)
        });
        resp.body(Body::from_stream(chunks)).unwrap()
    }
    let app = Router::new()
        .route("/v1/models", get(move || async move { Json(listing) }))
        .route("/v1/messages", post(forward))
        .with_state(Up(model.url.clone(), reqwest::Client::new()));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    url
}

/// What a model does for the flow's task: a contract with one criterion
/// checked against the world and one only a person can judge, a plan, a
/// file, its delivery (which asks), and a composed outcome.
fn replies(page: String) -> Vec<Reply> {
    let check = json!({ "tool": "web_fetch", "input": { "url": page }, "expect": [{ "pointer": "/text", "contains": "41.7M" }] });
    vec![
        tool("task_contract", json!({ "goal": "a one-line revenue summary", "assumptions": [], "irreversible_actions": [],
            "criteria": [
                { "id": "c1", "description": "The finance page still reports Q3 revenue of 41.7M", "check": check },
                { "id": "c2", "description": "The summary reads as one clear line" }
            ] })),
        tool("task_plan", json!({ "units": [{ "id": "u1", "goal": "write the summary", "done_when": "summary.txt exists", "status": "active" }, { "id": "u2", "goal": "deliver it", "done_when": "the person has the file" }] })),
        tool("sandbox_write_file", json!({ "path": "summary.txt", "content": "Q3 revenue: 41.7M, up from 38.2M in Q2.\n" })),
        tool("deliver_file", json!({ "path": "summary.txt" })),
        tool("task_complete", json!({
            "summary": "Wrote the Q3 revenue summary and delivered summary.txt.",
            "deliverable": [{ "type": "file", "file": "summary.txt" }, { "type": "stat", "label": "Q3 revenue", "value": "41.7M" }, { "type": "bar_chart", "title": "Revenue by quarter", "labels": ["Q2", "Q3"], "values": [38.2, 41.7], "unit": "M" }],
            "spoken": "Revenue in the third quarter was forty one point seven million, up from thirty eight point two."
        })),
    ]
}

pub struct Flow {
    pub app: Appliance,
    pub model: FakeModel,
    pub web: FakeWeb,
}

pub fn set_password(app: &Appliance, person: &str, pw: &str) -> bool {
    let mut c = std::process::Command::new(bin("magnus-appliance"))
        .args(["password", "--config"])
        .arg(app.dir.path().join("magnus.toml"))
        .args(["--person", person])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    writeln!(c.stdin.take().unwrap(), "{pw}").unwrap();
    c.wait().unwrap().success()
}

impl Flow {
    pub async fn start() -> Flow {
        let web = FakeWeb::start(&[("/finance/q3", "<p>Q3 revenue: 41.7M. Q2 revenue: 38.2M.</p>")]).await;
        // Chat turns answer; tasks follow the script.
        let task = steps(replies(web.url("/finance/q3")));
        let s = script(move |c| {
            let task = task.clone();
            async move {
                if c.body["system"].to_string().contains("in a chat") {
                    Reply::Text(CHAT_ANSWER.into())
                } else {
                    task(c).await
                }
            }
        });
        Flow::start_with(web, s).await
    }

    /// The same appliance, with the model following `s` instead.
    pub async fn start_with(web: FakeWeb, s: model::Script) -> Flow {
        let mut model = FakeModel::start(s).await;
        model.url = provider(&model).await;
        let app = Appliance::start(Spec::default(), &model).await;
        // Ask before any file leaves a task, so the flow has an approval.
        let ask = "forbid(principal, action == Action::\"execute\", resource == Tool::\"deliver_file\");";
        assert!(app.api(PERSON).put_policy(&format!("user/{PERSON}"), ask).await.is_success());
        assert!(set_password(&app, PERSON, PASSWORD));
        Flow { app, model, web }
    }

    /// Serve HTTPS as well, as a real appliance does. Returns its address and
    /// the certificate fingerprint it printed.
    pub async fn with_tls(&mut self) -> (String, String) {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let path = self.app.dir.path().join("magnus.toml");
        let mut cfg: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        cfg["appliance"].as_table_mut().unwrap().insert("tls_listen".into(), toml::Value::String(format!("127.0.0.1:{port}")));
        std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
        self.app.restart().await;
        let fp = eventually("the certificate fingerprint in the log", PATIENCE, || {
            let logs = self.app.logs();
            async move { logs.lines().rev().find_map(|l| l.split("certificate SHA-256 ").nth(1).map(|f| f.trim().to_string())) }
        })
        .await;
        (format!("https://127.0.0.1:{port}"), fp)
    }

    /// What the person saw is what the log says.
    pub async fn check_outcome(&self, key: &str) {
        let alice = self.app.api(PERSON);
        let tasks = alice.tasks().await;
        let id = tasks["tasks"][0]["task_id"].as_str().unwrap().to_string();
        let full = alice.task(&id).await;
        assert_eq!(full["status"], "completed", "{full}");
        assert_eq!(full["criteria"][0]["verified"], true);
        assert_eq!(full["criteria"][0]["verified_by"], "kernel");
        assert_eq!(full["criteria"][1]["verified_by"], format!("person:{PERSON}"));
        let approvals: Vec<Value> = alice.inbox().await.into_iter().filter(|i| i["kind"] == "approval").collect();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0]["resolution"]["by"], format!("person:{PERSON}"), "the approval was not made by the person");
        assert!(!self.app.db.dump().await.contains(key), "the API key reached the log");
        let used = self.model.requests().last().unwrap().headers.get("x-api-key").cloned().unwrap_or_default();
        assert_eq!(used, key, "the appliance did not use the key the person connected");
    }

    /// The task's state and trace, for a failure message.
    pub async fn report(&self) -> String {
        let alice = self.app.api(PERSON);
        let tasks = alice.tasks().await;
        let id = tasks["tasks"][0]["task_id"].as_str().unwrap_or_default().to_string();
        let (_, tl) = alice.get(&format!("/v1/tasks/{id}/timeline")).await;
        format!("--- task ---\n{}\n--- trace ---\n{}", alice.task(&id).await, serde_json::to_string_pretty(&tl).unwrap())
    }
}

/// The client id the fake identity provider knows.
pub const IDP_CLIENT: &str = "magnus-test";
/// What the sign-in button names when the appliance uses the fake provider.
pub const IDP_LABEL: &str = "Test identity";

#[derive(Default)]
struct IdpState {
    /// device_code -> (user_code, "pending" | "approved" | "denied")
    codes: HashMap<String, (String, String)>,
    /// Who approves, and the audience the ID token names.
    username: String,
    audience: String,
    /// More claims for the ID token (groups, roles).
    extra: Value,
}

/// An OpenID Connect provider with the device-code grant, as Keycloak
/// offers it: discovery, device authorization, token, and an approval page
/// (the person's browser visits it). Its ID tokens are unsigned; the
/// appliance relies on TLS to its token endpoint, not the signature.
pub struct FakeIdp {
    pub issuer: String,
    state: Arc<Mutex<IdpState>>,
}

impl FakeIdp {
    pub async fn start(username: &str) -> FakeIdp {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}/realms/test", l.local_addr().unwrap());
        let state = Arc::new(Mutex::new(IdpState { username: username.into(), audience: IDP_CLIENT.into(), ..Default::default() }));
        type S = State<(String, Arc<Mutex<IdpState>>)>;
        fn oauth_error(code: &str) -> Response {
            Response::builder().status(if code == "invalid_client" { 401 } else { 400 }).header("content-type", "application/json").body(Body::from(json!({ "error": code }).to_string())).unwrap()
        }
        async fn discovery(State((iss, _)): S) -> Json<Value> {
            Json(json!({ "issuer": iss, "token_endpoint": format!("{iss}/token"), "device_authorization_endpoint": format!("{iss}/device/auth"), "authorization_endpoint": format!("{iss}/auth") }))
        }
        async fn device(State((iss, st)): S, axum::Form(f): axum::Form<HashMap<String, String>>) -> Response {
            if f.get("client_id").map(String::as_str) != Some(IDP_CLIENT) {
                return oauth_error("invalid_client");
            }
            let n = st.lock().unwrap().codes.len();
            let (dc, uc) = (format!("dc-{n}-{}", canary("d")), format!("WXYZ-{:04}", n));
            st.lock().unwrap().codes.insert(dc.clone(), (uc.clone(), "pending".into()));
            Json(json!({ "device_code": dc, "user_code": uc, "verification_uri": format!("{iss}/device"), "verification_uri_complete": format!("{iss}/device?user_code={uc}"), "expires_in": 600, "interval": 1 })).into_response()
        }
        async fn approve(State((_, st)): S, axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>) -> Response {
            let uc = q.get("user_code").cloned().unwrap_or_default();
            let verdict = if q.contains_key("deny") { "denied" } else { "approved" };
            let mut s = st.lock().unwrap();
            // `as`: the person signs in on this page as someone else.
            if let Some(other) = q.get("as") {
                s.username = other.clone();
            }
            let who = s.username.clone();
            let found = s.codes.values_mut().find(|(c, _)| *c == uc).map(|e| e.1 = verdict.into()).is_some();
            let msg = if found { format!("<p>Signed in as {who}. You can close this page.</p>") } else { "<p>Unknown code.</p>".into() };
            Response::builder().header("content-type", "text/html").body(Body::from(msg)).unwrap()
        }
        async fn token(State((iss, st)): S, axum::Form(f): axum::Form<HashMap<String, String>>) -> Response {
            if f.get("grant_type").map(String::as_str) != Some("urn:ietf:params:oauth:grant-type:device_code") {
                return oauth_error("unsupported_grant_type");
            }
            let s = st.lock().unwrap();
            let Some((_, status)) = f.get("device_code").and_then(|d| s.codes.get(d)) else { return oauth_error("invalid_grant") };
            match status.as_str() {
                "pending" => oauth_error("authorization_pending"),
                "denied" => oauth_error("access_denied"),
                _ => {
                    use base64_url as b64;
                    let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 300;
                    let mut claims = json!({ "iss": iss, "aud": s.audience, "azp": s.audience, "exp": exp, "sub": format!("sub-{}", s.username), "preferred_username": s.username });
                    if let (Some(c), Some(x)) = (claims.as_object_mut(), s.extra.as_object()) {
                        c.extend(x.clone());
                    }
                    let id_token = format!("{}.{}.unsigned", b64::encode(&json!({ "alg": "RS256", "typ": "JWT" }).to_string()), b64::encode(&claims.to_string()));
                    Json(json!({ "access_token": "at-not-kept", "refresh_token": "rt-not-kept", "id_token": id_token, "token_type": "Bearer", "expires_in": 300 })).into_response()
                }
            }
        }
        let app = Router::new()
            .route("/realms/test/.well-known/openid-configuration", get(discovery))
            .route("/realms/test/device/auth", post(device))
            .route("/realms/test/device", get(approve))
            .route("/realms/test/token", post(token))
            .with_state((issuer.clone(), state.clone()));
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        FakeIdp { issuer, state }
    }

    /// Approve (or decline) a code, as the person does on the provider's page.
    pub async fn visit(&self, user_code: &str, deny: bool) {
        let url = format!("{}/device?user_code={user_code}{}", self.issuer, if deny { "&deny=1" } else { "" });
        assert!(reqwest::get(url).await.unwrap().status().is_success());
    }

    /// Approve as someone else from now on.
    pub fn set_username(&self, who: &str) {
        self.state.lock().unwrap().username = who.into();
    }

    /// Put these claims in ID tokens from now on (groups, roles).
    pub fn set_claims(&self, extra: Value) {
        self.state.lock().unwrap().extra = extra;
    }

    /// Sign in as an app does: start, approve on the provider's page, poll
    /// until done. Returns the session token.
    pub async fn sign_in(&self, app: &Appliance) -> String {
        let http = reqwest::Client::new();
        let s: Value = http.post(format!("{}/v1/sign-in/device", app.base)).header("x-magnus-app", "0.5.0").json(&json!({ "client": "app" })).send().await.unwrap().json().await.unwrap();
        self.visit(s["user_code"].as_str().unwrap(), false).await;
        let flow = s["flow"].as_str().unwrap();
        let end = std::time::Instant::now() + magnus_invariants::PATIENCE;
        loop {
            let r = http.post(format!("{}/v1/sign-in/device/{flow}", app.base)).send().await.unwrap();
            let st = r.status().as_u16();
            let v: Value = r.json().await.unwrap_or(Value::Null);
            if st == 200 {
                return v["token"].as_str().expect("an app sign-in returns a token").to_string();
            }
            assert!(st == 202 && std::time::Instant::now() < end, "sign-in failed: {st} {v}");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    /// Point the appliance at this provider, with more `[sign_in.oidc]`
    /// settings, and restart it.
    pub async fn configure_with(&self, app: &mut Appliance, more: &[(&str, &str)]) {
        self.configure_inner(app, more).await
    }

    /// Mint ID tokens for another client from now on.
    pub fn set_audience(&self, aud: &str) {
        self.state.lock().unwrap().audience = aud.into();
    }

    /// Point the appliance at this provider and restart it.
    pub async fn configure(&self, app: &mut Appliance) {
        self.configure_inner(app, &[]).await
    }

    async fn configure_inner(&self, app: &mut Appliance, more: &[(&str, &str)]) {
        let path = app.dir.path().join("magnus.toml");
        let mut cfg: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let sign_in = cfg.entry("sign_in").or_insert(toml::Value::Table(Default::default())).as_table_mut().unwrap();
        let mut oidc = toml::Table::new();
        oidc.insert("issuer".into(), toml::Value::String(self.issuer.clone()));
        oidc.insert("client_id".into(), toml::Value::String(IDP_CLIENT.into()));
        oidc.insert("label".into(), toml::Value::String(IDP_LABEL.into()));
        for (k, v) in more {
            oidc.insert(k.to_string(), toml::Value::String(v.to_string()));
        }
        sign_in.insert("oidc".into(), toml::Value::Table(oidc));
        std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
        app.restart().await;
    }
}

/// Change the appliance's config file, then restart it.
pub async fn reconfigure(app: &mut Appliance, f: impl FnOnce(&mut toml::Table)) {
    let path = app.dir.path().join("magnus.toml");
    let mut cfg: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    f(&mut cfg);
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    app.restart().await;
}

/// Base64url without padding, for the fake provider's tokens.
mod base64_url {
    pub fn encode(s: &str) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let b = s.as_bytes();
        let mut out = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..=c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        out
    }
}

pub fn chromium() -> PathBuf {
    std::env::var("MAGNUS_CHROMIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/ms-playwright/chromium-1234/chrome-linux64/chrome"))
}
