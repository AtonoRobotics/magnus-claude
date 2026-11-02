//! Signing in to the admin console (admin console spec, invariant tests,
//! Sign-in and authority; owner's rulings 1 and 2: no passkeys, so the
//! password, and the code when there is one, on the provider's own page),
//! against a real Keycloak in a real Chromium. Only the realm role `admin`
//! enters; a member's app session or token opens nothing; every change
//! carries the session's CSRF token; a step-up is a fresh password and
//! lapses; an admin turned off at the provider loses the console at the next
//! check, well within five minutes.

use magnus_invariants::*;
use magnus_scratch::admin::*;
use magnus_scratch::*;
use reqwest::StatusCode;
use serde_json::Value;

/// The browser signs in as `who` (and steps up after, if asked): the
/// console's session cookie, or the script's log when it fails.
async fn sign_in(p_url: &str, at: std::net::SocketAddr, who: &str, expect: &str, step_up: bool) -> Result<String, String> {
    let out = tokio::process::Command::new("node")
        .arg(magnus_invariants::appliance::workspace_root().join("desktop/tests/admin-sign-in.mjs"))
        .env("MAGNUS_ADMIN_URL", p_url)
        .env("MAGNUS_ADMIN_AT", at.to_string())
        .env("MAGNUS_PERSON", who.to_lowercase())
        .env("MAGNUS_PASSWORD", password_of(who))
        .env("MAGNUS_EXPECT", expect)
        .env("MAGNUS_STEP_UP", if step_up { "1" } else { "0" })
        .env("MAGNUS_CHROMIUM", chromium())
        .envs(shots())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(150), out).await.map_err(|_| "the browser did not finish in 150 seconds".to_string())?.unwrap();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return Err(log);
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or_default().trim().to_string())
}

