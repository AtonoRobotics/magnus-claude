//! For brokered connectors' tests: a server image built from this checkout
//! (`brokered/build.sh`), the config that runs it, and a local host
//! standing in for the provider's API (the provider's real API needs a
//! person's account at it; the contract tests run against one).

use axum::{extract::State, http::HeaderMap, http::Uri, Router};
use magnus_invariants::appliance::workspace_root;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A server image, built: where it is and its digest (`sha256:<hex>`).
pub struct Image {
    pub dir: tempfile::TempDir,
    pub digest: String,
}

impl Image {
    /// The image's file, named by its digest.
    pub fn file(&self) -> std::path::PathBuf {
        self.dir.path().join(format!("{}.cpio", self.digest.trim_start_matches("sha256:")))
    }
}

/// Build `package`'s server image as a release does.
pub fn image(package: &str) -> Image {
    build(package, None)
}

/// Build it with its own target directory, as another machine would.
pub fn image_elsewhere(package: &str, target: &Path) -> Image {
    build(package, Some(target))
}

fn build(package: &str, target: Option<&Path>) -> Image {
    let dir = tempfile::tempdir().unwrap();
    let mut c = std::process::Command::new(workspace_root().join("brokered/build.sh"));
    c.arg(package).arg(dir.path());
    if let Some(t) = target {
        c.env("CARGO_TARGET_DIR", t);
    }
    let out = c.output().expect("run brokered/build.sh");
    assert!(out.status.success(), "building {package}'s image failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let digest = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(digest.starts_with("sha256:"), "{digest}");
    Image { dir, digest }
}

/// The config that runs `image` as the connector `name`, reaching `hosts`,
/// whose people sign in at `issuer`, with its tools' classes; instances
/// stop after `idle_s` unused.
pub fn configure(cfg: &mut toml::Table, image: &Image, idle_s: u64, name: &str, hosts: &[String], issuer: &str, tools: &[(&str, &str)]) {
    let mut b = toml::Table::new();
    b.insert("images".into(), image.dir.path().to_str().unwrap().into());
    b.insert("idle_s".into(), (idle_s as i64).into());
    cfg.insert("brokered".into(), b.into());
    let mut c = toml::Table::new();
    for (k, v) in [("name", name), ("transport", "brokered"), ("url", &image.digest), ("issuer", issuer), ("auth", "oauth"), ("audience", "private"), ("label", "private+untrusted"), ("idempotency", "none")] {
        c.insert(k.into(), v.into());
    }
    c.insert("credential".into(), format!("connector_{name}").into());
    c.insert("hosts".into(), toml::Value::Array(hosts.iter().map(|h| h.as_str().into()).collect()));
    let mut t = toml::Table::new();
    for (tool, risk) in tools {
        let mut r = toml::Table::new();
        r.insert("risk".into(), (*risk).into());
        t.insert((*tool).into(), r.into());
    }
    c.insert("tools".into(), t.into());
    let list = cfg.entry("connectors").or_insert_with(|| toml::Value::Array(vec![]));
    list.as_array_mut().unwrap().push(c.into());
}

/// How many brokered microVMs are running on the appliance: each has its
/// folder under the state directory while it runs.
pub fn running(state_dir: &std::path::Path) -> usize {
    std::fs::read_dir(state_dir.join("brk")).map(|d| d.count()).unwrap_or(0)
}

/// A local host standing in for a provider's API: every request it is
/// sent, as (path, Authorization header), answered with a greeting.
#[derive(Clone)]
pub struct ProviderApi {
    /// `127.0.0.1:<port>`, as a manifest's hosts name it.
    pub host: String,
    pub seen: Arc<Mutex<Vec<(String, String)>>>,
}

impl ProviderApi {
    pub async fn start() -> ProviderApi {
        async fn answer(State(seen): State<Arc<Mutex<Vec<(String, String)>>>>, uri: Uri, h: HeaderMap) -> &'static str {
            let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            seen.lock().unwrap().push((uri.path().to_string(), auth));
            "hello from the provider"
        }
        let seen = Arc::new(Mutex::new(vec![]));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = l.local_addr().unwrap().to_string();
        let app = Router::new().fallback(answer).with_state(seen.clone());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        ProviderApi { host, seen }
    }
}

/// A host no manifest names: how many connections ever reached it.
pub struct Elsewhere {
    pub host: String,
    pub reached: Arc<AtomicUsize>,
}

impl Elsewhere {
    pub async fn start() -> Elsewhere {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = l.local_addr().unwrap().to_string();
        let reached = Arc::new(AtomicUsize::new(0));
        let r = reached.clone();
        tokio::spawn(async move {
            while l.accept().await.is_ok() {
                r.fetch_add(1, Ordering::SeqCst);
            }
        });
        Elsewhere { host, reached }
    }
}

/// One request Gmail's stand-in was sent: the name the client asked for in
/// its TLS hello, the method, path, `Host`, `Authorization` and body.
#[derive(Clone, Debug)]
pub struct GmailRequest {
    pub sni: String,
    pub method: String,
    pub path: String,
    pub host: String,
    pub auth: String,
    pub body: String,
}

