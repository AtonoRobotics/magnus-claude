//! Users in the admin console (admin console spec, Users; invariant "Removing,
//! disabling or demoting the last admin is refused"), against a real
//! Keycloak through the console's own service account: an admin invites
//! someone, Keycloak emails them its link (to a real SMTP server), they
//! choose their password on Keycloak's page and are active; a role changes
//! only with a fresh password; the last admin can be neither demoted nor
//! turned off; a person turned off loses every session, at the provider
//! and here, and their schedules pause; each change is in the audit log.

use magnus_invariants::*;
use magnus_scratch::admin::*;
use magnus_scratch::mail::{link_in, Inbox};
use magnus_scratch::*;
use reqwest::StatusCode;
use serde_json::{json, Value};

struct Setup {
    kc: Keycloak,
    app: Appliance,
    url: String,
    c: Console,
    b: reqwest::Client,
}

impl Setup {
    async fn start(step_up_seconds: i64, people: &[&str]) -> Setup {
        let kc = Keycloak::start().await;
        kc.add_person("Alice").await;
        kc.set_roles("Alice", &["admin"]).await;
        for p in people {
            kc.add_person(p).await;
        }
        let m = FakeModel::start(steps(vec![])).await;
        let mut app = Appliance::start(Spec::default(), &m).await;
        kc.configure(&mut app).await;
        let mine = format!("{}/32", lan_ip());
        let p = prepare(&app, &[&mine], &[("issuer", kc.issuer.clone().into()), ("step_up_seconds", step_up_seconds.into())]);
        let jwks = p.jwks().await;
        kc.add_admin_client("magnus-admin", &format!("{}/auth/callback", p.url()), &jwks).await;
        kc.add_admin_service("magnus-admin-svc", &jwks).await;
        let url = p.url();
        let c = launch(p).await;
        let b = browser(&c, false);
        Setup { kc, app, url, c, b }
    }

