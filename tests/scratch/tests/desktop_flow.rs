//! The desktop app's frontend, driven in a real browser against a real
//! appliance: sign in, connect Claude, type a task, approve in the inbox,
//! see the outcome with its evidence (`desktop/tests/flow.mjs`).

use magnus_invariants::appliance::workspace_root;
use magnus_invariants::canary;
use magnus_scratch::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_person_signs_in_connects_claude_approves_and_sees_evidence() {
    let f = Flow::start().await;
    let key = canary("anthropic-key");
    let out = tokio::process::Command::new("node")
        .arg(workspace_root().join("desktop/tests/flow.mjs"))
        .env("MAGNUS_URL", &f.app.base)
        .env("MAGNUS_PERSON", PERSON)
        .env("MAGNUS_PASSWORD", PASSWORD)
        .env("MAGNUS_API_KEY", &key)
        .env("MAGNUS_TASK", TASK)
        .env("MAGNUS_CHROMIUM", chromium())
        .envs(std::env::var("MAGNUS_SHOTS").ok().map(|s| ("MAGNUS_SHOTS", s)))
        .output()
        .await
        .unwrap();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the flow failed:\n{log}\n{}", f.report().await);
    f.check_outcome(&key).await;
}
