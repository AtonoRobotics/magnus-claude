//! The proactive loop (behavior step 7, #237): Magnus reviews each
//! person's state on its own, stays quiet when there is nothing to say,
//! and otherwise leaves an observation for their next session or, rarely,
//! interrupts them. Observations wait through quiet hours; interruptions
//! outside working hours or over the daily budget queue as observations
//! instead. The `[proactive]` kill switch stops all of it.
//!
//! The sweep fires before any test can set preferences, so each test
//! drives a review it triggers itself: preferences first, then a schedule
//! due in five minutes, whose coming-up review runs the script. Natural
//! cadence and overnight reviews finish quietly.

use chrono::Timelike;
use magnus_invariants::model::Script;
use magnus_invariants::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REVIEW: &str = "reviewing one person's state";

fn done(summary: &str) -> Reply {
    tool(
        "task_complete",
        json!({ "summary": summary, "deliverable": [{ "type": "text", "text": "Reviewed." }] }),
    )
}

/// Natural reviews finish quietly; the coming-up review the test
/// triggered calls the scripted tools in order, then finishes. Anything
/// else fails fast: no other run is under test.
fn model(calls: Arc<Mutex<Vec<(String, Value)>>>) -> Script {
    script(move |c| {
        let calls = calls.clone();
        async move {
            if !c.body["system"].to_string().contains(REVIEW) {
                return tool("task_fail", json!({ "reason": "not under test" }));
            }
            let first = c.messages().first().cloned().unwrap_or_default();
            let said: String = first["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if !said.contains("coming up") {
                return done("Reviewed quietly.");
            }
            match calls.lock().unwrap().get(c.turn()).cloned() {
                Some((t, input)) => tool(&t, input),
                None => done("Reviewed; nothing further."),
            }
        }
    })
}

async fn prefer(api: &Api, body: Value) {
    let (st, v) = api
        .send(reqwest::Method::PUT, "/v1/preferences", Some(body))
        .await;
    assert!(st.is_success(), "{st} {v}");
}

/// Set preferences, then a schedule due in five minutes: its coming-up
/// review starts after both land, so it always sees these preferences.
async fn trigger_review(api: &Api, prefs: Value) {
    prefer(api, prefs).await;
    let (st, v) = api
        .post(
            "/v1/schedules",
            json!({ "text": "Check the backups.", "every_minutes": 5 }),
        )
        .await;
    assert_eq!(st.as_u16(), 201, "{v}");
}

/// A one-hour window starting two hours from now, UTC: it never contains
/// the test's own run, whatever time it is.
fn away_from_now() -> Value {
    let h = (chrono::Utc::now().hour() as i64 + 2).rem_euclid(24);
    json!({ "start": format!("{h:02}:00"), "end": format!("{:02}:01", (h + 1).rem_euclid(24)) })
}

async fn wait_open(api: &Api, kind: &str) -> Value {
    let me = api.clone();
    eventually(
        &format!("open {kind} item"),
        Duration::from_secs(60),
        || {
            let me = me.clone();
            async move {
                me.inbox()
                    .await
                    .into_iter()
                    .find(|i| i["kind"] == kind && i["status"] == "open")
            }
        },
    )
    .await
}

fn notify_at(item: &Value) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(item["details"]["notify_at"].as_str().unwrap())
        .unwrap()
        .into()
}

