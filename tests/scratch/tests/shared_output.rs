//! Shared output (org context build spec, K5; T6): sending anything to
//! someone other than the person, after reading items in the org's
//! sources, asks the person first when a recipient cannot read one of those
//! items at its source; someone not known here can read none. A recipient
//! who can read everything, or a send after reading nothing, goes through.

use magnus_invariants::mcp::{ok, FakeMcp};
use magnus_invariants::*;
use magnus_scratch::reconfigure;
use serde_json::json;

const WIKI: &str = "Tell Carol what the renewal page says.";
const CRM_ALICE: &str = "Tell Alice Acme's CRM numbers.";
const OUTSIDE: &str = "Send the renewal page to our lawyer.";
const NOTHING: &str = "Send the lawyer a hello.";

#[tokio::test(flavor = "multi_thread")]
async fn sending_to_someone_who_cannot_read_what_the_task_read_asks_first() {
    let wiki = FakeMcp::builder()
        .tool("search", "Search the wiki.", json!({ "type": "object", "properties": { "q": { "type": "string" } } }), |_, _, _| ok(json!({ "results": [] })))
        .tool("get_page", "Read a page.", json!({ "type": "object", "properties": { "page_id": { "type": "string" } } }), |_, a, _| ok(json!({ "page_id": a["page_id"], "text": "Renewal in March." })))
        .start()
        .await;
    let crm = FakeMcp::builder()
        .tool("find", "Find companies.", json!({ "type": "object", "properties": { "query": { "type": "string" } } }), |_, _, _| ok(json!({ "companies": [] })))
        .tool("get_company", "Read a company.", json!({ "type": "object", "properties": { "id": { "type": "string" } } }), |_, a, _| ok(json!({ "id": a["id"], "arr": "120k" })))
        .start()
        .await;
    let messages = FakeMcp::builder()
        .honor_idempotency(true)
        .tool("send", "Send a message.", json!({ "type": "object", "properties": { "to": { "type": "string" }, "text": { "type": "string" } } }), |_, _, _| ok(json!({ "status": "sent" })))
        .start()
        .await;
    let m = FakeModel::start(script(|c| async move {
        let body = c.body.to_string();
        let t = c.turn();
        let steps: Vec<Reply> = if body.contains(WIKI) {
            vec![contract_unchecked("tell", "told"), tool("org_fetch", json!({ "locator": "wiki://p1" })), tool("messages__send", json!({ "to": "carol@corp.example", "text": "Renewal in March." })), complete("told")]
        } else if body.contains(CRM_ALICE) {
            vec![contract_unchecked("tell", "told"), tool("org_fetch", json!({ "locator": "crm://42" })), tool("messages__send", json!({ "to": "Alice <alice@corp.example>", "text": "Acme: 120k." })), complete("told")]
        } else if body.contains(OUTSIDE) {
            vec![contract_unchecked("send", "sent"), tool("org_fetch", json!({ "locator": "wiki://p1" })), tool("messages__send", json!({ "to": "counsel@lawfirm.example", "text": "Renewal in March." })), complete("sent")]
        } else if body.contains(NOTHING) {
            vec![contract_unchecked("send", "sent"), tool("messages__send", json!({ "to": "counsel@lawfirm.example", "text": "Hello." })), complete("sent")]
        } else {
            return Reply::Text(String::new());
        };
        steps.get(t).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    // Nothing here is labelled private, so the audience rule does not ask:
    // what asks is whether each recipient can read what was read.
    let spec = Spec::default()
        .app("wiki", &wiki, "internal", "principal", &[("search", "read"), ("get_page", "read")])
        .app("crm", &crm, "internal", "principal", &[("find", "read"), ("get_company", "read")])
        .app("messages", &messages, "internal", "principal", &[("send", "reversible_write")])
        .access("crm", &["finance"]);
    let mut app = Appliance::start(spec, &m).await;
    reconfigure(&mut app, |c| {
        for p in c["people"].as_array_mut().unwrap() {
            let t = p.as_table_mut().unwrap();
            let id = t["id"].as_str().unwrap().to_string();
            t.insert("email".into(), format!("{id}@corp.example").into());
        }
        for conn in c["connectors"].as_array_mut().unwrap() {
            let t = conn.as_table_mut().unwrap();
            let source: Option<toml::Table> = match t["name"].as_str().unwrap() {
                "wiki" => Some(toml::toml! { search = "search"
                query = "q"
                fetch = "get_page"
                id = "page_id" }),
                "crm" => Some(toml::toml! { search = "find"
                fetch = "get_company" }),
                _ => None,
            };
            if let Some(s) = source {
                t.insert("source".into(), toml::Value::Table(s));
            }
        }
    })
    .await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let sent = || messages.effects("send");

    // Carol can read the wiki page: the message goes.
    let t = alice.submit(WIKI).await;
    alice.wait_item(&t, "review").await;
    assert_eq!(sent(), 1, "a send to someone who can read what was read was held");

    // Alice may not use the CRM: Bob's message to her about it is asked about first.
    let t = bob.submit(CRM_ALICE).await;
    let a = bob.wait_item(&t, "approval").await;
    assert!(a.to_string().contains("shared output") && a.to_string().contains("crm://42"), "{a}");
    assert_eq!(sent(), 1, "it went before it was approved");
    // Approvals keep their roles: the owner approves this reversible send.
    assert!(alice.respond(a["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success());
    bob.wait_item(&t, "review").await;
    assert_eq!(sent(), 2);

    // Someone not known here can read nothing the task read.
    let t = alice.submit(OUTSIDE).await;
    let a = alice.wait_item(&t, "approval").await;
    assert!(a.to_string().contains("counsel@lawfirm.example"), "{a}");
    // A send after reading nothing from the sources does not ask.
    let t = alice.submit(NOTHING).await;
    alice.wait_item(&t, "review").await;
    assert_eq!(sent(), 3);
}