/// A local HTTPS host standing in for `gmail.googleapis.com` (the real one
/// needs a person's Google account; the contract test uses one), with a
/// certificate for that name from a CA of the test's own. It keeps drafts
/// and records each request; one answer may be set ahead.
pub struct StandInGmail {
    pub addr: String,
    /// The CA's certificate, DER, in a file: `[brokered] roots`.
    pub root: std::path::PathBuf,
    pub seen: Arc<Mutex<Vec<GmailRequest>>>,
    /// The next answer, instead of Gmail's usual one: status, headers, body.
    pub next: Arc<Mutex<Option<(u16, String, String)>>>,
    _dir: tempfile::TempDir,
}

impl StandInGmail {
    pub async fn start() -> StandInGmail {
        Self::with_drafts(0).await
    }

    /// One whose mailbox holds `drafts` drafts already (`d1`, ...), as if
    /// the person had written them.
    pub async fn with_drafts(drafts: usize) -> StandInGmail {
        let dir = tempfile::tempdir().unwrap();
        let mut p = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        p.distinguished_name.push(rcgen::DnType::CommonName, "A CA standing in for Google's (test)");
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca = p.self_signed(&ca_key).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["gmail.googleapis.com".to_string()]).unwrap().signed_by(&key, &ca, &ca_key).unwrap();
        let root = dir.path().join("root.der");
        std::fs::write(&root, ca.der()).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let seen: Arc<Mutex<Vec<GmailRequest>>> = Arc::default();
        let next: Arc<Mutex<Option<(u16, String, String)>>> = Arc::default();
        let (s2, n2) = (seen.clone(), next.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = l.accept().await {
                let (acceptor, seen, next) = (acceptor.clone(), s2.clone(), n2.clone());
                tokio::spawn(async move {
                    let Ok(mut s) = acceptor.accept(tcp).await else { return };
                    let sni = s.get_ref().1.server_name().unwrap_or_default().to_string();
                    let Some(r) = read_request(&mut s, sni).await else { return };
                    let (status, head, body) = next.lock().unwrap().take().unwrap_or_else(|| gmail_answer(&r, &seen.lock().unwrap(), drafts));
                    seen.lock().unwrap().push(r);
                    let out = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{head}\r\n{body}", body.len());
                    use tokio::io::AsyncWriteExt;
                    let _ = s.write_all(out.as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        StandInGmail { addr, root, seen, next, _dir: dir }
    }

    pub fn requests(&self) -> Vec<GmailRequest> {
        self.seen.lock().unwrap().clone()
    }
}

async fn read_request(s: &mut (impl tokio::io::AsyncRead + Unpin), sni: String) -> Option<GmailRequest> {
    use tokio::io::AsyncReadExt;
    let mut raw = vec![];
    let mut b = [0u8; 4096];
    let split = loop {
        let n = s.read(&mut b).await.ok().filter(|n| *n > 0)?;
        raw.extend_from_slice(&b[..n]);
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
    };
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let header = |name: &str| head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string())).unwrap_or_default();
    let len: usize = header("content-length").parse().unwrap_or(0);
    while raw.len() < split + 4 + len {
        let n = s.read(&mut b).await.ok().filter(|n| *n > 0)?;
        raw.extend_from_slice(&b[..n]);
    }
    let mut first = head.lines().next().unwrap_or_default().split_whitespace();
    let (method, path) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
    Some(GmailRequest { sni, method, path, host: header("host"), auth: header("authorization"), body: String::from_utf8_lossy(&raw[split + 4..split + 4 + len]).to_string() })
}

/// What Gmail answers, for the requests the tests make: one thread in the
/// inbox, drafts made `d1`, `d2`, ... (after those there already) and sent
/// only if made.
fn gmail_answer(r: &GmailRequest, before: &[GmailRequest], drafts: usize) -> (u16, String, String) {
    let base = "/gmail/v1/users/me";
    let path = r.path.strip_prefix(base).unwrap_or("");
    let made = drafts + before.iter().filter(|x| x.method == "POST" && x.path == format!("{base}/drafts")).count();
    let ok = |v: serde_json::Value| (200, String::new(), v.to_string());
    match (r.method.as_str(), path.split('?').next().unwrap_or("")) {
        ("GET", "/threads") => ok(serde_json::json!({ "threads": [{ "id": "t1", "snippet": "Can you send me the plan for Friday?" }] })),
        ("POST", "/drafts") => ok(serde_json::json!({ "id": format!("d{}", made + 1), "message": { "id": format!("dm{}", made + 1), "threadId": "t1", "labelIds": ["DRAFT"] } })),
        ("POST", "/drafts/send") => {
            let id = serde_json::from_str::<serde_json::Value>(&r.body).unwrap_or_default()["id"].as_str().unwrap_or_default().to_string();
            match id.strip_prefix('d').and_then(|n| n.parse::<usize>().ok()) {
                Some(n) if (1..=made).contains(&n) => ok(serde_json::json!({ "id": format!("s{n}"), "threadId": "t1", "labelIds": ["SENT"] })),
                _ => (404, String::new(), serde_json::json!({ "error": { "code": 404, "message": "Requested entity was not found." } }).to_string()),
            }
        }
        _ => (404, String::new(), serde_json::json!({ "error": { "code": 404, "message": "Requested entity was not found." } }).to_string()),
    }
}
