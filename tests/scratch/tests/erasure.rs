//! Erasure of a deleted source item (org context build spec, K11; T12 for
//! items): when an item is deleted at its source, a document drawn from it
//! and nothing else outside the bundle is erased with every version and the
//! calls that wrote it; a document drawn from it and more stays, without it
//! and without links to it, and stale.

use magnus_invariants::mcp::{ok, FakeMcp};
use magnus_invariants::*;
use magnus_scratch::reconfigure;
use serde_json::{json, Value};

const NOTE: &str = "Note the renewal pages.";
const LOOK: &str = "Read the renewal notes.";

fn gen() -> Value {
    json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
}

fn entity(slug: &str, from: &[&str], body: &str) -> Value {
    let sources: Vec<Value> = from.iter().map(|f| json!({ "resource": f })).collect();
    json!({ "path": format!("/entities/customer/{slug}.md"), "frontmatter": { "type": "Customer", "title": slug, "resource": from[0], "sources": sources,
        "generated": gen(), "identity": { "domain": format!("{slug}.example") }, "relations": [], "scope": "org" }, "body": body })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_item_takes_what_was_drawn_only_from_it() {
    let wiki = FakeMcp::builder()
        .tool("search", "Search the wiki.", json!({ "type": "object", "properties": { "q": { "type": "string" } } }), |_, _, _| ok(json!({ "results": [] })))
        .tool("get_page", "Read a page.", json!({ "type": "object", "properties": { "page_id": { "type": "string" } } }), |_, a, _| ok(json!({ "page_id": a["page_id"], "text": "A page." })))
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
                tool("bundle_write", entity("renewal", &["wiki://p1"], "Renews in March, per the sole page.")),
                tool("bundle_write", entity("both", &["wiki://p2", "wiki://p1"], "See [the first page](wiki://p1) and [the second](wiki://p2).")),
                complete("noted"),
            ]
        } else if body.contains(LOOK) {
            vec![contract_unchecked("read", "read"), tool("bundle_read", json!({ "path": "/entities/customer/renewal.md" })), tool("bundle_read", json!({ "path": "/entities/customer/both.md" })), complete("read")]
        } else {
            return Reply::Text(String::new());
        };
        steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let mut app = Appliance::start(Spec::default().app("wiki", &wiki, "internal", "principal", &[("search", "read"), ("get_page", "read")]), &m).await;
    reconfigure(&mut app, |c| {
        let t = c["connectors"].as_array_mut().unwrap()[0].as_table_mut().unwrap();
        t.insert("source".into(), toml::Value::Table(toml::toml! { search = "search"
        query = "q"
        fetch = "get_page"
        id = "page_id" }));
    })
    .await;
    let alice = app.api("alice");
    let result = |task: &'static str, turn: usize| {
        let m = &m;
        move || m.requests().into_iter().filter(|c| c.body.to_string().contains(task) && c.turn() == turn).last().map(|c| c.last_results()[0].clone()).unwrap_or_else(|| panic!("no turn {turn} of {task}"))
    };
    let t = alice.submit(NOTE).await;
    alice.wait_item(&t, "review").await;

    // The first page is deleted at the wiki.
    let (st, v) = alice.post("/v1/sources/wiki/changes", json!({ "changes": [{ "id": "p1", "change": "deleted" }] })).await;
    assert!(st.is_success(), "{v}");

    let t = alice.submit(LOOK).await;
    alice.wait_item(&t, "review").await;
    let r = result(LOOK, 2)();
    assert!(r.is_error && r.text.contains("no such document"), "a note drawn only from the deleted page is still there: {}", r.text);
    let r = result(LOOK, 3)();
    assert!(!r.is_error, "{}", r.text);
    assert!(!r.text.contains("wiki://p1") && r.text.contains("the first page") && r.text.contains("(wiki://p2)") && r.text.contains("stale_after"), "the other note still lists or links the deleted page: {}", r.text);

    // Its content is gone from the store, every version and the call that
    // wrote it; the other note's is not.
    let rows: Vec<String> = app.db.admin().await.query("SELECT body::text FROM public.content", &[]).await.unwrap().iter().map(|r| r.get(0)).collect();
    assert!(!rows.iter().any(|r| r.contains("per the sole page")), "the erased note's text is still stored");
    assert!(rows.iter().any(|r| r.contains("the second")), "the other note's text went too");
}
