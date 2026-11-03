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
