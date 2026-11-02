//! Magnus knows the person's apps while it works (connectors spec, How the
//! agent uses connectors; Inside work):
//! - every chat and task holds the person's app index: each app their
//!   organisation lets them use, with whether they have connected it, and
//!   none it does not;
//! - asked to do something an unconnected app does, Magnus finds it in their
//!   Library by what is wanted, and offers its card in the conversation
//!   with why; the card shows Connect, and nothing is connected until the
//!   person signs in;
//! - it cannot offer an app that is not in the person's Library, and says
//!   so when nothing fits.

use magnus_invariants::model::Script;
use magnus_invariants::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const CHAT: &str = "talking with the person you work for in a chat";

/// Each chat turn calls the tools listed for what the person said, one per
/// model call, then answers with the last result.
fn model(plan: Arc<Mutex<Vec<(String, Vec<(String, Value)>)>>>, seen: Arc<Mutex<Vec<(Value, Vec<String>)>>>) -> Script {
    script(move |c| {
        let (plan, seen) = (plan.clone(), seen.clone());
        async move {
            if !c.body["system"].to_string().contains(CHAT) {
                return complete("done");
            }
            seen.lock().unwrap().push((c.body.clone(), c.last_results().iter().map(|r| r.text.clone()).collect()));
            let first = c.messages().first().cloned().unwrap_or_default();
            let said = first["content"].as_array().and_then(|b| b.last()).and_then(|b| b["text"].as_str()).unwrap_or_default().to_string();
            let steps = plan.lock().unwrap().iter().find(|(s, _)| *s == said).map(|(_, t)| t.clone()).unwrap_or_default();
            match steps.get(c.turn()) {
                Some((t, input)) => tool(t, input.clone()),
                None => Reply::Text(c.last_results().first().map(|r| r.text.clone()).unwrap_or_else(|| "Done.".into())),
            }
        }
    })
}

/// Say `text` in a new conversation and wait for the answer: the thread's
/// id and its entries.
async fn say(api: &Api, plan: &Arc<Mutex<Vec<(String, Vec<(String, Value)>)>>>, text: &str, steps: Vec<(&str, Value)>) -> (String, Vec<Value>) {
    plan.lock().unwrap().push((text.into(), steps.into_iter().map(|(t, i)| (t.to_string(), i)).collect()));
    let (st, v) = api.post("/v1/chat", json!({ "text": text })).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    let thread = v["thread"].as_str().unwrap().to_string();
    let entries = eventually("the answer", PATIENCE, || async {
        let (_, t) = api.get(&format!("/v1/chat/{thread}")).await;
        let e = t["entries"].as_array().cloned().unwrap_or_default();
        e.iter().any(|x| x["kind"] == "answer").then_some(e)
    })
    .await;
    (thread, entries)
}

/// What the tools said back in this conversation's turns so far.
fn said_back(seen: &Arc<Mutex<Vec<(Value, Vec<String>)>>>) -> String {
    seen.lock().unwrap().iter().flat_map(|(_, r)| r.clone()).collect::<Vec<_>>().join("\n")
}

