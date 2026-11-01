//! The kill switch (B11, K53), one control for the owner: every action is
//! refused, every run is suspended, and the vault is sealed; each OAuth
//! sign-in is revoked at its provider and taken out of the vault, while API
//! keys and app secrets stay sealed (only their provider's console can revoke
//! them). Releasing it resumes nothing; the owner signs in again and resumes
//! each task. The same, as the owner sees it on Home (`desktop/tests/killswitch.mjs`).

use axum::{extract::State, routing::{get, post}, Form, Json, Router};
use magnus_invariants::appliance::{bin, workspace_root};
use magnus_invariants::*;
use magnus_scratch::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Seen = Arc<Mutex<Vec<HashMap<String, String>>>>;

/// A provider that publishes its revocation endpoint in its discovery
/// document, as OpenAI's and xAI's do, and records what it is asked to revoke.
async fn oauth_provider() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let u = url.clone();
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(move || async move { Json(json!({ "issuer": u, "token_endpoint": format!("{u}/token"), "revocation_endpoint": format!("{u}/revoke") })) }))
        .route("/token", post(|| async { Json(json!({ "access_token": "acc-2", "refresh_token": "ref-2", "expires_in": 3600 })) }))
        .route("/models", get(|| async { Json(json!({ "data": [{ "id": "oa-1" }] })) }))
        .route(
            "/revoke",
            post(|State(s): State<Seen>, Form(f): Form<HashMap<String, String>>| async move {
                s.lock().unwrap().push(f);
                ""
            }),
        )
        .with_state(seen.clone());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_kill_switch_revokes_sign_ins_seals_the_vault_and_releasing_resumes_nothing() {
    let (provider, revoked) = oauth_provider().await;
    let mail = mcp::mail().start().await;
    let in_turn = Arc::new(AtomicBool::new(false));
    let go = Arc::new(tokio::sync::Notify::new());
    let (t, g) = (in_turn.clone(), go.clone());
    let m = FakeModel::start(script(move |c| {
        let (t, g) = (t.clone(), g.clone());
        async move {
            let send = tool("mail__send", json!({ "to": "bob@corp.example", "subject": "hi", "body": "hello" }));
            match c.turn() {
                0 => contract_unchecked("send a note", "note sent"),
                // Hold the model call open until the kill switch is engaged.
                1 => {
                    if !t.swap(true, Ordering::SeqCst) {
                        g.notified().await;
                    }
                    send
                }
                2 if c.last_results().iter().any(|r| r.is_error) => send,
                _ => complete("Sent."),
            }
        }
    }))
    .await;
    let mut app = Appliance::start(Spec::default().app("mail", &mail, "public", "private", &[("send", "irreversible")]), &m).await;
    let alice = app.api("alice");
    let signed_in = json!({ "access_token": "acc-1", "refresh_token": "ref-1", "expires_at": 4_102_444_800i64 }).to_string();
    assert!(alice.put_secret("oa_credential", &signed_in).await.is_success());
    let path = app.dir.path().join("magnus.toml");
    let mut cfg: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    let oa = json!({
        "id": "oa",
        "command": [bin("magnus-adapter-openai")],
        "settings": { "api": "chat", "base_url": provider, "credential": "oa_credential",
            "auth": { "kind": "oauth_device", "token_url": format!("{provider}/token"), "client_id": "magnus-test" } }
    });
    cfg["adapters"].as_array_mut().unwrap().push(toml::Value::try_from(oa).unwrap());
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    app.restart().await;
    let alice = app.api("alice");
    let carol = app.api("carol");
    let before: Vec<String> = serde_json::from_value(alice.get("/v1/secrets").await.1["names"].clone()).unwrap();
    assert!(before.contains(&"oa_credential".to_string()) && before.contains(&"mail_token".to_string()), "{before:?}");
    assert!(alice.put_policy("org", &org_policy(&permit_execute("mail__send"))).await.is_success());

    // One control, the owner's only.
    assert!(matches!(carol.post("/v1/kill-switch", json!({ "engaged": true })).await.0.as_u16(), 401 | 403));
    let id = alice.submit("Email bob a hello.").await;
    eventually("the task is in a model call", PATIENCE, || {
        let t = in_turn.clone();
        async move { t.load(Ordering::SeqCst).then_some(()) }
    })
    .await;
    let (st, v) = alice.post("/v1/kill-switch", json!({ "engaged": true })).await;
    assert!(st.is_success(), "{v}");
    assert_eq!(v["revoking"], json!(["oa_credential"]), "only the OAuth sign-in is revoked: {v}");
    assert!(v["sealed"].as_array().unwrap().contains(&json!("mail_token")), "{v}");
    go.notify_one();

    // Suspended, and the action it then asked for was refused.
    let t = alice.wait_status(&id, &["suspended"]).await;
    assert_eq!(t["waiting_on"], "kill_switch", "{t}");
    assert!(mail.calls_to("send").is_empty(), "an action ran under the kill switch");

    // The sign-in was revoked at the provider: the refresh token (which ends
    // the grant) and the access token, and it left the vault.
    let asked = eventually("the provider was asked to revoke the sign-in", PATIENCE, || {
        let r = revoked.clone();
        async move {
            let r = r.lock().unwrap().clone();
            (r.len() == 2).then_some(r)
        }
    })
    .await;
    let pairs: Vec<(String, String)> = asked.iter().map(|f| (f["token_type_hint"].clone(), f["token"].clone())).collect();
    assert_eq!(pairs, [("refresh_token".into(), "ref-1".into()), ("access_token".into(), "acc-1".into())]);
    assert!(asked.iter().all(|f| f["client_id"] == "magnus-test"));
    let ks = eventually("the revocation is recorded", PATIENCE, || {
        let alice = alice.clone();
        async move {
            let v = alice.get("/v1/kill-switch").await.1;
            v["revoked"]["oa_credential"]["at_provider"].is_boolean().then_some(v)
        }
    })
    .await;
    assert_eq!(ks["engaged"], true);
    assert_eq!(ks["revoked"]["oa_credential"]["at_provider"], true, "{ks}");
    assert_eq!(ks["revoked"]["oa_credential"]["adapter"], "oa", "{ks}");
    assert!(ks["sealed"].as_array().unwrap().contains(&json!("mail_token")), "{ks}");
    assert!(matches!(carol.get("/v1/kill-switch").await.0.as_u16(), 401 | 403), "someone else saw the vault's names");
    let names: Vec<String> = serde_json::from_value(alice.get("/v1/secrets").await.1["names"].clone()).unwrap();
    assert!(!names.contains(&"oa_credential".to_string()), "the revoked sign-in is still in the vault: {names:?}");
    // Nothing goes into the vault while it is sealed, even across a restart.
    assert_eq!(alice.put_secret("oa_credential", &signed_in).await.as_u16(), 409);
    app.restart().await;
    let alice = app.api("alice");
    assert_eq!(alice.get("/v1/kill-switch").await.1["engaged"], true);
    assert_eq!(alice.put_secret("oa_credential", &signed_in).await.as_u16(), 409, "a restart unsealed the vault");

    // Released: nothing resumes by itself (K53).
    assert!(alice.post("/v1/kill-switch", json!({ "engaged": false })).await.0.is_success());
    never("a run resumed by itself after the kill switch was released", Duration::from_secs(4), || async {
        alice.task(&id).await["status"] != "suspended" || !mail.calls_to("send").is_empty()
    })
    .await;
    let ks = alice.get("/v1/kill-switch").await.1;
    assert_eq!(ks["revoked"]["oa_credential"]["at_provider"], true, "the owner is not told what to sign in to again: {ks}");
    // The owner signs in again, and resumes the task; the app secret was kept.
    assert!(alice.put_secret("oa_credential", &signed_in).await.is_success());
    assert!(alice.get("/v1/kill-switch").await.1["revoked"].as_object().unwrap().is_empty());
    assert!(alice.post(&format!("/v1/tasks/{id}/resume"), json!({})).await.0.is_success());
    eventually("the resumed task sends with the app's kept secret", PATIENCE, || async { (mail.effects("send") == 1).then_some(()) }).await;

    // The same control on Home, as the owner sees it.
    assert!(set_password(&app, "alice", PASSWORD));
    let out = tokio::process::Command::new("node")
        .arg(workspace_root().join("desktop/tests/killswitch.mjs"))
        .env("MAGNUS_URL", &app.base)
        .env("MAGNUS_PERSON", "alice")
        .env("MAGNUS_PASSWORD", PASSWORD)
        .env("MAGNUS_CHROMIUM", chromium())
        .envs(shots())
        .output()
        .await
        .unwrap();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the kill switch on Home failed:\n{log}");
    let ks = alice.get("/v1/kill-switch").await.1;
    assert_eq!(ks["engaged"], false, "{ks}");
    assert_eq!(ks["revoked"]["oa_credential"]["at_provider"], true, "{ks}");
}

