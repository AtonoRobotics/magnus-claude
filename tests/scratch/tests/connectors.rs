//! The general MCP connector client: the owner adds an app in the app; each
//! person signs in to it for themselves, with tokens kept in the vault and
//! renewed when they expire; every tool is pinned on first sight and offered
//! only once the owner has reviewed and classified it; results carry the
//! app's labels; and a message an app sends is signed as Magnus, for the
//! person (E6).

use magnus_invariants::{mcp, *};
use magnus_scratch::FakeOAuth;
use serde_json::{json, Value};
use std::time::Duration;

async fn add(api: &Api, c: Value) {
    let (st, v) = api.put(&format!("/v1/connectors/{}", c["name"].as_str().unwrap()), c.clone()).await;
    assert!(st.is_success(), "{c}: {v}");
}

fn mail_connector(mail: &mcp::FakeMcp, oauth: &FakeOAuth, kind: &str) -> Value {
    json!({ "name": "mail", "title": "Mail", "url": mail.url, "audience": "public", "label": "private+untrusted", "auth": oauth.auth(kind) })
}

/// Sign `api`'s person in to `name` with a device code they approve.
async fn sign_in(api: &Api, oauth: &FakeOAuth, name: &str) {
    let (st, s) = api.post(&format!("/v1/connectors/{name}/sign-in"), json!({})).await;
    assert!(st.is_success(), "{s}");
    assert_eq!(s["kind"], "device");
    let flow = s["flow"].as_str().unwrap();
    let path = format!("/v1/connectors/{name}/sign-in/complete");
    assert_eq!(api.post(&path, json!({ "flow": flow })).await.1["status"], "pending");
    oauth.decide(s["user_code"].as_str().unwrap(), true);
    assert_eq!(api.post(&path, json!({ "flow": flow })).await.1["status"], "done");
}

async fn review_item(api: &Api) -> Value {
    for _ in 0..100 {
        if let Some(i) = api.inbox().await.into_iter().find(|i| i["kind"] == "tool_review" && i["status"] == "open") {
            return i;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no tool review item");
}

fn class_of<'a>(item: &'a Value, tool: &str) -> &'a Value {
    &item["details"]["tools"].as_array().unwrap().iter().find(|t| t["tool"] == tool).unwrap_or_else(|| panic!("{tool} not in {item}"))["class"]
}

/// The log holds no token, whatever happens.
async fn log_holds_none(app: &Appliance, tokens: &[String]) {
    let db = app.db.admin().await;
    let rows = db.query("SELECT body::text FROM public.events", &[]).await.unwrap();
    for r in rows {
        let body: String = r.get(0);
        for t in tokens {
            assert!(!body.contains(t.as_str()), "a token is in the log: {body}");
        }
    }
}