#[tokio::test(flavor = "multi_thread")]
async fn review_observes_and_the_person_sees_it() {
    let calls = Arc::new(Mutex::new(vec![(
        "observe".to_string(),
        json!({ "what": "The backup finished late", "why": "It may overlap the morning run", "evidence": "schedule backup" }),
    )]));
    let m = FakeModel::start(model(calls)).await;
    let app = Appliance::start(Spec::default().proactive(json!({ "on": true })), &m).await;
    let bob = app.api("bob");
    // Working all day, never quiet now: the observation pings at once.
    trigger_review(&bob, json!({ "working_hours": { "start": "00:00", "end": "23:59" }, "quiet_hours": away_from_now() })).await;

    let item = wait_open(&bob, "observation").await;
    assert_eq!(item["summary"], "The backup finished late");
    assert_eq!(item["details"]["why"], "It may overlap the morning run");
    assert_eq!(item["details"]["evidence"], "schedule backup");
    assert!(
        notify_at(&item) <= chrono::Utc::now() + chrono::Duration::minutes(2),
        "it should ping at once: {item}"
    );

    // The review itself ran to completion, marked as a review.
    let me = bob.clone();
    let task = eventually("completed review task", Duration::from_secs(60), || {
        let me = me.clone();
        async move {
            let (_, v) = me.get("/v1/tasks").await;
            v["tasks"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .find(|t| t["review"] == true && t["status"] == "completed")
        }
    })
    .await;
    assert_eq!(task["person"], "bob");
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupt_outside_working_hours_queues_until_morning() {
    let calls = Arc::new(Mutex::new(vec![(
        "interrupt".to_string(),
        json!({ "what": "The deploy failed", "why": "The site is down" }),
    )]));
    let m = FakeModel::start(model(calls)).await;
    let app = Appliance::start(Spec::default().proactive(json!({ "on": true })), &m).await;
    let bob = app.api("bob");
    trigger_review(&bob, json!({ "working_hours": away_from_now() })).await;

    let item = wait_open(&bob, "observation").await;
    assert_eq!(item["summary"], "The deploy failed");
    assert!(
        item["details"]["evidence"]
            .as_str()
            .unwrap()
            .contains("outside working hours"),
        "{item}"
    );
    assert!(
        notify_at(&item) > chrono::Utc::now(),
        "it should wait for working hours: {item}"
    );

    assert!(
        bob.inbox()
            .await
            .into_iter()
            .all(|i| !(i["kind"] == "interrupt" && i["status"] == "open")),
        "an interruption landed outside working hours"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupt_in_quiet_hours_posts_now_and_pings_later() {
    let calls = Arc::new(Mutex::new(vec![(
        "interrupt".to_string(),
        json!({ "what": "The deploy failed", "why": "The site is down" }),
    )]));
    let m = FakeModel::start(model(calls)).await;
    let app = Appliance::start(Spec::default().proactive(json!({ "on": true })), &m).await;
    let bob = app.api("bob");
    // Working all day, quiet a two-hour span around now.
    let h = chrono::Utc::now().hour() as i64;
    let (s, e) = (
        format!("{:02}:00", (h - 1).rem_euclid(24)),
        format!("{:02}:00", (h + 1).rem_euclid(24)),
    );
    trigger_review(&bob, json!({ "working_hours": { "start": "00:00", "end": "23:59" }, "quiet_hours": { "start": s, "end": e } })).await;

    let item = wait_open(&bob, "interrupt").await;
    assert_eq!(item["summary"], "The deploy failed");
    assert!(
        notify_at(&item) > chrono::Utc::now(),
        "it should ping when quiet ends: {item}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn second_interrupt_same_day_queues_when_budget_spent() {
    let calls = Arc::new(Mutex::new(vec![
        (
            "interrupt".to_string(),
            json!({ "what": "First failure", "why": "It matters" }),
        ),
        (
            "interrupt".to_string(),
            json!({ "what": "Second failure", "why": "It also matters" }),
        ),
    ]));
    let m = FakeModel::start(model(calls)).await;
    let app = Appliance::start(
        Spec::default().proactive(json!({ "on": true, "interruptions_per_day": 1 })),
        &m,
    )
    .await;
    let bob = app.api("bob");
    trigger_review(&bob, json!({ "working_hours": { "start": "00:00", "end": "23:59" }, "quiet_hours": away_from_now() })).await;

    let me = bob.clone();
    let both = eventually(
        "an interruption and a queued observation",
        Duration::from_secs(60),
        || {
            let me = me.clone();
            async move {
                let inbox = me.inbox().await;
                let landed = inbox
                    .iter()
                    .find(|i| i["kind"] == "interrupt" && i["status"] == "open")
                    .cloned();
                let queued = inbox
                    .iter()
                    .find(|i| i["kind"] == "observation" && i["status"] == "open")
                    .cloned();
                landed.zip(queued)
            }
        },
    )
    .await;
    assert_eq!(both.0["summary"], "First failure");
    assert_eq!(both.1["summary"], "Second failure");
    assert!(
        both.1["details"]["evidence"]
            .as_str()
            .unwrap()
            .contains("budget spent"),
        "{:?}",
        both.1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn proactive_off_runs_no_reviews() {
    let calls = Arc::new(Mutex::new(vec![(
        "observe".to_string(),
        json!({ "what": "Something happened", "why": "It matters", "evidence": "somewhere" }),
    )]));
    let m = FakeModel::start(model(calls)).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    never(
        "any observation or interruption",
        Duration::from_secs(8),
        || {
            let (alice, bob) = (alice.clone(), bob.clone());
            async move {
                let mut all = alice.inbox().await;
                all.extend(bob.inbox().await);
                all.into_iter().any(|i| {
                    (i["kind"] == "observation" || i["kind"] == "interrupt")
                        && i["status"] == "open"
                })
            }
        },
    )
    .await;
}
