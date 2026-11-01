//! Who can reach the admin console (admin console spec, invariant tests,
//! Reach): only the organisation's network and VPN, by its own hostname,
//! over TLS 1.3. It runs as its own process, `magnus-admin`, on the
//! appliance's address on the network; the tunnel reaches services over
//! loopback, where the console never listens or answers.

use magnus_invariants::appliance::bin;
use magnus_invariants::*;

use magnus_scratch::admin::*;

/// The console refuses to start where its reach rules would not hold.
async fn refused(app: &Appliance, listen: &[String], allow: &[&str]) -> String {
    let (path, _) = config(app, admin(listen, allow));
    let out = tokio::process::Command::new(bin("magnus-admin")).arg("--config").arg(&path).output().await.unwrap();
    assert!(!out.status.success(), "magnus-admin started listening on {listen:?}, taking {allow:?}");
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_console_answers_only_its_own_network_by_its_own_name_over_tls_1_3() {
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let ip = lan_ip();
    let mine = format!("{ip}/32");
    let c = start(&app, &[&mine]).await;
    let url = |p: &str| format!("https://{HOST}:{}{p}", c.addr.port());

    // From the organisation's network, by its name: the console, hardened.
    let r = browser(&c, false).get(url("/")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let csp = r.headers()["content-security-policy"].to_str().unwrap().to_string();
    assert!(csp.contains("default-src 'none'") && csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"), "{csp}");
    let page = r.text().await.unwrap();
    assert!(page.contains("Acme") && page.contains("Admin console"), "{page}");
    assert!(!page.contains("https://") && !page.contains("http://"), "the page loads something from elsewhere: {page}");
    for asset in ["/assets/app.css", "/assets/theme.js", "/assets/admin.css", "/assets/admin.js"] {
        assert_eq!(browser(&c, false).get(url(asset)).send().await.unwrap().status(), 200, "{asset}");
    }
    let health: serde_json::Value = browser(&c, false).get(url("/api/v1/health")).send().await.unwrap().json().await.unwrap();
    assert_eq!(health["ok"], true, "{health}");
    // Nothing else answers without signing in.
    assert_eq!(browser(&c, false).get(url("/api/v1/users")).send().await.unwrap().status(), 401);

    // Any other name is not found, the console not even confirmed.
    let r = browser(&c, false).get(url("/")).header("host", "magnus.pyrallon.ai").send().await.unwrap();
    assert_eq!(r.status(), 404);
    let r = browser(&c, false).get(url("/api/v1/health")).header("host", format!("{ip}:{}", c.addr.port())).send().await.unwrap();
    assert_eq!(r.status(), 404);

    // TLS 1.3 only.
    assert!(browser(&c, true).get(url("/")).send().await.is_err(), "the console spoke TLS 1.2");

    // Loopback, where the tunnel reaches services, is never the console's.
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", c.addr.port())).await.is_err(), "the console listens on loopback");
    // Nor is the user API, which the tunnel serves.
    for p in ["/assets/admin.js", "/api/v1/health"] {
        let st = reqwest::get(format!("{}{p}", app.base)).await.unwrap().status();
        assert_eq!(st, 404, "the user API serves the console's {p}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_address_outside_the_allowed_subnets_is_refused() {
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    // This machine is outside the allowed VPN subnet: its connection is
    // dropped before TLS.
    let c = start(&app, &["198.51.100.0/24"]).await;
    let r = browser(&c, false).get(format!("https://{HOST}:{}/", c.addr.port())).send().await;
    assert!(r.is_err(), "a request from outside the allowed subnets was served: {:?}", r.map(|r| r.status()));
    // Inside them, it is served.
    let mine = format!("{}/32", lan_ip());
    let c = start(&app, &["198.51.100.0/24", &mine]).await;
    assert_eq!(browser(&c, false).get(format!("https://{HOST}:{}/", c.addr.port())).send().await.unwrap().status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_console_will_not_listen_on_every_address_or_loopback_nor_take_loopback() {
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    let mine = format!("{}/32", lan_ip());
    for listen in ["0.0.0.0:0", "[::]:0", "127.0.0.1:0", "[::1]:0"] {
        let said = refused(&app, &[listen.to_string()], &[&mine]).await;
        assert!(said.contains("admin.listen"), "{listen}: {said}");
    }
    let here = format!("{}:0", lan_ip());
    for allow in ["127.0.0.0/8", "127.0.0.1", "0.0.0.0/0", "::1"] {
        let said = refused(&app, &[here.clone()], &[allow]).await;
        assert!(said.contains("admin.allow") && said.contains("loopback"), "{allow}: {said}");
    }
}
