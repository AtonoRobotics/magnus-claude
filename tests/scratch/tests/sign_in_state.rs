//! A sign-in's state outlives a restart and is good once (connectors spec,
//! Connect flow: "it signs a state naming the appliance, user, connector,
//! a nonce and an expiry"). Carol starts connecting her calendar, the
//! appliance restarts while she is on the app's page, and her sign-in
//! still completes; the same answer again is refused, as is a state
//! someone changed, and a state of hers finished by Bob. Unit tests in
//! connectors.rs (`sign_in_state`).

use magnus_invariants::*;
use magnus_scratch::FakeOAuthApp;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_survives_a_restart_and_its_state_is_good_once_and_only_for_its_person() {
    let cal = FakeOAuthApp::start(3600).await;
    let m = FakeModel::start(steps(vec![])).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    let (st, v) = app.api("alice").post("/v1/connectors", json!({ "name": "calendar", "url": cal.url, "audience": "private", "label": "private+untrusted" })).await;
    assert!(st.is_success(), "{v}");

    let start = |app: &Appliance, who: &str| {
        let api = app.api(who);
        async move {
            let (st, v) = api.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
            assert!(st.is_success(), "{v}");
            FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await
        }
    };
    let (code, state) = start(&app, "carol").await;
    // The state is the appliance's signed claims, not a bare random value.
    assert_eq!(state.matches('.').count(), 1, "{state}");

    // The appliance restarts while Carol is on the app's page.
    app.restart().await;
    let carol = app.api("carol");
    // Bob cannot finish Carol's sign-in, and a changed state is refused.
    let (st, _) = app.api("bob").post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await;
    assert_eq!(st.as_u16(), 400, "Bob finished Carol's sign-in");
    let changed = format!("{}x", state);
    let (st, _) = carol.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": changed, "code": code })).await;
    assert_eq!(st.as_u16(), 400, "a changed state was accepted");
    // Hers completes after the restart.
    let (st, e) = carol.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await;
    assert_eq!(st.as_u16(), 204, "the sign-in did not survive the restart: {e}");
    let (_, c) = carol.get("/v1/connectors/calendar").await;
    assert_eq!(c["state"], "connected", "{c}");
    // Good once.
    let (st, e) = carol.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await;
    assert_eq!(st.as_u16(), 400, "the same state finished a second sign-in: {e}");
    assert!(e.to_string().contains("already finished"), "{e}");
}
