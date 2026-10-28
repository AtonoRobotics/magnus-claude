//! What the desktop app relies on from the kernel: sign-in and sessions,
//! inbox flags, plans, composed outcomes, the playbook, versions and the
//! developer view. Behavior through the HTTP API only.

use magnus_invariants::appliance::{bin, Person};
use magnus_invariants::{mcp, *};
use serde_json::{json, Value};
use std::io::Write;

const PASSWORD: &str = "correct horse battery";

fn spec() -> Spec {
    let mut s = Spec::default();
    s.people.push(Person {
        id: "dana".into(),
        name: "Dana".into(),
        token: format!("tok-dana-{}", uuid()),
        groups: vec!["staff".into()],
        roles: vec!["developer".into()],
    });
    s
}

fn uuid() -> String {
    canary("x").rsplit('-').next().unwrap().to_string()
}

fn set_password(app: &Appliance, person: &str, pw: &str) -> std::process::Output {
    let mut c = std::process::Command::new(bin("magnus-appliance"))
        .args(["password", "--config"])
        .arg(app.dir.path().join("magnus.toml"))
        .args(["--person", person])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(c.stdin.take().unwrap(), "{pw}").unwrap();
    c.wait_with_output().unwrap()
}

/// An API client acting with a session token instead of a config token.
fn with_token(app: &Appliance, token: &str) -> Api {
    let mut a = app.api("carol");
    a.token = token.to_string();
    a
}

