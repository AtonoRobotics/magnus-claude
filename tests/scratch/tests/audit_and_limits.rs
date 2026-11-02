//! The kernel's rules (org context build spec): a business limit is a Cedar
//! rule over the call's input, and its refusal names the rule (K6, T7);
//! every tool call leaves an audit record with the run, the person it was
//! for, the grant it used, the source items and documents it touched, and
//! whom it sent to (K12, T13).

use magnus_invariants::mcp::{ok, FakeMcp};
use magnus_invariants::appliance::Person;
use magnus_invariants::*;
use magnus_scratch::reconfigure;
use serde_json::{json, Value};

const REFUND: &str = "Refund the two orders.";
const LIMIT: &str = r#"
permit(principal, action == Action::"execute", resource)
  when { resource.risk == "read" || resource.risk == "reversible_write" || resource.risk == "sandbox" };
permit(principal, action == Action::"request_approval", resource);
@id("refunds_over_500")
forbid(principal, action, resource == Tool::"payments__refund")
  when { context.input has amount && context.input.amount > 500 };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_business_limit_refuses_by_name_and_every_call_is_audited() {
    let payments = FakeMcp::builder()
        .honor_idempotency(true)
        .tool("refund", "Refund an order.", json!({ "type": "object", "properties": { "order": { "type": "string" }, "amount": { "type": "number" }, "to": { "type": "string" } } }), |_, _, _| ok(json!({ "status": "refunded" })))
        .start()
        .await;
    let wiki = FakeMcp::builder()
        .tool("search", "Search the wiki.", json!({ "type": "object", "properties": { "q": { "type": "string" } } }), |_, _, _| ok(json!({ "results": [] })))
        .tool("get_page", "Read a page.", json!({ "type": "object", "properties": { "page_id": { "type": "string" } } }), |_, a, _| ok(json!({ "page_id": a["page_id"], "text": "Refunds go back to the card." })))
        .start()
        .await;
    let m = FakeModel::start(script(|c| async move {
        if !c.body.to_string().contains(REFUND) || c.body["system"].to_string().contains("long-term memory") {
            return Reply::Text(String::new());
        }
        let steps = [
            contract_unchecked("refund", "refunded"),
            tool("org_fetch", json!({ "locator": "wiki://refunds" })),
            tool("payments__refund", json!({ "order": "A1", "amount": 200, "to": "alice@corp.example" })),
            tool("payments__refund", json!({ "order": "A2", "amount": 900, "to": "alice@corp.example" })),
            complete("refunded one"),
        ];
        steps.get(c.turn()).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let mut spec = Spec::default();
    // Dana reads the raw log, as a developer.
    spec.people.push(Person { id: "dana".into(), name: "Dana".into(), token: format!("tok-dana-{}", std::process::id()), groups: vec!["staff".into()], roles: vec!["developer".into()] });
    let spec = spec
        .app("payments", &payments, "internal", "principal", &[("refund", "reversible_write")])
        .app("wiki", &wiki, "internal", "principal", &[("search", "read"), ("get_page", "read")]);
    let mut app = Appliance::start(spec, &m).await;
    reconfigure(&mut app, |c| {
        for p in c["people"].as_array_mut().unwrap() {
            let t = p.as_table_mut().unwrap();
            let id = t["id"].as_str().unwrap().to_string();
            t.insert("email".into(), format!("{id}@corp.example").into());
        }
        for conn in c["connectors"].as_array_mut().unwrap() {
            let t = conn.as_table_mut().unwrap();
            if t["name"].as_str() == Some("wiki") {
                t.insert("source".into(), toml::Value::Table(toml::toml! { search = "search"
                query = "q"
                fetch = "get_page"
                id = "page_id" }));
            }
        }
    })
    .await;
    let alice = app.api("alice");
    assert!(alice.put_policy("org", LIMIT).await.is_success());
    let t = alice.submit(REFUND).await;
    alice.wait_item(&t, "review").await;
    let result = |turn: usize| m.requests().into_iter().filter(|c| c.body.to_string().contains(REFUND) && c.turn() == turn).last().map(|c| c.last_results()[0].clone()).unwrap();

    // Under the limit it goes; over it, refused, naming the rule.
    assert!(!result(3).is_error, "{}", result(3).text);
    let r = result(4);
    assert!(r.is_error && r.text.contains("refunds_over_500"), "the refusal does not name its rule: {}", r.text);
    assert_eq!(payments.effects("refund"), 1);

    // Every call left an audit record.
    let run = alice.task(&t).await["run_id"].as_str().unwrap().to_string();
    let (_, log) = app.api("dana").get(&format!("/v1/dev/runs/{run}/log")).await;
    let audits: Vec<Value> = log["events"].as_array().unwrap().iter().filter(|e| e["kind"] == "audit").map(|e| e["body"].clone()).collect();
    let of = |tool: &str| audits.iter().filter(|a| a["tool"] == tool).cloned().collect::<Vec<_>>();
    let fetch = of("org_fetch");
    assert_eq!(fetch.len(), 1, "{audits:?}");
    assert_eq!((fetch[0]["requester"].as_str(), fetch[0]["items"][0].as_str()), (Some("alice"), Some("wiki://refunds")), "{fetch:?}");
    assert!(fetch[0]["grant"].as_str().unwrap().contains("wiki"), "{fetch:?}");
    let refunds = of("payments__refund");
    assert_eq!(refunds.len(), 2, "{audits:?}");
    assert!(refunds.iter().all(|a| a["recipients"][0] == "alice@corp.example" && a["grant"].as_str().unwrap().contains("payments")), "{refunds:?}");
    assert!(refunds.iter().any(|a| a["status"] == "denied"), "the refused call left no audit record: {refunds:?}");
}

/// The console's policy tester decides as the boundary does, on what the
/// call carries: the same limit refuses a refund of 750 by name and leaves
/// one of 100 to ask, through the kernel's control socket as the console calls it.
#[tokio::test(flavor = "multi_thread")]
async fn the_policy_tester_decides_by_what_the_call_carries() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let test = |body: Value| {
        let sock = app.state_dir.join("control.sock");
        async move {
            let mut s = tokio::net::UnixStream::connect(sock).await.expect("the kernel's control socket");
            let b = body.to_string();
            s.write_all(format!("POST /v1/admin/policy/test HTTP/1.1\r\nHost: kernel\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            let (head, body) = out.split_once("\r\n\r\n").unwrap();
            assert!(head.contains(" 200 "), "{out}");
            serde_json::from_str::<Value>(body).unwrap()
        }
    };
    let ask = |amount: i64| json!({ "principal": "alice", "resource": "payments__refund", "risk": "financial", "cedar": { "org": LIMIT }, "input": { "order": "A1", "amount": amount } });
    let over = test(ask(750)).await;
    assert_eq!((over["decision"].as_str(), over["rule"].as_str()), (Some("deny"), Some("org:refunds_over_500")), "{over}");
    let under = test(ask(100)).await;
    assert_eq!(under["decision"], "ask", "a refund under the limit should only ask: {under}");
    // Nothing carried: the limit's `has` guard holds, and it asks.
    let mut bare = ask(0);
    bare.as_object_mut().unwrap().remove("input");
    assert_eq!(test(bare).await["decision"], "ask");
}
