//! Change events (org context build spec, K10; T11, T4 by event): a source
//! that lists its changes is asked what changed, and each change is an
//! event. Every document listing a changed item is stale from then, and
//! what the sources said about who can read it is forgotten, so access
//! revoked at the source takes effect at once, not after the cache expires.
//! The owner may report changes too (a connector's push).

use magnus_invariants::mcp::{err, ok, FakeMcp};
use magnus_invariants::*;
use magnus_scratch::reconfigure;
use serde_json::{json, Value};
use std::time::Duration;

const NOTE: &str = "Note the renewal and offsite pages.";
const BOB_LOOKS: &str = "Read the renewal note.";
const ALICE_LOOKS: &str = "Read both notes.";

fn gen() -> Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

fn entity(slug: &str, from: &str) -> Value {
    json!({ "path": format!("/entities/customer/{slug}.md"), "frontmatter": { "type": "Customer", "title": slug, "resource": from, "sources": [{ "resource": from }],
        "generated": gen(), "identity": { "domain": format!("{slug}.example") }, "relations": [], "scope": "org" }, "body": format!("From {from}.") })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_at_a_source_marks_its_documents_stale_and_its_permissions_asked_again() {
    let wiki = FakeMcp::builder()
        .world("locked", json!([]))
        .world("changes", json!([]))
        .tool("search", "Search the wiki.", json!({ "type": "object", "properties": { "q": { "type": "string" } } }), |_, _, _| ok(json!({ "results": [] })))
        .tool("get_page", "Read a page.", json!({ "type": "object", "properties": { "page_id": { "type": "string" } } }), |w, a, _| {
            if w["locked"].as_array().unwrap().contains(&a["page_id"]) {
                return err("You do not have access to this page.");
            }
            ok(json!({ "page_id": a["page_id"], "text": "A page." }))
        })
        .tool("changes", "What changed since a time.", json!({ "type": "object", "properties": { "since": { "type": "string" } } }), |w, _, _| {
            let c = w["changes"].clone();
            w["changes"] = json!([]);
            ok(json!({ "changes": c }))
        })
        .start()
        .await;
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(NOTE) {
            vec![
                contract_unchecked("note", "noted"),
                tool("bundle_write", json!({ "path": "/types/customer.md", "frontmatter": { "type": "Entity Type", "title": "Customer", "description": "A company.", "generated": gen(), "status": "draft",
                    "system_of_record": { "source": "/sources/wiki.md" }, "identity_keys": ["domain"], "relations": [], "cedar_entity": "Customer", "scope": "org" }, "body": "Customers." })),
                tool("bundle_write", entity("renewal", "wiki://p1")),
                tool("bundle_write", entity("offsite", "wiki://p2")),
                complete("noted"),
            ]
        } else if body.contains(BOB_LOOKS) {
            vec![contract_unchecked("read", "read"), tool("bundle_read", json!({ "path": "/entities/customer/renewal.md" })), complete("read")]
        } else if body.contains(ALICE_LOOKS) {
            vec![contract_unchecked("read", "read"), tool("bundle_read", json!({ "path": "/entities/customer/renewal.md" })), tool("bundle_read", json!({ "path": "/entities/customer/offsite.md" })), complete("read")]
        } else {
            return Reply::Text(String::new());
        };
        steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let spec = Spec::default().app("wiki", &wiki, "internal", "principal", &[("search", "read"), ("get_page", "read"), ("changes", "read")]);
    let mut app = Appliance::start(spec, &m).await;
    reconfigure(&mut app, |c| {
        let t = c["connectors"].as_array_mut().unwrap()[0].as_table_mut().unwrap();
        t.insert("source".into(), toml::Value::Table(toml::toml! { search = "search"
        query = "q"
        fetch = "get_page"
        id = "page_id"
        changes = "changes"
        poll_seconds = 1 }));
    })
    .await;
    let (alice, bob, carol) = (app.api("alice"), app.api("bob"), app.api("carol"));
    let result = |task: &'static str, turn: usize| {
        let m = &m;
        move || m.requests().into_iter().filter(|c| c.body.to_string().contains(task) && c.turn() == turn).last().map(|c| c.last_results()[0].clone()).unwrap_or_else(|| panic!("no turn {turn} of {task}"))
    };
    let run = |api: &Api, text: &'static str| {
        let api = api.clone();
        async move {
            let t = api.submit(text).await;
            api.wait_item(&t, "review").await;
        }
    };
    run(&alice, NOTE).await;
    run(&bob, BOB_LOOKS).await;
    assert!(!result(BOB_LOOKS, 2)().is_error, "bob cannot see a note drawn from a page he can read: {}", result(BOB_LOOKS, 2)().text);

    // The page is closed to him and the wiki reports it: at once (the cache
    // would otherwise hold for an hour), the note leaves his view.
    wiki.set_world("locked", json!(["p1"]));
    wiki.set_world("changes", json!([{ "page_id": "p1", "permissions_changed": true }]));
    tokio::time::sleep(Duration::from_secs(4)).await;
    run(&bob, BOB_LOOKS).await;
    let r = result(BOB_LOOKS, 2)();
    assert!(r.is_error && r.text.contains("no such document"), "a permission change at the source did not reach the bundle: {}", r.text);

    // Owners report a change too; carol may not. A changed item's notes are
    // stale; the other note is not, until reported.
    assert_eq!(carol.post("/v1/sources/wiki/changes", json!({ "changes": [{ "id": "p2" }] })).await.0.as_u16(), 403);
    wiki.set_world("locked", json!([]));
    run(&alice, ALICE_LOOKS).await;
    assert!(result(ALICE_LOOKS, 2)().text.contains("stale_after"), "{}", result(ALICE_LOOKS, 2)().text);
    assert!(!result(ALICE_LOOKS, 3)().text.contains("stale_after"), "{}", result(ALICE_LOOKS, 3)().text);
    let (st, v) = alice.post("/v1/sources/wiki/changes", json!({ "changes": [{ "id": "p2" }] })).await;
    assert!(st.is_success(), "{v}");
    run(&alice, ALICE_LOOKS).await;
    assert!(result(ALICE_LOOKS, 3)().text.contains("stale_after"), "a reported change did not mark its note stale: {}", result(ALICE_LOOKS, 3)().text);
}
