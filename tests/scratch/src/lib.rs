//! Shared setup for the desktop app's flow tests: an appliance with a fake
//! Anthropic API, a policy that asks before a file leaves a task, and a
//! person with a password.

use axum::{body::Body, extract::State, http::HeaderMap, response::Response, routing::{get, post}, Json, Router};
use magnus_invariants::appliance::bin;
use magnus_invariants::*;
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;

pub const PERSON: &str = "alice";
pub const PASSWORD: &str = "correct horse battery";
pub const TASK: &str = "Write me a one-line summary of Q3 revenue (41.7M, Q2 was 38.2M) and send me the file.";

/// `/v1/models` answered here, since connecting a key checks it there;
/// everything else goes to the scripted model.
pub async fn provider(model: &FakeModel) -> String {
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
        resp.body(Body::from(r.bytes().await.unwrap())).unwrap()
    }
    let app = Router::new()
        .route("/v1/models", get(|| async { Json(json!({ "data": [{ "id": "claude-test", "display_name": "Claude (test)", "type": "model" }], "has_more": false })) }))
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
            "view": [{ "type": "stat", "label": "Q3 revenue", "value": "41.7M" }, { "type": "bar_chart", "title": "Revenue by quarter", "labels": ["Q2", "Q3"], "values": [38.2, 41.7], "unit": "M" }],
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
        let mut model = FakeModel::start(steps(replies(web.url("/finance/q3")))).await;
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

pub fn chromium() -> PathBuf {
    std::env::var("MAGNUS_CHROMIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/ms-playwright/chromium-1234/chrome-linux64/chrome"))
}
