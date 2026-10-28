//! The ontology (O1, O2): policies people write and the agent proposes are
//! checked against the Cedar schema generated from it, and a contract's
//! checks against the input schemas of the tools they call.

use magnus_invariants::*;
use serde_json::json;

/// A policy naming an attribute, action or word the ontology lacks would
/// never match, or deny everything when evaluated; it is refused when
/// written. One in the ontology's terms is accepted.
#[tokio::test(flavor = "multi_thread")]
async fn a_policy_outside_the_ontology_is_refused_when_written() {
    let model = FakeModel::start(steps(vec![
        tool("propose_policy", json!({ "layer": "user/alice", "cedar": "forbid(principal, action, resource) when { resource.rsik == \"irreversible\" };", "rationale": "x" })),
        tool("propose_policy", json!({ "layer": "user/alice", "cedar": "forbid(principal, action, resource) when { resource.risk == \"irreversible\" };", "rationale": "x" })),
        fail("done"),
    ]))
    .await;
    let app = Appliance::start(Spec::default(), &model).await;
    let alice = app.api("alice");
    for bad in [
        "permit(principal, action == Action::\"execute\", resource) when { resource.risk == \"Read\" };",
        "permit(principal, action == Action::\"run\", resource);",
        "forbid(principal, action, resource) when { context.tainted };",
    ] {
        let (st, v) = alice.put("/v1/policies/user/alice", json!({ "cedar": bad })).await;
        assert_eq!(st.as_u16(), 400, "{bad}: {v}");
        assert!(v["error"].as_str().unwrap_or_default().contains("refused"), "{v}");
    }
    assert!(alice.put_policy("user/alice", "forbid(principal, action == Action::\"execute\", resource == Tool::\"deliver_file\");").await.is_success());

    let id = alice.submit("Tighten my policy.").await;
    alice.wait_settled(&id).await;
    let typo = model.at_turn(1).unwrap().last_results()[0].clone();
    assert!(typo.is_error && typo.text.contains("rsik"), "a proposal outside the ontology was sent to the person: {}", typo.text);
    let ok = model.at_turn(2).unwrap().last_results()[0].clone();
    assert!(!ok.is_error, "{}", ok.text);
    let proposals: Vec<_> = alice.inbox().await.into_iter().filter(|i| i["kind"] == "policy_proposal").collect();
    assert_eq!(proposals.len(), 1, "{proposals:?}");
}

/// A check whose input does not fit its tool could never run; the contract
/// is refused when recorded, with what does not fit.
#[tokio::test(flavor = "multi_thread")]
async fn a_check_that_does_not_fit_its_tool_is_refused() {
    let contract = |input: serde_json::Value| {
        tool("task_contract", json!({ "goal": "g", "assumptions": [], "irreversible_actions": [],
            "criteria": [{ "id": "c1", "description": "the page says done", "check": { "tool": "web_fetch", "input": input, "expect": [{ "pointer": "/text", "contains": "done" }] } }] }))
    };
    let model = FakeModel::start(steps(vec![contract(json!({ "link": "https://example.com" })), contract(json!({ "url": 7 })), contract(json!({ "url": "https://example.com" })), fail("done")])).await;
    let app = Appliance::start(Spec::default(), &model).await;
    let alice = app.api("alice");
    let id = alice.submit("Check the page.").await;
    alice.wait_settled(&id).await;
    let result = |turn: usize| model.at_turn(turn).unwrap().last_results()[0].clone();
    assert!(result(1).is_error && result(1).text.contains("input has no url"), "{}", result(1).text);
    assert!(result(2).is_error && result(2).text.contains("input.url must be string"), "{}", result(2).text);
    assert!(!result(3).is_error, "{}", result(3).text);
    assert_eq!(alice.task(&id).await["contract_version"], 1);
}
