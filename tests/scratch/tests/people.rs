//! Several people on one appliance, each with their own rights (E7, E2):
//! approval rights by risk tier, policy-change rights by layer, and tasks
//! bounded by the requester's own access. Where the owner names the ID
//! token's claims, a person's groups and roles come from the identity
//! provider at each sign-in, and every decision uses them.

use magnus_invariants::appliance::Person;
use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::{json, Value};
use std::time::Duration;

fn person(id: &str, groups: &[&str], roles: &[&str]) -> Person {
    Person { id: id.into(), name: id.into(), token: format!("tok-{id}-people"), groups: groups.iter().map(|s| s.to_string()).collect(), roles: roles.iter().map(|s| s.to_string()).collect() }
}

/// The task's own words, from the first message of its model call.
fn asked(c: &Conversation) -> String {
    c.messages().first().map(|m| m.to_string()).unwrap_or_default()
}

/// Tasks act by what they were asked: send through `mail` (irreversible),
/// pay through `post` (financial, a second mail server), or read the Q3
/// report (finance, open only to the finance group).
fn worker() -> magnus_invariants::model::Script {
    script(|c| async move {
        let a = asked(&c);
        let act = if a.contains("Pay") {
            tool("post__send", json!({ "to": "supplier@corp.example", "subject": "payment", "body": "EUR 120" }))
        } else if a.contains("report") {
            tool("finance__get_report", json!({ "quarter": "Q3" }))
        } else {
            tool("mail__send", json!({ "to": "bob@corp.example", "subject": "hi", "body": "hello" }))
        };
        match c.turn() {
            0 => contract_unchecked("do it", "done"),
            1 => act,
            _ => fail("done"),
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn approval_rights_follow_the_risk_tier_and_policy_rights_the_layer() {
    let (mail, post) = (mcp::mail().start().await, mcp::mail().start().await);
    let m = FakeModel::start(worker()).await;
    let mut spec = Spec::default()
        .app("mail", &mail, "public", "private", &[("send", "irreversible")])
        .app("post", &post, "public", "private", &[("send", "financial")]);
    spec.people.push(person("dana", &["staff"], &["approver:irreversible"]));
    spec.people.push(person("erin", &["staff", "finance"], &["policy:group:finance"]));
    let app = Appliance::start(spec, &m).await;
    let (bob, carol, dana, erin) = (app.api("bob"), app.api("carol"), app.api("dana"), app.api("erin"));

    // A financial action: dana may approve irreversible ones only.
    let pay = carol.submit("Pay the supplier EUR 120.").await;
    let item = carol.wait_item(&pay, "approval").await;
    let iid = item["id"].as_str().unwrap().to_string();
    let seen = dana.inbox().await.into_iter().find(|i| i["id"] == iid.as_str());
    assert!(seen.as_ref().is_none_or(|i| i["can_respond"] == false), "dana is offered a financial approval: {seen:?}");
    let st = dana.respond(&iid, json!({ "decision": "approve", "scope": "once" })).await;
    assert!(st == 403 || st == 404, "an irreversible-tier approver approved a financial action: {st}");
    never("an approval from the wrong tier took effect", Duration::from_secs(2), || async { post.effects("send") > 0 }).await;
    assert!(bob.respond(&iid, json!({ "decision": "approve", "scope": "once" })).await.is_success());
    eventually("the financial approver's approval took effect", PATIENCE, || async { (post.effects("send") == 1).then_some(()) }).await;

    // An irreversible action: dana's tier.
    let send = carol.submit("Email bob hello.").await;
    let item = carol.wait_item(&send, "approval").await;
    let iid = item["id"].as_str().unwrap().to_string();
    assert!(dana.inbox().await.iter().any(|i| i["id"] == iid.as_str() && i["can_respond"] == true), "dana cannot answer her own tier");
    assert!(dana.respond(&iid, json!({ "decision": "approve", "scope": "once" })).await.is_success());
    eventually("dana's approval took effect", PATIENCE, || async { (mail.effects("send") == 1).then_some(()) }).await;

    // Policy: a group's layer is changed by holders of that group's right only.
    let rule = r#"forbid(principal, action, resource == Tool::"post__send");"#;
    assert!(erin.put_policy("group/finance", rule).await.is_success(), "erin may set the finance group's layer");
    for (who, layer) in [(&erin, "group/staff"), (&erin, "org"), (&erin, "user/carol"), (&dana, "group/finance")] {
        let st = who.put_policy(layer, rule).await;
        assert!(st == 401 || st == 403, "{layer} accepted a policy without the right: {st}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_and_roles_come_from_keycloak_at_each_sign_in() {
    let (mail, finance) = (mcp::mail().start().await, mcp::finance().start().await);
    let m = FakeModel::start(worker()).await;
    let spec = Spec::default()
        .app("mail", &mail, "public", "private", &[("send", "irreversible")])
        .app("finance", &finance, "internal", "private", &[("get_report", "read")])
        .access("finance", &["finance"]);
    let mut app = Appliance::start(spec, &m).await;
    let kc = Keycloak::start().await;
    kc.add_person("carol").await;
    kc.configure_with(&mut app, &[("groups_claim", "/groups"), ("roles_claim", "/realm_access/roles")]).await;
    let alice = app.api("alice");
    // The organization lets the agent read the report; sending still asks.
    assert!(alice.put_policy("org", &org_policy(&permit_execute("finance__get_report"))).await.is_success());
    let sign_in = || async {
        let (st, v) = kc.app_sign_in(&app, "carol").await;
        assert_eq!(st, 200, "{v}");
        let mut a = app.api("carol");
        a.token = v["token"].as_str().unwrap().to_string();
        a
    };
    let groups = |me: &Value| {
        let mut g: Vec<String> = me["groups"].as_array().unwrap().iter().map(|g| g.as_str().unwrap().to_string()).collect();
        g.sort();
        g
    };
    let has_role = |me: &Value, r: &str| me["roles"].as_array().unwrap().iter().any(|x| x == r);

    // In the config carol is staff with no roles. In Keycloak she is in
    // finance and approves irreversible actions (group paths, realm roles).
    kc.set_groups("carol", &["staff", "finance"]).await;
    kc.set_roles("carol", &["approver:irreversible"]).await;
    let carol = sign_in().await;
    let me = carol.get("/v1/me").await.1;
    assert_eq!(groups(&me), ["finance", "staff"], "{me}");
    assert!(has_role(&me, "approver:irreversible"), "{me}");

    // Her task reaches what finance may, by her rights from the provider.
    let r = carol.submit("Get me the Q3 finance report.").await;
    carol.wait_settled(&r).await;
    assert_eq!(finance.calls_to("get_report").len(), 1, "the provider's group did not reach the boundary");
    // She approves an irreversible action on someone else's task.
    let send = alice.submit("Email bob hello.").await;
    let item = alice.wait_item(&send, "approval").await;
    assert!(carol.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success(), "the provider's role did not count");
    eventually("carol's approval took effect", PATIENCE, || async { (mail.effects("send") == 1).then_some(()) }).await;

    // Keycloak takes finance and the role away; at her next sign-in both are gone.
    kc.set_groups("carol", &["staff"]).await;
    kc.set_roles("carol", &[]).await;
    let carol = sign_in().await;
    let me: Value = carol.get("/v1/me").await.1;
    assert_eq!(groups(&me), ["staff"], "{me}");
    assert!(!has_role(&me, "approver:irreversible"), "{me}");
    let r = carol.submit("Get me the Q3 finance report.").await;
    carol.wait_settled(&r).await;
    assert_eq!(finance.calls_to("get_report").len(), 1, "a task used a group the provider took away");
    assert!(carol.open_items(&r, "approval").await.is_empty(), "an approval was offered to get around her rights");
    let send = alice.submit("Email bob hello again.").await;
    let item = alice.wait_item(&send, "approval").await;
    let st = carol.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await;
    assert!(st == 403 || st == 404, "a role the provider took away still approves: {st}");
    // It holds across a restart: it is in the log.
    app.restart().await;
    let me = app.api("carol").get("/v1/me").await.1;
    assert_eq!(groups(&me), ["staff"], "{me}");
    assert!(!has_role(&me, "approver:irreversible"), "{me}");
}
