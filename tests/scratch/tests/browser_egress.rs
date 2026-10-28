//! The browser's only way out is the boundary's egress proxy: every request
//! is decided there per destination, a site's credential is added there and
//! never enters the browser, the site's session cookies are held there, and
//! a task holding private and untrusted content may fetch but not send.
//!
//! These drive real Chromium through the sidecar on this host, pointed at the
//! proxy; on the appliance the same proxy is the browser microVM's only exit
//! (`browser_vm.rs`), which needs Firecracker and is not exercised here.

use magnus_invariants::appliance::workspace_root;
use magnus_invariants::model::Script;
use magnus_invariants::*;
use magnus_scratch::{chromium, TestSite};
use serde_json::{json, Value};

const SECRET: &str = "tok-SECRET-7a1f3c9e";

/// An appliance whose browser runs on this host through the egress proxy,
/// trusting the test site's CA for the site (not for the browser).
async fn appliance(site: &TestSite, m: &FakeModel) -> Appliance {
    let mut app = Appliance::start(Spec::default(), m).await;
    let ca = app.dir.path().join("site-ca.pem");
    std::fs::write(&ca, &site.ca_pem).unwrap();
    let path = app.dir.path().join("magnus.toml");
    let mut cfg: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    let root = workspace_root();
    let mut browser = toml::Table::new();
    browser.insert("command".into(), toml::Value::Array(vec!["node".into(), root.join("browser/sidecar.js").to_string_lossy().to_string().into()]));
    browser.insert("chromium".into(), chromium().to_string_lossy().to_string().into());
    if !root.join("browser/node_modules/playwright-core").exists() {
        let np = std::env::var("MAGNUS_NODE_PATH").unwrap_or_else(|_| "/opt/node-tools/node_modules".into());
        browser.insert("node_path".into(), np.into());
    }
    cfg.insert("browser".into(), toml::Value::Table(browser));
    let web = cfg.entry("web").or_insert_with(|| toml::Value::Table(Default::default())).as_table_mut().unwrap();
    web.insert("allow_private_networks".into(), true.into());
    web.insert("extra_ca".into(), toml::Value::Array(vec![ca.to_string_lossy().to_string().into()]));
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    app.restart().await;
    app
}

/// Browser calls by task, chosen by the task's words, one per turn.
fn model(steps_for: impl Fn(&str) -> Vec<Reply> + Send + Sync + 'static) -> Script {
    let steps_for = std::sync::Arc::new(steps_for);
    script(move |c| {
        let steps_for = steps_for.clone();
        async move {
            let first = c.messages().first().map(|m| m.to_string()).unwrap_or_default();
            let mut list = vec![contract_unchecked("look", "looked")];
            list.extend(steps_for(&first));
            list.get(c.turn()).cloned().unwrap_or_else(|| fail("done"))
        }
    })
}

fn nav(url: String) -> Reply {
    tool("browser_navigate", json!({ "url": url }))
}

