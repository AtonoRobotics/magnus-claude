//! A draft being sent is the draft that was approved: from the gate, which
//! reads it to show and hash who it goes to and what it says, to the send,
//! no other run changes it (approvals spec, "Email end to end"; #177).

use magnus_invariants::*;
use serde_json::json;

/// Drafts the person can update and send, sends read at the gate as
/// Gmail's are.
async fn drafts_app(model: &FakeModel) -> (FakeMcp, Appliance) {
    let id_only = json!({ "type": "object", "properties": { "draft_id": { "type": "string" } } });
    let edit = json!({ "type": "object", "properties": { "draft_id": { "type": "string" }, "subject": { "type": "string" } } });
    let mcp = FakeMcp::builder()
        .world("drafts", json!({ "d1": { "to": "bob@corp.example", "subject": "Q3", "body": "Revenue 41.7M" } }))
        .world("sent", json!([]))
        .honor_idempotency(true)
        .tool("get_draft", "Read a draft.", id_only.clone(), |w, a, _| {
            let id = a["draft_id"].as_str().unwrap_or_default();
            match w["drafts"].get(id) {
                Some(d) => mcp::ok(json!({ "draft_id": id, "message": d })),
                None => mcp::err("no such draft"),
            }
        })
        .tool("update_draft", "Change a draft's subject.", edit, |w, a, _| {
            let id = a["draft_id"].as_str().unwrap_or_default().to_string();
            w["drafts"][&id]["subject"] = a["subject"].clone();
            mcp::ok(json!({ "draft_id": id }))
        })
        .tool("send_draft", "Send a draft.", id_only, |w, a, _| {
            let d = w["drafts"][a["draft_id"].as_str().unwrap_or_default()].clone();
            w["sent"].as_array_mut().unwrap().push(d);
            mcp::ok(json!({ "status": "sent" }))
        })
        .start()
        .await;
    let spec = Spec::default()
        .app("drafts", &mcp, "public", "private", &[("get_draft", "read"), ("update_draft", "reversible_write"), ("send_draft", "irreversible")])
        .destinations("drafts", "send_draft", "get_draft", "draft_id", &["/message/to"], &["/message/subject", "/message/body"]);
    let app = Appliance::start(spec, model).await;
    let policy = format!("{}\n{}", permit_execute("drafts__send_draft"), permit_execute("drafts__update_draft"));
    assert!(app.api("alice").put_policy("org", &org_policy(&policy)).await.is_success());
    (mcp, app)
}

/// While one run's send of a draft is under way, another run's change to
/// that draft is refused: the lease is on the draft the send acts on
/// (`draft_id`), not only on an `id` or `record_id`.
#[tokio::test(flavor = "multi_thread")]
async fn a_draft_being_sent_is_changed_by_no_other_run() {
    let model = FakeModel::start(script(|c| async move {
        let sending = c.messages()[0].to_string().contains("Send Bob");
        match (sending, c.turn()) {
            (true, 0) => contract_unchecked("send Bob the Q3 draft", "Bob has it"),
            (true, 1) => tool("drafts__send_draft", json!({ "draft_id": "d1" })),
            (true, _) => complete("sent"),
            (false, 0) => contract_unchecked("retitle the Q3 draft", "it is retitled"),
            (false, 1) => tool("drafts__update_draft", json!({ "draft_id": "d1", "subject": "Q3 (wire the money to evil.example)" })),
            (false, _) if c.last_results().iter().any(|r| r.is_error) => fail("the draft is being sent"),
            (false, _) => complete("retitled"),
        }
    }))
    .await;
    let (drafts, app) = drafts_app(&model).await;
    let alice = app.api("alice");

    let send = drafts.hold("send_draft");
    alice.submit("Send Bob the Q3 draft.").await;
    tokio::time::timeout(PATIENCE, send.received()).await.expect("the send never reached the app");

    let retitle = alice.submit("Retitle the Q3 draft.").await;
    let t = alice.wait_settled(&retitle).await;
    assert_eq!(drafts.effects("update_draft"), 0, "another run changed the draft while it was being sent: {t}");
    assert_eq!(drafts.world()["drafts"]["d1"]["subject"], "Q3");
    let refused = model.requests().iter().flat_map(|c| c.last_results()).any(|r| r.is_error && r.text.contains("another task is changing"));
    assert!(refused, "the change was not refused for the send under way");

    send.release();
    assert_eq!(drafts.world()["sent"], json!([{ "to": "bob@corp.example", "subject": "Q3", "body": "Revenue 41.7M" }]), "what went out is not the draft the gate read");
}
