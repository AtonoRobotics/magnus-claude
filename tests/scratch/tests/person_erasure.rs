//! A person deleted is gone for good (admin console spec, Users; connectors
//! spec, Isolation), against a real Keycloak: nothing the appliance kept for
//! them outlives them, so whoever is later known by their name starts with
//! nothing of theirs, and nobody else's sign-ins go with them.

use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::{json, Value};

async fn roles(alice: &Api, who: &str) -> Value {
    let (_, v) = alice.get("/v1/people").await;
    v["people"].as_array().unwrap().iter().find(|p| p["id"] == who).cloned().unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_deleted_keeps_no_roles_and_takes_only_their_own_sign_ins() {
    let m = FakeModel::start(steps(vec![])).await;
    // Someone whose name ends the way a sign-in of Bob's does.
    let mut spec = Spec::default();
    let mut ops = spec.people.iter().find(|p| p.id == "carol").unwrap().clone();
    (ops.id, ops.name, ops.token) = ("ops__bob".into(), "ops__bob".into(), format!("tok-ops-{}", std::process::id()));
    spec.people.push(ops);
    let mut app = Appliance::start(spec, &m).await;
    let kc = Keycloak::start().await;
    kc.add_person("bob").await;
    kc.configure(&mut app).await;
    let (st, v) = kc.app_sign_in(&app, "bob").await;
    assert_eq!(st, 200, "{v}");
    let mut bob = app.api("bob");
    bob.token = v["token"].as_str().unwrap().to_string();
    let (alice, ops) = (app.api("alice"), app.api("ops__bob"));

    // The owner makes Bob an admin; Bob and ops__bob each sign in to an app.
    assert_eq!(alice.put("/v1/people/bob/roles", json!({ "roles": ["admin"] })).await.0.as_u16(), 204);
    assert_eq!(roles(&alice, "bob").await["roles"], json!(["admin"]));
    let cal = FakeOAuthApp::start(3600).await;
    let (st, v) = alice.post("/v1/connectors", json!({ "name": "calendar", "url": cal.url, "audience": "private", "label": "private" })).await;
    assert!(st.is_success(), "{v}");
    for api in [&bob, &ops] {
        let (st, v) = api.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
        assert!(st.is_success(), "{v}");
        let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
        assert_eq!(api.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    }

    // Root deletes Bob, as the console names him: by his Keycloak account.
    let (_, people) = control(&app, "GET", "/v1/people", Value::Null).await;
    let sub = people["people"].as_object().unwrap().keys().next().expect("bob's account is bound").clone();
    let (st, v) = control(&app, "POST", &format!("/v1/people/{sub}/erase"), json!({ "actor": "admin:root" })).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["sign_ins_revoked"], 1, "{v}");

    // Bob keeps no role the owner gave him: whoever is next known as Bob
    // starts from the organisation's config, not as an admin.
    let after = roles(&alice, "bob").await;
    assert_eq!(after["roles_from"], "config", "{after}");
    assert!(!after["roles"].as_array().unwrap().iter().any(|r| r == "admin"), "bob is still an admin after he was deleted: {after}");
    // ops__bob's sign-in was not Bob's.
    let (_, c) = ops.get("/v1/connectors/calendar").await;
    assert_eq!(c["state"], "connected", "deleting bob ended ops__bob's sign-in: {c}");
}