/// What the model was given back for its browser calls in a task.
fn seen(m: &FakeModel, words: &str) -> String {
    m.requests().into_iter().filter(|c| c.messages().first().is_some_and(|f| f.to_string().contains(words))).map(|c| c.text()).collect::<Vec<_>>().join("\n")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_browser_reaches_the_web_only_through_the_boundary() {
    let site = TestSite::start().await;
    let page = site.http_url("/page");
    let m = FakeModel::start(model(move |_| vec![nav(page.clone())])).await;
    let app = appliance(&site, &m).await;
    let alice = app.api("alice");
    let t = alice.submit("Open the plain page.").await;
    alice.wait_settled(&t).await;
    assert!(seen(&m, "Open the plain page").contains("Plain page from the test site."), "{}", seen(&m, "Open the plain page"));
    let before = site.hits().iter().filter(|h| h.path == "/page").count();
    assert_eq!(before, 1);

    // The owner closes the site: the proxy refuses it, loopback included, so
    // nothing reached it except through the boundary.
    assert!(alice.put("/v1/browser/sites/127.0.0.1", json!({ "host": "127.0.0.1", "access": "deny" })).await.0.is_success());
    let t = alice.submit("Open the plain page again.").await;
    alice.wait_settled(&t).await;
    assert!(seen(&m, "Open the plain page again").contains("Blocked by the Magnus appliance"), "{}", seen(&m, "Open the plain page again"));
    assert_eq!(site.hits().iter().filter(|h| h.path == "/page").count(), before, "the browser reached a denied site");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_credential_is_added_at_the_boundary_and_never_seen() {
    let site = TestSite::start().await;
    let account = site.https_url("/account");
    let m = FakeModel::start(model(move |_| vec![nav(account.clone()), nav(account.clone())])).await;
    let app = appliance(&site, &m).await;
    let alice = app.api("alice");
    assert!(alice.put_secret("site-token", SECRET).await.is_success());
    let rule = json!({ "host": "localhost", "credential": { "header": "Authorization", "value": "Bearer {secret}", "vault": "site-token" } });
    assert!(alice.put("/v1/browser/sites/localhost", rule).await.0.is_success());
    let t = alice.submit("Check my account.").await;
    // The first page was the person's own (private) and the web's
    // (untrusted): the second visit asks, and the person approves it.
    let item = alice.wait_item(&t, "approval").await;
    assert_eq!(alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.as_u16(), 200);
    alice.wait_settled(&t).await;

    let hits: Vec<_> = site.hits().into_iter().filter(|h| h.path == "/account").collect();
    assert_eq!(hits.len(), 2, "{hits:?}");
    for h in &hits {
        assert_eq!(h.scheme, "https");
        assert_eq!(h.headers.get("authorization").map(String::as_str), Some(format!("Bearer {SECRET}").as_str()), "the credential was not added: {h:?}");
    }
    // The site's session cookie is held at the boundary and sent from there.
    assert!(!hits[0].headers.contains_key("cookie"));
    assert_eq!(hits[1].headers.get("cookie"), Some(&format!("session={}", site.session)), "the held cookie was not sent: {:?}", hits[1]);

    let text = seen(&m, "Check my account");
    assert!(text.contains("Your account"), "{text}");
    assert!(!text.contains(SECRET) && !text.contains(&site.session), "a credential reached the model: {text}");
    assert!(text.contains("[credential removed]"), "the site's echo of the credential was not removed: {text}");
    assert!(text.contains("page cookies: []"), "the page could read the session cookie: {text}");
    let task = alice.task(&t).await;
    assert_eq!(task["taint"]["private"], true, "a credentialed page is not private content: {task}");
    let dump = app.db.dump().await;
    assert!(!dump.contains(SECRET) && !dump.contains(&site.session), "a credential reached the log");
}

/// Once a task has read private and untrusted content, its browser may fetch
/// pages but not send, even after the person approves the click; and a
/// site's rule can limit its methods for every task.
#[tokio::test(flavor = "multi_thread")]
async fn a_task_holding_private_and_untrusted_content_may_fetch_but_not_send() {
    let site = TestSite::start().await;
    let (account, form) = (site.https_url("/account"), site.http_url("/form"));
    let m = FakeModel::start(model(move |first| {
        let submit = tool("browser_type", json!({ "target": "e1", "text": "the CFO's numbers", "submit": true }));
        if first.contains("Fill the form after my account") {
            vec![nav(account.clone()), nav(form.clone()), submit]
        } else {
            vec![nav(form.clone()), submit]
        }
    }))
    .await;
    let app = appliance(&site, &m).await;
    let alice = app.api("alice");
    assert!(alice.put_secret("site-token", SECRET).await.is_success());
    let rule = json!({ "host": "localhost", "credential": { "header": "Authorization", "value": "Bearer {secret}", "vault": "site-token" } });
    assert!(alice.put("/v1/browser/sites/localhost", rule).await.0.is_success());

    let t = alice.submit("Fill the form after my account.").await;
    // With both taints every browser call asks; the person approves both.
    for _ in 0..2 {
        let item = alice.wait_item(&t, "approval").await;
        assert_eq!(alice.respond(item["id"].as_str().unwrap(), json!({ "decision": "approve", "scope": "once" })).await.as_u16(), 200);
    }
    alice.wait_settled(&t).await;
    let text = seen(&m, "Fill the form after my account");
    assert!(text.contains("may fetch pages but not send"), "{text}");
    assert!(!site.hits().iter().any(|h| h.method == "POST"), "the form was sent: {:?}", site.hits());

    // A clean task may send, unless the site's rule limits its methods.
    assert!(alice.put("/v1/browser/sites/127.0.0.1", json!({ "host": "127.0.0.1", "methods": ["GET"] })).await.0.is_success());
    let t = alice.submit("Fill the form.").await;
    alice.wait_settled(&t).await;
    assert!(seen(&m, "Fill the form.").contains("only GET"), "{}", seen(&m, "Fill the form."));
    assert!(!site.hits().iter().any(|h| h.method == "POST"));
    assert_eq!(alice.send(reqwest::Method::DELETE, "/v1/browser/sites/127.0.0.1", None).await.0.as_u16(), 204);
    let t = alice.submit("Fill the form, now allowed.").await;
    alice.wait_settled(&t).await;
    assert!(site.hits().iter().any(|h| h.method == "POST" && h.path == "/submit"), "a clean task could not send: {:?}", site.hits());
}

#[tokio::test(flavor = "multi_thread")]
async fn site_rules_are_the_owners_and_must_be_well_formed() {
    let site = TestSite::start().await;
    let m = FakeModel::start(steps(vec![])).await;
    let app = appliance(&site, &m).await;
    let (alice, bob) = (app.api("alice"), app.api("bob"));
    assert_eq!(bob.put("/v1/browser/sites/example.com", json!({ "host": "example.com" })).await.0.as_u16(), 403);
    for bad in [
        json!({ "host": "Example.com" }),
        json!({ "host": "example.com", "access": "maybe" }),
        json!({ "host": "example.com", "methods": ["get"] }),
        json!({ "host": "example.com", "credential": { "header": "Authorization", "value": "Bearer", "vault": "x" } }),
        json!({ "host": "example.com", "credential": { "header": "Authorization", "value": "{secret}", "vault": "x", "per_person": true } }),
    ] {
        assert_eq!(alice.put("/v1/browser/sites/example.com", bad.clone()).await.0.as_u16(), 400, "{bad}");
    }
    let ok: Value = json!({ "host": "*.example.com", "credential": { "header": "Authorization", "value": "Bearer {secret}", "per_person": true } });
    assert!(alice.put("/v1/browser/sites/*.example.com", ok).await.0.is_success());
    assert_eq!(bob.put("/v1/browser/sites/*.example.com/credential", json!({ "value": "bob-token" })).await.0.as_u16(), 204);
    assert!(!app.db.dump().await.contains("bob-token"), "a site credential reached the log");
}