async fn sign_in(app: &Appliance, person: &str, pw: &str) -> (reqwest::StatusCode, Value) {
    let r = reqwest::Client::new()
        .post(format!("{}/v1/sign-in", app.base))
        .json(&json!({ "person": person, "password": pw, "client": "app" }))
        .send()
        .await
        .unwrap();
    let st = r.status();
    (st, r.json().await.unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn sign_in_opens_a_session_that_never_reaches_the_log() {
    let model = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(spec(), &model).await;

    let short = set_password(&app, "carol", "short");
    assert!(!short.status.success(), "a short password was accepted");
    assert!(set_password(&app, "carol", PASSWORD).status.success());

    let (st, _) = sign_in(&app, "carol", "wrong password here").await;
    assert_eq!(st, 401);
    let (st, unknown) = sign_in(&app, "nobody", PASSWORD).await;
    assert_eq!(st, 401);
    assert_eq!(unknown["error"], "The name or password is wrong.", "an unknown name must look like a wrong password");

    let (st, v) = sign_in(&app, "Carol", PASSWORD).await;
    assert_eq!(st, 200, "{v}");
    let token = v["token"].as_str().expect("an app sign-in returns a token").to_string();
    let carol = with_token(&app, &token);
    let (st, me) = carol.get("/v1/me").await;
    assert_eq!((st.as_u16(), me["id"].as_str()), (200, Some("carol")));

    // The token is a credential: not in the log, not in the database.
    let dump = app.db.dump().await;
    assert!(!dump.contains(&token), "the session token reached the database");
    assert!(dump.contains("signed_in"), "the sign-in is not recorded");

    let (st, _) = carol.post("/v1/sign-out", json!({})).await;
    assert_eq!(st, 200);
    let (st, _) = carol.get("/v1/me").await;
    assert_eq!(st, 401, "a signed-out token still works");
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_wrong_passwords_lock_the_name() {
    let model = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(spec(), &model).await;
    assert!(set_password(&app, "carol", PASSWORD).status.success());
    for _ in 0..5 {
        assert_eq!(sign_in(&app, "carol", "wrong password here").await.0, 401);
    }
    let (st, v) = sign_in(&app, "carol", PASSWORD).await;
    assert_eq!(st, 429, "the right password got through a lockout: {v}");
    assert!(v["retry_at"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_session_needs_the_client_header_to_change_anything() {
    let model = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(spec(), &model).await;
    assert!(set_password(&app, "carol", PASSWORD).status.success());
    let http = reqwest::Client::new();
    let r = http.post(format!("{}/v1/sign-in", app.base)).json(&json!({ "person": "carol", "password": PASSWORD })).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let cookie = r.headers().get("set-cookie").expect("a browser sign-in sets a cookie").to_str().unwrap().to_string();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"), "{cookie}");
    let body: Value = r.json().await.unwrap();
    assert!(body.get("token").is_none(), "a browser sign-in must not hand the page the token");
    let pair = cookie.split(';').next().unwrap().to_string();

    let me = http.get(format!("{}/v1/me", app.base)).header("cookie", &pair).send().await.unwrap();
    assert_eq!(me.status(), 200);
    let forged = http.post(format!("{}/v1/tasks", app.base)).header("cookie", &pair).json(&json!({ "text": "hi" })).send().await.unwrap();
    assert_eq!(forged.status(), 403, "a cookie alone changed something");
}

/// The whole operator flow: plan, an approval that blocks, a review card,
/// and an outcome with its composition and evidence.
#[tokio::test(flavor = "multi_thread")]
async fn an_approval_blocks_and_the_outcome_carries_its_view() {
    let model = FakeModel::start(script(|c| async move {
        let last = c.last_results();
        match c.turn() {
            0 => contract_unchecked("note the figure", "the figure is remembered"),
            1 => tool("task_plan", json!({ "units": [
                { "id": "u1", "goal": "remember the figure", "done_when": "memory has it", "status": "active" },
                { "id": "u2", "goal": "report", "done_when": "summary sent" }
            ] })),
            2 => tool("memory_write", json!({ "fact": "Q3 revenue is 41.7M", "topic": "finance" })),
            3 => tool("task_complete", json!({ "summary": "Remembered.", "view": [{ "type": "nonsense" }] })),
            4 => {
                assert!(last.iter().any(|r| r.is_error && r.text.contains("unknown type")), "a bad view was not refused: {:?}", last.iter().map(|r| &r.text).collect::<Vec<_>>());
                tool("task_complete", json!({
                    "summary": "Remembered the Q3 figure.",
                    "view": [
                        { "type": "stat", "label": "Q3 revenue", "value": "41.7M" },
                        { "type": "bar_chart", "labels": ["Q2", "Q3"], "values": [38.2, 41.7], "unit": "M" }
                    ],
                    "spoken": "Q3 revenue was 41.7 million, up from 38.2."
                }))
            }
            _ => fail("unexpected turn"),
        }
    }))
    .await;
    let app = Appliance::start(spec(), &model).await;
    let alice = app.api("alice");
    assert!(alice.put_policy("org", &org_policy("forbid(principal, action == Action::\"execute\", resource == Tool::\"memory_write\");")).await.is_success());
    let id = alice.submit("Remember that Q3 revenue was 41.7M.").await;

    let card = eventually("a blocking approval", PATIENCE, || {
        let alice = alice.clone();
        let id = id.clone();
        async move { alice.inbox().await.into_iter().find(|i| i["task_id"] == id.as_str() && i["kind"] == "approval" && i["blocking"] == true) }
    })
    .await;
    assert_eq!(card["tier"], "reversible_write");
    assert!(card["consequence"].as_str().is_some_and(|c| c.contains("undone")), "{card}");
    assert_eq!(card["details"]["tool"], "memory_write");
    assert_eq!(card["details"]["input"]["fact"], "Q3 revenue is 41.7M", "the card must show exactly what will happen");

    let task = alice.task(&id).await;
    assert_eq!(task["plan_version"], 1);
    assert_eq!(task["plan"]["units"][0]["id"], "u1");
    assert_eq!(task["waiting_item"], card["id"]);

    assert!(alice.respond(card["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.is_success());
    let review = alice.wait_item(&id, "review").await;
    // A review is not an approval: it does not interrupt (the run waits, but
    // the card is about checking, not acting) unless the run is paused on it.
    assert!(review["blocking"].is_boolean());
    assert!(alice.respond(review["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    let done = alice.wait_status(&id, &["completed"]).await;
    assert_eq!(done["outcome_view"][0]["type"], "stat");
    assert_eq!(done["spoken"], "Q3 revenue was 41.7 million, up from 38.2.");

    let (_, tl) = alice.get(&format!("/v1/tasks/{id}/timeline")).await;
    let calls: Vec<&Value> = tl["steps"].as_array().unwrap().iter().flat_map(|s| s["calls"].as_array().unwrap()).collect();
    let mw = calls.iter().find(|c| c["tool"] == "memory_write").expect("memory_write in the timeline");
    assert_eq!(mw["issuer"], "model");
    assert!(mw["decisions"].as_array().unwrap().iter().any(|d| d["outcome"] == "ask"), "{mw}");
    assert!(mw["decisions"].as_array().unwrap().iter().any(|d| d["outcome"] == "allow"), "{mw}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_developer_view_needs_the_role_and_drills_down_to_the_log() {
    let model = FakeModel::start(steps(vec![contract_unchecked("x", "x"), tool("memory_search", json!({ "query": "anything" })), fail("nothing to do")])).await;
    let app = Appliance::start(spec(), &model).await;
    let alice = app.api("alice");
    let dana = app.api("dana");
    let id = alice.submit("Look something up.").await;
    let task = alice.wait_status(&id, &["failed"]).await;
    let run = task["run_id"].as_str().unwrap().to_string();

    for path in ["/v1/dev/runs", "/v1/dev/metrics", "/v1/dev/experiments", "/v1/dev/policy", "/v1/dev/playbook", &format!("/v1/dev/runs/{run}/log")] {
        assert_eq!(alice.get(path).await.0, 403, "{path} is open to a person without the developer role");
        assert_eq!(dana.get(path).await.0, 200, "{path}");
    }

    let (_, runs) = dana.get("/v1/dev/runs").await;
    let row = runs["runs"].as_array().unwrap().iter().find(|r| r["run"] == run.as_str()).expect("the run is listed");
    assert_eq!(row["failure"], "agent");

    let (_, ins) = dana.get(&format!("/v1/dev/runs/{run}")).await;
    let steps = ins["steps"].as_array().unwrap();
    assert!(steps.len() >= 3, "{ins}");
    let sources = steps[0]["context"]["sources"].as_array().unwrap();
    for s in ["tool definitions", "instructions", "task"] {
        assert!(sources.iter().any(|x| x["source"] == s && x["tokens"].as_u64().is_some()), "no {s} in {sources:?}");
    }
    let later = steps[2]["context"]["sources"].as_array().unwrap();
    assert!(later.iter().any(|x| x["source"] == "tool results: memory_search"), "results are not attributed: {later:?}");
    let searches: Vec<&Value> = ins["actions"].as_array().unwrap().iter().filter(|a| a["tool"] == "memory_search").collect();
    assert_eq!(searches.len(), 1, "one call is one action");
    let search = searches[0];
    assert!(search["decisions"].as_array().unwrap().iter().any(|d| d["stage"] == "policy"));
    assert_eq!(search["issuer"], "model");
    assert!(search["result"]["text"].is_string(), "{}", serde_json::to_string_pretty(&ins).unwrap());
    assert!(ins["harness"].as_array().unwrap().iter().any(|h| h["kind"] == "failed"));

    let (_, log) = dana.get(&format!("/v1/dev/runs/{run}/log")).await;
    assert_eq!(log["events"].as_array().unwrap().len() as u64, log["total"].as_u64().unwrap());
    assert!(log["events"].as_array().unwrap().iter().any(|e| e["kind"] == "task_created"));

    // Every number drills down to runs.
    let (_, m) = dana.get("/v1/dev/metrics").await;
    let failures = m["panels"].as_array().unwrap().iter().find(|p| p["id"] == "failures").unwrap();
    assert!(failures["runs"].as_array().unwrap().iter().any(|r| r == run.as_str()));
    let capture = m["panels"].as_array().unwrap().iter().find(|p| p["id"] == "capture").unwrap();
    assert!(capture["value"].as_f64().is_some(), "capture overhead is not measured: {capture}");
}

#[tokio::test(flavor = "multi_thread")]
async fn experiments_report_pass_at_k_from_repeated_trials() {
    // Case "good" completes, case "bad" fails; the grader marks one good trial wrong.
    let model = FakeModel::start(script(|c| async move {
        let good = c.messages()[0].to_string().contains("GOOD");
        match c.turn() {
            0 => contract_checked("x", "mail__list_sent", json!({}), "/count", json!(0)),
            _ if good => complete("done"),
            _ => fail("cannot"),
        }
    }))
    .await;
    let mail = mcp::mail().start().await;
    let app = Appliance::start(spec().app("mail", &mail, "public", "private", &[("list_sent", "read")]), &model).await;
    let dana = app.api("dana");
    let mut runs = vec![];
    for (case, text) in [("good", "GOOD task"), ("bad", "BAD task")] {
        for trial in 0..3 {
            let (st, v) = dana.post("/v1/dev/experiments/e1/trials", json!({ "case": case, "trial": trial, "text": text })).await;
            assert_eq!(st, 201, "{v}");
            runs.push((case, v["task_id"].as_str().unwrap().to_string(), v["run_id"].as_str().unwrap().to_string()));
        }
    }
    let (st, _) = dana.post("/v1/dev/experiments/e1/trials", json!({ "case": "good", "trial": 0, "text": "GOOD task" })).await;
    assert_eq!(st, 409, "a trial ran twice");
    for (_, task, _) in &runs {
        dana.wait_settled(task).await;
    }
    let good: Vec<&String> = runs.iter().filter(|r| r.0 == "good").map(|r| &r.2).collect();
    for r in &good {
        let passed = *r != good[0];
        let (st, _) = dana.post(&format!("/v1/dev/runs/{r}/grades"), json!({ "grader": "check", "passed": passed })).await;
        assert_eq!(st, 200);
    }
    let (_, e) = dana.get("/v1/dev/experiments/e1?k=1").await;
    let cfg = &e["configs"][0];
    let case = |name: &str| cfg["cases"].as_array().unwrap().iter().find(|c| c["case"] == name).unwrap().clone();
    assert_eq!(case("good")["passed"], 2);
    assert_eq!(case("bad")["passed"], 0);
    // pass@1 = c/n per case; the mean over cases is (2/3 + 0) / 2.
    let p = cfg["pass_at_k"]["mean"].as_f64().unwrap();
    assert!((p - 1.0 / 3.0).abs() < 1e-3, "pass@1 = {p}");
    let (_, e3) = dana.get("/v1/dev/experiments/e1?k=3").await;
    let good3 = e3["configs"][0]["cases"].as_array().unwrap().iter().find(|c| c["case"] == "good").unwrap().clone();
    assert_eq!(good3["pass_at_k"], 1.0);
    assert_eq!(good3["pass_hat_k"], 0.0);
    let tags: Vec<&str> = cfg["failures"].as_array().unwrap().iter().map(|f| f["where"].as_str().unwrap()).collect();
    assert!(tags.contains(&"grader_disagrees") && tags.contains(&"agent"), "{tags:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn playbook_items_reach_later_tasks_and_carry_their_taint() {
    let web = FakeWeb::start(&[("/howto", "<p>Invoices go to finance@example.com.</p>")]).await;
    let howto = web.url("/howto");
    let model = FakeModel::start(script(move |c| {
        let howto = howto.clone();
        async move {
            let first = c.messages()[0].to_string().contains("Learn");
            match (first, c.turn()) {
                (_, 0) => contract_unchecked("x", "x"),
                (true, 1) => tool("web_fetch", json!({ "url": howto })),
                (true, 2) => tool("playbook_update", json!({ "op": "add", "text": "Invoices go to finance@example.com.", "addresses": "where invoices go" })),
                (true, _) => fail("done"),
                (false, _) => {
                    assert!(c.messages()[0].to_string().contains("Invoices go to finance@example.com."), "the playbook item did not reach the next task");
                    fail("done")
                }
            }
        }
    }))
    .await;
    let app = Appliance::start(spec(), &model).await;
    let alice = app.api("alice");
    let a = alice.submit("Learn where invoices go.").await;
    alice.wait_status(&a, &["failed"]).await;
    let b = alice.submit("Send the invoice.").await;
    let later = alice.wait_status(&b, &["failed"]).await;
    assert_eq!(later["taint"]["untrusted"], true, "an item learned from a web page laundered its taint: {}", later["taint"]);

    let (_, pb) = app.api("dana").get("/v1/dev/playbook").await;
    let item = &pb["items"][0];
    assert_eq!(item["person"], "alice");
    assert_eq!(item["label"]["untrusted"], true, "{item}");
    assert_eq!(item["with_item"]["runs"].as_array().unwrap().len(), 1, "{pb}");
    let (_, v) = alice.get("/v1/versions").await;
    assert!(v["changes"].as_array().unwrap().iter().any(|c| c["what"] == "playbook"), "{v}");
    assert!(v["changes"].as_array().unwrap().iter().any(|c| c["what"] == "harness"), "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_change_is_a_version_change() {
    let model = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(spec(), &model).await;
    let alice = app.api("alice");
    let (_, before) = alice.get("/v1/versions").await;
    let (st, v) = alice.put("/v1/model", json!({ "adapter": "anthropic", "model": "claude-other" })).await;
    assert!(st.is_success(), "{v}");
    let (_, after) = alice.get("/v1/versions").await;
    assert_eq!(after["model"], "anthropic/claude-other");
    assert!(after["latest_seq"].as_i64() > before["latest_seq"].as_i64());
}
