//! The admin console set up on an appliance by `release/admin-on-appliance.sh`
//! (DECISIONS.md, "The admin console on strix-alpha"): from an appliance's
//! config with no `[admin]`, it writes a hostname on the local network,
//! listens only on the machine's address there, takes only that network, and
//! makes the organisation's CA and the console's certificate from it. The
//! console starts on what it wrote and answers its hostname over TLS that
//! the new CA verifies, and nothing else; run again, it writes nothing more.

use magnus_invariants::appliance::{bin, workspace_root};
use magnus_invariants::*;
use magnus_scratch::admin::lan_ip;
use std::net::SocketAddr;

fn run(config: &std::path::Path, lan: &str, port: u16) {
    let out = std::process::Command::new(workspace_root().join("release/admin-on-appliance.sh"))
        .arg(config)
        .arg(bin("magnus-admin").parent().unwrap())
        .arg("--config-only")
        .env("MAGNUS_ADMIN_LAN", lan)
        .env("MAGNUS_ADMIN_TAILSCALE", "none")
        .env("MAGNUS_ADMIN_PORT", port.to_string())
        .output()
        .unwrap();
    assert!(out.status.success(), "the setup failed:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_setup_writes_an_admin_console_that_answers_its_hostname_on_the_network_only() {
    let m = FakeModel::start(steps(vec![])).await;
    let app = Appliance::start(Spec::default(), &m).await;
    // An appliance signing people in through its identity provider, as
    // strix-alpha does; the console never asks it before anyone signs in.
    let config = app.dir.path().join("magnus.toml");
    let mut cfg: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    let mut oidc = toml::Table::new();
    oidc.insert("issuer".into(), "http://127.0.0.1:9/realms/unused".into());
    oidc.insert("client_id".into(), "magnus-claude".into());
    let mut sign_in = toml::Table::new();
    sign_in.insert("oidc".into(), oidc.into());
    cfg.insert("sign_in".into(), sign_in.into());
    std::fs::write(&config, toml::to_string(&cfg).unwrap()).unwrap();

    let ip = lan_ip();
    let port = std::net::TcpListener::bind((ip, 0)).unwrap().local_addr().unwrap().port();
    run(&config, &format!("{ip}/24"), port);
    let written = std::fs::read_to_string(&config).unwrap();
    let cfg: toml::Table = written.parse().unwrap();
    let a = cfg["admin"].as_table().unwrap();
    let host = a["hostname"].as_str().unwrap().to_string();
    assert!(host.ends_with(".local"), "{host}");
    assert_eq!(a["listen"].as_array().unwrap(), &vec![toml::Value::from(format!("{ip}:{port}"))]);
    let net = a["allow"].as_array().unwrap()[0].as_str().unwrap().to_string();
    assert!(net.ends_with("/24") && !net.starts_with("0.0.0.0"), "{net}");
    assert!(a["database_url"].as_str().unwrap().starts_with("postgres://magnus_root:"));
    let tls = app.dir.path().join("admin-tls");
    let mode = |f: &str| std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(tls.join(f)).unwrap().permissions()) & 0o777;
    assert_eq!((mode("ca.key"), mode("console.key")), (0o600, 0o600), "a key is readable by others");

    // Run again: nothing more is written.
    run(&config, &format!("{ip}/24"), port);
    assert_eq!(std::fs::read_to_string(&config).unwrap().matches("[admin]").count(), 1, "the setup wrote [admin] twice");

    // The console starts on it, and answers its hostname over TLS the CA verifies.
    let mut child = tokio::process::Command::new(bin("magnus-admin")).arg("--config").arg(&config).kill_on_drop(true).spawn().unwrap();
    let addr = SocketAddr::new(ip, port);
    eventually("the console listening", PATIENCE, || async { tokio::net::TcpStream::connect(addr).await.ok().map(|_| ()) }).await;
    let ca = reqwest::Certificate::from_pem(&std::fs::read(tls.join("ca.pem")).unwrap()).unwrap();
    let browser = reqwest::Client::builder().add_root_certificate(ca).resolve(&host, addr).no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = browser.get(format!("https://{host}:{port}/")).send().await.expect("the console's certificate is not trusted through the new CA");
    assert!(r.status().is_success() || r.status().is_redirection(), "{}", r.status());
    let other = browser.get(format!("https://{host}:{port}/")).header("Host", "elsewhere.example").send().await.unwrap();
    assert_eq!(other.status().as_u16(), 404, "the console answered another hostname");
    let _ = child.start_kill();
}
