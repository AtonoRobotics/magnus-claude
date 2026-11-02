//! The curator run (org context build spec, Background runs): on demand (and
//! nightly at the admin's time), ordinary runs given the goal of keeping the
//! bundle accurate and the kernel's brief of what stands out; a document
//! drawn from a source goes to the run of whoever connected it (K1). A new
//! version takes a document out of the next brief.

use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::{json, Value};

const NOTE: &str = "Note the weekly meeting and the customer type.";

fn gen() -> Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

fn weekly(body: &str) -> Value {
    json!({ "path": "/entities/customer/weekly.md", "frontmatter": { "type": "Customer", "title": "Weekly", "resource": "calendar://e1", "sources": [{ "resource": "calendar://e1" }],
        "generated": gen(), "identity": { "domain": "weekly.example" }, "relations": [], "scope": "org" }, "body": body })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_curator_gets_the_kernels_brief_each_run_under_the_grant_that_reads_it() {
    let cal = FakeOAuthApp::start(3600).await;
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        if c.body["system"].to_string().contains("long-term memory") {
            return Reply::Text(String::new());
        }
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(NOTE) {
            vec![
                contract_unchecked("note", "noted"),
                tool("bundle_write", json!({ "path": "/types/customer.md", "frontmatter": { "type": "Entity Type", "title": "Customer", "description": "A company.", "generated": gen(), "status": "draft",
                    "system_of_record": { "source": "/sources/calendar.md" }, "identity_keys": ["domain"], "relations": [], "cedar_entity": "Customer", "scope": "org" }, "body": "Customers." })),
                tool("bundle_write", weekly("Every Monday.")),
                complete("noted"),
            ]
        } else if body.contains("Curate the organisation") && body.contains("weekly.md") {
            vec![contract_unchecked("curate", "curated"), tool("bundle_write", weekly("Every Monday, rechecked.")), complete("curated")]
        } else if body.contains("Curate the organisation") {
            vec![contract_unchecked("curate", "curated"), complete("curated")]
        } else {
            return Reply::Text(String::new());
        };
        steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let mut app = Appliance::start(Spec::default(), &m).await;
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
    // Bob connects the calendar first: it is his to read for the curator.
    let (_, v) = bob.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(bob.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    let t = alice.submit(NOTE).await;
    alice.wait_item(&t, "review").await;
    // The meeting changed at the calendar.
    assert!(alice.post("/v1/sources/calendar/changes", json!({ "changes": [{ "id": "e1" }] })).await.0.is_success());

    assert_eq!(carol.post("/v1/bundle/curate", json!({})).await.0.as_u16(), 403);
    let (st, v) = alice.post("/v1/bundle/curate", json!({})).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    let tasks: Vec<String> = v["tasks"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    let text = |api: &Api, id: &str| {
        let (api, id) = (api.clone(), id.to_string());
        // Someone else's task is not theirs to see: no text.
        async move { api.get(&format!("/v1/tasks/{id}")).await.1["text"].as_str().unwrap_or_default().to_string() }
    };
    // Bob's run has the stale meeting, drawn from his calendar; the owner's
    // the draft type waiting on review.
    let mut bobs = None;
    let mut owners = None;
    for id in &tasks {
        let (b, a) = (text(&bob, id).await, text(&alice, id).await);
        if b.contains("weekly.md") {
            bobs = Some(id.clone());
        }
        if a.contains("/types/customer.md") {
            owners = Some(id.clone());
        }
    }
    let bobs = bobs.unwrap_or_else(|| panic!("no curator run of bob's names the stale meeting: {tasks:?}"));
    assert!(text(&bob, &bobs).await.contains("changed at its source"), "{}", text(&bob, &bobs).await);
    let owners = owners.unwrap_or_else(|| panic!("no curator run of the owner's names the draft: {tasks:?}"));
    assert!(text(&alice, &owners).await.contains("waiting on an owner's review"));
    bob.wait_item(&bobs, "review").await;

    // Rechecked, the meeting is out of the next brief.
    let (_, v) = alice.post("/v1/bundle/curate", json!({})).await;
    for id in v["tasks"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        assert!(!text(&bob, id).await.contains("weekly.md") && !text(&alice, id).await.contains("weekly.md"), "a rechecked document is still in the brief");
    }
}
