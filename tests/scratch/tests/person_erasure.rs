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
    // Bob also signs in to an app whose name ends in `_`, as names may.
    let (cal, notes) = (FakeOAuthApp::start(3600).await, FakeOAuthApp::start(3600).await);
    for (name, url) in [("calendar", &cal.url), ("notes_", &notes.url)] {
        let (st, v) = alice.post("/v1/connectors", json!({ "name": name, "url": url, "audience": "private", "label": "private" })).await;
        assert!(st.is_success(), "{v}");
    }
    for (api, name) in [(&bob, "calendar"), (&ops, "calendar"), (&bob, "notes_")] {
        let (st, v) = api.post(&format!("/v1/connectors/{name}/sign-in"), json!({ "redirect_uri": "http://127.0.0.1:9/callback" })).await;
        assert!(st.is_success(), "{v}");
        let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
        assert_eq!(api.post(&format!("/v1/connectors/{name}/sign-in/complete"), json!({ "state": state, "code": code })).await.0, 204);
    }

    // Root deletes Bob, as the console names him: by his Keycloak account.
    let (_, people) = control(&app, "GET", "/v1/people", Value::Null).await;
    let sub = people["people"].as_object().unwrap().keys().next().expect("bob's account is bound").clone();
    let (st, v) = control(&app, "POST", &format!("/v1/people/{sub}/erase"), json!({ "actor": "admin:root" })).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["sign_ins_revoked"], 2, "bob's two sign-ins, and only his: {v}");
    assert!(!notes.state.lock().unwrap().revoked.is_empty(), "bob's sign-in to notes_ was not revoked at the app");

    // Bob keeps no role the owner gave him: whoever is next known as Bob
    // starts from the organisation's config, not as an admin.
    let after = roles(&alice, "bob").await;
    assert_eq!(after["roles_from"], "config", "{after}");
    assert!(!after["roles"].as_array().unwrap().iter().any(|r| r == "admin"), "bob is still an admin after he was deleted: {after}");
    // ops__bob's sign-in was not Bob's.
    let (_, c) = ops.get("/v1/connectors/calendar").await;
    assert_eq!(c["state"], "connected", "deleting bob ended ops__bob's sign-in: {c}");
}

/// Bob as he signs in at the realm now: the account Keycloak has for him.
async fn signed_in(kc: &Keycloak, app: &Appliance) -> Api {
    let (st, v) = kc.app_sign_in(app, "bob").await;
    assert_eq!(st, 200, "{v}");
    let mut bob = app.api("bob");
    bob.token = v["token"].as_str().unwrap().to_string();
    bob
}

/// Whether the task's update is asked about (true) or runs (false).
async fn asked(api: &Api, t: &str, crm: &FakeMcp, before: usize) -> bool {
    eventually("the update is asked about or runs", PATIENCE, || {
        let (api, t) = (api.clone(), t.to_string());
        async move {
            if crm.effects("update_record") > before {
                return Some(false);
            }
            (!api.open_items(&t, "approval").await.is_empty()).then_some(true)
        }
    })
    .await
}

/// A person deleted takes the approvals given to them (#191): the owner's
/// standing approval of Bob's update covers nobody later known as Bob, a
/// new Keycloak account bound to his name, neither live nor once the state
/// is rebuilt from the log. What Bob approved for Carol stays hers: it was
/// valid when given and is about her work.
#[tokio::test(flavor = "multi_thread")]
async fn a_person_deleted_takes_the_approvals_given_to_them_but_not_those_they_gave() {
    let crm = mcp::crm().start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("close the record", "the record is closed"),
            1 => tool("crm__update_record", json!({ "id": "42", "fields": { "status": "closed" } })),
            _ => complete("closed"),
        }
    }))
    .await;
    let mut app = Appliance::start(Spec::default().app("crm", &crm, "internal", "private", &[("update_record", "irreversible")]), &m).await;
    let kc = Keycloak::start().await;
    kc.add_person("bob").await;
    kc.configure(&mut app).await;
    let (alice, carol) = (app.api("alice"), app.api("carol"));
    let bob = signed_in(&kc, &app).await;

    // The owner approves Bob's update for good; Bob approves Carol's for an hour.
    let t = bob.submit("Close record 42.").await;
    let item = bob.wait_item(&t, "approval").await;
    assert!(alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "standing" })).await.is_success());
    bob.wait_settled(&t).await;
    assert_eq!(crm.effects("update_record"), 1);
    let t = carol.submit("Close record 42.").await;
    let item = carol.wait_item(&t, "approval").await;
    assert!(bob.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "window", "minutes": 60 })).await.is_success());
    carol.wait_settled(&t).await;
    assert_eq!(crm.effects("update_record"), 2);

    // Root deletes Bob.
    let (_, people) = control(&app, "GET", "/v1/people", Value::Null).await;
    let old = people["people"].as_object().unwrap().keys().next().expect("bob's account is bound").clone();
    let (st, v) = control(&app, "POST", &format!("/v1/people/{old}/erase"), json!({ "actor": "admin:root" })).await;
    assert_eq!(st, 200, "{v}");

    // What Bob approved for Carol still covers her update.
    let t = carol.submit("Close record 42 again.").await;
    assert!(!asked(&carol, &t, &crm, 2).await, "deleting bob took back the approval he gave carol");
    carol.wait_settled(&t).await;

    // Bob's account is made again: a new account, bound to his name.
    kc.delete("bob").await;
    kc.add_person("bob").await;
    let bob = signed_in(&kc, &app).await;
    let (_, people) = control(&app, "GET", "/v1/people", Value::Null).await;
    assert!(!people["people"].as_object().unwrap().contains_key(&old), "the old account is still bound: {people}");
    let n = crm.effects("update_record");
    let t = bob.submit("Close record 42 again.").await;
    assert!(asked(&bob, &t, &crm, n).await, "an approval given to the deleted bob covered the new account known as bob");

    // And once the state is rebuilt from the log.
    app.restart().await;
    let bob = signed_in(&kc, &app).await;
    let t = bob.submit("Close record 42 once more.").await;
    assert!(asked(&bob, &t, &crm, n).await, "after a restart, an approval given to the deleted bob covered the new bob");
    assert_eq!(crm.effects("update_record"), n);
}
