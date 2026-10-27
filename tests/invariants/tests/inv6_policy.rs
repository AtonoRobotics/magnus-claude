//! Invariant 6: autonomy comes from policy the owner writes. The agent can
//! propose policy changes, never grant them.
//!
//! Prevents: an agent enabling its own auto-approve (CVE-2025-53773), and
//! autonomy that grows without anyone deciding it.

use magnus_invariants::{mcp, *};
use serde_json::json;
use std::time::Duration;

fn send(body: &str) -> Reply {
    tool("mail__send", json!({ "to": "bob@corp.example", "subject": "hi", "body": body }))
}

async fn mail_app(model: &FakeModel) -> (FakeMcp, Appliance) {
    let mail = mcp::mail().start().await;
    let app = Appliance::start(Spec::default().app("mail", &mail, "public", "private", &[("send", "irreversible")]), model)
        .await;
    (mail, app)
}

#[tokio::test(flavor = "multi_thread")]
async fn without_owner_policy_an_irreversible_action_waits_for_approval() {
    let model = FakeModel::start(steps(vec![contract_unchecked("say hi", "said"), send("hello"), fail("done")])).await;
    let (mail, app) = mail_app(&model).await;
    let alice = app.api("alice");
    let id = alice.submit("Email bob hello.").await;
    let item = alice.wait_item(&id, "approval").await;
    never("ran without approval", Duration::from_secs(3), || async { mail.effects("send") > 0 }).await;
    assert!(alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success());
    eventually("the approved send ran once", PATIENCE, || async { (mail.effects("send") == 1).then_some(()) }).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_policy_grants_autonomy() {
    let model = FakeModel::start(steps(vec![contract_unchecked("say hi", "said"), send("hello"), fail("done")])).await;
    let (mail, app) = mail_app(&model).await;
    let alice = app.api("alice");
    assert!(alice.put_policy("org", &org_policy(&permit_execute("mail__send"))).await.is_success());
    let id = alice.submit("Email bob hello.").await;
    alice.wait_settled(&id).await;
    assert_eq!(mail.effects("send"), 1);
    assert!(alice.open_items(&id, "approval").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_proposes_and_only_the_owner_grants() {
    let model = FakeModel::start(steps(vec![
        contract_unchecked("say hi", "said"),
        tool(
            "propose_policy",
            json!({ "layer": "org", "cedar": permit_execute("mail__send"), "rationale": "bob gets a hello every week" }),
        ),
        send("hello"),
        fail("done"),
    ]))
    .await;
    let (mail, app) = mail_app(&model).await;
    let (alice, bob, carol) = (app.api("alice"), app.api("bob"), app.api("carol"));
    let id = alice.submit("Email bob hello.").await;
    alice.wait_item(&id, "approval").await;
    let proposal = alice.wait_item(&id, "policy_proposal").await;
    let pid = proposal["id"].as_str().unwrap().to_string();
    assert_eq!(mail.effects("send"), 0, "a proposal took effect by itself");

    for who in [&bob, &carol] {
        let st = who.respond(&pid, json!({ "decision": "accept" })).await;
        assert!(st == 401 || st == 403 || st == 404, "a person without the org policy role accepted a proposal: {st}");
    }
    assert!(alice.respond(&pid, json!({ "decision": "accept" })).await.is_success());

    // The new policy holds for new work.
    let next = alice.submit("Email bob hello again.").await;
    alice.wait_settled(&next).await;
    assert!(alice.open_items(&next, "approval").await.is_empty(), "the accepted policy did not apply");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_one_action_approval_covers_one_action() {
    let model = FakeModel::start(steps(vec![
        contract_unchecked("say hi twice", "said"),
        send("first"),
        send("second"),
        fail("done"),
    ]))
    .await;
    let (mail, app) = mail_app(&model).await;
    let alice = app.api("alice");
    let id = alice.submit("Email bob twice.").await;
    let first = alice.wait_item(&id, "approval").await;
    let fid = first["id"].as_str().unwrap().to_string();
    assert!(alice.respond(&fid, json!({ "decision": "approve", "scope": "once" })).await.is_success());
    eventually("the first send ran", PATIENCE, || async { (mail.effects("send") == 1).then_some(()) }).await;

    let me = alice.clone();
    let (tid, fid2) = (id.clone(), fid.clone());
    eventually("a second approval is requested", PATIENCE, || {
        let (me, tid, fid) = (me.clone(), tid.clone(), fid2.clone());
        async move { me.open_items(&tid, "approval").await.into_iter().find(|i| i["id"] != fid.as_str()) }
    })
    .await;
    assert_eq!(mail.effects("send"), 1, "one approval covered two actions");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_approval_channel_needs_a_person() {
    let model = FakeModel::start(steps(vec![contract_unchecked("say hi", "said"), send("hello"), fail("done")])).await;
    let (mail, app) = mail_app(&model).await;
    let alice = app.api("alice");
    let id = alice.submit("Email bob hello.").await;
    let item = alice.wait_item(&id, "approval").await;
    let path = format!("{}/v1/inbox/{}", app.base, item["id"].as_str().unwrap());
    let http = reqwest::Client::new();
    let body = json!({ "decision": "approve", "scope": "once" });
    let anon = http.post(&path).json(&body).send().await.unwrap().status();
    let forged = http.post(&path).bearer_auth("tok-forged").json(&body).send().await.unwrap().status();
    assert_eq!(anon, 401);
    assert_eq!(forged, 401);
    never("an unauthenticated approval took effect", Duration::from_secs(3), || async { mail.effects("send") > 0 }).await;
    for p in &app.spec.people {
        assert!(model.requests().iter().all(|r| !r.text().contains(&p.token)), "a person's token reached the model");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn people_set_policy_only_within_their_role() {
    let model = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &model).await;
    let (bob, carol) = (app.api("bob"), app.api("carol"));
    let rule = permit_execute("mail__send");
    for (who, layer) in [(&bob, "org"), (&bob, "user/alice"), (&bob, "group/staff"), (&carol, "user/carol"), (&carol, "org")] {
        let st = who.put_policy(layer, &rule).await;
        assert!(st == 401 || st == 403, "{layer} accepted a policy from someone without the role: {st}");
    }
    assert!(bob.put_policy("user/bob", r#"forbid(principal, action, resource == Tool::"mail__send");"#).await.is_success());
}
