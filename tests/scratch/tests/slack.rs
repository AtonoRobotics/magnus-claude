//! Slack's connector, against Slack: no fakes (owner, 2026-11-01). The
//! catalog's Slack is in every Library, and a person signs in through
//! Alpha Vector's own Slack app at Slack's hosted MCP server. The secret
//! is provisioned to the appliance, as Google's is; these tests never
//! reach the token exchange, so they provision a stand-in for it.

use magnus_invariants::*;
use magnus_scratch::FakeOAuthApp;
use serde_json::json;
use std::collections::HashMap;

fn client() -> (String, String) {
    let path = magnus_invariants::appliance::workspace_root().join("catalog/registrations.toml");
    let r: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    let id = r["slack"]["client_id"].as_str().expect("catalog/registrations.toml names Alpha Vector's Slack client").to_string();
    (id, "not-used-before-the-token-exchange".into())
}

fn manifest() -> toml::Table {
    let path = magnus_invariants::appliance::workspace_root().join("catalog/slack.toml");
    std::fs::read_to_string(&path).unwrap().parse().unwrap()
}

/// Slack is in every Library. Until the secret is provisioned the card says
/// it is not ready. With it, a person's sign-in starts at Slack's own page,
/// for exactly the manifest's access, with PKCE, and the answer comes back
/// to the one address Slack has registered (it matches the port).
#[tokio::test(flavor = "multi_thread")]
async fn slack_is_in_every_library_and_signs_in_at_slack_s_own_page() {
    let (id, secret) = client();
    let m = manifest();
    let scopes: Vec<String> = m["scopes"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
    let port = m["loopback"].as_integer().unwrap();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let model = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &model).await;
    let (alice, carol) = (app.api("alice"), app.api("carol"));

    let (_, list) = carol.get("/v1/connectors").await;
    let slack = list["connectors"].as_array().unwrap().iter().find(|c| c["name"] == "slack").cloned().unwrap_or_else(|| panic!("slack is not in carol's Library: {list}"));
    assert_eq!((slack["title"].as_str(), slack["state"].as_str(), slack["catalog"].as_bool(), slack["ready"].as_bool(), slack["loopback"].as_i64()), (Some("Slack"), Some("available"), Some(true), Some(false), Some(port)), "{slack}");
    let (st, v) = carol.post("/v1/connectors/slack/sign-in", json!({ "redirect_uri": redirect })).await;
    assert!(!st.is_success(), "a sign-in started with no secret provisioned: {v}");

    assert_eq!(carol.put("/v1/registrations/slack", json!({ "client_secret": secret })).await.0, 403);
    assert_eq!(alice.put("/v1/registrations/slack", json!({ "client_secret": secret })).await.0, 204);
    let (_, all) = alice.get("/v1/connectors").await;
    assert!(!all.to_string().contains(&secret), "the client secret is shown: {all}");

    let (st, v) = carol.post("/v1/connectors/slack/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    assert!(!st.is_success() && v.to_string().contains(&redirect), "a sign-in on another port was accepted: {v}");

    let (st, v) = carol.post("/v1/connectors/slack/sign-in", json!({ "redirect_uri": redirect })).await;
    assert!(st.is_success(), "Magnus could not find Slack's sign-in: {v}");
    let u = url::Url::parse(v["authorize_url"].as_str().unwrap()).unwrap();
    assert_eq!(u.host_str(), Some("slack.com"), "{u}");
    assert_eq!(u.path(), "/oauth/v2_user/authorize", "{u}");
    let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
    assert_eq!(q.get("client_id"), Some(&id), "{u}");
    assert_eq!(q.get("redirect_uri"), Some(&redirect), "{u}");
    assert_eq!(q.get("code_challenge_method").map(String::as_str), Some("S256"), "{u}");
    assert_eq!(q.get("resource").map(String::as_str), Some("https://mcp.slack.com"), "{u}");
    assert_eq!(q.get("scope").map(|s| s.split(' ').map(String::from).collect::<Vec<_>>()), Some(scopes), "{u}");

    // Slack answers the address Magnus built with its own page, not an error.
    let page = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).user_agent("Mozilla/5.0 (X11; Linux x86_64) Magnus catalog test").build().unwrap().get(u.as_str()).send().await.unwrap();
    let to = page.headers().get("location").and_then(|l| l.to_str().ok()).unwrap_or_default().to_string();
    assert!(page.status().is_success() || page.status().is_redirection(), "Slack answered {}", page.status());
    assert!(!to.contains("error="), "Slack refused the sign-in request: {to}");

    // The card says what Magnus can do, and never a protocol word.
    let (st, c) = carol.get("/v1/connectors/slack").await;
    assert!(st.is_success(), "{c}");
    assert_eq!(c["summary"], m["summary"].as_str().unwrap(), "{c}");
    let shown = format!("{} {} {}", c["summary"], c["allows"], c["listing"]);
    for word in ["oauth", "scope", "token", "mcp", "port"] {
        assert!(!shown.to_lowercase().split(|ch: char| !ch.is_alphanumeric()).any(|w| w == word), "carol's card says {word:?}: {shown}");
    }
}

/// Slack answers a dead refresh with HTTP 200 and `ok: false`, not an HTTP
/// error. That sign-in needs the person again, and the dead grant is not
/// redeemed a second time.
#[tokio::test(flavor = "multi_thread")]
async fn a_dead_slack_refresh_marks_the_sign_in_as_needing_attention_and_is_not_redeemed_again() {
    let cal = FakeOAuthApp::start(30).await;
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    assert_eq!(alice.post("/v1/connectors", json!({ "name": "calendar", "url": cal.url, "audience": "private", "label": "private" })).await.0, 201);
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (_, v) = alice.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": format!("http://127.0.0.1:{port}/callback") })).await;
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(alice.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);

    cal.state.lock().unwrap().dead_refresh = true;
    let (_, list) = alice.get("/v1/connectors").await;
    let card = list["connectors"].as_array().unwrap().iter().find(|c| c["name"] == "calendar").unwrap();
    assert_eq!(card["state"], "needs_attention", "{card}");
    let once = cal.state.lock().unwrap().refreshes;
    assert_eq!(once, 1, "the dead refresh was not tried");
    let _ = alice.get("/v1/connectors").await;
    assert_eq!(cal.state.lock().unwrap().refreshes, 1, "a dead refresh was redeemed again");
}
