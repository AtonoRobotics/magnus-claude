//! Gmail, brokered (connectors spec, Brokered API connectors; owner,
//! 2026-11-03: Google's hosted Gmail server cannot send email, so Magnus
//! sends only drafts the person approves). The catalog's Gmail manifest
//! names the image built from this checkout (`brokered/gmail`), which runs
//! in each person's own microVM and reaches only `gmail.googleapis.com`,
//! through the appliance's egress proxy. A local HTTPS host stands in for
//! Gmail's API, with a certificate for its name from the test's own CA, and
//! FakeOAuthApp for Google's sign-in (the real ones need a Google account:
//! `gmail_contract.rs` uses the owner's test account).

use magnus_invariants::*;
use magnus_scratch::brokered::*;
use magnus_scratch::*;
use serde_json::{json, Value};

const HOST: &str = "gmail.googleapis.com";

fn manifest() -> toml::Table {
    std::fs::read_to_string(magnus_invariants::appliance::workspace_root().join("catalog/google-gmail.toml")).unwrap().parse().unwrap()
}

/// The appliance with Gmail as its manifest has it, its image built from
/// this checkout, Gmail's address given to the proxy as the stand-in's and
/// the stand-in's CA trusted in the microVM; Alice signed in. The image is
/// kept as long as the appliance runs.
async fn gmail_for_alice(m: &FakeModel, gmail: &StandInGmail) -> (Appliance, FakeOAuthApp, Image) {
    let mf = manifest();
    let image = image("magnus-gmail");
    assert_eq!(mf["endpoint"].as_str(), Some(image.digest.as_str()), "catalog/google-gmail.toml names another image than the one built from this commit");
    let hosts: Vec<String> = mf["hosts"].as_array().unwrap().iter().map(|h| h.as_str().unwrap().to_string()).collect();
    assert_eq!(hosts, [HOST]);
    let classes: Vec<(String, String)> = mf["tools"].as_table().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
    let tools: Vec<(&str, &str)> = classes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let provider = FakeOAuthApp::provider_api(3600).await;
    let mut app = Appliance::start(Spec::default(), m).await;
    let (addr, root) = (gmail.addr.clone(), gmail.root.clone());
    reconfigure(&mut app, |cfg| {
        configure(cfg, &image, 300, "gmail", &hosts, &provider.base, &tools);
        let b = cfg["brokered"].as_table_mut().unwrap();
        let mut resolve = toml::Table::new();
        resolve.insert(format!("{HOST}:443"), addr.into());
        b.insert("resolve".into(), resolve.into());
        b.insert("roots".into(), toml::Value::Array(vec![root.display().to_string().into()]));
        let c = cfg["connectors"].as_array_mut().unwrap().iter_mut().find(|c| c["name"].as_str() == Some("gmail")).unwrap().as_table_mut().unwrap();
        c.insert("audience".into(), mf["audience"].clone());
        c.insert("scopes".into(), mf["scopes"].clone());
    })
    .await;
    let alice = app.api("alice");
    let (st, v) = alice.post("/v1/connectors/gmail/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    assert!(st.is_success(), "{v}");
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(alice.post("/v1/connectors/gmail/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    (app, provider, image)
}

/// Every request reached Gmail's name, by TLS and by `Host`, at its API, as
/// Alice, and the proxy refused nothing.
fn only_gmail(app: &Appliance, gmail: &StandInGmail, provider: &FakeOAuthApp) {
    let seen = gmail.requests();
    assert!(!seen.is_empty());
    for r in &seen {
        assert_eq!((r.sni.as_str(), r.host.as_str()), (HOST, HOST), "{r:?}");
        assert!(r.path.starts_with("/gmail/v1/users/me/"), "{r:?}");
        assert!(r.auth.strip_prefix("Bearer ").is_some_and(|t| provider.issued(t)), "not Alice's access token: {r:?}");
    }
    assert!(!app.logs().contains("the egress proxy refused"), "the server asked for a host other than Gmail's");
}

/// The open approval for `task`, once there is one: the action it asks
/// about.
async fn asked(p: &Api, task: &str) -> (Value, String) {
    let ask = p.wait_item(task, "approval").await;
    let t = ask.to_string();
    let tool = ["create_draft", "send_draft"].into_iter().find(|x| t.contains(x)).unwrap_or_else(|| panic!("{ask}"));
    (ask, tool.to_string())
}

async fn approve(p: &Api, ask: &Value) {
    assert!(p.respond(ask["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success());
}

fn calls(gmail: &StandInGmail) -> Vec<(String, String)> {
    gmail.requests().iter().map(|r| (r.method.clone(), r.path.split('?').next().unwrap().replace("/gmail/v1/users/me", ""))).collect()
}

/// Sending is never undone, so it always asks first: here Magnus has read
/// nothing yet (nothing else makes it ask), and nothing reaches Gmail
/// until Alice approves the send.
#[tokio::test(flavor = "multi_thread")]
async fn gmail_sends_a_draft_only_once_the_person_approves_it_reaching_only_gmail() {
    let gmail = StandInGmail::with_drafts(1).await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("send Bob the draft I wrote him", "Bob has it"),
            1 => tool("gmail__send_draft", json!({ "draft_id": "d1" })),
            _ => complete("sent Bob your draft"),
        }
    }))
    .await;
    let (app, provider, _image) = gmail_for_alice(&m, &gmail).await;
    let alice = app.api("alice");

    let task = alice.submit("Send Bob the draft I wrote him.").await;
    let (ask, tool) = asked(&alice, &task).await;
    assert_eq!(tool, "send_draft");
    assert!(calls(&gmail).is_empty(), "Gmail was called before Alice approved: {:?}", calls(&gmail));
    approve(&alice, &ask).await;

    let review = alice.wait_item(&task, "review").await;
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    assert_eq!(alice.wait_settled(&task).await["status"], "completed");
    assert_eq!(calls(&gmail), [("POST".to_string(), "/drafts/send".to_string())]);
    assert_eq!(serde_json::from_str::<Value>(&gmail.requests()[0].body).unwrap(), json!({ "id": "d1" }));
    let said = m.requests().iter().filter(|c| c.text().contains("Send Bob the draft I wrote him.")).map(|c| c.text()).max_by_key(|t| t.len()).unwrap();
    assert!(said.contains("SENT"), "the send's answer did not reach Magnus: {said}");
    only_gmail(&app, &gmail, &provider);
}

/// Having read her mail (private, and written by others), Magnus asks
/// before it writes anything there (the Rule of Two), then again before
/// the send.
#[tokio::test(flavor = "multi_thread")]
async fn having_read_mail_gmail_asks_before_the_draft_and_again_before_the_send() {
    let gmail = StandInGmail::start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("send Bob the plan he asked for", "Bob has the plan"),
            1 => tool("gmail__search_threads", json!({ "query": "from:bob plan" })),
            2 => tool("gmail__create_draft", json!({ "to": ["bob@example.com"], "subject": "The plan", "body": "Friday at ten." })),
            3 => tool("gmail__send_draft", json!({ "draft_id": "d1" })),
            _ => complete("sent Bob the plan"),
        }
    }))
    .await;
    let (app, provider, _image) = gmail_for_alice(&m, &gmail).await;
    let alice = app.api("alice");
    let get = |p: &str| ("GET".to_string(), p.to_string());
    let post = |p: &str| ("POST".to_string(), p.to_string());

    let task = alice.submit("Send Bob the plan he asked for.").await;
    let (ask, tool) = asked(&alice, &task).await;
    assert_eq!((tool.as_str(), calls(&gmail)), ("create_draft", vec![get("/threads")]));
    approve(&alice, &ask).await;
    let (ask, tool) = asked(&alice, &task).await;
    assert_eq!((tool.as_str(), calls(&gmail)), ("send_draft", vec![get("/threads"), post("/drafts")]));
    let draft: Value = serde_json::from_str(&gmail.requests()[1].body).unwrap();
    let raw = String::from_utf8(base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE, draft["message"]["raw"].as_str().unwrap()).unwrap()).unwrap();
    assert!(raw.starts_with("To: bob@example.com\r\nSubject: The plan\r\n"), "{raw}");
    approve(&alice, &ask).await;

    let review = alice.wait_item(&task, "review").await;
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    assert_eq!(alice.wait_settled(&task).await["status"], "completed");
    assert_eq!(calls(&gmail), [get("/threads"), post("/drafts"), post("/drafts/send")]);
    only_gmail(&app, &gmail, &provider);
}

