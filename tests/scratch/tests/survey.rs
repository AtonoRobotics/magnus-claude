//! The survey run (org context build spec, Background runs; T15): a
//! source's first connection starts an ordinary run, as the person who
//! connected it, with the goal of making the source navigable; afterwards
//! the bundle has its source document and a line in the top index. The
//! owner can ask for a survey again.

use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::{json, Value};

fn gen() -> Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

#[tokio::test(flavor = "multi_thread")]
async fn connecting_a_source_surveys_it_into_the_bundle_as_whoever_connected_it() {
    let cal = FakeOAuthApp::start(3600).await;
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        if !body.contains("Survey ") || c.body["system"].to_string().contains("long-term memory") {
            return Reply::Text(String::new());
        }
        let steps = [
            contract_unchecked("make the calendar navigable", "its source document and index line exist"),
            tool("bundle_write", json!({ "path": "/sources/calendar.md", "frontmatter": { "type": "Source", "title": "Calendar", "description": "Meetings and their attendees.",
                "resource": "calendar://", "sources": [{ "resource": "calendar://" }], "generated": gen(), "scope": "org" }, "body": "Search events by title or attendee." })),
            tool("bundle_write", json!({ "path": "/index.md", "frontmatter": { "okf_version": "0.2" }, "body": "# Sources\n* [Calendar](/sources/calendar.md) - meetings and attendees\n" })),
            complete("surveyed"),
        ];
        steps.get(c.turn()).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    // The calendar is one of the org's sources, each person signing in.
    let url = cal.url.clone();
    reconfigure(&mut app, move |c| {
        let mut t = toml::Table::new();
        t.insert("name".into(), "calendar".into());
        t.insert("url".into(), url.into());
        t.insert("credential".into(), "calendar_oauth".into());
        t.insert("auth".into(), "oauth".into());
        t.insert("audience".into(), "private".into());
        t.insert("label".into(), "private".into());
        t.insert("tools".into(), toml::Value::Table(toml::toml! { [list_events]
        risk = "read" }));
        t.insert("source".into(), toml::Value::Table(toml::toml! { search = "list_events" }));
        c.entry("connectors").or_insert(toml::Value::Array(vec![])).as_array_mut().unwrap().push(toml::Value::Table(t));
    })
    .await;
    let (alice, bob, carol) = (app.api("alice"), app.api("bob"), app.api("carol"));

    // Bob connects it first: the survey runs as him.
    let (st, v) = bob.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    assert!(st.is_success(), "{v}");
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(bob.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    let survey = eventually("the survey's run", PATIENCE, || async { bob.tasks().await["tasks"].as_array().unwrap().iter().find(|t| t["text"].as_str().is_some_and(|x| x.starts_with("Survey "))).and_then(|t| t["task_id"].as_str().map(String::from)) }).await;
    assert!(bob.task(&survey).await["text"].as_str().unwrap().contains("connected by bob"));
    bob.wait_item(&survey, "review").await;

    // Afterwards the bundle has its source document and a line in the index.
    let results: Vec<String> = m.requests().into_iter().filter(|c| c.body.to_string().contains("Survey ")).flat_map(|c| c.last_results()).map(|r| r.text).collect();
    assert!(results.iter().any(|r| r.contains("wrote /sources/calendar.md")) && results.iter().any(|r| r.contains("wrote /index.md")), "{results:?}");

    // A second connection does not survey again; the owner can, carol cannot.
    let before = bob.tasks().await["tasks"].as_array().unwrap().len() + alice.tasks().await["tasks"].as_array().unwrap().len();
    let (_, v) = alice.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(alice.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(bob.tasks().await["tasks"].as_array().unwrap().len() + alice.tasks().await["tasks"].as_array().unwrap().len(), before, "a second connection surveyed again");
    assert_eq!(carol.post("/v1/sources/calendar/survey", json!({})).await.0.as_u16(), 403);
    let (st, v) = alice.post("/v1/sources/calendar/survey", json!({})).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    let again = v["task_id"].as_str().unwrap();
    assert!(bob.task(again).await["text"].as_str().is_some_and(|t| t.contains("connected by bob")), "the survey again is not bob's");
}