/// The opening message of the first request.
fn opening(seen: &Arc<Mutex<Vec<(Value, Vec<String>)>>>) -> String {
    seen.lock().unwrap().first().map(|(b, _)| b["messages"][0]["content"].to_string()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn magnus_finds_an_unconnected_app_by_what_is_wanted_and_offers_its_card_with_why() {
    let plan = Arc::new(Mutex::new(vec![]));
    let seen: Arc<Mutex<Vec<(Value, Vec<String>)>>> = Arc::new(Mutex::new(vec![]));
    let m = FakeModel::start(model(plan.clone(), seen.clone())).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let carol = app.api("carol");
    assert!(magnus_scratch::set_password(&app, "carol", magnus_scratch::PASSWORD));
    // The organisation's calendar, added by its address: ready to connect.
    let cal = magnus_scratch::FakeOAuthApp::start(3600).await;
    let (st, v) = app.api("alice").post("/v1/connectors", json!({ "url": cal.url, "audience": "public", "label": "private+untrusted" })).await;
    assert!(st.is_success(), "{v}");
    let name = v["name"].as_str().unwrap().to_string();

    // The calendar is in Carol's Library, not connected; she asks for a time.
    let (id, thread) = say(&carol, &plan, "Find a time with my team next week.", vec![
        ("find_apps", json!({ "query": "find a time on my calendar" })),
        ("offer_app", json!({ "name": name, "why": "To find a time your team is free" })),
    ]).await;

    // The index was in the context, before her words.
    let opening = opening(&seen);
    assert!(opening.contains("The person's apps") && opening.contains(&format!("({name})")) && opening.contains("(google-calendar)") && opening.contains("Not connected."), "no app index in the context: {opening}");
    // find_apps found the calendars by what was wanted.
    let back = said_back(&seen);
    assert!(back.contains(&format!("\"name\":\"{name}\"")) && back.contains("google-calendar"), "find_apps did not find the calendars: {back}");

    // Its card is in the conversation with why, and offers Connect.
    let card = thread.iter().find(|e| e["kind"] == "connector_card").unwrap_or_else(|| panic!("no app card in the thread: {thread:?}"));
    assert_eq!((card["connector"].as_str(), card["state"].as_str(), card["why"].as_str(), card["ready"].as_bool()), (Some(name.as_str()), Some("available"), Some("To find a time your team is free"), Some(true)), "{card}");
    assert!(card["title"].as_str().is_some_and(|t| !t.is_empty()) && card["letters"].is_string(), "{card}");
    // Nothing was connected for her.
    let (_, lib) = carol.get(&format!("/v1/connectors/{name}")).await;
    assert_eq!(lib["state"], "available", "offering the card connected the app: {lib}");

    // The card in the app, in every look and at phone width.
    let out = tokio::process::Command::new("node")
        .arg(magnus_invariants::appliance::workspace_root().join("desktop/tests/chat-app-card.mjs"))
        .env("MAGNUS_URL", &app.base)
        .env("MAGNUS_PERSON", "carol")
        .env("MAGNUS_PASSWORD", magnus_scratch::PASSWORD)
        .env("MAGNUS_THREAD", &id)
        .env("MAGNUS_APP", &name)
        .env("MAGNUS_WHY", "To find a time your team is free")
        .env("MAGNUS_CHROMIUM", magnus_scratch::chromium())
        .envs(magnus_scratch::shots())
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "the card in the app:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[tokio::test(flavor = "multi_thread")]
async fn magnus_cannot_offer_an_app_outside_the_persons_library() {
    let plan = Arc::new(Mutex::new(vec![]));
    let seen: Arc<Mutex<Vec<(Value, Vec<String>)>>> = Arc::new(Mutex::new(vec![]));
    let m = FakeModel::start(model(plan.clone(), seen.clone())).await;
    let app = Appliance::start(Spec::default(), &m).await;
    // Gmail is blocked for finance; Bob is in finance.
    let mut s = tokio::net::UnixStream::connect(app.state_dir.join("control.sock")).await.unwrap();
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let b = json!({ "actor": "admin:root", "blocked_groups": ["finance"] }).to_string();
        s.write_all(format!("PATCH /v1/admin/connectors/google-gmail HTTP/1.1\r\nHost: kernel\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    }
    let bob = app.api("bob");
    let _ = say(&bob, &plan, "Can you work with my Gmail?", vec![
        ("find_apps", json!({ "query": "gmail email inbox" })),
        ("offer_app", json!({ "name": "google-gmail", "why": "To read your email" })),
    ]).await;
    let opening = opening(&seen);
    assert!(!opening.contains("(google-gmail)"), "an app blocked for Bob is in his index: {opening}");
    let back = said_back(&seen);
    assert!(back.contains("not in the person's Library"), "an app outside Bob's Library was offered: {back}");
    assert!(!back.contains("\"name\":\"google-gmail\""), "find_apps listed an app outside Bob's Library: {back}");

    // Nothing fits: the person is told plainly.
    let _ = say(&bob, &plan, "Can you post in our Slack?", vec![("find_apps", json!({ "query": "slack channel" }))]).await;
    assert!(said_back(&seen).contains("No app in the person's Library does that"), "{}", said_back(&seen));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_that_needs_an_app_shows_its_card_with_why() {
    let cal = magnus_scratch::FakeOAuthApp::start(3600).await;
    let name = Arc::new(Mutex::new(String::new()));
    let n = name.clone();
    let m = FakeModel::start(script(move |c| {
        let n = n.lock().unwrap().clone();
        async move {
            match c.turn() {
                0 => contract_unchecked("find a time", "a time is found"),
                1 => tool("offer_app", json!({ "name": n, "why": "To read your team's calendars" })),
                _ => complete("I need your calendar to go on."),
            }
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (st, v) = app.api("alice").post("/v1/connectors", json!({ "url": cal.url, "audience": "public", "label": "private+untrusted" })).await;
    assert!(st.is_success(), "{v}");
    *name.lock().unwrap() = v["name"].as_str().unwrap().to_string();
    let carol = app.api("carol");
    let task = carol.submit("Find a time with my team next week.").await;
    // An unchecked contract waits for the person's review when it ends.
    carol.wait_item(&task, "review").await;
    let (_, done) = carol.get(&format!("/v1/tasks/{task}")).await;
    let offers = done["app_offers"].as_array().cloned().unwrap_or_default();
    let n = name.lock().unwrap().clone();
    assert_eq!(offers.len(), 1, "the task does not carry the app it asked for: {done}");
    assert_eq!((offers[0]["connector"].as_str(), offers[0]["state"].as_str(), offers[0]["why"].as_str()), (Some(n.as_str()), Some("available"), Some("To read your team's calendars")), "{}", offers[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_the_app_refused_is_offered_again_as_a_card_and_its_tools_wait() {
    let plan = Arc::new(Mutex::new(vec![]));
    let seen: Arc<Mutex<Vec<(Value, Vec<String>)>>> = Arc::new(Mutex::new(vec![]));
    let m = FakeModel::start(model(plan.clone(), seen.clone())).await;
    let app = Appliance::start(Spec::default(), &m).await;
    // 30-second tokens: every use refreshes, so a refused refresh shows at once.
    let cal = magnus_scratch::FakeOAuthApp::start(30).await;
    let (st, v) = app.api("alice").post("/v1/connectors", json!({ "url": cal.url, "audience": "public", "label": "private+untrusted" })).await;
    assert!(st.is_success(), "{v}");
    let name = v["name"].as_str().unwrap().to_string();
    let carol = app.api("carol");
    let (st, v) = carol.post(&format!("/v1/connectors/{name}/sign-in"), json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    assert!(st.is_success(), "{v}");
    let (code, state) = magnus_scratch::FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(carol.post(&format!("/v1/connectors/{name}/sign-in/complete"), json!({ "state": state, "code": code })).await.0, 204);

    // Carol ends Magnus's access in the app's own settings; the next use finds it refused.
    cal.revoke_everything();
    let (_, c) = carol.get(&format!("/v1/connectors/{name}")).await;
    assert_eq!(c["state"], "needs_attention", "{c}");

    let (_, thread) = say(&carol, &plan, "What's on my calendar tomorrow?", vec![("offer_app", json!({ "name": name, "why": "To read your calendar again" }))]).await;
    let opening = opening(&seen);
    assert!(opening.contains(&format!("({name})")) && opening.contains("Needs signing in again"), "the index does not say the calendar needs signing in again: {opening}");
    let first = seen.lock().unwrap().first().cloned().unwrap().0;
    let offered: Vec<String> = first["tools"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(String::from)).collect();
    assert!(offered.iter().all(|t| !t.starts_with(&format!("{name}__"))), "a refused sign-in's tools are still offered: {offered:?}");
    let card = thread.iter().find(|e| e["kind"] == "connector_card").unwrap_or_else(|| panic!("no card: {thread:?}"));
    assert_eq!((card["connector"].as_str(), card["state"].as_str()), (Some(name.as_str()), Some("needs_attention")), "{card}");

    // Signed in again, the calendar is connected again.
    let (_, v) = carol.post(&format!("/v1/connectors/{name}/sign-in"), json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    let (code, state) = magnus_scratch::FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(carol.post(&format!("/v1/connectors/{name}/sign-in/complete"), json!({ "state": state, "code": code })).await.0, 204);
    let (_, c) = carol.get(&format!("/v1/connectors/{name}")).await;
    assert_eq!(c["state"], "connected", "{c}");
}
