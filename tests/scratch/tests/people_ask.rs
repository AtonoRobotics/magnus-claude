//! `people.ask` (org context build spec, Tools): Magnus asks the owner of an
//! area (its people page) a question in the app, under its own name and
//! saying whom it is for; the task waits, and the owner's answer comes back
//! to it as the call's result. A channel Magnus cannot send to, the person
//! it works for, or someone unknown is refused with the reason.

use magnus_invariants::*;
use serde_json::json;

const ASK: &str = "Find out Acme's credit limit.";

#[tokio::test(flavor = "multi_thread")]
async fn magnus_asks_an_areas_owner_and_the_answer_comes_back_to_the_task() {
    let m = FakeModel::start(script(|c| async move {
        if !c.body.to_string().contains(ASK) || c.body["system"].to_string().contains("long-term memory") {
            return Reply::Text(String::new());
        }
        let steps = [
            contract_unchecked("find the limit", "the limit is known"),
            tool("bundle_write", json!({ "path": "/people/finance.md", "frontmatter": { "type": "Area", "title": "Finance", "owner": "bob", "ask_channel": "magnus", "scope": "org" }, "body": "Credit limits and invoices." })),
            tool("bundle_write", json!({ "path": "/people/legal.md", "frontmatter": { "type": "Area", "title": "Legal", "owner": "carol", "ask_channel": "slack:#legal", "scope": "org" }, "body": "Contracts." })),
            tool("people_ask", json!({ "area": "legal", "question": "Is the contract signed?" })),
            tool("people_ask", json!({ "user": "alice", "question": "Hm?" })),
            tool("people_ask", json!({ "user": "zed", "question": "Hm?" })),
            tool("people_ask", json!({ "area": "finance", "question": "What is Acme's credit limit?" })),
            complete("The limit is 50k."),
        ];
        steps.get(c.turn()).cloned().unwrap_or_else(|| tool("task_fail", json!({ "reason": "done" })))
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob, carol) = (app.api("alice"), app.api("bob"), app.api("carol"));
    let result = |turn: usize| {
        let m = &m;
        move || m.requests().into_iter().filter(|c| c.body.to_string().contains(ASK) && c.turn() == turn).last().map(|c| c.last_results()[0].clone()).unwrap_or_else(|| panic!("no turn {turn}"))
    };
    let t = alice.submit(ASK).await;

    // Bob, the area's owner, is asked in the app, saying whom it is for.
    let q = bob.wait_item(&t, "question").await;
    assert!(q["summary"].as_str().unwrap().contains("for alice") && q["summary"].as_str().unwrap().contains("credit limit"), "{q}");
    for (turn, why) in [(4, "cannot send to yet"), (5, "ask_principal"), (6, "not someone at the organisation")] {
        let r = result(turn)();
        assert!(r.is_error && r.text.contains(why), "turn {turn}: {}", r.text);
    }
    assert_eq!(alice.task(&t).await["status"], "suspended", "the task did not wait for the answer");
    // Not Carol's to answer.
    assert!(!carol.respond(q["id"].as_str().unwrap(), json!({ "decision": "answer", "text": "1M" })).await.is_success(), "carol answered bob's question");

    assert!(bob.respond(q["id"].as_str().unwrap(), json!({ "decision": "answer", "text": "50k, set in March." })).await.is_success());
    alice.wait_item(&t, "review").await;
    let r = result(7)();
    assert!(!r.is_error && r.text.contains("(bob) answered: 50k, set in March."), "the answer did not come back to the task: {}", r.text);
}