/// The owner adds an app; a person signs in with a code; the tools are
/// offered once the owner has reviewed them, only in that person's tasks,
/// and called with that person's token.
#[tokio::test(flavor = "multi_thread")]
async fn a_person_signs_in_to_an_app_and_its_tools_are_theirs_once_reviewed() {
    let oauth = FakeOAuth::start().await;
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("x", "x"),
            1 if c.offered_tools().iter().any(|t| t == "mail__list_inbox") => tool("mail__list_inbox", json!({})),
            _ => fail("done"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    let c = mail_connector(&mail, &oauth, "device");
    assert_eq!(bob.put("/v1/connectors/mail", c.clone()).await.0.as_u16(), 403, "a person who is not the owner added an app");
    add(&alice, c).await;
    let listed = alice.get("/v1/connectors").await.1;
    assert_eq!(listed["connectors"][0]["signed_in"], false, "{listed}");
    assert!(listed["presets"].as_array().is_some_and(|p| p.iter().any(|p| p["id"] == "gmail")), "{listed}");

    // Not signed in: nothing from the app is offered, and nothing is listed.
    let t = alice.submit("Check my mail.").await;
    alice.wait_settled(&t).await;
    assert!(m.requests().iter().all(|c| !c.offered_tools().iter().any(|t| t.starts_with("mail__"))));
    assert!(mail.calls().is_empty(), "the app was called before anyone signed in");

    sign_in(&alice, &oauth, "mail").await;
    assert_eq!(alice.get("/v1/connectors").await.1["connectors"][0]["signed_in"], true);
    // First sight: every tool is pinned and put to the owner with a
    // proposed class; none is offered until they decide.
    let t = alice.submit("Check my mail.").await;
    alice.wait_settled(&t).await;
    let item = review_item(&alice).await;
    assert_eq!(class_of(&item, "mail__list_inbox")["risk"], "read");
    assert_eq!(class_of(&item, "mail__send")["risk"], "irreversible");
    assert_eq!(class_of(&item, "mail__send")["sends"], "body");
    assert!(mail.calls_to("list_inbox").is_empty(), "an unreviewed tool ran");
    assert_eq!(alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve" })).await.as_u16(), 200);

    let t = alice.submit("Check my mail.").await;
    alice.wait_settled(&t).await;
    let calls = mail.calls_to("list_inbox");
    assert_eq!(calls.len(), 1, "the reviewed tool was not used");
    let bearer = calls[0].bearer().unwrap().to_string();
    assert_eq!(Some(&bearer), oauth.issued().last(), "the call did not carry alice's token");
    let tl = alice.get(&format!("/v1/tasks/{t}/timeline")).await.1.to_string();
    assert!(!tl.contains(&bearer), "the token reached the task's timeline");

    // Bob is not signed in: alice's sign-in is not his.
    let b = bob.submit("Check my mail.").await;
    bob.wait_settled(&b).await;
    assert_eq!(mail.calls_to("list_inbox").len(), 1, "bob's task used alice's sign-in");
    log_holds_none(&app, &oauth.issued()).await;

    // Signing out ends it.
    assert_eq!(alice.send(reqwest::Method::DELETE, "/v1/connectors/mail/sign-in", None).await.0.as_u16(), 204);
    assert_eq!(alice.get("/v1/connectors").await.1["connectors"][0]["signed_in"], false);
}

/// An access token that has expired is renewed with the refresh token,
/// once however many calls need it; a refused renewal signs the person out.
#[tokio::test(flavor = "multi_thread")]
async fn an_expired_token_is_renewed_once_and_a_revoked_one_signs_out() {
    let oauth = FakeOAuth::start().await;
    oauth.set_expires_in(2);
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("x", "x"),
            1 if c.offered_tools().iter().any(|t| t == "mail__list_inbox") => tool("mail__list_inbox", json!({})),
            _ => fail("done"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    add(&alice, mail_connector(&mail, &oauth, "device")).await;
    sign_in(&alice, &oauth, "mail").await;
    let t = alice.submit("Look.").await;
    alice.wait_settled(&t).await;
    let item = review_item(&alice).await;
    alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve" })).await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let before = oauth.refresh_grants();
    let t = alice.submit("Look.").await;
    alice.wait_settled(&t).await;
    assert!(oauth.refresh_grants() > before, "the expired token was not renewed");
    let last = oauth.issued().last().cloned();
    assert_eq!(mail.calls_to("list_inbox").last().and_then(|c| c.bearer().map(String::from)), last, "the call did not use the renewed token");
    log_holds_none(&app, &oauth.issued()).await;

    // Each expired token is renewed once, however many lookups the task makes.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let before = oauth.refresh_grants();
    let t = alice.submit("Look.").await;
    alice.wait_settled(&t).await;
    assert_eq!(oauth.refresh_grants(), before + 1, "one expired token was renewed more than once");

    // The provider revokes the grant: the person is signed out.
    oauth.revoke();
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let t = alice.submit("Look.").await;
    alice.wait_settled(&t).await;
    assert_eq!(alice.get("/v1/connectors").await.1["connectors"][0]["signed_in"], false, "a revoked sign-in still counts");
}

/// A provider that does not allow device codes (Google for Gmail): the
/// person signs in in their browser, which returns to their own machine;
/// the appliance exchanges the code with the PKCE verifier only it holds.
#[tokio::test(flavor = "multi_thread")]
async fn a_loopback_sign_in_returns_to_the_person_s_machine_with_pkce() {
    let oauth = FakeOAuth::start().await;
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    add(&alice, mail_connector(&mail, &oauth, "loopback")).await;
    for bad in ["https://evil.example/cb", "http://192.168.1.5:9000/", "http://127.0.0.1/"] {
        let (st, v) = alice.post("/v1/connectors/mail/sign-in", json!({ "redirect_uri": bad })).await;
        assert_eq!(st.as_u16(), 400, "{bad} was accepted: {v}");
    }
    let start = |redirect: &'static str| {
        let alice = alice.clone();
        async move { alice.post("/v1/connectors/mail/sign-in", json!({ "redirect_uri": redirect })).await.1 }
    };
    let s = start("http://127.0.0.1:53682/").await;
    let url = s["url"].as_str().unwrap();
    assert!(url.contains("code_challenge=") && url.contains("code_challenge_method=S256") && !url.contains("code_verifier"), "{url}");
    // The person's browser follows the provider's redirect to their machine.
    let browser = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let back = |url: String| {
        let browser = browser.clone();
        async move { browser.get(url).send().await.unwrap().headers()["location"].to_str().unwrap().to_string() }
    };
    let landed = back(url.to_string()).await;
    assert!(landed.starts_with("http://127.0.0.1:53682/?code="), "{landed}");
    // A redirect for another sign-in does not finish this one.
    let other = start("http://127.0.0.1:53682/").await;
    let tampered = format!("{}&state=forged", landed.split("&state=").next().unwrap());
    let r = alice.post("/v1/connectors/mail/sign-in/complete", json!({ "flow": other["flow"], "input": { "redirect_url": tampered } })).await.1;
    assert_eq!(r["status"], "failed", "{r}");
    let r = alice.post("/v1/connectors/mail/sign-in/complete", json!({ "flow": s["flow"], "input": { "redirect_url": landed } })).await.1;
    assert_eq!(r["status"], "done", "{r}");
    assert_eq!(alice.get("/v1/connectors").await.1["connectors"][0]["signed_in"], true);
    log_holds_none(&app, &oauth.issued()).await;
}

/// An inbox task: it reads the person's mail (private, and untrusted: others
/// wrote it), so its reply waits for the person's approval (the Rule of
/// Two). The card shows the exact message, signed as Magnus for the person,
/// and that is what is sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_reply_to_the_inbox_waits_for_approval_and_is_signed_as_magnus() {
    let oauth = FakeOAuth::start().await;
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("reply to the CFO", "the reply is sent"),
            1 if c.offered_tools().iter().any(|t| t == "mail__send") => tool("mail__list_inbox", json!({})),
            2 => tool("mail__send", json!({ "to": "cfo@corp.example", "subject": "Re: Q3 numbers", "body": "Thanks, received." })),
            3 => complete("replied"),
            _ => fail("done"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    add(&alice, mail_connector(&mail, &oauth, "device")).await;
    sign_in(&alice, &oauth, "mail").await;
    let t = alice.submit("Look.").await;
    alice.wait_settled(&t).await;
    let item = review_item(&alice).await;
    alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve" })).await;

    let id = alice.submit("Reply to the CFO's email.").await;
    let approval = alice.wait_item(&id, "approval").await;
    assert_eq!(mail.effects("send"), 0, "the reply went before approval");
    let shown = approval.to_string();
    assert!(shown.contains("Sent by Magnus, a digital worker, on behalf of alice."), "the card does not show the signed message: {approval}");
    assert_eq!(alice.respond(approval["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.as_u16(), 200);
    alice.wait_settled(&id).await;
    let sent = mail.world()["sent"].clone();
    assert_eq!(sent.as_array().unwrap().len(), 1, "{sent}");
    let body = sent[0]["body"].as_str().unwrap();
    assert_eq!(body, "Thanks, received.\n\n--\nSent by Magnus, a digital worker, on behalf of alice.", "{body}");
}

/// The owner can change a tool's class later; a tool classed prohibited is
/// never offered.
#[tokio::test(flavor = "multi_thread")]
async fn the_owner_reclassifies_a_tool() {
    let oauth = FakeOAuth::start().await;
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("x", "x"),
            _ => fail("done"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    add(&alice, mail_connector(&mail, &oauth, "device")).await;
    sign_in(&alice, &oauth, "mail").await;
    let t = alice.submit("Look.").await;
    alice.wait_settled(&t).await;
    let item = review_item(&alice).await;
    let classes = json!({ "mail__whoami": { "risk": "prohibited" } });
    alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "classes": classes })).await;
    assert_eq!(alice.put("/v1/connectors/mail/tools/list_sent", json!({ "risk": "nonsense" })).await.0.as_u16(), 400);
    assert!(alice.put("/v1/connectors/mail/tools/list_sent", json!({ "risk": "read", "label": "private" })).await.0.is_success());
    let t = alice.submit("Look.").await;
    alice.wait_settled(&t).await;
    let offered = m.requests().last().unwrap().offered_tools();
    assert!(offered.contains(&"mail__list_sent".to_string()) && !offered.contains(&"mail__whoami".to_string()), "{offered:?}");
    let view = alice.get("/v1/connectors").await.1;
    let tools = view["connectors"][0]["tools"].as_array().unwrap().clone();
    let sent = tools.iter().find(|t| t["tool"] == "mail__list_sent").unwrap();
    assert_eq!(sent["class"]["label"], "private", "{sent}");
}

/// A Files app the appliance runs itself over stdio: it is started once,
/// with a cleared environment, kept for later calls, and its results carry
/// the app's label.
#[tokio::test(flavor = "multi_thread")]
async fn a_stdio_app_runs_on_the_appliance_with_a_cleared_environment() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plan.txt"), "Ship on Friday.").unwrap();
    let server = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/stdio_files.py");
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("x", "x"),
            1 if c.offered_tools().iter().any(|t| t == "files__read_file") => tools(vec![("files__read_file", json!({ "path": "plan.txt" })), ("files__whoami", json!({}))]),
            2 => tool("files__whoami", json!({})),
            _ => fail("done"),
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    add(&alice, json!({ "name": "files", "transport": "mcp_stdio", "command": ["python3", "-I", server, dir.path()], "audience": "private", "label": "private" })).await;
    let t = alice.submit("Read the plan.").await;
    alice.wait_settled(&t).await;
    let item = review_item(&alice).await;
    assert_eq!(class_of(&item, "files__read_file")["risk"], "read");
    let classes = json!({ "files__whoami": { "risk": "read" } });
    alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "classes": classes })).await;
    let t = alice.submit("Read the plan.").await;
    alice.wait_settled(&t).await;
    let r = m.requests().into_iter().rev().find(|c| c.turn() == 2).unwrap().last_results();
    assert_eq!(r[0].text, "Ship on Friday.", "{:?}", r[0]);
    let who: Value = serde_json::from_str(&r[1].text).unwrap();
    // Python sets LC_CTYPE itself when the locale is unset (PEP 538).
    assert!(who["env"].as_array().unwrap().iter().all(|k| k == "PATH" || k == "HOME" || k == "LC_CTYPE"), "the server saw the appliance's environment: {who}");
    let later = m.requests().into_iter().rev().find(|c| c.turn() == 3).unwrap().last_results();
    let again: Value = serde_json::from_str(&later[0].text).unwrap();
    assert_eq!(again["pid"], who["pid"], "the server was not kept running");
    let task = alice.task(&t).await;
    assert_eq!(task["taint"], json!({ "untrusted": false, "private": true }), "the results do not carry the app's label: {task}");
}

