//! The org context bundle, step 1 (org context build spec, Bundle format;
//! tests T9, T10 and T2 for the bundle's own reads): Magnus writes OKF
//! documents through `bundle_write`, every write checked by the kernel and a
//! malformed one refused with the reason (nothing stored); each write is a
//! new version; a type is made stable only by an admin; a person's own
//! documents are theirs alone; an org document drawn from items in the org's
//! systems is seen only by whom the kernel knows may read them, and links to
//! it leave other people's view of the index.

use magnus_invariants::*;
use serde_json::{json, Value};

const ALICE: &str = "Map the CRM for the bundle.";
const BOB: &str = "Look up Acme in the bundle.";

fn gen() -> Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

fn customer(status: &str) -> Value {
    json!({ "path": "/types/customer.md", "frontmatter": { "type": "Entity Type", "title": "Customer", "description": "A company that buys from us.", "generated": gen(), "status": status,
        "system_of_record": { "source": "/sources/hubspot.md", "locator": "hubspot://company/*" }, "identity_keys": ["hubspot_id", "domain"], "relations": ["owned_by"], "cedar_entity": "Customer", "scope": "org" }, "body": "A customer is a company with a closed deal." })
}

fn acme(identity: Value) -> Value {
    json!({ "path": "/entities/customer/acme.md", "frontmatter": { "type": "Customer", "title": "Acme", "resource": "hubspot://company/42", "sources": [{ "resource": "hubspot://company/42" }],
        "generated": gen(), "identity": identity, "relations": [{ "type": "owned_by", "to": "/people/sales.md" }], "scope": "org" }, "body": "Acme's record is in HubSpot." })
}

#[tokio::test(flavor = "multi_thread")]
async fn magnus_writes_the_bundle_through_the_kernel_which_checks_every_write_and_guards_who_sees_what() {
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        if body.contains(ALICE) {
            let steps: Vec<Reply> = vec![
                contract_unchecked("map the CRM", "the bundle maps it"),
                tool("bundle_write", customer("draft")),
                tool("bundle_write", customer("stable")),
                tool("bundle_write", acme(json!({ "vat": "GB1" }))),
                tool("bundle_write", acme(json!({ "domain": "acme.example" }))),
                tool("bundle_write", acme(json!({ "domain": "acme.example", "hubspot_id": "42" }))),
                tool("bundle_write", json!({ "path": "/index.md", "frontmatter": { "okf_version": "0.2" }, "body": "# Customers\n* [Customer](/types/customer.md) - what a customer is\n* [Acme](/entities/customer/acme.md) - our largest customer\n" })),
                tool("bundle_write", json!({ "path": "/index.md", "frontmatter": {}, "body": format!("# All\n{}", "* [x](/sources/x.md) - a source\n".repeat(600)) })),
                tool("bundle_write", json!({ "path": "/users/alice/notes.md", "frontmatter": { "type": "Note", "scope": "user", "owner_user": "alice" }, "body": "Alice prefers short answers." })),
                tool("bundle_write", json!({ "path": "/users/bob/notes.md", "frontmatter": { "type": "Note", "scope": "user", "owner_user": "bob" }, "body": "Planted." })),
                tool("bundle_read", json!({ "path": "/entities/customer/acme.md" })),
                tool("task_complete", json!({ "summary": "Mapped.", "deliverable": [{ "type": "text", "text": "Mapped." }] })),
            ];
            return steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })));
        }
        if body.contains(BOB) {
            let steps: Vec<Reply> = vec![
                contract_unchecked("look up Acme", "found or not"),
                tool("bundle_read", json!({ "path": "/entities/customer/acme.md" })),
                tool("bundle_read", json!({ "path": "/users/alice/notes.md" })),
                tool("bundle_read", json!({ "path": "/index.md" })),
                tool("bundle_search", json!({ "query": "acme" })),
                tool("bundle_read", json!({ "path": "/types/customer.md" })),
                tool("task_complete", json!({ "summary": "Looked.", "deliverable": [{ "type": "text", "text": "Looked." }] })),
            ];
            return steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })));
        }
        Reply::Text(String::new())
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let a = alice.submit(ALICE).await;
    alice.wait_item(&a, "review").await;
    // Each step's result, as Magnus saw it.
    let result = |task: &'static str, turn: usize| {
        let m = &m;
        move || m.requests().into_iter().find(|c| c.body.to_string().contains(task) && c.turn() == turn).map(|c| c.last_results()[0].clone()).unwrap_or_else(|| panic!("no turn {turn} of {task}"))
    };
    let r = result(ALICE, 2)();
    assert!(!r.is_error && r.text.contains("wrote /types/customer.md (version 1)"), "{}", r.text);
    let r = result(ALICE, 3)();
    assert!(r.is_error && r.text.contains("stable only with an admin"), "a type was made stable by Magnus: {}", r.text);
    let r = result(ALICE, 4)();
    assert!(r.is_error && r.text.contains("identity key \"vat\""), "an undeclared identity key was taken: {}", r.text);
    let r = result(ALICE, 5)();
    assert!(!r.is_error && r.text.contains("version 1"), "{}", r.text);
    let r = result(ALICE, 6)();
    assert!(!r.is_error && r.text.contains("version 2"), "a second write is not a new version: {}", r.text);
    let r = result(ALICE, 7)();
    assert!(!r.is_error, "{}", r.text);
    let r = result(ALICE, 8)();
    assert!(r.is_error && r.text.contains("16KB"), "an index over the cap was taken: {}", r.text);
    let r = result(ALICE, 9)();
    assert!(!r.is_error, "{}", r.text);
    let r = result(ALICE, 10)();
    assert!(r.is_error && r.text.contains("another person's document"), "alice wrote into bob's memory: {}", r.text);
    // Alice, whose run read Acme's record, reads it as OKF.
    let r = result(ALICE, 11)();
    assert!(r.text.starts_with("---\n") && r.text.contains("title: Acme") && r.text.contains("hubspot_id: '42'") && r.text.contains("Acme's record is in HubSpot."), "{}", r.text);

    let b = bob.submit(BOB).await;
    bob.wait_item(&b, "review").await;
    // Bob cannot see Acme (drawn from a record only Alice's grant read), nor
    // Alice's own notes; the index he sees has no link to Acme; a search
    // does not find it; the type, drawn from nothing outside, he reads.
    for (turn, what) in [(2, "Acme"), (3, "alice's notes")] {
        let r = result(BOB, turn)();
        assert!(r.is_error && r.text.contains("no such document"), "bob read {what}: {}", r.text);
    }
    let r = result(BOB, 4)();
    assert!(r.text.contains("[Customer](/types/customer.md)") && !r.text.contains("acme"), "bob's index shows Acme: {}", r.text);
    let r = result(BOB, 5)();
    assert!(!r.text.contains("acme"), "bob's search found Acme: {}", r.text);
    let r = result(BOB, 6)();
    assert!(!r.is_error && r.text.contains("title: Customer"), "{}", r.text);
}
