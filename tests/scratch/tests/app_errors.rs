//! Provider errors, rate limits and retry-after included, return to the
//! agent intact as tool errors, with no retry by the client (connectors
//! spec, Remote MCP connectors). An app over its rate answers 429 with
//! Retry-After: the agent reads the status, the wait and what the app said,
//! and the app saw the call once. An app's own error reaches the agent
//! with its code, message and data, also once.

use magnus_invariants::*;
use magnus_scratch::FakeOAuthApp;
use serde_json::{json, Value};

async fn read_calendar(cal: &FakeOAuthApp) -> String {
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("list the events", "the events are listed"),
            1 => tool("calendar__list_events", json!({})),
            _ => complete("could not"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let (st, v) = alice.post("/v1/connectors", json!({ "name": "calendar", "url": cal.url, "audience": "private", "label": "private+untrusted" })).await;
    assert!(st.is_success(), "{v}");
    assert_eq!(alice.put("/v1/connectors/calendar/tools/list_events", json!({ "risk": "read" })).await.0, 204);
    let (_, v) = alice.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(alice.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    let task = alice.submit("What is on my calendar?").await;
    alice.wait_item(&task, "review").await;
    // The result the agent read for its call.
    let after = m.requests().into_iter().find(|c| c.turn() == 2).expect("no model call after the read");
    let result: Value = after.body["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "user").unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "tool_result")
        .cloned()
        .expect("no tool result");
    assert_eq!(result["is_error"], true, "{result}");
    result["content"].to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limit_reaches_the_agent_with_its_wait_and_is_not_retried() {
    let cal = FakeOAuthApp::start(3600).await;
    cal.state.lock().unwrap().rate_limited = Some(42);
    let said = read_calendar(&cal).await;
    assert!(said.contains("HTTP 429"), "{said}");
    assert!(said.contains("wait 42 seconds"), "the app's Retry-After did not reach the agent: {said}");
    assert!(said.contains("Rate limit exceeded: 100 requests per minute."), "what the app said did not reach the agent: {said}");
    assert_eq!(cal.state.lock().unwrap().attempts, 1, "the call was retried");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_apps_own_error_reaches_the_agent_with_its_code_message_and_data() {
    let cal = FakeOAuthApp::start(3600).await;
    cal.state.lock().unwrap().rpc_error = Some((-32602, "Calendar not found".into(), json!({ "calendar": "team" })));
    let said = read_calendar(&cal).await;
    assert!(said.contains("-32602") && said.contains("Calendar not found") && said.contains("team"), "the app's error did not reach the agent intact: {said}");
    assert_eq!(cal.state.lock().unwrap().attempts, 1, "the call was retried");
}
