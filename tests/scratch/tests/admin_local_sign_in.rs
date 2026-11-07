//! Signing in to the admin console against the local store (#429): no
//! provider involved. An active admin with a password (and code where
//! enrolled) signs in at `/auth/local`; a member, a wrong password and an
//! unknown name refuse alike with no session; changes still need the CSRF
//! token; step-up confirms with the password; disabling ends the session
//! at the next check.

use magnus_invariants::appliance::{bin, Appliance, Spec};
use magnus_invariants::*;
use magnus_scratch::admin::*;
use magnus_scratch::*;
use reqwest::StatusCode;
use serde_json::Value;
use std::io::Write;

fn cli(app: &Appliance, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut c = std::process::Command::new(bin("magnus-appliance"));
    c.args(args).arg("--config").arg(app.dir.path().join("magnus.toml"));
    if stdin.is_some() {
        c.stdin(std::process::Stdio::piped());
    }
    let mut c = c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    if let Some(text) = stdin {
        writeln!(c.stdin.take().unwrap(), "{text}").unwrap();
    }
    c.wait_with_output().unwrap()
}

fn password(app: &Appliance, person: &str, pw: &str) {
    let out = cli(app, &["password", "--person", person], Some(pw));
    assert!(out.status.success(), "magnus password: {}", String::from_utf8_lossy(&out.stderr));
}

fn enroll(app: &Appliance, person: &str) -> String {
    let out = cli(app, &["totp-enroll", "--person", person], None);
    assert!(out.status.success(), "magnus totp-enroll: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn role(app: &Appliance, person: &str, role: &str) {
    let out = cli(app, &["person-role", "--person", person, "--role", role], None);
    assert!(out.status.success(), "magnus person-role: {}", String::from_utf8_lossy(&out.stderr));
}

fn status(app: &Appliance, person: &str, status: &str) {
    let out = cli(app, &["person-status", "--person", person, "--status", status], None);
    assert!(out.status.success(), "magnus person-status: {}", String::from_utf8_lossy(&out.stderr));
}

fn code(uri: &str) -> String {
    let secret = uri.split("secret=").nth(1).and_then(|s| s.split('&').next()).unwrap();
    let raw = magnus::totp::base32_decode(secret).unwrap();
    magnus::totp::generate(&raw, (chrono::Utc::now().timestamp().div_euclid(30)) as u64, 6)
}

/// The console session cookie of a response, if it set one.
fn sid_of(r: &reqwest::Response) -> Option<String> {
    r.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next()?.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .find(|(k, _)| k == "__Host-magnus_admin")
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_signs_in_locally_and_a_member_and_a_wrong_password_do_not() {
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    password(&app, "alice", "alice-long-password");
    role(&app, "alice", "admin");
    password(&app, "bob", "bob-long-password");
    role(&app, "bob", "admin");
    password(&app, "carol", "carol-long-password");
    let mine = format!("{}/32", lan_ip());
    let p = prepare(&app, &[&mine], &[("refresh_seconds", 2.into()), ("step_up_seconds", 90.into())]);
    let url = p.url();
    let c = launch(p).await;
    let api = |path: &str| format!("{url}/api/v1{path}");
    let b = browser(&c, false);

    // The form is there, naming the local way in.
    let r = b.get(format!("{url}/auth/local")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let body = r.text().await.unwrap();
    assert!(body.contains("<form") && body.contains("name=password"), "no local sign-in form: {body}");

    // A wrong password, an unknown name and a member refuse alike, with no session.
    for (who, pw) in [("alice", "wrong-password"), ("nobody", "whatever-password"), ("carol", "carol-long-password")] {
        let r = b.post(format!("{url}/auth/local")).form(&[("person", who), ("password", pw), ("then", "/")]).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{who} was not refused");
        assert!(sid_of(&r).is_none(), "{who} was given a console session");
        let body = r.text().await.unwrap();
        assert!(body.contains("wrong"), "{who} got no alike refusal: {body}");
    }

    // The admin signs in with the password; she is stepped up from it.
    let r = b.post(format!("{url}/auth/local")).form(&[("person", "alice"), ("password", "alice-long-password"), ("then", "/")]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER, "the password did not sign in");
    let sid = sid_of(&r).expect("no session cookie on local sign-in");
    let me: Value = with_cookie(b.get(api("/session")), &sid).send().await.unwrap().json().await.unwrap();
    assert!(me["csrf"].as_str().is_some_and(|t| t.len() >= 20) && me["stepped_up_until"].is_string(), "{me}");
    let csrf = me["csrf"].as_str().unwrap().to_string();

    // A change without the session's token is refused; with it, done.
    let r = with_cookie(b.post(api("/session/step-up")), &sid).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN, "a change without the CSRF token was taken");
    let r = with_cookie(b.post(api("/session/step-up")), &sid).header("x-csrf-token", &csrf).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT, "a fresh step-up did not count");

    // Step-up for a local session goes to the local form, which confirms
    // with the password and comes back.
    let r = with_cookie(b.get(format!("{url}/auth/step-up?then=/auth/done")), &sid).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let to = r.headers().get(reqwest::header::LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(to.contains("/auth/local/step-up"), "local step-up did not route locally: {to}");
    let r = with_cookie(b.post(format!("{url}/auth/local/step-up")), &sid).form(&[("password", "alice-long-password"), ("then", "/auth/done")]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER, "the password did not confirm the step-up");

    // Disabling ends the session at the next check, within seconds.
    status(&app, "alice", "disabled");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let r = with_cookie(b.get(api("/session")), &sid).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "the disabled admin kept the console");
    status(&app, "alice", "active");

    // Enrolling the code asks for it from then on: the password alone
    // shows the code screen, and with the code she is in again.
    // (Enrolled last so no code is ever used twice in this test.)
    let uri = enroll(&app, "alice");
    let r = b.post(format!("{url}/auth/local")).form(&[("person", "alice"), ("password", "alice-long-password"), ("then", "/")]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(sid_of(&r).is_none(), "the password alone opened a session past the code");
    assert!(r.text().await.unwrap().contains("6-digit code"), "no code screen");
    let r = b.post(format!("{url}/auth/local")).form(&[("person", "alice"), ("password", "alice-long-password"), ("code", &code(&uri)), ("then", "/")]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER, "the code did not sign in");
    let sid = sid_of(&r).expect("no session cookie past the code");

    // Signing out ends the session.
    let me: Value = with_cookie(b.get(api("/session")), &sid).send().await.unwrap().json().await.unwrap();
    let csrf = me["csrf"].as_str().unwrap().to_string();
    let r = with_cookie(b.delete(api("/session")), &sid).header("x-csrf-token", &csrf).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(with_cookie(b.get(api("/session")), &sid).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
}
