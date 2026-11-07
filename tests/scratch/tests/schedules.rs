//! Schedules cannot run away (owner, 2026-10-30). A test schedule's run
//! proposed a second schedule, the person accepted it, and both fired every
//! ten minutes while their earlier runs sat waiting: 75 runs piled up, with
//! no way to stop them in the app. Each guard on a real appliance, with
//! schedules allowed every minute (`[schedules] min_every_minutes`) so
//! they fire within a test:
//! - a scheduled run cannot propose another schedule; a schedule whose run
//!   waits on a person pauses until they answer, and one whose run is still
//!   open otherwise starts no other;
//! - a schedule ends after its runs, at its time, or when its own run ends it;
//! - one that keeps failing pauses itself until its person resumes it, and
//!   the app can resume it and stop it;
//! - one held on its waiting run can be paused by its person, and stays
//!   paused when the hold ends;
//! - a proposed schedule keeps the budget and taint of the run that
//!   proposed it.

use magnus_invariants::appliance::workspace_root;
use magnus_invariants::{mcp, *};
use magnus_scratch::{chromium, reconfigure, set_password, shots, PASSWORD};
use serde_json::{json, Value};
use std::time::Duration;

/// Long enough for a schedule due every minute to fire, with the sweep's
/// half-second tick and a run's start.
const MINUTE: Duration = Duration::from_secs(100);

/// A minute to become due, then the run that ends the schedule, on a busy
/// build box. The 90-second patience lapsed first (runs 41949527809,
/// 41957157578, 41960121035).
const NOTICE: Duration = Duration::from_secs(180);

#[test]
fn a_schedule_notice_outlasts_a_quiet_minute() {
    assert!(NOTICE > MINUTE);
}

fn simple_contract() -> Reply {
    tool(
        "task_contract",
        json!({ "goal": "do the work", "assumptions": [], "irreversible_actions": [],
            "criteria": [{ "id": "c1", "description": "the work is done", "check": { "tool": "sandbox_exec", "input": { "command": "true" }, "expect": [{ "pointer": "/exit_code", "equals": 0 }] } }] }),
    )
}

fn done(summary: &str) -> Reply {
    tool("task_complete", json!({ "summary": summary, "deliverable": [{ "type": "text", "text": summary }] }))
}

/// Schedules may repeat every minute, and pause after `failures` in a row.
async fn every_minute(app: &mut Appliance, failures: u32) {
    reconfigure(app, |c| {
        let mut t = toml::Table::new();
        t.insert("min_every_minutes".into(), 1.into());
        t.insert("pause_after_failures".into(), (failures as i64).into());
        c.insert("schedules".into(), toml::Value::Table(t));
    })
    .await;
}

