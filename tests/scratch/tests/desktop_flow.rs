//! The desktop app's frontend, driven in a real browser against a real
//! appliance: sign in, connect Claude, type a task, approve in the inbox,
//! see the outcome with its evidence (`desktop/tests/flow.mjs`). The model is
//! a fake Anthropic API that also lists models, since connecting checks the
//! key against `/v1/models`.

use axum::{body::Body, extract::State, http::HeaderMap, response::Response, routing::{get, post}, Json, Router};
use magnus_invariants::appliance::{bin, workspace_root};
use magnus_invariants::*;
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

const PASSWORD: &str = "correct horse battery";

/// `/v1/models` answered here; everything else goes to the scripted model.
async fn provider(model: &FakeModel) -> String {
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
        let status = r.status();
        let ct = r.headers().get("content-type").cloned();
        let bytes = r.bytes().await.unwrap();
        let mut resp = Response::builder().status(status);
        if let Some(ct) = ct {
            resp = resp.header("content-type", ct);
        }
        resp.body(Body::from(bytes)).unwrap()
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

fn chromium() -> PathBuf {
    std::env::var("MAGNUS_CHROMIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/ms-playwright/chromium-1234/chrome-linux64/chrome"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_signs_in_connects_claude_approves_and_sees_evidence() {
    let web = FakeWeb::start(&[("/finance/q3", "<p>Q3 revenue: 41.7M. Q2 revenue: 38.2M.</p>")]).await;
    let check = json!({ "tool": "web_fetch", "input": { "url": web.url("/finance/q3") }, "expect": [{ "pointer": "/text", "contains": "41.7M" }] });
    let mut model = FakeModel::start(steps(vec![
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
    ]))
    .await;
    model.url = provider(&model).await;
    let app = Appliance::start(Spec::default(), &model).await;
    let alice = app.api("alice");
    // Ask before any file leaves a task, so the flow has an approval.
    let ask = "forbid(principal, action == Action::\"execute\", resource == Tool::\"deliver_file\");";
    assert!(alice.put_policy("user/alice", ask).await.is_success());

    let mut c = std::process::Command::new(bin("magnus-appliance"))
        .args(["password", "--config"])
        .arg(app.dir.path().join("magnus.toml"))
        .args(["--person", "alice"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(c.stdin.take().unwrap(), "{PASSWORD}").unwrap();
    assert!(c.wait().unwrap().success());

    let key = canary("anthropic-key");
    let shots = std::env::var("MAGNUS_SHOTS").ok();
    let out = tokio::process::Command::new("node")
        .arg(workspace_root().join("desktop/tests/flow.mjs"))
        .env("MAGNUS_URL", &app.base)
        .env("MAGNUS_PERSON", "alice")
        .env("MAGNUS_PASSWORD", PASSWORD)
        .env("MAGNUS_API_KEY", &key)
        .env("MAGNUS_TASK", "Write me a one-line summary of Q3 revenue (41.7M, Q2 was 38.2M) and send me the file.")
        .env("MAGNUS_CHROMIUM", chromium())
        .envs(shots.map(|s| ("MAGNUS_SHOTS", s)))
        .output()
        .await
        .unwrap();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        let tasks = alice.tasks().await;
        let id = tasks["tasks"][0]["task_id"].as_str().unwrap_or_default().to_string();
        let (_, tl) = alice.get(&format!("/v1/tasks/{id}/timeline")).await;
        panic!("the flow failed:\n{log}\n--- task ---\n{}\n--- trace ---\n{}", alice.task(&id).await, serde_json::to_string_pretty(&tl).unwrap());
    }

    // What the page showed is what the log says.
    let tasks = alice.tasks().await;
    let t = &tasks["tasks"][0];
    let id = t["task_id"].as_str().unwrap();
    let full = alice.task(id).await;
    assert_eq!(full["status"], "completed");
    assert_eq!(full["criteria"][0]["verified"], true);
    assert_eq!(full["criteria"][1]["verified_by"], "person:alice");
    let approvals: Vec<_> = alice.inbox().await.into_iter().filter(|i| i["kind"] == "approval").collect();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0]["resolution"]["by"], "person:alice", "the approval was not made by the person on the card");
    // The key went to the vault, not the log.
    assert!(!app.db.dump().await.contains(&key), "the API key reached the log");
    let used = model.requests().last().unwrap().headers.get("x-api-key").cloned().unwrap_or_default();
    assert_eq!(used, key, "the appliance did not use the key the person connected");
}
