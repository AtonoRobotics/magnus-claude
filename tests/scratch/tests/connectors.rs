//! The person's apps over MCP, added from the app: the owner adds one by its
//! address, signs in under MCP's authorization spec (the code comes back to
//! a loopback address on their computer), classifies its tools (the app's
//! hints only suggest), and the worker is offered only what was classified.
//! Tokens refresh as they expire; the kill switch revokes them at the app.

use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::{json, Value};

fn tool_of<'a>(list: &'a Value, app: &str, tool: &str) -> &'a Value {
    let c = list["connectors"].as_array().unwrap().iter().find(|c| c["name"] == app).expect("the app is listed");
    c["tools"].as_array().unwrap().iter().find(|t| t["name"] == tool).unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_adds_an_app_signs_in_classifies_its_tools_and_the_kill_switch_revokes_the_sign_in() {
    // Tokens last 30 s, inside the minute Magnus refreshes ahead: every use refreshes.
    let cal = FakeOAuthApp::start(30).await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("add the standup", "the standup is on the calendar"),
            1 => tool("calendar__create_event", json!({ "title": "Standup" })),
            2 => tool("calendar__list_events", json!({})),
            _ => complete("added"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, carol) = (app.api("alice"), app.api("carol"));
    let add = json!({ "name": "calendar", "url": cal.url, "audience": "private", "label": "private" });

    // Only the owner adds apps; the app itself says it needs a sign-in.
    assert_eq!(carol.post("/v1/connectors", add.clone()).await.0, 403);
    let (st, v) = alice.post("/v1/connectors", add.clone()).await;
    assert_eq!((st.as_u16(), v["sign_in"].clone()), (201, json!(true)), "{v}");
    assert_eq!(alice.post("/v1/connectors", add).await.0, 409);
    let (_, list) = alice.get("/v1/connectors").await;
    assert_eq!(list["connectors"][0]["signed_in"], false, "{list}");

    // The code goes only to the person's own computer.
    let (st, _) = alice.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": "https://elsewhere.example/cb" })).await;
    assert!(!st.is_success(), "a sign-in redirected off the person's computer");

    // The desktop app listens on a loopback address and opens the browser there.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (st, v) = alice.post("/v1/connectors/calendar/sign-in", json!({ "redirect_uri": format!("http://127.0.0.1:{port}/callback") })).await;
    assert!(st.is_success(), "{v}");
    let (code, state) = FakeOAuthApp::browse(v["authorize_url"].as_str().unwrap()).await;
    assert_eq!(state, v["state"].as_str().unwrap());
    assert_eq!(alice.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0, 204);
    assert!(!alice.post("/v1/connectors/calendar/sign-in/complete", json!({ "state": state, "code": code })).await.0.is_success(), "a sign-in completed twice");

    // Its tools, each with what the app's hints suggest, none offered yet.
    let (_, list) = alice.get("/v1/connectors").await;
    for (t, suggested) in [("list_events", "read"), ("create_event", "reversible_write"), ("delete_all", "irreversible")] {
        let tool = tool_of(&list, "calendar", t);
        assert_eq!((tool["suggested"].as_str(), tool["risk"].clone()), (Some(suggested), Value::Null), "{list}");
    }
    assert_eq!(carol.put("/v1/connectors/calendar/tools/list_events", json!({ "risk": "read" })).await.0, 403);
    assert_eq!(alice.put("/v1/connectors/calendar/tools/list_events", json!({ "risk": "read" })).await.0, 204);
    assert_eq!(alice.put("/v1/connectors/calendar/tools/create_event", json!({ "risk": "reversible_write" })).await.0, 204);
    assert_eq!(alice.put("/v1/connectors/calendar/tools/delete_all", json!({ "risk": "anything" })).await.0, 400);
    assert!(alice.put_policy("org", &org_policy(&permit_execute("calendar__create_event"))).await.is_success());

    // A person's task uses the classified tools; the unclassified one is never offered.
    let t = carol.submit("Put the standup on my calendar.").await;
    let done = carol.wait_settled(&t).await;
    assert_eq!(done["waiting_on"], "review", "{done}");
    assert_eq!((cal.calls("create_event"), cal.calls("list_events")), (1, 1));
    let offered: Vec<String> = m.requests().iter().flat_map(|c| c.offered_tools()).collect();
    assert!(offered.contains(&"calendar__create_event".to_string()) && !offered.iter().any(|t| t == "calendar__delete_all"), "{offered:?}");
    assert!(cal.state.lock().unwrap().refreshes >= 2, "expired tokens were not refreshed (or a rotated refresh token was reused)");

    // The kill switch revokes the sign-in at the app.
    assert!(alice.post("/v1/kill-switch", json!({ "engaged": true })).await.0.is_success());
    let ks = eventually("the app's sign-in is revoked", PATIENCE, || async {
        let (_, ks) = alice.get("/v1/kill-switch").await;
        (ks["revoked"]["connector_calendar"]["at_provider"] == true).then_some(ks)
    })
    .await;
    assert!(cal.state.lock().unwrap().revoked.iter().any(|t| t.starts_with("rt-")), "{ks}");
    assert!(alice.post("/v1/kill-switch", json!({ "engaged": false })).await.0.is_success());
    let (_, list) = alice.get("/v1/connectors").await;
    assert_eq!(list["connectors"][0]["signed_in"], false, "a revoked sign-in still counts: {list}");

    // Removed, it is gone; the worker is no longer offered its tools.
    let (st, _) = alice.send(reqwest::Method::DELETE, "/v1/connectors/calendar", None).await;
    assert_eq!(st, 204);
    let (_, list) = alice.get("/v1/connectors").await;
    assert!(list["connectors"].as_array().unwrap().is_empty(), "{list}");
}