async fn add(api: &Api, body: Value) -> String {
    let (st, v) = api.post("/v1/schedules", body).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn schedule(api: &Api, id: &str) -> Option<Value> {
    api.get("/v1/schedules").await.1["schedules"].as_array().unwrap().iter().find(|s| s["id"] == id).cloned()
}

async fn tasks_with(api: &Api, text: &str) -> Vec<Value> {
    api.tasks().await["tasks"].as_array().unwrap().iter().filter(|t| t["text"] == text).cloned().collect()
}

async fn notice(api: &Api, starts: &str, schedule: &str) -> Value {
    eventually(&format!("a notice \"{starts}\" for {schedule}"), NOTICE, || async {
        api.inbox().await.into_iter().find(|i| i["kind"] == "notice" && i["summary"].as_str().unwrap_or_default().starts_with(starts) && i["details"]["evidence"]["schedule"] == schedule)
    })
    .await
}

/// The event written just before notice `n` (an inbox item), and whether
/// both have one time. One statement gives its rows one time, so the same
/// time means one write (K24): no crash between them leaves a schedule
/// ended or paused untold, and nobody sees it so before its notice.
async fn written_with(app: &Appliance, n: &Value) -> (String, bool) {
    let rows = app.db.admin().await.query("SELECT kind, at::text, body->>'id' FROM public.events ORDER BY seq", &[]).await.unwrap();
    let rows: Vec<(String, String, Option<String>)> = rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect();
    let w = rows.windows(2).find(|w| w[1].0 == "item" && w[1].2.as_deref() == n["id"].as_str()).unwrap_or_else(|| panic!("no event for notice {n}"));
    (w[0].0.clone(), w[0].1 == w[1].1)
}

/// The loop, replayed: a scheduled run proposes a schedule (refused) and
/// then waits on its person. The next firing starts nothing while it
/// waits; once it ends, the schedule runs again. (It waits on a question:
/// a confirmed schedule's sends no longer ask its owner each time, owner
/// 2026-11-02.)
#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_run_cannot_set_up_a_schedule_and_its_schedule_waits_for_it() {
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(script(|c| async move {
        // A schedule's runs have the verifier: it finds the work done.
        if c.offered_tools().is_empty() && c.text().contains("The agent's criteria") {
            return Reply::Text("The criteria cover the request.".into());
        }
        if c.offered_tools().contains(&"verdict".to_string()) {
            return tool("verdict", json!({ "criteria": [{ "id": "c1", "holds": true, "evidence": "the sweep ran" }] }));
        }
        if !c.body.to_string().contains("Sweep the mailbox.") {
            return tool("task_fail", json!({ "reason": "not scripted" }));
        }
        match c.turn() {
            0 => simple_contract(),
            // What the test schedule's run did on 2026-10-30.
            1 => tool("propose_schedule", json!({ "text": "Tidy the folders.", "every_minutes": 1 })),
            2 => tool("ask_principal", json!({ "question": "Archive the old threads too?" })),
            _ => done("Swept the mailbox."),
        }
    }))
    .await;
    let mut app = Appliance::start(Spec::default().app("mail", &mail, "public", "private", &[("send", "irreversible"), ("list_sent", "read")]), &m).await;
    let alice = app.api("alice");
    let (st, v) = alice.post("/v1/schedules", json!({ "text": "Sweep the mailbox.", "every_minutes": 1 })).await;
    assert_eq!(st.as_u16(), 400, "every minute was allowed by default: {v}");
    every_minute(&mut app, 3).await;
    let alice = app.api("alice");
    let id = add(&alice, json!({ "text": "Sweep the mailbox.", "every_minutes": 1 })).await;

    let first = eventually("the schedule's first run", MINUTE, || async { tasks_with(&alice, "Sweep the mailbox.").await.first().map(|t| t["task_id"].as_str().unwrap().to_string()) }).await;
    let item = alice.wait_item(&first, "question").await;
    let refused = m.requests().into_iter().find(|c| c.body.to_string().contains("Sweep the mailbox.") && c.turn() == 2).expect("the run went on after proposing").last_results();
    assert!(refused[0].is_error && refused[0].text.contains("cannot set up another schedule"), "a scheduled run proposed a schedule: {}", refused[0].text);
    assert!(!alice.inbox().await.iter().any(|i| i["kind"] == "schedule_proposal"), "a proposal reached the inbox");

    // Its run waits on alice, so the schedule pauses until she answers
    // (owner, 2026-10-30), says why, and cannot be resumed past it.
    let s = eventually("the schedule pauses on its waiting run", PATIENCE, || async { schedule(&alice, &id).await.filter(|s| s["paused_on"] == first.as_str()) }).await;
    assert!(s["paused"].as_str().unwrap().contains("waiting on you (question)"), "{s}");
    assert!(s["next"].is_null(), "a paused schedule has a next run: {s}");
    assert!(s["budget"]["max_usd"].is_number(), "no budget shown: {s}");
    let n = notice(&alice, "A schedule paused until you answer its run", &id).await;
    assert_eq!(n["details"]["evidence"]["task"], first.as_str(), "{n}");
    assert_eq!(written_with(&app, &n).await, ("schedule_paused".into(), true), "the pause and its notice are two writes");
    assert_eq!(alice.post(&format!("/v1/schedules/{id}/resume"), json!({})).await.0.as_u16(), 409, "resumed past the run waiting on her");
    // Past when it would have fired: nothing started, nothing skipped.
    tokio::time::sleep(MINUTE).await;
    let s = schedule(&alice, &id).await.unwrap();
    assert_eq!(tasks_with(&alice, "Sweep the mailbox.").await.len(), 1, "a second run started while the first waited");
    assert!(s["skipped"].is_null() && s["paused_on"] == first.as_str(), "{s}");

    // Once she answers, it goes on by itself.
    assert!(alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "answer", "text": "No, just sweep." })).await.is_success());
    assert_eq!(alice.wait_settled(&first).await["status"], "completed");
    eventually("the schedule goes on", PATIENCE, || async { schedule(&alice, &id).await.filter(|s| s["paused"].is_null()) }).await;
    eventually("a second run after the first ended", MINUTE, || async { (tasks_with(&alice, "Sweep the mailbox.").await.len() == 2).then_some(()) }).await;
    assert_eq!(app.api("carol").send(reqwest::Method::DELETE, &format!("/v1/schedules/{id}"), None).await.0.as_u16(), 404, "someone else stopped it");
    assert!(alice.send(reqwest::Method::DELETE, &format!("/v1/schedules/{id}"), None).await.0.is_success());
    assert!(schedule(&alice, &id).await.is_none());
}

