//! Found on strix-alpha (2026-10-29): Postgres dropped every connection
//! ("terminating connection because of crash of another server process"),
//! and the appliance never opened its log again. For forty minutes every
//! write failed while health said ok. Now health says the log is down while
//! it is, and the log opens again, with its single-writer lock, by itself.

use magnus_invariants::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_log_connection_opens_again_and_health_tells_the_truth_meanwhile() {
    // Every task ends at once; what matters is that its end is written.
    let m = FakeModel::start(steps(vec![fail("nothing to do")])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let first = alice.submit("Say hello.").await;
    alice.wait_settled(&first).await;

    // Postgres drops the kernel's connections, as when another of its processes crashes.
    let admin = app.db.admin().await;
    let dropped = admin
        .query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = $1", &[&app.db.kernel_role])
        .await
        .unwrap();
    assert!(!dropped.is_empty(), "the kernel had no connection to drop");

    // The log opens again by itself, with its single-writer lock; health is
    // ok only once it can write (terminating is not instant, so wait for both).
    let held = || async {
        admin
            .query_one("SELECT count(*) > 0 FROM pg_locks l JOIN pg_stat_activity a USING (pid) WHERE l.locktype = 'advisory' AND l.objid = 7239001 AND a.usename = $1", &[&app.db.kernel_role])
            .await
            .unwrap()
            .get::<_, bool>(0)
    };
    let health = eventually("the log open again, with its lock", PATIENCE, || async {
        let (st, v) = alice.get("/v1/health").await;
        (st.as_u16() == 200 && held().await).then_some(v)
    })
    .await;
    assert_eq!(health["log"], "up", "{health}");

    // Work goes on, written to the log.
    let second = alice.submit("Say goodbye.").await;
    let t = alice.wait_settled(&second).await;
    assert_eq!(t["status"], "failed", "{t}");
    let n: i64 = admin.query_one("SELECT count(*) FROM public.events WHERE kind = 'failed'", &[]).await.unwrap().get(0);
    assert_eq!(n, 2, "a task's end is missing from the log");
}