/// The Files preset with the reference filesystem server, which npx fetches
/// from the npm registry: run it on a machine where that is allowed
/// (`-- --ignored`). Not yet run.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_reference_files_server_works_as_a_stdio_app() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plan.txt"), "Ship on Friday.").unwrap();
    let file = dir.path().join("plan.txt").to_string_lossy().to_string();
    let m = FakeModel::start(script(move |c| {
        let file = file.clone();
        async move {
            match c.turn() {
                0 => contract_unchecked("x", "x"),
                1 if c.offered_tools().iter().any(|t| t == "files__read_text_file") => tool("files__read_text_file", json!({ "path": file })),
                _ => fail("done"),
            }
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let preset = alice.get("/v1/connectors").await.1["presets"].as_array().unwrap().iter().find(|p| p["id"] == "files").unwrap()["connector"].clone();
    let c: Value = serde_json::from_str(&preset.to_string().replace("{folder}", dir.path().to_str().unwrap())).unwrap();
    add(&alice, c).await;
    let t = alice.submit("Read the plan.").await;
    alice.wait_settled(&t).await;
    let item = review_item(&alice).await;
    assert_eq!(class_of(&item, "files__read_text_file")["risk"], "read");
    alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve" })).await;
    let t = alice.submit("Read the plan.").await;
    alice.wait_settled(&t).await;
    let r = m.requests().into_iter().rev().find(|c| c.turn() == 2).unwrap().last_results();
    assert!(r[0].text.contains("Ship on Friday."), "{:?}", r[0]);
}
