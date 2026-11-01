//! Who can reach the admin console (admin console spec, invariant tests,
//! Reach): only the organisation's network and VPN, by its own hostname,
//! over TLS 1.3. It runs as its own process, `magnus-admin`, on the
//! appliance's address on the network; the tunnel reaches services over
//! loopback, where the console never listens or answers.

use magnus_invariants::appliance::bin;
use magnus_invariants::*;
use std::net::{IpAddr, SocketAddr};
use std::process::Stdio;

const HOST: &str = "admin.magnus.test";

/// This machine's address on its network: what the console listens on, and
/// where this test's requests come from.
fn lan_ip() -> IpAddr {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    s.connect("10.255.255.255:1").unwrap();
    let ip = s.local_addr().unwrap().ip();
    assert!(!ip.is_loopback(), "this machine has no address but loopback");
    ip
}

fn free_port(ip: IpAddr) -> u16 {
    std::net::TcpListener::bind((ip, 0)).unwrap().local_addr().unwrap().port()
}

struct Console {
    child: tokio::process::Child,
    addr: SocketAddr,
    ca: reqwest::Certificate,
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// The appliance's own config with an `[admin]` section, and a certificate
/// for the console's hostname.
fn config(app: &Appliance, admin: toml::Table) -> (std::path::PathBuf, reqwest::Certificate) {
    let cert = rcgen::generate_simple_self_signed(vec![HOST.to_string()]).unwrap();
    let (c, k) = (app.dir.path().join("admin-cert.pem"), app.dir.path().join("admin-key.pem"));
    std::fs::write(&c, cert.cert.pem()).unwrap();
    std::fs::write(&k, cert.key_pair.serialize_pem()).unwrap();
    let mut cfg: toml::Table = std::fs::read_to_string(app.dir.path().join("magnus.toml")).unwrap().parse().unwrap();
    let mut a = admin;
    a.insert("hostname".into(), HOST.into());
    a.insert("org".into(), "Acme".into());
    a.insert("tls_cert".into(), c.display().to_string().into());
    a.insert("tls_key".into(), k.display().to_string().into());
    cfg.insert("admin".into(), a.into());
    let path = app.dir.path().join(format!("magnus-admin-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    (path, reqwest::Certificate::from_pem(cert.cert.pem().as_bytes()).unwrap())
}

fn admin(listen: &[String], allow: &[&str]) -> toml::Table {
    let mut t = toml::Table::new();
    t.insert("listen".into(), toml::Value::Array(listen.iter().map(|l| l.as_str().into()).collect()));
    t.insert("allow".into(), toml::Value::Array(allow.iter().map(|a| (*a).into()).collect()));
    t
}

async fn start(app: &Appliance, allow: &[&str]) -> Console {
    let ip = lan_ip();
    let addr = SocketAddr::new(ip, free_port(ip));
    let (path, ca) = config(app, admin(&[addr.to_string()], allow));
    let child = tokio::process::Command::new(bin("magnus-admin")).arg("--config").arg(&path).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).spawn().unwrap();
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Console { child, addr, ca };
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("magnus-admin did not start on {addr}");
}

/// A browser on the organisation's network: it trusts the console's
/// certificate and finds the hostname at the console's address.
fn browser(c: &Console, max_tls12: bool) -> reqwest::Client {
    let mut b = reqwest::Client::builder().add_root_certificate(c.ca.clone()).resolve(HOST, c.addr).no_proxy();
    if max_tls12 {
        b = b.max_tls_version(reqwest::tls::Version::TLS_1_2);
    }
    b.build().unwrap()
}

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
