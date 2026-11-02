//! `entity_resolve` (org context build spec, Tools; T2 for entities): an
//! entity is found by one of its identity values or by its name, optionally
//! of one type, and only when the person may see it.

use magnus_invariants::*;
use serde_json::{json, Value};

const MAP: &str = "Map our customers.";
const LOOK: &str = "Who are Acme and Globex?";

fn gen() -> Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

fn entity(slug: &str, title: &str, domain: &str, sources: Value) -> Value {
    json!({ "path": format!("/entities/customer/{slug}.md"), "frontmatter": { "type": "Customer", "title": title, "resource": format!("hubspot://company/{slug}"), "sources": sources,
        "generated": gen(), "identity": { "domain": domain }, "relations": [], "scope": "org" }, "body": format!("{title}.") })
}

#[tokio::test(flavor = "multi_thread")]
async fn an_entity_is_found_by_its_identity_or_name_only_by_whom_may_see_it() {
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(MAP) {
            vec![
                contract_unchecked("map", "mapped"),
                tool("bundle_write", json!({ "path": "/types/customer.md", "frontmatter": { "type": "Entity Type", "title": "Customer", "description": "A company that buys from us.", "generated": gen(), "status": "draft",
                    "system_of_record": { "source": "/sources/hubspot.md" }, "identity_keys": ["domain"], "relations": [], "cedar_entity": "Customer", "scope": "org" }, "body": "Customers." })),
                // Acme is drawn from a record only Alice's grant read; Globex from the bundle alone.
                tool("bundle_write", entity("acme", "Acme", "acme.example", json!([{ "resource": "hubspot://company/42" }]))),
                tool("bundle_write", entity("globex", "Globex", "globex.example", json!([{ "resource": "/sources/hubspot.md" }]))),
                tool("entity_resolve", json!({ "key": "ACME.example" })),
                tool("entity_resolve", json!({ "key": "acme.example", "type": "Supplier" })),
                complete("mapped"),
            ]
        } else if body.contains(LOOK) {
            vec![contract_unchecked("look", "looked"), tool("entity_resolve", json!({ "key": "acme.example" })), tool("entity_resolve", json!({ "name": "globex", "type": "Customer" })), tool("entity_resolve", json!({})), complete("looked")]
        } else {
            return Reply::Text(String::new());
        };
        steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let result = |task: &'static str, turn: usize| {
        let m = &m;
        move || m.requests().into_iter().filter(|c| c.body.to_string().contains(task) && c.turn() == turn).last().map(|c| c.last_results()[0].clone()).unwrap_or_else(|| panic!("no turn {turn} of {task}"))
    };
    let a = alice.submit(MAP).await;
    alice.wait_item(&a, "review").await;
    for turn in 2..=4 {
        assert!(!result(MAP, turn)().is_error, "write {turn}: {}", result(MAP, turn)().text);
    }
    let r = result(MAP, 5)();
    assert!(r.text.contains("/entities/customer/acme.md") && r.text.contains("acme.example") && !r.text.contains("globex"), "by key, any case: {}", r.text);
    let r = result(MAP, 6)();
    assert!(r.text.contains("No entity"), "a type that does not match was found: {}", r.text);

    let b = bob.submit(LOOK).await;
    bob.wait_item(&b, "review").await;
    let r = result(LOOK, 2)();
    assert!(r.text.contains("No entity"), "bob resolved an entity he may not see: {}", r.text);
    let r = result(LOOK, 3)();
    assert!(r.text.contains("/entities/customer/globex.md"), "by name and type: {}", r.text);
    let r = result(LOOK, 4)();
    assert!(r.is_error, "{}", r.text);
}
