//! The Library and an app's page as a person's connection changes
//! (connectors spec, Library and The connector card), with a real sign-in
//! and real calls: connected, the card shows a check and the account, and
//! the app's page shows the account, when it was connected and last used,
//! the recent actions from the log, and Disconnect; once the app refuses
//! the sign-in, the card and the page ask to sign in again, and Discover
//! lists it under Needs attention. In every look and at phone width.

use magnus_invariants::*;
use magnus_scratch::FakeOAuthApp;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn the_library_follows_a_connection_from_connected_to_needing_a_new_sign_in() {
    // 30-second tokens: every use refreshes, so a refused sign-in shows at once.
    let cal = FakeOAuthApp::start(30).await;
    cal.state.lock().unwrap().account = Some("carol@calendar.test".into());
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("list the events", "the events are listed"),
            1 => tool("calendar__list_events", json!({})),
            _ => complete("listed"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let (st, v) = alice.post("/v1/connectors", json!({ "name": "calendar", "url": cal.url, "audience": "private", "label": "private+untrusted" })).await;
    assert!(st.is_success(), "{v}");
    assert_eq!(alice.put("/v1/connectors/calendar/tools/list_events", json!({ "risk": "read" })).await.0, 204);
    let carol = app.api("carol");
    assert!(magnus_scratch::set_password(&app, "carol", magnus_scratch::PASSWORD));
    let (_, v) = carol.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(carol.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    let task = carol.submit("What is on my calendar?").await;
    carol.wait_item(&task, "review").await;
    assert_eq!(cal.calls("list_events"), 1);
    let (_, c) = carol.get("/v1/connectors/calendar").await;
    assert_eq!((c["state"].as_str(), c["account"].as_str()), (Some("connected"), Some("carol@calendar.test")), "{c}");
    assert!(c["last_used"].is_string() && c["connected_at"].is_string() && c["recent"][0]["action"] == "list events", "{c}");

    let browse = |phase: &'static str| {
        let app_base = app.base.clone();
        let task = task.clone();
        async move {
            let out = tokio::process::Command::new("node")
                .arg(magnus_invariants::appliance::workspace_root().join("desktop/tests/library-states.mjs"))
                .env("MAGNUS_URL", app_base)
                .env("MAGNUS_PERSON", "carol")
                .env("MAGNUS_PASSWORD", magnus_scratch::PASSWORD)
                .env("MAGNUS_APP", "calendar")
                .env("MAGNUS_TASK", task)
                .env("MAGNUS_PHASE", phase)
                .env("MAGNUS_CHROMIUM", magnus_scratch::chromium())
                .envs(magnus_scratch::shots())
                .output()
                .await
                .unwrap();
            assert!(out.status.success(), "the Library, {phase}:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        }
    };
    browse("connected").await;

    // Carol removes Magnus in the app's own settings.
    cal.revoke_everything();
    let (_, c) = carol.get("/v1/connectors/calendar").await;
    assert_eq!(c["state"], "needs_attention", "{c}");
    browse("needs_attention").await;
}
