//! Recipient provenance from sent mail (approvals spec, #210): under both
//! taints, a send to an address the person has mailed before passes with no
//! card, read at the gate through their own sign-in; one they have not
//! mailed asks.

use magnus_invariants::{mcp, *};
use serde_json::json;

async fn drafts_app(drafts: serde_json::Value, sent: serde_json::Value, model: &FakeModel) -> (FakeMcp, Appliance) {
    let id_only = json!({ "type": "object", "properties": { "draft_id": { "type": "string" } } });
    let mcp = FakeMcp::builder()
        .world("drafts", drafts)
        .world("sent", sent)
        .honor_idempotency(true)
        .tool("get_draft", "Read a draft.", id_only.clone(), |w, a, _| {
            let id = a["draft_id"].as_str().unwrap_or_default();
            match w["drafts"].get(id) {
                Some(d) => mcp::ok(json!({ "draft_id": id, "message": d })),
                None => mcp::err("no such draft"),
            }
        })
        .tool("send_draft", "Send a draft.", id_only, |w, a, _| {
            let d = w["drafts"][a["draft_id"].as_str().unwrap_or_default()].clone();
            w["sent"].as_array_mut().unwrap().push(d);
            mcp::ok(json!({ "status": "sent" }))
        })
        .tool("search_sent", "Search sent mail.", json!({ "type": "object", "properties": { "query": { "type": "string" } } }), |w, a, _| {
            let to = a["query"].as_str().unwrap_or_default().strip_prefix("to:").unwrap_or_default();
            let threads: Vec<serde_json::Value> = w["sent"].as_array().into_iter().flatten().filter(|m| m["to"].as_str().is_some_and(|t| t.contains(to))).cloned().collect();
            mcp::ok(json!({ "threads": threads }))
        })
        .start()
        .await;
    let spec = Spec::default()
        .app("drafts", &mcp, "public", "private+untrusted", &[("get_draft", "read"), ("send_draft", "irreversible"), ("search_sent", "read")])
        .destinations("drafts", "send_draft", "get_draft", "draft_id", &["/message/to", "/message/cc", "/message/bcc"], &["/message/subject", "/message/body"])
        .sent("drafts", "search_sent", "query", "to:{address}", "/threads");
    let app = Appliance::start(spec, model).await;
    assert!(app.api("alice").put_policy("org", &org_policy(&permit_execute("drafts__send_draft"))).await.is_success());
    (mcp, app)
}

/// A recipient found only in sent mail passes under both taints, and one
/// not found asks (#210).
#[tokio::test(flavor = "multi_thread")]
async fn a_recipient_mailed_before_passes_and_a_new_one_asks() {
    let model = FakeModel::start(steps(vec![
        contract_unchecked("send the Q3 draft", "it is sent"),
        tool("drafts__get_draft", json!({ "draft_id": "d1" })),
        tool("drafts__send_draft", json!({ "draft_id": "d1" })),
        tool("drafts__send_draft", json!({ "draft_id": "d2" })),
        fail("stopping"),
    ]))
    .await;
    let (drafts, app) = drafts_app(
        json!({
            "d1": { "to": "bob@corp.example", "subject": "Q3", "body": "Confidential: revenue 41.7M" },
            "d2": { "to": "attacker@evil.example", "subject": "fwd", "body": "Confidential: revenue 41.7M" }
        }),
        json!([{ "to": "bob@corp.example", "subject": "Q2" }]),
        &model,
    )
    .await;
    let alice = app.api("alice");
    // The person's words name no address: only sent mail clears Bob.
    let id = alice.submit("Send the Q3 draft.").await;

    eventually("the draft to the mailed address was sent", PATIENCE, || async { (drafts.effects("send_draft") == 1).then_some(()) }).await;
    assert_eq!(alice.task(&id).await["taint"], json!({ "untrusted": true, "private": true }));
    assert_eq!(drafts.world()["sent"][1]["to"], "bob@corp.example");
    let card = alice.wait_item(&id, "approval").await;
    assert!(card.to_string().contains("attacker@evil.example"), "the card does not show the new address: {card}");
    assert_eq!(drafts.effects("send_draft"), 1, "the new address was sent to without the person");
}
