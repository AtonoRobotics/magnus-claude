//! Regressions from the review of master (2026-10-29), the log and backups:
//! - an append that reached the log without being applied (its caller was
//!   dropped mid-statement) is caught at the next write, and the kernel
//!   starts again from the log;
//! - `magnus restore` refuses while a kernel holds the log;
//! - after a rollback, a backup holding an event of a later release's kind
//!   still passes its restore test, as the log itself still loads.

use magnus_invariants::appliance::bin;
use magnus_invariants::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn an_append_the_kernel_never_applied_is_caught_at_the_next_write() {
    let m = FakeModel::start(steps(vec![fail("nothing to do")])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let first = alice.submit("Say hello.").await;
    alice.wait_settled(&first).await;

    // The owner engaged the kill switch; the insert committed, but the
    // handler was dropped before it applied the event.
    let admin = app.db.admin().await;
    admin
        .execute("INSERT INTO public.events (kind, actor, run_id, body) VALUES ('kill_switch', 'person:alice', NULL, $1)", &[&json!({ "kind": "kill_switch", "engaged": true })])
        .await
        .unwrap();
    assert_eq!(alice.get("/v1/kill-switch").await.1["engaged"], false, "the test needs the event unapplied");

    // The next write notices the gap and the kernel starts again from the
    // log, where the switch is engaged.
    // (Its own answer may be lost as the process starts again.)
    let api = app.api("alice");
    let _ = tokio::spawn(async move { api.post("/v1/tasks", json!({ "text": "Say goodbye." })).await }).await;
    let state = eventually("the kernel applies what is in its log", PATIENCE, || async {
        // Until the process listens again, the request is refused: not yet.
        let r = reqwest::Client::new().get(format!("{}/v1/kill-switch", alice.base)).bearer_auth(&alice.token).send().await.ok()?;
        let v: serde_json::Value = r.error_for_status().ok()?.json().await.ok()?;
        (v["engaged"] == true).then_some(v)
    })
    .await;
    assert_eq!(state["engaged"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restore_waits_for_no_kernel_and_a_rolled_back_log_still_backs_up() {
    let m = FakeModel::start(steps(vec![fail("nothing to do")])).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let t = alice.submit("Say hello.").await;
    alice.wait_settled(&t).await;

    // A later release wrote a kind this one does not know, then the
    // appliance went back one release: the log loads, skipping it...
    app.kill().await;
    let admin = app.db.admin().await;
    admin
        .execute("INSERT INTO public.events (kind, actor, run_id, body) VALUES ('from_a_later_release', 'kernel', NULL, $1)", &[&json!({ "kind": "from_a_later_release", "x": 1 })])
        .await
        .unwrap();
    app.restart().await;
    // ...and so does its backup's restore test.
    let (st, b) = alice.post("/v1/backups", json!({})).await;
    assert_eq!(st, 201, "{b}");
    assert_eq!(b["restored"], true, "a rolled-back log's backup fails its restore test: {b}");
    let file = app.state_dir.join("backups").join(b["file"].as_str().unwrap());

    // The log and the vault are lost; a kernel (here, its lock) is running
    // on the empty log, so the restore is refused.
    app.kill().await;
    admin.batch_execute("DROP TABLE public.content, public.events").await.unwrap();
    std::fs::remove_dir_all(app.state_dir.join("vault")).unwrap();
    let (config, url) = (app.dir.path().join("magnus.toml"), app.db.admin_url());
    let restore = || {
        std::process::Command::new(bin("magnus-appliance")).args(["restore", "--config"]).arg(&config).args(["--admin-url", &url, "--from"]).arg(&file).output().unwrap()
    };
    let lock = app.db.admin().await;
    assert!(lock.query_one("SELECT pg_try_advisory_lock(7239001)", &[]).await.unwrap().get::<_, bool>(0));
    let out = restore();
    assert!(!out.status.success(), "a restore ran under a live kernel");
    assert!(String::from_utf8_lossy(&out.stderr).contains("a kernel is running"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!app.state_dir.join("vault/secrets.json").exists(), "a refused restore wrote the vault");
    drop(lock);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let out = restore();
    assert!(out.status.success(), "restore failed: {}", String::from_utf8_lossy(&out.stderr));
    app.restart().await;
    assert_eq!(alice.task(&t).await["status"], "failed");
}

/// Review of master (2026-10-31): a kernel started in place of one just
/// killed (a crash, a restart) found the killed one's lock still held, as
/// Postgres ends its session a moment later, and exited; under load the
/// invariant 3 crash test failed so in CI. It waits for the lock a while.
#[tokio::test(flavor = "multi_thread")]
async fn a_kernel_started_in_place_of_one_just_killed_waits_for_its_lock() {
    let m = FakeModel::start(steps(vec![fail("nothing to do")])).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    app.kill().await;
    // The killed kernel's session, as Postgres keeps it a moment longer.
    let lock = app.db.admin().await;
    assert!(lock.query_one("SELECT pg_try_advisory_lock(7239001)", &[]).await.unwrap().get::<_, bool>(0));
    let release = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        drop(lock);
    });
    // The harness fails if the appliance exits instead of coming up.
    app.restart().await;
    release.await.unwrap();
    let alice = app.api("alice");
    let t = alice.submit("Say hello.").await;
    alice.wait_settled(&t).await;
}