/// A schedule ends after its runs, at its time, or when its own run ends
/// it; its person is told each time.
#[tokio::test(flavor = "multi_thread")]
async fn a_schedule_ends_after_its_runs_at_its_time_or_by_its_own_run() {
    let m = FakeModel::start(script(|c| async move {
        let b = c.body.to_string();
        match c.turn() {
            0 => simple_contract(),
            1 if b.contains("Ring the bell.") || b.contains("Ring the gong.") => tool("end_schedule", json!({ "reason": "the bell has rung" })),
            1 if b.contains("Wait for the kettle.") => tool("wait_until", json!({ "at": (chrono::Utc::now() + chrono::Duration::seconds(150)).to_rfc3339() })),
            _ => done("Done."),
        }
    }))
    .await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    every_minute(&mut app, 3).await;
    let alice = app.api("alice");

    let soon = |s: i64| (chrono::Utc::now() + chrono::Duration::seconds(s)).to_rfc3339();
    let (st, v) = alice.post("/v1/schedules", json!({ "text": "Too late.", "every_minutes": 1, "until": soon(-60) })).await;
    assert_eq!(st.as_u16(), 400, "an end in the past was accepted: {v}");
    // "" and 0 are a model's way of leaving a field out.
    let open = add(&alice, json!({ "text": "Never ends.", "daily_at": "03:00", "until": "", "max_runs": 0 })).await;
    let s = schedule(&alice, &open).await.unwrap();
    assert!(s["until"].is_null() && s["max_runs"].is_null(), "{s}");
    assert!(alice.send(reqwest::Method::DELETE, &format!("/v1/schedules/{open}"), None).await.0.is_success());

    let twice = add(&alice, json!({ "text": "Count the stars.", "every_minutes": 1, "max_runs": 2 })).await;
    let dated = add(&alice, json!({ "text": "Water the plants.", "every_minutes": 1, "until": soon(90) })).await;
    let own = add(&alice, json!({ "text": "Ring the bell.", "every_minutes": 1 })).await;
    let kettle = add(&alice, json!({ "text": "Wait for the kettle.", "every_minutes": 1 })).await;

    // Its own run ends it, and the run goes on to finish.
    let n = notice(&alice, "A schedule ended itself", &own).await;
    assert_eq!(n["details"]["evidence"]["why"], "the bell has rung", "{n}");
    assert_eq!(written_with(&app, &n).await, ("schedule_ended".into(), true), "the end and its notice are two writes");
    assert!(schedule(&alice, &own).await.is_none(), "it ended itself but is still set");
    let rung = tasks_with(&alice, "Ring the bell.").await;
    assert_eq!(rung.len(), 1);
    assert_eq!(alice.wait_settled(rung[0]["task_id"].as_str().unwrap()).await["status"], "completed");
    // A task no schedule started has none to end.
    let t = alice.submit("Ring the gong.").await;
    alice.wait_settled(&t).await;
    let r = m.requests().into_iter().find(|c| c.body.to_string().contains("Ring the gong.") && c.turn() == 2).unwrap().last_results();
    assert!(r[0].is_error && r[0].text.contains("not started by a schedule"), "{}", r[0].text);

    // A run waiting for a time it set holds its schedule without pausing it.
    let s = eventually("a firing skipped for a run that waits for a time", MINUTE * 2, || async { schedule(&alice, &kettle).await.filter(|s| s["skipped"].is_object()) }).await;
    assert!(s["skipped"]["why"].as_str().unwrap().contains("waiting for a time"), "{s}");
    assert!(s["paused"].is_null(), "a run waiting for a time paused its schedule: {s}");
    // Paused, it starts no more runs; the run already waiting goes on
    // (cancelling would stop that run too: behavior spec, Cancellation).
    assert_eq!(alice.post(&format!("/v1/schedules/{kettle}/pause"), json!({})).await.0.as_u16(), 204);

    let n = notice(&alice, "A schedule ended", &dated).await;
    assert!(n["details"]["evidence"]["why"].as_str().unwrap().contains("reached its end"), "{n}");
    assert_eq!(written_with(&app, &n).await, ("schedule_ended".into(), true), "the end and its notice are two writes");
    assert_eq!(tasks_with(&alice, "Water the plants.").await.len(), 1, "it ran past its end");

    eventually("the second schedule ends after two runs", MINUTE * 2, || async { schedule(&alice, &twice).await.is_none().then_some(()) }).await;
    let n = notice(&alice, "A schedule ended", &twice).await;
    assert!(n["details"]["evidence"]["why"].as_str().unwrap().contains("ran 2 times"), "{n}");
    assert_eq!(written_with(&app, &n).await, ("schedule_ended".into(), true), "the end and its notice are two writes");
    assert_eq!(tasks_with(&alice, "Count the stars.").await.len(), 2);

    // The kettle's run is told its wait has ended in the same write that
    // resumes it.
    let rows = || async {
        let rows = app.db.admin().await.query("SELECT kind, at::text FROM public.events ORDER BY seq", &[]).await.unwrap();
        let rows: Vec<(String, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
        rows.windows(2).find(|w| w[1].0 == "resumed").map(|w| (w[0].0.clone(), w[0].1 == w[1].1))
    };
    let resumed = eventually("the kettle's run resumes after its wait", MINUTE * 2, rows).await;
    assert_eq!(resumed, ("notice".to_string(), true), "the wait's end told apart from the resume");
}

/// Runs that keep failing pause their schedule, and its person is told.
/// The person resumes it and stops it in the app.
#[tokio::test(flavor = "multi_thread")]
async fn a_schedule_that_keeps_failing_pauses_until_its_person_resumes_it() {
    let m = FakeModel::start(script(|_| async move { tool("task_fail", json!({ "reason": "the report site was down (HTTP 503)" })) })).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    every_minute(&mut app, 2).await;
    let alice = app.api("alice");
    let id = add(&alice, json!({ "title": "Report", "text": "Fetch the report.", "every_minutes": 1 })).await;
    let other = add(&alice, json!({ "text": "Never fails.", "daily_at": "03:00" })).await;
    assert_eq!(alice.post(&format!("/v1/schedules/{other}/resume"), json!({})).await.0.as_u16(), 409, "a running schedule was resumed");

    let s = eventually("the schedule pauses", MINUTE * 3, || async { schedule(&alice, &id).await.filter(|s| s["paused"].is_string()) }).await;
    assert_eq!(s["started"], 2, "{s}");
    assert!(s["next"].is_null(), "a paused schedule has a next run: {s}");
    let n = notice(&alice, "A schedule paused", &id).await;
    assert!(n["details"]["evidence"]["why"].as_str().unwrap().contains("last 2 runs failed"), "{n}");
    assert_eq!(written_with(&app, &n).await, ("schedule_paused".into(), true), "the pause and its notice are two writes");
    assert_eq!(app.api("carol").post(&format!("/v1/schedules/{id}/resume"), json!({})).await.0.as_u16(), 404, "someone else resumed it");

    assert!(set_password(&app, "alice", PASSWORD));
    let out = tokio::process::Command::new("node")
        .arg(workspace_root().join("desktop/tests/schedules.mjs"))
        .env("MAGNUS_URL", &app.base)
        .env("MAGNUS_PERSON", "alice")
        .env("MAGNUS_PASSWORD", PASSWORD)
        .env("MAGNUS_SCHEDULE", &id)
        .env("MAGNUS_CHROMIUM", chromium())
        .envs(shots())
        .output()
        .await
        .unwrap();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the Schedules screen check failed:\n{log}");
    assert!(schedule(&alice, &id).await.is_none(), "Stop schedule left it set");
    assert_eq!(alice.post(&format!("/v1/schedules/{id}/resume"), json!({})).await.0.as_u16(), 404);
}

/// A schedule a run proposes gets that run's budget, kept as it was, and
/// its runs start with that run's taint.
#[tokio::test(flavor = "multi_thread")]
async fn a_proposed_schedule_keeps_the_budget_and_taint_of_the_run_that_proposed_it() {
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(script(|c| async move {
        let b = c.body.to_string();
        match c.turn() {
            0 => simple_contract(),
            1 if b.contains("Watch the inbox.") => tool("mail__list_inbox", json!({})),
            2 if b.contains("Watch the inbox.") => tool("propose_schedule", json!({ "text": "Summarise the inbox.", "every_minutes": 1, "max_runs": 1 })),
            _ => done("Done."),
        }
    }))
    .await;
    let mut app = Appliance::start(Spec::default().app("mail", &mail, "public", "private+untrusted", &[("list_inbox", "read")]), &m).await;
    every_minute(&mut app, 3).await;
    let alice = app.api("alice");
    let t = alice.submit("Watch the inbox.").await;
    let item = alice.wait_item(&t, "schedule_proposal").await;
    assert_eq!(alice.wait_settled(&t).await["status"], "completed");
    let proposer = alice.task(&t).await;
    assert_eq!(proposer["taint"], json!({ "untrusted": true, "private": true }), "{proposer}");
    let budget: toml::Table = std::fs::read_to_string(app.dir.path().join("magnus.toml")).unwrap().parse::<toml::Table>().unwrap()["budget"].as_table().unwrap().clone();

    assert!(alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "accept" })).await.is_success());
    let (_, v) = alice.get("/v1/schedules").await;
    let s = v["schedules"].as_array().unwrap().iter().find(|s| s["text"] == "Summarise the inbox.").cloned().expect("the accepted proposal is not scheduled");
    let id = s["id"].as_str().unwrap().to_string();
    assert_eq!(s["max_runs"], 1, "{s}");
    // The appliance's budget changes later; the schedule keeps the one its
    // proposing run had.
    reconfigure(&mut app, |c| {
        c["budget"].as_table_mut().unwrap().insert("max_usd".into(), 1000.0.into());
    })
    .await;
    let alice = app.api("alice");
    let s = schedule(&alice, &id).await.unwrap();
    assert_eq!(s["budget"]["max_usd"].as_f64(), budget["max_usd"].as_float().or(budget["max_usd"].as_integer().map(|i| i as f64)), "{s}");
    assert_eq!(s["budget"]["max_model_calls"].as_i64(), budget["max_model_calls"].as_integer(), "{s}");

    let run = eventually("the proposed schedule's run", MINUTE, || async { tasks_with(&alice, "Summarise the inbox.").await.first().map(|t| t["task_id"].as_str().unwrap().to_string()) }).await;
    assert_eq!(alice.task(&run).await["taint"], json!({ "untrusted": true, "private": true }), "the proposed schedule's run started clean");
    notice(&alice, "A schedule ended", &id).await;
}