    fn api(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.url)
    }

    /// Alice's console session and its CSRF token, stepped up if asked.
    /// The tests' step-up windows (60 seconds) are long enough for the
    /// browser to hand the session back (that has taken 23 seconds after
    /// the password on a loaded machine).
    async fn alice(&self, step_up: bool) -> (String, String) {
        let sid = sign_in(&self.url, self.c.addr, "Alice", "admin", step_up).await.unwrap_or_else(|e| panic!("{e}"));
        let me: Value = with_cookie(self.b.get(self.api("/session")), &sid).send().await.unwrap().json().await.unwrap();
        (sid, me["csrf"].as_str().unwrap().to_string())
    }

    async fn send(&self, method: reqwest::Method, path: &str, s: &(String, String), body: Option<Value>) -> (StatusCode, Value) {
        let mut rq = with_cookie(self.b.request(method, self.api(path)), &s.0).header("x-csrf-token", &s.1);
        if let Some(b) = body {
            rq = rq.json(&b);
        }
        let r = rq.send().await.unwrap();
        let st = r.status();
        (st, r.json().await.unwrap_or(Value::Null))
    }

    async fn users(&self, s: &(String, String)) -> Vec<Value> {
        let (st, v) = self.send(reqwest::Method::GET, "/users", s, None).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        v["users"].as_array().unwrap().clone()
    }

    async fn user(&self, s: &(String, String), email: &str) -> Option<Value> {
        self.users(s).await.into_iter().find(|u| u["email"] == email)
    }

    async fn audited(&self, s: &(String, String)) -> Vec<String> {
        let (_, v) = self.send(reqwest::Method::GET, "/audit", s, None).await;
        v["entries"].as_array().unwrap().iter().filter_map(|e| e["action"].as_str().map(String::from)).collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_invites_someone_who_chooses_their_password_from_the_providers_email() {
    let s = Setup::start(60, &[]).await;
    let inbox = Inbox::start().await;
    s.kc.set_smtp(inbox.addr).await;
    let alice = s.alice(false).await;

    let me = s.user(&alice, "alice@example.test").await.expect("Alice is not in the list");
    assert_eq!((me["role"].as_str(), me["status"].as_str()), (Some("admin"), Some("active")), "{me}");

    // A member is invited without a step-up: the account waits for its
    // password, and the provider emails the link to choose it.
    let (st, v) = s.send(reqwest::Method::POST, "/users", &alice, Some(json!({ "email": "Dee@Acme.test", "name": "Dee Ray", "role": "member" }))).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["email_sent"], true, "{v}; the SMTP server at {} took {} connections", inbox.addr, inbox.connections());
    let dee = s.user(&alice, "dee@acme.test").await.expect("the invitation is not in the list");
    assert_eq!((dee["name"].as_str(), dee["role"].as_str(), dee["status"].as_str()), (Some("Dee Ray"), Some("member"), Some("invited")), "{dee}");
    let mail = inbox.wait_for("dee@acme.test").await;
    let link = link_in(&mail.data, "login-actions/action-token").unwrap_or_else(|| panic!("no link to choose a password in:\n{}", mail.data));
    assert!(link.starts_with(&s.kc.issuer), "the link is not the provider's: {link}");
    // Nobody signs in before choosing a password.
    assert!(!s.kc.password_works("dee@acme.test", &password_of("dee@acme.test")).await);

    // Resend sends it again.
    let (st, v) = s.send(reqwest::Method::POST, &format!("/users/{}/invite", dee["id"].as_str().unwrap()), &alice, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    for _ in 0..100 {
        if inbox.count_for("dee@acme.test") >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(inbox.count_for("dee@acme.test"), 2, "Resend sent nothing");

    // Dee follows the link, chooses a password on the provider's page, and
    // is an active member who can sign in.
    let landed = s.kc.browser(&link, "dee@acme.test").await;
    assert!(s.kc.password_works("dee@acme.test", &password_of("dee@acme.test")).await, "the password chosen from the email does not sign in ({landed:?})");
    let dee = s.user(&alice, "dee@acme.test").await.unwrap();
    assert_eq!(dee["status"], "active", "{dee}");
    // An account in use is no invitation: nothing to resend or cancel.
    let (st, v) = s.send(reqwest::Method::DELETE, &format!("/users/{}/invite", dee["id"].as_str().unwrap()), &alice, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::CONFLICT, Some("not_invited")), "{v}");

    // Inviting an admin needs a fresh password: refused once the sign-in's
    // own step-up has lapsed, done with a fresh one.
    tokio::time::sleep(std::time::Duration::from_secs(61)).await;
    let eve = json!({ "email": "eve@acme.test", "name": "Eve Ito", "role": "admin" });
    let (st, v) = s.send(reqwest::Method::POST, "/users", &alice, Some(eve.clone())).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("step_up_required")), "{v}");
    assert!(s.user(&alice, "eve@acme.test").await.is_none(), "an admin was invited without a fresh password");
    let fresh = s.alice(true).await;
    let (st, v) = s.send(reqwest::Method::POST, "/users", &fresh, Some(eve)).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let eve = s.user(&fresh, "eve@acme.test").await.unwrap();
    assert_eq!((eve["role"].as_str(), eve["status"].as_str()), (Some("admin"), Some("invited")), "{eve}");
    // Cancelled, the invitation and its account are gone.
    let (st, v) = s.send(reqwest::Method::DELETE, &format!("/users/{}/invite", eve["id"].as_str().unwrap()), &fresh, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert!(s.user(&fresh, "eve@acme.test").await.is_none(), "the cancelled invitation is still there");

    // Bad invitations change nothing.
    for bad in [json!({ "email": "not-an-address", "name": "X", "role": "member" }), json!({ "email": "x@acme.test", "name": "", "role": "member" }), json!({ "email": "x@acme.test", "name": "X", "role": "owner" })] {
        let (st, _) = s.send(reqwest::Method::POST, "/users", &fresh, Some(bad.clone())).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }

    // The list renders in every look, and at phone width with no sideways scroll.
    sign_in_and_visit(&s.url, s.c.addr, "Alice", "admin", false, Some("#/users")).await.unwrap_or_else(|e| panic!("{e}"));

    let actions = s.audited(&fresh).await;
    for a in ["user_invited", "user_invite_resent", "user_invite_cancelled"] {
        assert!(actions.iter().any(|x| x == a), "{a} is not in the audit: {actions:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_admin_stays_and_a_person_turned_off_loses_every_session_and_schedule() {
    let s = Setup::start(60, &["Bob"]).await;

    // Bob signs in to the app and sets up a schedule.
    let (st, v) = s.kc.app_sign_in(&s.app, "Bob").await;
    assert_eq!(st, 200, "{v}");
    let bob_token = v["token"].as_str().unwrap().to_string();
    let http = reqwest::Client::new();
    let r = http.post(format!("{}/v1/schedules", s.app.base)).bearer_auth(&bob_token).json(&json!({ "text": "Water the plants.", "daily_at": "03:00" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 201, "{}", r.text().await.unwrap());

    let alice = s.alice(true).await;
    let ids = |list: &[Value], email: &str| list.iter().find(|u| u["email"] == email).unwrap()["id"].as_str().unwrap().to_string();
    let list = s.users(&alice).await;
    let (a_id, b_id) = (ids(&list, "alice@example.test"), ids(&list, "bob@example.test"));
    let bob = list.iter().find(|u| u["id"] == b_id.as_str()).unwrap();
    assert!(bob["last_sign_in"].is_string() && bob["sessions"] == 1, "Bob's sign-in here is not shown: {bob}");
    let (st, detail) = s.send(reqwest::Method::GET, &format!("/users/{b_id}"), &alice, None).await;
    assert_eq!(st, StatusCode::OK, "{detail}");
    assert_eq!(detail["sessions"].as_array().map(Vec::len), Some(1), "{detail}");

    // Alice is the last admin: she can be neither turned off nor demoted.
    let (st, v) = s.send(reqwest::Method::POST, &format!("/users/{a_id}/disable"), &alice, None).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::CONFLICT, Some("last_admin")), "{v}");
    let (st, v) = s.send(reqwest::Method::PATCH, &format!("/users/{a_id}"), &alice, Some(json!({ "role": "member" }))).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::CONFLICT, Some("last_admin")), "{v}");
    assert!(s.kc.password_works("alice", &password_of("Alice")).await, "the last admin was turned off");

    // With a fresh password, Bob is made an admin, then a member again.
    let (st, v) = s.send(reqwest::Method::PATCH, &format!("/users/{b_id}"), &alice, Some(json!({ "role": "admin" }))).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(s.user(&alice, "bob@example.test").await.unwrap()["role"], "admin");
    let (st, v) = s.send(reqwest::Method::PATCH, &format!("/users/{b_id}"), &alice, Some(json!({ "role": "member" }))).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(s.user(&alice, "bob@example.test").await.unwrap()["role"], "member");

    // Turned off: the provider refuses him, his app session is over, and his
    // schedule is paused.
    let (st, v) = s.send(reqwest::Method::POST, &format!("/users/{b_id}/disable"), &alice, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!((v["sessions_ended"].as_u64(), v["schedules_paused"].as_u64()), (Some(1), Some(1)), "{v}");
    assert_eq!(s.user(&alice, "bob@example.test").await.unwrap()["status"], "disabled");
    assert!(!s.kc.password_works("bob", &password_of("Bob")).await, "the provider still signs in a person turned off");
    let r = http.get(format!("{}/v1/schedules", s.app.base)).bearer_auth(&bob_token).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 401, "a session of a person turned off still works");
    let (_, sc) = s.app.api("bob").get("/v1/schedules").await;
    assert!(sc["schedules"][0]["paused"].is_string(), "the schedule of a person turned off still runs: {sc}");

    // Turned back on, he signs in again; ending his sessions signs him out.
    let (st, v) = s.send(reqwest::Method::POST, &format!("/users/{b_id}/enable"), &alice, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(s.user(&alice, "bob@example.test").await.unwrap()["status"], "active");
    let (st, v) = s.kc.app_sign_in(&s.app, "Bob").await;
    assert_eq!(st, 200, "{v}");
    let bob_token = v["token"].as_str().unwrap().to_string();
    let (st, v) = s.send(reqwest::Method::DELETE, &format!("/users/{b_id}/sessions"), &alice, None).await;
    assert_eq!((st, v["ended"].as_u64()), (StatusCode::OK, Some(1)), "{v}");
    let r = http.get(format!("{}/v1/schedules", s.app.base)).bearer_auth(&bob_token).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 401, "an ended session still works");
    // One session, by its id.
    let (_, v) = s.kc.app_sign_in(&s.app, "Bob").await;
    let bob_token = v["token"].as_str().unwrap().to_string();
    let (_, detail) = s.send(reqwest::Method::GET, &format!("/users/{b_id}"), &alice, None).await;
    let sid = detail["sessions"][0]["id"].as_str().unwrap_or_else(|| panic!("{detail}")).to_string();
    let (st, v) = s.send(reqwest::Method::DELETE, &format!("/users/{b_id}/sessions/{sid}"), &alice, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let r = http.get(format!("{}/v1/schedules", s.app.base)).bearer_auth(&bob_token).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 401, "the ended session still works");

    // Once the step-up lapses, a role change says so and changes nothing.
    tokio::time::sleep(std::time::Duration::from_secs(61)).await;
    let (st, v) = s.send(reqwest::Method::PATCH, &format!("/users/{b_id}"), &alice, Some(json!({ "role": "admin" }))).await;
    assert_eq!((st, v["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("step_up_required")), "{v}");
    assert_eq!(s.user(&alice, "bob@example.test").await.unwrap()["role"], "member");

    // A person's page renders in every look, and at phone width.
    sign_in_and_visit(&s.url, s.c.addr, "Alice", "admin", false, Some(&format!("#/users/{b_id}/sessions"))).await.unwrap_or_else(|e| panic!("{e}"));

    let actions = s.audited(&alice).await;
    for a in ["user_role_changed", "user_disabled", "user_enabled", "user_sessions_ended", "user_session_ended"] {
        assert!(actions.iter().any(|x| x == a), "{a} is not in the audit: {actions:?}");
    }
}
