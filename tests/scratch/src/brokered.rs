//! For brokered connectors' tests: a server image built from this checkout
//! (`brokered/build.sh`), the config that runs it, and a local host
//! standing in for the provider's API (the provider's real API needs a
//! person's account at it; the contract tests run against one).

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

/// A CA of the test's own: its certificate, DER, in a file
/// (`[brokered] roots`), and an acceptor with a certificate from it for
/// `name`.
pub struct TestCa {
    pub root: std::path::PathBuf,
    cert: rcgen::Certificate,
    key: rcgen::KeyPair,
    _dir: tempfile::TempDir,
}

impl TestCa {
    pub fn new(cn: &str) -> TestCa {
        let dir = tempfile::tempdir().unwrap();
        let mut p = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        p.distinguished_name.push(rcgen::DnType::CommonName, cn);
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = p.self_signed(&key).unwrap();
        let root = dir.path().join("root.der");
        std::fs::write(&root, cert.der()).unwrap();
        TestCa { root, cert, key, _dir: dir }
    }

    /// TLS for a host named `name` (a DNS name or an IP address).
    pub fn acceptor(&self, name: &str) -> tokio_rustls::TlsAcceptor {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap().signed_by(&key, &self.cert, &self.key).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
            .unwrap();
        tokio_rustls::TlsAcceptor::from(Arc::new(tls))
    }
}

/// `cfg`'s servers trust `ca`'s certificates too.
pub fn trust(cfg: &mut toml::Table, ca: &TestCa) {
    let b = cfg.entry("brokered").or_insert_with(|| toml::Table::new().into()).as_table_mut().unwrap();
    let roots = b.entry("roots").or_insert_with(|| toml::Value::Array(vec![])).as_array_mut().unwrap();
    roots.push(ca.root.display().to_string().into());
}

/// A local host standing in for a provider's API, over HTTPS with a
/// certificate for `127.0.0.1` from its own CA: every request it is sent,
/// as (path, Authorization header), answered with a greeting; and every
/// byte that reached it before TLS, as it came.
#[derive(Clone)]
pub struct ProviderApi {
    /// `127.0.0.1:<port>`, as a manifest's hosts name it.
    pub host: String,
    pub seen: Arc<Mutex<Vec<(String, String)>>>,
    pub raw: Arc<Mutex<Vec<u8>>>,
    pub ca: Arc<TestCa>,
}

impl ProviderApi {
    pub async fn start() -> ProviderApi {
        Self::serve(true).await
    }

    /// One that answers HTTP in the clear, as a wrong host might.
    pub async fn in_the_clear() -> ProviderApi {
        Self::serve(false).await
    }

    async fn serve(tls: bool) -> ProviderApi {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn answer(mut s: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin, seen: &Mutex<Vec<(String, String)>>, raw: Option<&Mutex<Vec<u8>>>) {
            let mut got = vec![];
            let mut b = [0u8; 4096];
            while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = match s.read(&mut b).await {
                    Ok(n) if n > 0 => n,
                    _ => return,
                };
                got.extend_from_slice(&b[..n]);
                if let Some(r) = raw {
                    r.lock().unwrap().extend_from_slice(&b[..n]);
                }
                // What is not HTTP is answered as an HTTP server does.
                if !got[0].is_ascii_uppercase() {
                    let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                }
            }
            let head = String::from_utf8_lossy(&got).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or_default().to_string();
            let auth = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("authorization")).map(|(_, v)| v.trim().to_string())).unwrap_or_default();
            seen.lock().unwrap().push((path, auth));
            let body = "hello from the provider";
            let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            let _ = s.shutdown().await;
        }
        let ca = Arc::new(TestCa::new("A CA standing in for a provider's (test)"));
        let acceptor = ca.acceptor("127.0.0.1");
        let (seen, raw): (Arc<Mutex<Vec<(String, String)>>>, Arc<Mutex<Vec<u8>>>) = Default::default();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = l.local_addr().unwrap().to_string();
        let (s2, r2) = (seen.clone(), raw.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = l.accept().await {
                let (acceptor, seen, raw) = (acceptor.clone(), s2.clone(), r2.clone());
                tokio::spawn(async move {
                    if !tls {
                        return answer(tcp, &seen, Some(&raw)).await;
                    }
                    let mut first = [0u8; 4096];
                    let n = tcp.peek(&mut first).await.unwrap_or(0);
                    raw.lock().unwrap().extend_from_slice(&first[..n]);
                    if let Ok(s) = acceptor.accept(tcp).await {
                        answer(s, &seen, None).await;
                    }
                });
            }
        });
        ProviderApi { host, seen, raw, ca }
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