/// Google's refusals reach the boundary as what they mean: a sign-in
/// without Gmail's access asks the person for it with the app's card, and a
/// send with no clear answer is an unknown outcome, never a failure that
/// could be tried again as if nothing happened.
#[tokio::test(flavor = "multi_thread")]
async fn gmail_s_refusals_reach_the_boundary_as_what_they_mean() {
    let gmail = StandInGmail::start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("send Bob the plan", "Bob has the plan"),
            1 => tool("gmail__search_threads", json!({ "query": "from:bob" })),
            2 => tool("gmail__create_draft", json!({ "to": ["bob@example.com"], "subject": "The plan", "body": "Friday." })),
            3 => tool("gmail__send_draft", json!({ "draft_id": "d1" })),
            _ => complete("done"),
        }
    }))
    .await;
    let (app, provider, _image) = gmail_for_alice(&m, &gmail).await;
    let alice = app.api("alice");
    let modify = "https://www.googleapis.com/auth/gmail.modify";
    *gmail.next.lock().unwrap() = Some((
        403,
        format!("WWW-Authenticate: Bearer realm=\"https://accounts.google.com/\", error=\"insufficient_scope\", scope=\"{modify}\"\r\n"),
        json!({ "error": { "code": 403, "message": "Request had insufficient authentication scopes.", "errors": [{ "reason": "insufficientPermissions" }] } }).to_string(),
    ));
    let task = alice.submit("Send Bob the plan.").await;
    let mut ask = asked(&alice, &task).await;
    let said = m.requests().iter().filter(|c| c.text().contains("Send Bob the plan.")).map(|c| c.text()).max_by_key(|t| t.len()).unwrap();
    assert!(said.contains("needs more access from the person for search_threads"), "the refusal was not a request for access: {said}");
    if ask.1 == "create_draft" {
        approve(&alice, &ask.0).await;
        ask = asked(&alice, &task).await;
    }
    assert_eq!(ask.1, "send_draft");
    // The send: Gmail breaks off with 503, so it may have gone.
    *gmail.next.lock().unwrap() = Some((503, String::new(), json!({ "error": { "code": 503, "message": "The service is currently unavailable." } }).to_string()));
    approve(&alice, &ask.0).await;
    eventually("the send's unknown outcome", std::time::Duration::from_secs(60), || async { app.logs().contains("send_draft: outcome unknown: Gmail answered HTTP 503 and whether it did this is unknown.").then_some(()) }).await;
    only_gmail(&app, &gmail, &provider);
}
