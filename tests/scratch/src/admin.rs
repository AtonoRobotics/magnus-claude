//! The admin console as tests run it: `magnus-admin` started as its own
//! process on this machine's address on its network, with a certificate of
//! the test's own for the console's hostname.

use magnus_invariants::appliance::{bin, Appliance};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;

pub const HOST: &str = "admin.magnus.test";

/// This machine's address on its network: what the console listens on, and
/// where the tests' requests come from.
pub fn lan_ip() -> IpAddr {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    s.connect("10.255.255.255:1").unwrap();
    let ip = s.local_addr().unwrap().ip();
    assert!(!ip.is_loopback(), "this machine has no address but loopback");
    ip
}

fn free_port(ip: IpAddr) -> u16 {
    std::net::TcpListener::bind((ip, 0)).unwrap().local_addr().unwrap().port()
}

/// A console made ready to start: where it will listen, its config, and
/// the certificate a browser on the network trusts for it.
pub struct Prepared {
    pub addr: SocketAddr,
    pub config: PathBuf,
    pub ca: reqwest::Certificate,
}

impl Prepared {
    /// Its address as a browser on the network types it.
    pub fn url(&self) -> String {
        format!("https://{HOST}:{}", self.addr.port())
    }

    /// The console's public key, for its client in the realm.
    pub async fn jwks(&self) -> serde_json::Value {
        let out = tokio::process::Command::new(bin("magnus-admin")).arg("jwk").arg("--config").arg(&self.config).output().await.unwrap();
        assert!(out.status.success(), "magnus-admin jwk: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

pub struct Console {
    pub child: tokio::process::Child,
    pub addr: SocketAddr,
    pub ca: reqwest::Certificate,
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// `[admin]` with these addresses and subnets.
pub fn admin(listen: &[String], allow: &[&str]) -> toml::Table {
    let mut t = toml::Table::new();
    t.insert("listen".into(), toml::Value::Array(listen.iter().map(|l| l.as_str().into()).collect()));
    t.insert("allow".into(), toml::Value::Array(allow.iter().map(|a| (*a).into()).collect()));
    t
}

/// The appliance's own config with an `[admin]` section, and a certificate
/// for the console's hostname.
pub fn config(app: &Appliance, admin: toml::Table) -> (PathBuf, reqwest::Certificate) {
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
    if !a.contains_key("issuer") {
        // Reach is decided before anyone signs in: this provider is never asked.
        a.insert("issuer".into(), "http://127.0.0.1:9/realms/unused".into());
    }
    cfg.insert("admin".into(), a.into());
    let path = app.dir.path().join(format!("magnus-admin-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    (path, reqwest::Certificate::from_pem(cert.cert.pem().as_bytes()).unwrap())
}

/// A console on this machine's address taking `allow`, with `more` in its
/// `[admin]` section.
pub fn prepare(app: &Appliance, allow: &[&str], more: &[(&str, toml::Value)]) -> Prepared {
    let ip = lan_ip();
    let addr = SocketAddr::new(ip, free_port(ip));
    let mut a = admin(&[addr.to_string()], allow);
    for (k, v) in more {
        a.insert(k.to_string(), v.clone());
    }
    let (config, ca) = config(app, a);
    Prepared { addr, config, ca }
}

pub async fn launch(p: Prepared) -> Console {
    let child = tokio::process::Command::new(bin("magnus-admin")).arg("--config").arg(&p.config).stdout(Stdio::null()).stderr(Stdio::inherit()).kill_on_drop(true).spawn().unwrap();
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(p.addr).await.is_ok() {
            return Console { child, addr: p.addr, ca: p.ca };
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("magnus-admin did not start on {}", p.addr);
}

pub async fn start(app: &Appliance, allow: &[&str]) -> Console {
    launch(prepare(app, allow, &[])).await
}

/// A browser on the organisation's network: it trusts the console's
/// certificate and finds the hostname at the console's address. It does
/// not follow redirects, so a test sees where the console sends it.
pub fn browser(c: &Console, max_tls12: bool) -> reqwest::Client {
    let mut b = reqwest::Client::builder().add_root_certificate(c.ca.clone()).resolve(HOST, c.addr).no_proxy().redirect(reqwest::redirect::Policy::none());
    if max_tls12 {
        b = b.max_tls_version(reqwest::tls::Version::TLS_1_2);
    }
    b.build().unwrap()
}