/// A person can pause a schedule held on them, and the hold's end does
/// not resume it: their pause is kept (#262).
#[tokio::test(flavor = "multi_thread")]
async fn a_person_can_pause_a_schedule_held_on_them_and_it_stays_paused() {
    let m = FakeModel::start(script(|c| async move {
        if c.offered_tools().is_empty() && c.text().contains("The agent's criteria") {
            return Reply::Text("The criteria cover the request.".into());
        }
        if c.offered_tools().contains(&"verdict".to_string()) {
            return tool("verdict", json!({ "criteria": [{ "id": "c1", "holds": true, "evidence": "held, then answered" }] }));
        }
        if !c.body.to_string().contains("Watch the gate.") {
            return tool("task_fail", json!({ "reason": "not scripted" }));
        }
        match c.turn() {
            0 => simple_contract(),
            1 => tool("ask_principal", json!({ "question": "Raise the alarm too?" })),
            _ => done("Watched the gate."),
        }
    }))
    .await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    every_minute(&mut app, 0).await;
    let alice = app.api("alice");
    let id = add(&alice, json!({ "text": "Watch the gate.", "every_minutes": 1 })).await;

    let first = eventually("the schedule's first run", MINUTE, || async { tasks_with(&alice, "Watch the gate.").await.first().map(|t| t["task_id"].as_str().unwrap().to_string()) }).await;
    alice.wait_item(&first, "question").await;
    eventually("the schedule held on its waiting run", PATIENCE, || async { schedule(&alice, &id).await.filter(|s| s["paused_on"] == first.as_str()) }).await;

    // Pausing a held schedule works.
    assert_eq!(alice.post(&format!("/v1/schedules/{id}/pause"), json!({})).await.0.as_u16(), 204);
    let s = schedule(&alice, &id).await.unwrap();
    assert!(s["paused_on"].is_null() && s["paused"].as_str().unwrap().contains("Paused by you"), "{s}");

    // The hold ends with its run; the person's pause is kept, not resumed.
    // (The run is cancelled rather than answered: answering needs a run
    // that completes, which the loaded build box does not give; #313.)
    assert!(alice.post(&format!("/v1/tasks/{first}/cancel"), json!({})).await.0.is_success());
    tokio::time::sleep(Duration::from_secs(10)).await;
    let s = schedule(&alice, &id).await.unwrap();
    assert!(!s["paused"].is_null(), "the hold's end resumed the person's pause: {s}");

    // Resumed, it fires again as usual.
    assert_eq!(alice.post(&format!("/v1/schedules/{id}/resume"), json!({})).await.0.as_u16(), 204);
    eventually("a second run after the resume", MINUTE * 2, || async { (tasks_with(&alice, "Watch the gate.").await.len() == 2).then_some(()) }).await;
}
