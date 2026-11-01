//! The walls between people in the content store (owner's ruling 9 and
//! "recall only", 2026-11-01; `docs/proposals/recall-only.md`, step 1):
//! every row of content records whose it is, and the per-person reader
//! role sees only that person's rows, whatever query it runs.

use magnus_invariants::appliance::bin;
use magnus_invariants::*;
use serde_json::json;
use tokio_postgres::Client;

/// Content rows of runs whose person is not the one their row records,
/// and how many rows each person has.
async fn owners(admin: &Client) -> (i64, i64, i64) {
    let wrong = admin
        .query_one(
            "SELECT count(*) FROM public.content c JOIN public.events e USING (seq)
             WHERE e.run_id IS NOT NULL AND c.person IS DISTINCT FROM
                 (SELECT t.body->>'person' FROM public.events t WHERE t.kind = 'task_created' AND t.run_id = e.run_id LIMIT 1)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let count = |who: &'static str| async move { admin.query_one("SELECT count(*) FROM public.content WHERE person = $1", &[&who]).await.unwrap().get::<_, i64>(0) };
    (wrong, count("alice").await, count("carol").await)
}

/// What `{role}_person` returns to a query, reading for `person`.
async fn as_reader(kernel: &mut Client, role: &str, person: Option<&str>, sql: &str) -> Result<Vec<String>, tokio_postgres::Error> {
    let tx = kernel.transaction().await?;
    tx.batch_execute(&format!("SET LOCAL ROLE {role}_person")).await?;
    if let Some(p) = person {
        tx.execute("SELECT set_config('magnus.person', $1, true)", &[&p]).await?;
    }
    let rows = tx.query(sql, &[]).await?;
    tx.commit().await?;
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

#[tokio::test(flavor = "multi_thread")]
async fn each_persons_content_is_walled_from_everyone_else_in_the_database() {
    let m = FakeModel::start(steps(vec![fail("nothing to do")])).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    let (alice, carol) = (app.api("alice"), app.api("carol"));
    let (alices, carols) = (canary("alice-words"), canary("carol-words"));
    let a = alice.submit(&format!("Draft the board letter. {alices}")).await;
    let c = carol.submit(&format!("Plan the offsite. {carols}")).await;
    alice.wait_settled(&a).await;
    carol.wait_settled(&c).await;
    let (st, v) = carol.post("/v1/schedules", json!({ "text": format!("Water the plants. {carols}"), "daily_at": "03:00" })).await;
    assert_eq!(st.as_u16(), 201, "{v}");

    // Each row records its person: the task's text, and its run's replies.
    let admin = app.db.admin().await;
    let (wrong, a_rows, c_rows) = owners(&admin).await;
    assert_eq!(wrong, 0, "a run's content is recorded as someone else's, or no one's");
    assert!(a_rows >= 2 && c_rows >= 3, "alice {a_rows}, carol {c_rows}");

    // Reading for carol, the wall returns carol's content and none of alice's.
    let role = app.db.kernel_role.clone();
    let (mut kernel, conn) = tokio_postgres::connect(&app.db.kernel_url(), tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    let seen = as_reader(&mut kernel, &role, Some("carol"), "SELECT body::text FROM public.content").await.unwrap();
    assert!(seen.iter().any(|b| b.contains(&carols)), "carol's own content is walled from her");
    assert!(!seen.iter().any(|b| b.contains(&alices)), "alice's content reached a reader for carol");
    let asked = as_reader(&mut kernel, &role, Some("carol"), "SELECT body::text FROM public.content WHERE person = 'alice' OR person IS NULL").await.unwrap();
    assert!(asked.is_empty(), "a query asking for alice's rows got them: {asked:?}");
    // The log itself is not readable through the wall at all.
    let joined = as_reader(&mut kernel, &role, Some("carol"), "SELECT c.body::text FROM public.content c JOIN public.events e USING (seq)").await;
    assert!(joined.is_err(), "the reader role reads the event log: {joined:?}");
    // Reading for no one returns nothing.
    assert!(as_reader(&mut kernel, &role, None, "SELECT body::text FROM public.content").await.unwrap().is_empty());
    // The kernel's own role still reads every row, as replay needs.
    let all: i64 = kernel.query_one("SELECT count(*) FROM public.content", &[]).await.unwrap().get(0);
    assert!(all >= a_rows + c_rows);

    // Rows written before the column existed are filled by `migrate`.
    admin.batch_execute("UPDATE public.content SET person = NULL").await.unwrap();
    let out = std::process::Command::new(bin("magnus-appliance"))
        .args(["migrate", "--config"])
        .arg(app.dir.path().join("magnus.toml"))
        .args(["--admin-url", &app.db.admin_url()])
        .output()
        .unwrap();
    assert!(out.status.success(), "migrate: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(owners(&admin).await, (0, a_rows, c_rows), "migrate did not fill whose each row is");

    // And so are rows restored from a backup, which does not say.
    let (st, b) = alice.post("/v1/backups", json!({})).await;
    assert_eq!(st, 201, "{b}");
    let file = app.state_dir.join("backups").join(b["file"].as_str().unwrap());
    app.kill().await;
    drop(kernel);
    admin.batch_execute("DROP TABLE public.content, public.events").await.unwrap();
    std::fs::remove_dir_all(app.state_dir.join("vault")).unwrap();
    let out = std::process::Command::new(bin("magnus-appliance"))
        .args(["restore", "--config"])
        .arg(app.dir.path().join("magnus.toml"))
        .args(["--admin-url", &app.db.admin_url(), "--from"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(out.status.success(), "restore: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(owners(&admin).await, (0, a_rows, c_rows), "restored rows do not say whose they are");
    app.restart().await;
    assert_eq!(alice.get("/v1/me").await.0, 200);
}
