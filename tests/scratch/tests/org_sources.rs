//! The org's sources (org context build spec, step 4; K1, T1 in part):
//! `org_search` runs each source's own search at once and `org_fetch` reads
//! one item by its locator, each as the person: a source they may not use
//! is never reached, not even searched; what comes back carries each
//! source's label.

use magnus_invariants::mcp::{ok, FakeMcp};
use magnus_invariants::*;
use magnus_scratch::reconfigure;
use serde_json::{json, Value};

const ALICE: &str = "Find what we know about the renewal.";
const BOB: &str = "Find what we know about Acme.";

#[tokio::test(flavor = "multi_thread")]
async fn org_search_asks_every_source_the_person_may_reach_and_org_fetch_reads_as_them() {
    let wiki = FakeMcp::builder()
        .world("pages", json!([
            { "page_id": "p1", "title": "Acme renewal", "snippet": "Renewal is due in March.", "author": "Ann", "updated": "2026-10-20", "text": "Acme renews in March; the owner is Ann." },
            { "page_id": "p2", "title": "Offsite", "snippet": "June.", "author": "Bo", "updated": "2026-09-01", "text": "The offsite is in June." },
        ]))
        .tool("search", "Search the wiki.", json!({ "type": "object", "properties": { "q": { "type": "string" } } }), |w, a, _| {
            let q = a["q"].as_str().unwrap_or_default().to_lowercase();
            let hits: Vec<Value> = w["pages"].as_array().unwrap().iter().filter(|p| p.to_string().to_lowercase().contains(&q)).map(|p| json!({ "page_id": p["page_id"], "title": p["title"], "snippet": p["snippet"], "author": p["author"], "updated": p["updated"] })).collect();
            ok(json!({ "results": hits }))
        })
        .tool("get_page", "Read a page.", json!({ "type": "object", "properties": { "page_id": { "type": "string" } } }), |w, a, _| {
            let p = w["pages"].as_array().unwrap().iter().find(|p| p["page_id"] == a["page_id"]).cloned().unwrap_or(Value::Null);
            ok(json!({ "page": p }))
        })
        .start()
        .await;
    let crm = FakeMcp::builder()
        .tool("find", "Find companies.", json!({ "type": "object", "properties": { "query": { "type": "string" } } }), |_, a, _| {
            let q = a["query"].as_str().unwrap_or_default().to_lowercase();
            ok(json!({ "companies": if "acme renewal".contains(&q) || q.contains("acme") { vec![json!({ "id": "42", "name": "Acme", "summary": "Renews in March for 120k." })] } else { vec![] } }))
        })
        .tool("get_company", "Read a company.", json!({ "type": "object", "properties": { "id": { "type": "string" } } }), |_, a, _| ok(json!({ "id": a["id"], "name": "Acme", "arr": "120k" })))
        .start()
        .await;
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(ALICE) {
            vec![
                contract_unchecked("find", "found"),
                tool("org_search", json!({ "query": "renewal" })),
                tool("org_fetch", json!({ "locator": "wiki://p1" })),
                tool("org_fetch", json!({ "locator": "crm://42" })),
                complete("found"),
            ]
        } else if body.contains(BOB) {
            vec![contract_unchecked("find", "found"), tool("org_search", json!({ "query": "acme" })), tool("org_fetch", json!({ "locator": "crm://42" })), complete("found")]
        } else {
            return Reply::Text(String::new());
        };
        steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let spec = Spec::default()
        .app("wiki", &wiki, "internal", "private+untrusted", &[("search", "read"), ("get_page", "read")])
        .app("crm", &crm, "internal", "private", &[("find", "read"), ("get_company", "read")])
        .access("crm", &["finance"]);
    let mut app = Appliance::start(spec, &m).await;
    reconfigure(&mut app, |c| {
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

    // Alice is not in finance: the CRM is not even searched for her.
    let a = alice.submit(ALICE).await;
    alice.wait_item(&a, "review").await;
    let r = result(ALICE, 2)();
    let v: Value = serde_json::from_str(&r.text).unwrap_or_else(|_| panic!("{}", r.text));
    let hit = &v["hits"][0];
    assert_eq!((hit["locator"].as_str(), hit["title"].as_str(), hit["author"].as_str()), (Some("wiki://p1"), Some("Acme renewal"), Some("Ann")), "{v}");
    assert!(!r.text.contains("crm://"), "alice's search reached the CRM: {v}");
    assert!(crm.calls_to("find").is_empty(), "the CRM was searched for someone who may not use it");
    let r = result(ALICE, 3)();
    assert!(!r.is_error && r.text.contains("the owner is Ann") && r.text.contains("\"read_at\""), "{}", r.text);
    let r = result(ALICE, 4)();
    assert!(r.is_error && r.text.contains("not a source the person can reach"), "{}", r.text);
    assert!(crm.calls_to("get_company").is_empty());
    // What she read is from outside the appliance: her task carries it.
    let (_, t) = alice.get(&format!("/v1/tasks/{a}")).await;
    assert_eq!(t["taint"]["untrusted"], true, "{}", t["taint"]);

    // Bob is: both sources are searched, and he reads the CRM's record.
    let b = bob.submit(BOB).await;
    bob.wait_item(&b, "review").await;
    let r = result(BOB, 2)();
    assert!(r.text.contains("crm://42") && r.text.contains("wiki://p1"), "{}", r.text);
    assert_eq!(crm.calls_to("find").len(), 1);
    let r = result(BOB, 3)();
    assert!(!r.is_error && r.text.contains("120k"), "{}", r.text);
}