fn with_cookie(b: reqwest::RequestBuilder, sid: &str) -> reqwest::RequestBuilder {
    b.header("cookie", format!("__Host-magnus_admin={sid}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_admin_signs_in_on_the_providers_page_and_the_session_holds_to_its_rules() {
    let kc = Keycloak::start().await;
    kc.add_person("Alice").await;
    kc.set_roles("Alice", &["admin"]).await;
    kc.add_person("Carol").await;
    let m = FakeModel::start(steps(vec![])).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    kc.configure(&mut app).await;
    let mine = format!("{}/32", lan_ip());
    // Short windows, so the test sees a step-up lapse and a turned-off
    // admin found out without waiting minutes.
    let p = prepare(&app, &[&mine], &[("issuer", kc.issuer.clone().into()), ("step_up_seconds", 8.into()), ("refresh_seconds", 2.into())]);
    kc.add_admin_client("magnus-admin", &format!("{}/auth/callback", p.url()), &p.jwks().await).await;
    let url = p.url();
    let c = launch(p).await;
    let api = |path: &str| format!("{url}/api/v1{path}");
    let b = browser(&c, false);

    // A member signs in at the provider and is turned away at the door.
    let carol = sign_in(&url, c.addr, "Carol", "refused", false).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(carol.is_empty(), "a member was given a console session");
    // A member's app session, or the admin's own, opens nothing here.
    let (st, v) = kc.app_sign_in(&app, "Alice").await;
    assert_eq!(st, 200, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    for path in ["/session", "/users", "/session/step-up"] {
        let r = b.get(api(path)).bearer_auth(&token).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{path} took the app's token");
        let r = b.post(api(path)).header("x-magnus-token", &token).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{path} took the app's token");
    }

    // The admin signs in on the provider's page, and steps up there too.
    let sid = sign_in(&url, c.addr, "Alice", "admin", true).await.unwrap_or_else(|e| panic!("{e}"));
    assert!(!sid.is_empty(), "the admin has no console session");
    let me: Value = with_cookie(b.get(api("/session")), &sid).send().await.unwrap().json().await.unwrap();
    assert!(me["csrf"].as_str().is_some_and(|t| t.len() >= 20) && me["stepped_up_until"].is_string(), "{me}");
    let csrf = me["csrf"].as_str().unwrap().to_string();

    // A change without the session's token is refused; with it, done.
    let r = with_cookie(b.post(api("/session/step-up")), &sid).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN, "a change without the CSRF token was taken");
    let r = with_cookie(b.post(api("/session/step-up")), &sid).header("x-csrf-token", &csrf).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT, "a fresh step-up did not count");
    // Once the step-up lapses, an action that needs one says so.
    tokio::time::sleep(std::time::Duration::from_secs(9)).await;
    let r = with_cookie(b.post(api("/session/step-up")), &sid).header("x-csrf-token", &csrf).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"], "step_up_required", "{v}");

    // Signing out ends the session.
    let r = with_cookie(b.delete(api("/session")), &sid).header("x-csrf-token", &csrf).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(with_cookie(b.get(api("/session")), &sid).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);

    // Signed in again, then turned off at the provider: out at the next check.
    let sid = sign_in(&url, c.addr, "Alice", "admin", false).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(with_cookie(b.get(api("/session")), &sid).send().await.unwrap().status(), StatusCode::OK);
    kc.disable("Alice").await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let r = with_cookie(b.get(api("/session")), &sid).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "an admin turned off at the provider kept the console");
    kc.enable("Alice").await;
    assert_eq!(with_cookie(b.get(api("/session")), &sid).send().await.unwrap().status(), StatusCode::UNAUTHORIZED, "the ended session came back");

    // Every one of those is in the audit log, with who and from where; the
    // log's CSV says the same.
    let sid = sign_in(&url, c.addr, "Alice", "admin", false).await.unwrap_or_else(|e| panic!("{e}"));
    let audit: Value = with_cookie(b.get(api("/audit")), &sid).send().await.unwrap().json().await.unwrap();
    let actions: Vec<&str> = audit["entries"].as_array().unwrap().iter().filter_map(|e| e["action"].as_str()).collect();
    for a in ["sign_in_refused", "sign_in", "step_up", "sign_out", "session_ended_by_provider"] {
        assert!(actions.contains(&a), "{a} is not in the audit: {audit}");
    }
    let ip = lan_ip().to_string();
    assert!(audit["entries"].as_array().unwrap().iter().all(|e| e["source"] == ip.as_str() && e["at"].is_string()), "{audit}");
    let filtered: Value = with_cookie(b.get(api("/audit?action=sign_out")), &sid).send().await.unwrap().json().await.unwrap();
    assert!(filtered["entries"].as_array().unwrap().iter().all(|e| e["action"] == "sign_out") && !filtered["entries"].as_array().unwrap().is_empty(), "{filtered}");
    let csv = with_cookie(b.get(api("/audit.csv")), &sid).send().await.unwrap().text().await.unwrap();
    assert!(csv.starts_with("seq,at,admin,name,action,target,source,before,after") && csv.contains("\"sign_in_refused\""), "{csv}");
}

/// A sign-in that comes back to another browser than the one that started
/// it, or twice, finishes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_finishes_only_in_the_browser_that_started_it_and_once() {
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let kc = Keycloak::start().await;
    let mine = format!("{}/32", lan_ip());
    let p = prepare(&app, &[&mine], &[("issuer", kc.issuer.clone().into())]);
    kc.add_admin_client("magnus-admin", &format!("{}/auth/callback", p.url()), &p.jwks().await).await;
    let url = p.url();
    let c = launch(p).await;
    let b = browser(&c, false);
    let r = b.get(format!("{url}/auth/login")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let to = url::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
    assert!(to.as_str().starts_with(&kc.issuer), "the sign-in does not go to the provider: {to}");
    let q: std::collections::HashMap<String, String> = to.query_pairs().into_owned().collect();
    assert_eq!((q["client_id"].as_str(), q["code_challenge_method"].as_str(), q["redirect_uri"].as_str()), ("magnus-admin", "S256", format!("{url}/auth/callback").as_str()));
    // Another browser, with the right state but not its cookie, is refused.
    let r = b.get(format!("{url}/auth/callback?state={}&code=x", q["state"])).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(!r.headers().get_all("set-cookie").iter().any(|v| v.to_str().unwrap_or_default().starts_with("__Host-magnus_admin=")));
    // A code the provider never issued opens nothing, even with the cookie.
    let r = b.get(format!("{url}/auth/callback?state={}&code=forged", q["state"])).header("cookie", format!("__Host-magnus_admin_login={}", q["state"])).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // And the state is spent.
    let r = b.get(format!("{url}/auth/callback?state={}&code=forged", q["state"])).header("cookie", format!("__Host-magnus_admin_login={}", q["state"])).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}
