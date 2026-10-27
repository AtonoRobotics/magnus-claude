//! Invariant 3: the append-only event log is the source of truth. A crashed run
//! resumes without repeating side effects.
//!
//! Prevents: lost sessions on crash, and retries that repeat a side effect
//! after a lost response.

use magnus_invariants::{mcp, *};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

fn send() -> Reply {
    tool("mail__send", json!({ "to": "bob@corp.example", "subject": "report", "body": "attached" }))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_kernel_cannot_rewrite_the_log() {
    let model = FakeModel::start(steps(vec![contract_unchecked("nothing", "nothing"), fail("done")])).await;
    let app = Appliance::start(Spec::default(), &model).await;
    let alice = app.api("alice");
    let id = alice.submit("Do nothing.").await;
    alice.wait_settled(&id).await;

    let admin = app.db.admin().await;
    let count = |c: tokio_postgres::Client| async move {
        c.query_one("SELECT count(*) FROM public.events", &[]).await.unwrap().get::<_, i64>(0)
    };
    let before = count(app.db.admin().await).await;
    assert!(before > 0, "the run left no events");
    let col: String = admin
        .query_one(
            "SELECT column_name::text FROM information_schema.columns
             WHERE table_schema = 'public' AND table_name = 'events' ORDER BY ordinal_position LIMIT 1",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    let k = app.db.as_kernel().await;
    for sql in [
        format!("UPDATE public.events SET {col} = {col}"),
        "DELETE FROM public.events".to_string(),
        "TRUNCATE public.events".to_string(),
        "DROP TABLE public.events".to_string(),
        "ALTER TABLE public.events DISABLE TRIGGER ALL".to_string(),
    ] {
        assert!(k.batch_execute(&sql).await.is_err(), "the kernel role may run: {sql}");
    }
    assert_eq!(count(app.db.admin().await).await, before, "events changed");
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_after_a_side_effect_does_not_repeat_it() {
    let mail = mcp::mail().start().await;
    let model = FakeModel::start(steps(vec![
        contract_checked("send the report", "mail__list_sent", json!({}), "/count", json!(1)),
        send(),
        complete("sent"),
    ]))
    .await;
    let mut app = Appliance::start(
        Spec::default().app("mail", &mail, "public", "private", &[("send", "irreversible"), ("list_sent", "read")]),
        &model,
    )
    .await;
    let alice = app.api("alice");
    assert!(alice.put_policy("org", &org_policy(&permit_execute("mail__send"))).await.is_success());

    let hold = mail.hold("send");
    let id = alice.submit("Email bob the report.").await;
    tokio::time::timeout(PATIENCE, hold.received()).await.expect("the send reached the app");
    app.kill().await; // the effect happened; its response is lost
    hold.release();
    app.restart().await;

    let t = alice.wait_settled(&id).await;
    assert_eq!(t["status"], "completed", "{t}");
    assert_eq!(mail.effects("send"), 1, "the email was sent more than once");
    let keys: BTreeSet<_> = mail.calls_to("send").iter().map(|c| c.idempotency_key().map(String::from)).collect();
    assert_eq!(keys.len(), 1, "a resend used a different idempotency key: {keys:?}");
    assert!(keys.iter().next().unwrap().is_some(), "the send carried no idempotency key");
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_against_an_app_without_idempotency_asks_instead_of_resending() {
    let mail = mcp::mail().honor_idempotency(false).start().await;
    let model = FakeModel::start(steps(vec![
        contract_checked("send the report", "mail__list_sent", json!({}), "/count", json!(1)),
        send(),
        complete("sent"),
    ]))
    .await;
    let mut app = Appliance::start(
        Spec::default()
            .app("mail", &mail, "public", "private", &[("send", "irreversible"), ("list_sent", "read")])
            .not_idempotent("mail"),
        &model,
    )
    .await;
    let alice = app.api("alice");
    assert!(alice.put_policy("org", &org_policy(&permit_execute("mail__send"))).await.is_success());

    let hold = mail.hold("send");
    let id = alice.submit("Email bob the report.").await;
    tokio::time::timeout(PATIENCE, hold.received()).await.expect("the send reached the app");
    app.kill().await;
    hold.release();
    app.restart().await;

    let me = alice.clone();
    let tid = id.clone();
    eventually("an inbox item asks about the unknown outcome", PATIENCE, || {
        let (me, tid) = (me.clone(), tid.clone());
        async move {
            me.inbox().await.into_iter().find(|i| i["task_id"] == tid.as_str() && i["status"] == "open")
        }
    })
    .await;
    never("the send was repeated", Duration::from_secs(5), || async { mail.calls_to("send").len() > 1 }).await;
}

fn snapshot_tasks(v: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = v["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["task_id"].as_str().unwrap().to_string(), t["status"].as_str().unwrap().to_string()))
        .collect();
    out.sort();
    out
}

fn snapshot_inbox(items: &[Value]) -> Vec<(String, String, String, String)> {
    let mut out: Vec<_> = items
        .iter()
        .map(|i| {
            let s = |k: &str| i[k].as_str().unwrap_or_default().to_string();
            (s("id"), s("kind"), s("task_id"), s("status"))
        })
        .collect();
    out.sort();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn all_state_is_rebuilt_from_the_event_log_alone() {
    let mail = mcp::mail().start().await;
    let model = FakeModel::start(steps(vec![
        contract_checked("send the report", "mail__list_sent", json!({}), "/count", json!(1)),
        send(),
        complete("sent"),
    ]))
    .await;
    let mut app = Appliance::start(
        Spec::default().app("mail", &mail, "public", "private", &[("send", "irreversible"), ("list_sent", "read")]),
        &model,
    )
    .await;
    let alice = app.api("alice");
    // No permit for send: the run waits for approval.
    let id = alice.submit("Email bob the report.").await;
    alice.wait_item(&id, "approval").await;

    let tasks_before = snapshot_tasks(&alice.tasks().await);
    let inbox_before = snapshot_inbox(&alice.inbox().await);

    app.kill().await;
    let admin = app.db.admin().await;
    for (s, t) in app.db.tables().await {
        if !(s == "public" && t == "events") {
            admin.batch_execute(&format!("TRUNCATE \"{s}\".\"{t}\" CASCADE")).await.unwrap();
        }
    }
    app.restart().await;

    assert_eq!(snapshot_tasks(&alice.tasks().await), tasks_before, "task state did not survive on the log alone");
    assert_eq!(snapshot_inbox(&alice.inbox().await), inbox_before, "inbox did not survive on the log alone");
    assert_eq!(mail.effects("send"), 0);

    let item = alice.wait_item(&id, "approval").await;
    let st = alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await;
    assert!(st.is_success(), "approve: {st}");
    let t = alice.wait_settled(&id).await;
    assert_eq!(t["status"], "completed", "{t}");
    assert_eq!(mail.effects("send"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_crashes_never_repeat_an_effect() {
    let chat = mcp::chat().start().await;
    let model = FakeModel::start(script(|c| async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        match c.turn() {
            0 => contract_checked("post four updates", "chat__list_posts", json!({}), "/count", json!(4)),
            n @ 1..=4 => tool("chat__post", json!({ "channel": "team", "text": format!("update {n}") })),
            _ => complete("posted"),
        }
    }))
    .await;
    let mut app = Appliance::start(
        Spec::default().app("chat", &chat, "internal", "private", &[("post", "irreversible"), ("list_posts", "read")]),
        &model,
    )
    .await;
    let alice = app.api("alice");
    assert!(alice.put_policy("org", &org_policy(&permit_execute("chat__post"))).await.is_success());
    let id = alice.submit("Post four updates to the team channel.").await;

    let stop = Arc::new(AtomicBool::new(false));
    for _ in 0..6 {
        let wait = 200 + (uuid::Uuid::new_v4().as_u128() % 1300) as u64;
        tokio::time::sleep(Duration::from_millis(wait)).await;
        if alice.task(&id).await["status"] == "completed" {
            stop.store(true, Ordering::SeqCst);
            break;
        }
        app.restart().await;
    }
    let t = alice.wait_settled(&id).await;
    assert_eq!(t["status"], "completed", "{t}");
    let posts = chat.world()["posts"].as_array().unwrap().clone();
    let texts: Vec<_> = posts.iter().map(|p| p["text"].as_str().unwrap().to_string()).collect();
    for n in 1..=4 {
        let k = texts.iter().filter(|t| **t == format!("update {n}")).count();
        assert_eq!(k, 1, "update {n} was posted {k} times after {} starts: {texts:?}", app.starts());
    }
}
