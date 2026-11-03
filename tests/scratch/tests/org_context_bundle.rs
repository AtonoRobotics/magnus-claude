//! The org context bundle, step 1 (org context build spec, Bundle format;
//! tests T9, T10 and T2 for the bundle's own reads): Magnus writes OKF
//! documents through `bundle_write`, every write checked by the kernel and a
//! malformed one refused with the reason (nothing stored); each write is a
//! new version; a type is made stable only by an admin; a person's own
//! documents are theirs alone; an org document drawn from items in the org's
//! systems is seen only by whom the kernel knows may read them, and links to
//! it leave other people's view of the index. Who that is comes from what
//! the writing run read, never from the sources its document cites.

use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::{json, Value};

const ALICE: &str = "Map the CRM for the bundle.";
const BOB: &str = "Look up Acme in the bundle.";
const PAY: &str = "Note the team's pay in the bundle.";
const BOB_PAY: &str = "Look up the team's pay in the bundle.";
const BOB_REWRITES: &str = "Rewrite the finance page for everyone.";

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

/// A run that read Alice's own file writes what it said into an org
/// document that cites only Alice: the kernel knows what the run read, so
/// the document is hers alone (K2), and stays so when an admin edits it.
#[tokio::test(flavor = "multi_thread")]
async fn an_org_document_from_private_content_is_its_writers_alone_whatever_it_cites() {
    use base64::Engine;
    let finance = || json!({ "type": "Area", "title": "Finance", "owner": "human:alice", "ask_channel": "#finance", "scope": "org", "sources": [{ "resource": "human:alice" }] });
    let m = FakeModel::start(script(move |c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(BOB_PAY) {
            vec![contract_unchecked("look up pay", "found or not"), tool("bundle_read", json!({ "path": "/people/finance.md" })), tool("bundle_search", json!({ "query": "carol" })), complete("Looked.")]
        } else if body.contains(PAY) {
            vec![contract_unchecked("note pay", "it is noted"), tool("bundle_write", json!({ "path": "/people/finance.md", "frontmatter": finance(), "body": "Carol earns 91,000 a year." })), complete("Noted.")]
        } else {
            vec![]
        };
        steps.get(t).cloned().unwrap_or_else(|| Reply::Text(String::new()))
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let (st, f) = alice.post("/v1/files", json!({ "name": "pay.txt", "media_type": "text/plain", "data_b64": base64::engine::general_purpose::STANDARD.encode("Carol earns 91,000 a year.") })).await;
    assert_eq!(st.as_u16(), 201, "{f}");
    let (st, v) = alice.post("/v1/tasks", json!({ "text": PAY, "files": [f["id"]] })).await;
    assert!(st.is_success(), "{v}");
    let task = v["task_id"].as_str().or(v["id"].as_str()).unwrap().to_string();
    // Her file is untrusted and private, so the write waits for her (Rule of Two).
    let ask = alice.wait_item(&task, "approval").await;
    assert!(alice.respond(ask["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success());
    alice.wait_item(&task, "review").await;
    let wrote = m.requests().into_iter().find(|c| c.body.to_string().contains(PAY) && c.turn() == 2).map(|c| c.last_results()[0].clone()).unwrap();
    assert!(!wrote.is_error && wrote.text.contains("wrote /people/finance.md"), "{}", wrote.text);

    // Bob neither reads it nor finds it.
    let bob_looks = |n: usize| {
        let (m, bob) = (&m, &bob);
        async move {
            let b = bob.submit(BOB_PAY).await;
            bob.wait_item(&b, "review").await;
            let runs: Vec<_> = m.requests().into_iter().filter(|c| c.body.to_string().contains(BOB_PAY)).collect();
            let at = |turn: usize| runs.iter().filter(|c| c.turn() == turn).nth(n).map(|c| c.last_results()[0].clone()).unwrap();
            (at(2), at(3))
        }
    };
    let (read, search) = bob_looks(0).await;
    assert!(read.is_error && read.text.contains("no such document"), "bob read what alice's file said: {}", read.text);
    assert!(!search.text.contains("finance"), "bob's search found it: {}", search.text);

    // An admin's edit launders nothing: still hers, still not Bob's.
    let (st, v) = control(&app, "POST", "/v1/admin/knowledge/doc", json!({ "sub": "admin-1", "path": "/people/finance.md", "frontmatter": finance(), "body": "Carol earns 91,000 a year (checked)." })).await;
    assert_eq!(st, 200, "{v}");
    let (read, _) = bob_looks(1).await;
    assert!(read.is_error && read.text.contains("no such document"), "after an admin's edit bob read it: {}", read.text);
    let (st, d) = control(&app, "GET", "/v1/admin/knowledge/doc?path=/people/finance.md", Value::Null).await;
    assert_eq!(st, 200, "{d}");
    assert_eq!(d["writer"], "alice", "the admin's edit lost whose run read its sources: {d}");
}

/// A document drawn from Alice's own file, removed by an admin, comes back
/// from the log as it was written: rolled back, its removal undone, or a
/// clean version written after its removal undone (Bob's, with her task in
/// the trash, or an admin's), it is still hers alone (K2), with the taint and the
/// private sources it was written from.
#[tokio::test(flavor = "multi_thread")]
async fn a_version_put_back_keeps_the_taint_and_the_reader_it_was_written_with() {
    use base64::Engine;
    let finance = || json!({ "type": "Area", "title": "Finance", "owner": "human:alice", "ask_channel": "#finance", "scope": "org", "sources": [{ "resource": "human:alice" }] });
    let m = FakeModel::start(script(move |c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(BOB_PAY) {
            vec![contract_unchecked("look up pay", "found or not"), tool("bundle_read", json!({ "path": "/people/finance.md" })), complete("Looked.")]
        } else if body.contains(PAY) {
            vec![contract_unchecked("note pay", "it is noted"), tool("bundle_write", json!({ "path": "/people/finance.md", "frontmatter": finance(), "body": "Carol earns 91,000 a year." })), complete("Noted.")]
        } else if body.contains(BOB_REWRITES) {
            vec![contract_unchecked("rewrite finance", "it is rewritten"), tool("bundle_write", json!({ "path": "/people/finance.md", "frontmatter": finance(), "body": "Finance pays the team." })), complete("Rewritten.")]
        } else {
            vec![]
        };
        steps.get(t).cloned().unwrap_or_else(|| Reply::Text(String::new()))
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let (st, f) = alice.post("/v1/files", json!({ "name": "pay.txt", "media_type": "text/plain", "data_b64": base64::engine::general_purpose::STANDARD.encode("Carol earns 91,000 a year.") })).await;
    assert_eq!(st.as_u16(), 201, "{f}");
    let (st, v) = alice.post("/v1/tasks", json!({ "text": PAY, "files": [f["id"]] })).await;
    assert!(st.is_success(), "{v}");
    let task = v["task_id"].as_str().or(v["id"].as_str()).unwrap().to_string();
    let ask = alice.wait_item(&task, "approval").await;
    assert!(alice.respond(ask["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success());
    let review = alice.wait_item(&task, "review").await;

    let doc = || control(&app, "GET", "/v1/admin/knowledge/doc?path=/people/finance.md", Value::Null);
    let remove = || control(&app, "DELETE", "/v1/admin/knowledge/doc", json!({ "sub": "admin-1", "path": "/people/finance.md" }));
    // The nth time Bob looks.
    let still_hers = |n: usize, how: &'static str| {
        let (m, bob, doc) = (&m, &bob, &doc);
        async move {
            let (st, d) = doc().await;
            assert_eq!(st, 200, "{how}: {d}");
            assert_eq!(d["body"], "Carol earns 91,000 a year.", "{how}: {d}");
            assert_eq!(d["writer"], "alice", "{how}: it lost whose run read it: {d}");
            assert!(d["label"]["untrusted"] == true && d["label"]["private"] == true, "{how}: it came back clean: {d}");
            assert!(d["sources"].as_array().unwrap().iter().any(|s| s == "private:library"), "{how}: it lost what its run read: {d}");
            let b = bob.submit(BOB_PAY).await;
            bob.wait_item(&b, "review").await;
            let read = m.requests().into_iter().filter(|c| c.body.to_string().contains(BOB_PAY) && c.turn() == 2).nth(n).map(|c| c.last_results()[0].clone()).unwrap();
            assert!(read.is_error && read.text.contains("no such document"), "{how}: bob read what alice's file said: {}", read.text);
        }
    };
    still_hers(0, "as written").await;

    // Alice puts her task in the trash; the document removed, Bob's run
    // writes a new one from nothing private, and an admin undoes that: what
    // comes back is hers again, not Bob's.
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    let (st, v) = alice.post("/v1/trash", json!({ "item": task })).await;
    assert!(st.is_success(), "{v}");
    assert_eq!(remove().await.0, 200);
    let b = bob.submit(BOB_REWRITES).await;
    bob.wait_item(&b, "review").await;
    let (_, d) = doc().await;
    assert_eq!(d["writer"], "bob", "{d}");
    let (_, a) = control(&app, "GET", "/v1/admin/knowledge/activity", Value::Null).await;
    let bobs = a["changes"].as_array().unwrap()[0]["seq"].clone();
    let (st, v) = control(&app, "POST", "/v1/admin/knowledge/revert", json!({ "sub": "admin-1", "seqs": [bobs] })).await;
    assert_eq!(st, 200, "{v}");
    still_hers(1, "bob's version undone, her task in the trash").await;

    // Removed, then rolled back to the version put back above (its 2:
    // Bob's new document began again at 1).
    assert_eq!(remove().await.0, 200);
    let (st, v) = control(&app, "POST", "/v1/admin/knowledge/rollback", json!({ "sub": "admin-1", "path": "/people/finance.md", "version": 2 })).await;
    assert_eq!(st, 200, "{v}");
    still_hers(2, "rolled back after removal").await;

    // Removed, then the removal undone.
    assert_eq!(remove().await.0, 200);
    let (_, a) = control(&app, "GET", "/v1/admin/knowledge/activity", Value::Null).await;
    let removal = a["changes"].as_array().unwrap().iter().find(|c| c["change"] == "removed").unwrap()["seq"].clone();
    let (st, v) = control(&app, "POST", "/v1/admin/knowledge/revert", json!({ "sub": "admin-1", "seqs": [removal] })).await;
    assert_eq!(st, 200, "{v}");
    still_hers(3, "its removal undone").await;

    // Removed, written afresh by an admin from nothing private, and that
    // undone: what comes back is Alice's, not the admin's clean version.
    assert_eq!(remove().await.0, 200);
    let (st, v) = control(&app, "POST", "/v1/admin/knowledge/doc", json!({ "sub": "admin-1", "path": "/people/finance.md", "frontmatter": finance(), "body": "Finance pays the team." })).await;
    assert_eq!(st, 200, "{v}");
    let (_, a) = control(&app, "GET", "/v1/admin/knowledge/activity", Value::Null).await;
    let fresh = a["changes"].as_array().unwrap()[0]["seq"].clone();
    let (st, v) = control(&app, "POST", "/v1/admin/knowledge/revert", json!({ "sub": "admin-1", "seqs": [fresh] })).await;
    assert_eq!(st, 200, "{v}");
    still_hers(4, "an admin's clean version undone").await;
}

/// An entity moved by merging its type into another keeps what it was
/// written from: its taint, the private content its run read and whose run
/// that was, so the move shows it to no one new and hides it from no one.
#[tokio::test(flavor = "multi_thread")]
async fn an_entity_moved_by_merging_types_keeps_its_taint_and_its_reader() {
    use base64::Engine;
    const NOTES: &str = "Note Acme from my file in the bundle.";
    let ty = |title: &str| json!({ "type": "Entity Type", "title": title, "description": "Who buys.", "generated": { "by": "human:admin-1", "at": "2026-11-02T12:00:00+00:00" }, "status": "draft", "system_of_record": "/sources/hubspot.md", "identity_keys": ["domain"], "relations": [], "cedar_entity": title, "scope": "org" });
    let m = FakeModel::start(script(|c| async move {
        if !c.body.to_string().contains(NOTES) {
            return Reply::Text(String::new());
        }
        let acme = json!({ "path": "/entities/customer/acme.md", "frontmatter": { "type": "Customer", "title": "Acme", "resource": "/sources/hubspot.md", "sources": [{ "resource": "/sources/hubspot.md" }], "generated": gen(), "identity": { "domain": "acme.example" }, "relations": [], "scope": "org" }, "body": "Acme is renegotiating." });
        let steps = [contract_unchecked("note acme", "it is noted"), tool("bundle_write", acme), complete("Noted.")];
        steps.get(c.turn()).cloned().unwrap_or_else(|| Reply::Text(String::new()))
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    for (path, title) in [("/types/customer.md", "Customer"), ("/types/client.md", "Client")] {
        let (st, v) = control(&app, "POST", "/v1/admin/knowledge/doc", json!({ "sub": "admin-1", "path": path, "frontmatter": ty(title), "body": "Who buys." })).await;
        assert_eq!(st, 200, "{v}");
    }
    let alice = app.api("alice");
    let (st, f) = alice.post("/v1/files", json!({ "name": "acme.txt", "media_type": "text/plain", "data_b64": base64::engine::general_purpose::STANDARD.encode("Acme is renegotiating.") })).await;
    assert_eq!(st.as_u16(), 201, "{f}");
    let (st, v) = alice.post("/v1/tasks", json!({ "text": NOTES, "files": [f["id"]] })).await;
    assert!(st.is_success(), "{v}");
    let task = v["task_id"].as_str().or(v["id"].as_str()).unwrap().to_string();
    let ask = alice.wait_item(&task, "approval").await;
    assert!(alice.respond(ask["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success());
    alice.wait_item(&task, "review").await;
    let (st, before) = control(&app, "GET", "/v1/admin/knowledge/doc?path=/entities/customer/acme.md", Value::Null).await;
    assert_eq!(st, 200, "{before}");
    assert_eq!(before["writer"], "alice", "{before}");

    let (st, v) = control(&app, "POST", "/v1/admin/knowledge/merge", json!({ "sub": "admin-1", "from": "/types/customer.md", "into": "/types/client.md" })).await;
    assert_eq!((st, v["entities_moved"].as_u64()), (200, Some(1)), "{v}");
    let (st, d) = control(&app, "GET", "/v1/admin/knowledge/doc?path=/entities/client/acme.md", Value::Null).await;
    assert_eq!((st, d["frontmatter"]["type"].as_str()), (200, Some("Client")), "{d}");
    assert_eq!(d["label"], before["label"], "the move changed its taint: {d}");
    assert_eq!(d["writer"], "alice", "the move lost whose run read it: {d}");
    assert!(d["sources"].as_array().unwrap().iter().any(|s| s == "private:library"), "the move lost what its run read: {d}");
}