/// Releasing the switch and telling each person which of their tasks it
/// stopped are one write: no release is left untold by a crash between them.
#[tokio::test(flavor = "multi_thread")]
async fn releasing_the_switch_and_its_notices_are_one_write() {
    let in_turn = Arc::new(AtomicBool::new(false));
    let go = Arc::new(tokio::sync::Notify::new());
    let (t, g) = (in_turn.clone(), go.clone());
    let m = FakeModel::start(script(move |c| {
        let (t, g) = (t.clone(), g.clone());
        async move {
            match c.turn() {
                0 => contract_unchecked("write a note", "note written"),
                // Hold the model call open until the kill switch is engaged.
                _ => {
                    if !t.swap(true, Ordering::SeqCst) {
                        g.notified().await;
                    }
                    complete("Written.")
                }
            }
        }
    }))
    .await;
    let app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let id = alice.submit("Write a note.").await;
    eventually("the task is in a model call", PATIENCE, || {
        let t = in_turn.clone();
        async move { t.load(Ordering::SeqCst).then_some(()) }
    })
    .await;
    assert!(alice.post("/v1/kill-switch", json!({ "engaged": true })).await.0.is_success());
    go.notify_one();
    assert_eq!(alice.wait_status(&id, &["suspended"]).await["waiting_on"], "kill_switch");
    let (st, v) = alice.post("/v1/kill-switch", json!({ "engaged": false })).await;
    assert!(st.is_success() && v["told"] == 1, "{v}");

    let rows = app.db.admin().await.query("SELECT kind, at::text, body->>'item_kind' FROM public.events ORDER BY seq", &[]).await.unwrap();
    let rows: Vec<(String, String, Option<String>)> = rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect();
    let release = rows.iter().rposition(|r| r.0 == "kill_switch").expect("the release is in the log");
    let next = rows.get(release + 1).expect("a notice after the release");
    assert_eq!((next.0.as_str(), next.2.as_deref()), ("item", Some("notice")), "{rows:?}");
    assert_eq!(next.1, rows[release].1, "the release and its notice were written apart: {rows:?}");
}
