//! Permission checks (org context build spec, K2, K3; T2 and T4): an org
//! document drawn from items in the org's sources is seen by whoever the
//! sources say can read every one of them, not only by whom wrote it; the
//! answer is kept for a while and asked again after, so access revoked at
//! the source takes the document out of view.

use magnus_invariants::mcp::{err, ok, FakeMcp};
use magnus_invariants::*;
use magnus_scratch::reconfigure;
use serde_json::{json, Value};
use std::time::Duration;

const ALICE: &str = "Note the renewal page in the bundle.";
const BOB: &str = "Note Acme's CRM record in the bundle.";
const ALICE_LOOKS: &str = "Read what the bundle has on Acme.";
const BOB_LOOKS: &str = "Read what the bundle has on the renewal.";

fn gen() -> Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

fn customer_type() -> Value {
    json!({ "path": "/types/customer.md", "frontmatter": { "type": "Entity Type", "title": "Customer", "description": "A company that buys from us.", "generated": gen(), "status": "draft",
        "system_of_record": { "source": "/sources/crm.md" }, "identity_keys": ["domain"], "relations": [], "cedar_entity": "Customer", "scope": "org" }, "body": "Customers." })
}

fn entity(slug: &str, title: &str, from: &str) -> Value {
    json!({ "path": format!("/entities/customer/{slug}.md"), "frontmatter": { "type": "Customer", "title": title, "resource": from, "sources": [{ "resource": from }],
        "generated": gen(), "identity": { "domain": format!("{slug}.example") }, "relations": [], "scope": "org" }, "body": format!("{title}, from {from}.") })
}

#[tokio::test(flavor = "multi_thread")]
async fn who_sees_an_org_document_follows_what_each_person_can_read_at_its_sources() {
    let wiki = FakeMcp::builder()
        .world("locked", json!([]))
        .tool("search", "Search the wiki.", json!({ "type": "object", "properties": { "q": { "type": "string" } } }), |_, _, _| ok(json!({ "results": [] })))
        .tool("get_page", "Read a page.", json!({ "type": "object", "properties": { "page_id": { "type": "string" } } }), |w, a, _| {
            if w["locked"].as_array().unwrap().contains(&a["page_id"]) {
                return err("You do not have access to this page.");
            }
            ok(json!({ "page_id": a["page_id"], "text": "Acme renews in March." }))
        })
        .start()
        .await;
    let crm = FakeMcp::builder()
        .tool("find", "Find companies.", json!({ "type": "object", "properties": { "query": { "type": "string" } } }), |_, _, _| ok(json!({ "companies": [] })))
        .tool("get_company", "Read a company.", json!({ "type": "object", "properties": { "id": { "type": "string" } } }), |_, a, _| ok(json!({ "id": a["id"], "name": "Acme" })))
        .start()
        .await;
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(ALICE) {
            vec![contract_unchecked("note", "noted"), tool("org_fetch", json!({ "locator": "wiki://p1" })), tool("bundle_write", customer_type()), tool("bundle_write", entity("renewal", "Renewal", "wiki://p1")), complete("noted")]
        } else if body.contains(BOB) {
            vec![contract_unchecked("note", "noted"), tool("org_fetch", json!({ "locator": "crm://42" })), tool("bundle_write", entity("acme", "Acme", "crm://42")), complete("noted")]
        } else if body.contains(ALICE_LOOKS) {
            vec![contract_unchecked("read", "read"), tool("bundle_read", json!({ "path": "/entities/customer/acme.md" })), tool("bundle_search", json!({ "query": "acme" })), complete("read")]
        } else if body.contains(BOB_LOOKS) {
            vec![contract_unchecked("read", "read"), tool("bundle_read", json!({ "path": "/entities/customer/renewal.md" })), complete("read")]
        } else {
            return Reply::Text(String::new());
        };
        steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let spec = Spec::default()
        .app("wiki", &wiki, "internal", "private", &[("search", "read"), ("get_page", "read")])
        .app("crm", &crm, "internal", "private", &[("find", "read"), ("get_company", "read")])
        .access("crm", &["finance"]);
    let mut app = Appliance::start(spec, &m).await;
    reconfigure(&mut app, |c| {
        let mut b = toml::Table::new();
        b.insert("permission_seconds".into(), 2.into());
        c.insert("bundle".into(), toml::Value::Table(b));
        for conn in c["connectors"].as_array_mut().unwrap() {
            let t = conn.as_table_mut().unwrap();
            let source: toml::Table = match t["name"].as_str().unwrap() {
                "wiki" => toml::toml! { search = "search"
                query = "q"
                fetch = "get_page"
                id = "page_id" },
                _ => toml::toml! { search = "find"
                fetch = "get_company" },
            };
            t.insert("source".into(), toml::Value::Table(source));
        }
    })
    .await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
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
    run(&alice, ALICE).await;
    for turn in 2..=4 {
        assert!(!result(ALICE, turn)().is_error, "alice's turn {turn}: {}", result(ALICE, turn)().text);
    }
    run(&bob, BOB).await;
    for turn in 2..=3 {
        assert!(!result(BOB, turn)().is_error, "bob's turn {turn}: {}", result(BOB, turn)().text);
    }

    // Alice may not use the CRM: Acme, drawn from it, is not hers to see.
    run(&alice, ALICE_LOOKS).await;
    let r = result(ALICE_LOOKS, 2)();
    assert!(r.is_error && r.text.contains("no such document"), "alice read a document drawn from what she cannot read: {}", r.text);
    assert!(!result(ALICE_LOOKS, 3)().text.contains("acme.md"), "alice's search found it");
    // Bob can read the wiki page Alice's note was drawn from: he sees it.
    run(&bob, BOB_LOOKS).await;
    let r = result(BOB_LOOKS, 2)();
    assert!(!r.is_error && r.text.contains("title: Renewal"), "bob cannot see a document drawn from a page he can read: {}", r.text);

    // The page is closed to him at the source; once what was known has
    // expired, the document leaves his view (T4).
    wiki.set_world("locked", json!(["p1"]));
    tokio::time::sleep(Duration::from_secs(3)).await;
    run(&bob, BOB_LOOKS).await;
    let r = result(BOB_LOOKS, 2)();
    assert!(r.is_error && r.text.contains("no such document"), "access revoked at the source still shows the document: {}", r.text);
}
