//! Context assembly (org context build spec, step 2): every run carries the
//! organisation's top index as its person may see it, then their own notes'
//! index, ahead of the task. A line whose links all point at what the person
//! cannot see is dropped (K2). The index a run carries is the one there when
//! it began, so its context only grows at its end (C1).

use magnus_invariants::*;
use serde_json::json;

const WRITE: &str = "Write the bundle index.";
const WAIT: &str = "Wait for my answer, then finish.";
const LOOK: &str = "Say what the index lists.";

fn gen() -> serde_json::Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

#[tokio::test(flavor = "multi_thread")]
async fn each_run_carries_the_index_its_person_may_see_as_it_was_when_the_run_began() {
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        if body.contains(WRITE) {
            let steps = vec![
                contract_unchecked("write the index", "the index exists"),
                tool("bundle_write", json!({ "path": "/types/customer.md", "frontmatter": { "type": "Entity Type", "title": "Customer", "description": "A company that buys from us.", "generated": gen(), "status": "draft",
                    "system_of_record": { "source": "/sources/hubspot.md" }, "identity_keys": ["domain"], "relations": [], "cedar_entity": "Customer", "scope": "org" }, "body": "Customers." })),
                tool("bundle_write", json!({ "path": "/entities/customer/acme.md", "frontmatter": { "type": "Customer", "title": "Acme", "resource": "hubspot://company/42", "sources": [{ "resource": "hubspot://company/42" }],
                    "generated": gen(), "identity": { "domain": "acme.example" }, "relations": [], "scope": "org" }, "body": "Acme." })),
                tool("bundle_write", json!({ "path": "/index.md", "frontmatter": { "okf_version": "0.2" }, "body": "# Customers\n* [Customer](/types/customer.md) - what a customer is\n* [Acme](/entities/customer/acme.md) - the account Acme\n" })),
                tool("bundle_write", json!({ "path": "/users/alice/index.md", "frontmatter": {}, "body": "# Notes\n* [Preferences](/users/alice/prefs.md) - how Alice likes her reports\n" })),
                complete("written"),
            ];
            return steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })));
        }
        if body.contains(WAIT) {
            return match t {
                0 => contract_unchecked("wait then finish", "finished"),
                1 => tool("ask_principal", json!({ "question": "Ready?" })),
                _ => complete("finished"),
            };
        }
        if body.contains(LOOK) {
            return match t {
                0 => contract_unchecked("say what it lists", "said"),
                _ => complete("listed"),
            };
        }
        Reply::Text(String::new())
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    // The first message of a task's first model call.
    let first = |task: &'static str| {
        let m = &m;
        move || m.requests().into_iter().find(|c| c.body.to_string().contains(task) && c.turn() == 0).map(|c| c.messages()[0].to_string()).unwrap_or_else(|| panic!("no first call of {task}"))
    };

    // A task begun before there is an index, waiting on Alice.
    let waiting = alice.submit(WAIT).await;
    let q = alice.wait_item(&waiting, "question").await;
    let before = first(WAIT)();
    assert!(!before.contains("context index"), "an index before there was one: {before}");

    let w = alice.submit(WRITE).await;
    alice.wait_item(&w, "review").await;

    // Bob's run: the type he may see, not Acme (drawn from a record only
    // Alice's grant read), and none of Alice's notes; ahead of his task.
    let b = bob.submit(LOOK).await;
    bob.wait_item(&b, "review").await;
    let bobs = first(LOOK)();
    assert!(bobs.contains("[Customer](/types/customer.md)"), "bob's run has no index: {bobs}");
    assert!(!bobs.contains("acme") && !bobs.contains("Acme"), "bob's index shows what he may not see: {bobs}");
    assert!(!bobs.contains("Preferences"), "bob's run carries alice's notes: {bobs}");
    assert!(bobs.find("context index").unwrap() < bobs.find(LOOK).unwrap(), "the index is not ahead of the task: {bobs}");

    // Alice's next run sees all of it, her notes' index after the org's.
    assert!(alice.respond(q["id"].as_str().unwrap(), json!({ "decision": "answer", "text": "Ready." })).await.is_success());
    alice.wait_item(&waiting, "review").await;
    let hers = m.requests().into_iter().find(|c| c.body.to_string().contains(WAIT) && c.turn() == 2).map(|c| c.messages()[0].to_string()).expect("the waiting task's later call");
    // The waiting task's context is the one it began with (C1).
    assert_eq!(hers, before, "a run's first message changed under it");
    let a2 = alice.submit(LOOK).await;
    alice.wait_item(&a2, "review").await;
    let alices = m.requests().into_iter().filter(|c| c.body.to_string().contains(LOOK) && c.turn() == 0).map(|c| c.messages()[0].to_string()).find(|s| s.contains("Acme")).unwrap_or_else(|| panic!("alice's run does not show Acme"));
    assert!(alices.contains("Preferences") && alices.find("Acme").unwrap() < alices.find("Preferences").unwrap(), "alice's notes are not after the org's index: {alices}");
}
