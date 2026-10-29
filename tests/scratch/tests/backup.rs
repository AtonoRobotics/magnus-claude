//! Backup and tested restore of the event log and the vault (K10): a
//! backup is encrypted, restored in memory as soon as it is written, kept
//! no longer than its keeping time, and after the log and the vault are
//! lost, `magnus restore` brings back the appliance's history and secrets.

use magnus_invariants::appliance::bin;
use magnus_invariants::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn a_backup_restores_the_log_and_the_vault_after_both_are_lost() {
    let mail = mcp::mail().start().await;
    let m = FakeModel::start(script(|c| async move {
        match c.turn() {
            0 => contract_unchecked("read the inbox", "the inbox was read"),
            1 => tool("mail__list_inbox", json!({})),
            _ => complete("read"),
        }
    }))
    .await;
    let mut app = Appliance::start(Spec::default().app("mail", &mail, "public", "private", &[("list_inbox", "read")]), &m).await;
    let alice = app.api("alice");
    let words = canary("task-words");
    let t1 = alice.submit(&format!("Read my inbox. {words}")).await;
    alice.wait_settled(&t1).await;
    assert!(alice.put_policy("user/alice", r#"forbid(principal, action, resource == Tool::"mail__send");"#).await.is_success());

    // Only the owner backs up; an expired backup goes when a new one is made.
    assert_eq!(app.api("carol").post("/v1/backups", json!({})).await.0, 403);
    let dir = app.state_dir.join("backups");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("magnus-20200101T000000.000Z.backup"), b"old").unwrap();
    let (st, b) = alice.post("/v1/backups", json!({})).await;
    assert_eq!(st, 201, "{b}");
    assert_eq!(b["restored"], true, "the backup did not restore in memory: {b}");
    assert_eq!(b["removed"], json!(["magnus-20200101T000000.000Z.backup"]), "{b}");
    let (_, list) = alice.get("/v1/backups").await;
    assert_eq!(list["latest"]["restored"], true, "{list}");
    assert_eq!(list["files"].as_array().unwrap().len(), 1, "{list}");

    // Encrypted: neither the person's words nor a secret is in the file.
    let file = dir.join(b["file"].as_str().unwrap());
    let raw = std::fs::read(&file).unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains(&words) && !text.contains(&app.app_secrets["mail"]), "the backup is readable");

    let before = alice.tasks().await;
    // The disaster: the log and the vault are gone. The backups and their key are kept.
    app.kill().await;
    app.db.admin().await.batch_execute("DROP TABLE public.events").await.unwrap();
    std::fs::remove_dir_all(app.state_dir.join("vault")).unwrap();

    let (config, admin) = (app.dir.path().join("magnus.toml"), app.db.admin_url());
    let restore = |from: &std::path::Path| {
        std::process::Command::new(bin("magnus-appliance"))
            .args(["restore", "--config"])
            .arg(&config)
            .args(["--admin-url", &admin, "--from"])
            .arg(from)
            .output()
            .unwrap()
    };
    let out = restore(&file);
    assert!(out.status.success(), "restore failed: {}", String::from_utf8_lossy(&out.stderr));
    app.restart().await;

    // The history is back, and so are the policy and the app's credential.
    let after = alice.tasks().await;
    assert_eq!(before, after, "the tasks differ after the restore");
    let (_, p) = alice.get("/v1/policies").await;
    assert!(p.to_string().contains("mail__send"), "the policy was not restored: {p}");
    let t2 = alice.submit("Read my inbox again.").await;
    let done = alice.wait_settled(&t2).await;
    assert_eq!(done["waiting_on"], "review", "{done}");
    assert_eq!(mail.calls_to("list_inbox").len(), 2, "the app's credential did not come back");

    // A restore never writes over a log that has history.
    let out = restore(&file);
    assert!(!out.status.success(), "a restore wrote over a live log");
}
