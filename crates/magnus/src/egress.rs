//! The browser's only way out: an HTTP proxy in the kernel. The browser runs
//! in a microVM with no network device, and its proxy is a forwarder that
//! reaches this over vsock; on a development host, Chromium is pointed here
//! with a per-run proxy credential. Every request is decided here, per
//! destination (the owner's sites, then the default), and the credentials
//! for a site are added here, on the way out, from the vault: they never
//! enter the browser, so no page and no model can read them.
//!
//! To see requests inside TLS, the proxy ends the browser's TLS with a
//! certificate from the appliance's own CA, which only the browser trusts,
//! and opens its own TLS to the site, checking the site's certificate as
//! any client does.

use crate::config::BrowserSite;
use crate::kernel::Kernel;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Resp = Response<Full<Bytes>>;

const MAX_UP: usize = 20 << 20;
const MAX_DOWN: usize = 50 << 20;
/// Headers that belong to one hop, never forwarded.
const HOP: &[&str] = &["connection", "keep-alive", "proxy-authorization", "proxy-authenticate", "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade", "host"];

/// The proxy's state: its CA, the per-run proxy credentials (development
/// host only), the cookie jars of credentialed sites, and what each run has
/// fetched with a credential.
pub struct Egress {
    ca_key: rcgen::KeyPair,
    ca_cert: rcgen::Certificate,
    leaves: Mutex<HashMap<String, Arc<rustls::sign::CertifiedKey>>>,
    tokens: Mutex<HashMap<String, String>>,
    /// (run, host) -> cookie name -> "name=value"
    jar: Mutex<HashMap<(String, String), BTreeMap<String, String>>>,
    /// Runs that fetched with a credential since their last browser result.
    credentialed: Mutex<HashSet<String>>,
    /// Values added for each run (credentials, held cookies), removed from
    /// everything the browser returns.
    secrets: Mutex<HashMap<String, HashSet<String>>>,
    upstream: reqwest::Client,
    port: std::sync::OnceLock<u16>,
}

fn ca_params() -> rcgen::CertificateParams {
    let mut p = rcgen::CertificateParams::default();
    p.distinguished_name.push(rcgen::DnType::CommonName, "Magnus browser egress");
    p.distinguished_name.push(rcgen::DnType::OrganizationName, "Magnus appliance");
    p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    p.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::CrlSign];
    p.not_before = rcgen::date_time_ymd(2025, 1, 1);
    p.not_after = rcgen::date_time_ymd(2045, 1, 1);
    p
}

/// Resolve names for the upstream client, refusing private addresses as
/// web_fetch does, so a page cannot reach the appliance or the LAN.
struct PublicOnly {
    allow_private: bool,
}

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow = self.allow_private;
        Box::pin(async move {
            let host = name.as_str().to_string();
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let ok: Vec<std::net::SocketAddr> = addrs.into_iter().filter(|a| allow || !crate::web::private_ip(a.ip())).collect();
            if ok.is_empty() {
                return Err(format!("{host} resolves only to private addresses; the browser may not reach them").into());
            }
            Ok(Box::new(ok.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl Egress {
    /// Open the proxy's CA (its key kept in the state directory) and its
    /// upstream client.
    pub fn open(state_dir: &Path, allow_private: bool, extra_ca: &[std::path::PathBuf]) -> Result<Egress> {
        let path = state_dir.join("egress-ca.key");
        let ca_key = match std::fs::read_to_string(&path) {
            Ok(pem) => rcgen::KeyPair::from_pem(&pem).context("read the browser egress CA key")?,
            Err(_) => {
                let k = rcgen::KeyPair::generate()?;
                crate::vault::write_private(&path, k.serialize_pem().as_bytes())?;
                k
            }
        };
        let ca_cert = ca_params().self_signed(&ca_key)?;
        let mut upstream = reqwest::Client::builder();
        for p in extra_ca {
            let pem = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
            upstream = upstream.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
        }
        let upstream = upstream
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(90))
            .dns_resolver(Arc::new(PublicOnly { allow_private }))
            .build()?;
        Ok(Egress {
            ca_key,
            ca_cert,
            leaves: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
            jar: Mutex::new(HashMap::new()),
            credentialed: Mutex::new(HashSet::new()),
            secrets: Mutex::new(HashMap::new()),
            upstream,
            port: std::sync::OnceLock::new(),
        })
    }

    /// The CA's public key hash, as Chromium's
    /// `--ignore-certificate-errors-spki-list` takes it: the browser trusts
    /// certificates from this CA, and nothing else needs to.
    pub fn spki(&self) -> String {
        use sha2::Digest;
        base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(self.ca_key.public_key_der()))
    }

    pub fn ca_pem(&self) -> String {
        self.ca_cert.pem()
    }

    /// The loopback port the proxy listens on for a development host.
    pub fn port(&self) -> Option<u16> {
        self.port.get().copied()
    }

    /// A proxy credential naming a run (development host).
    pub fn token(&self, run: &str) -> String {
        let mut t = self.tokens.lock().unwrap();
        if let Some((k, _)) = t.iter().find(|(_, r)| *r == run) {
            return k.clone();
        }
        let k = format!("egr_{}", uuid::Uuid::new_v4().simple());
        t.insert(k.clone(), run.to_string());
        k
    }

    fn run_of(&self, token: &str) -> Option<String> {
        self.tokens.lock().unwrap().get(token).cloned()
    }

    /// Forget a run: its proxy credential, cookies and secrets.
    pub fn forget(&self, run: &str) {
        self.tokens.lock().unwrap().retain(|_, r| r != run);
        self.jar.lock().unwrap().retain(|(r, _), _| r != run);
        self.credentialed.lock().unwrap().remove(run);
        self.secrets.lock().unwrap().remove(run);
    }

    /// Whether the run fetched with a credential since this was last asked:
    /// what the browser shows then is the person's private content.
    pub fn take_credentialed(&self, run: &str) -> bool {
        self.credentialed.lock().unwrap().remove(run)
    }

    /// What must never leave in a browser result for this run.
    pub fn secrets(&self, run: &str) -> Vec<String> {
        self.secrets.lock().unwrap().get(run).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    fn remember_secret(&self, run: &str, v: &str) {
        if v.len() >= 4 {
            self.secrets.lock().unwrap().entry(run.to_string()).or_default().insert(v.to_string());
        }
    }

    fn leaf(&self, host: &str) -> Result<Arc<rustls::sign::CertifiedKey>> {
        if let Some(k) = self.leaves.lock().unwrap().get(host) {
            return Ok(k.clone());
        }
        let key = rcgen::KeyPair::generate()?;
        let mut p = rcgen::CertificateParams::new(vec![host.to_string()])?;
        p.distinguished_name.push(rcgen::DnType::CommonName, host);
        // Valid from a day back to thirty days on.
        use chrono::Datelike;
        let ymd = |d: chrono::DateTime<chrono::Utc>| rcgen::date_time_ymd(d.year(), d.month() as u8, d.day() as u8);
        let now = chrono::Utc::now();
        p.not_before = ymd(now - chrono::Duration::days(1));
        p.not_after = ymd(now + chrono::Duration::days(30));
        let cert = p.signed_by(&key, &self.ca_cert, &self.ca_key)?;
        let der = rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into());
        let signing = rustls::crypto::ring::sign::any_supported_type(&der).map_err(|e| anyhow!("{e}"))?;
        let ck = Arc::new(rustls::sign::CertifiedKey::new(vec![cert.der().clone(), self.ca_cert.der().clone()], signing));
        self.leaves.lock().unwrap().insert(host.to_string(), ck.clone());
        Ok(ck)
    }
}

/// The owner's rule for a host: the exact host, else the longest `*.suffix`.
pub fn site_for<'a>(sites: &'a BTreeMap<String, BrowserSite>, host: &str) -> Option<&'a BrowserSite> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if let Some(s) = sites.get(&host) {
        return Some(s);
    }
    sites.values().filter(|s| s.host.strip_prefix("*.").is_some_and(|suf| host.ends_with(&format!(".{suf}")))).max_by_key(|s| s.host.len())
}

fn safe(m: &Method) -> bool {
    matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn text(status: StatusCode, msg: &str) -> Resp {
    Response::builder().status(status).header("content-type", "text/plain; charset=utf-8").body(Full::new(Bytes::from(msg.to_string()))).unwrap()
}

/// A request the boundary refused: the page, and so the agent, reads why.
fn blocked(why: &str) -> Resp {
    text(StatusCode::FORBIDDEN, &format!("Blocked by the Magnus appliance: {why}."))
}

impl Kernel {
    /// Whether a run's browser may make this request, and the owner's rule
    /// for the site (for its credential).
    pub fn browse_decision(&self, run: &str, u: &url::Url, method: &Method) -> std::result::Result<Option<BrowserSite>, String> {
        let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
        let (rule, both) = self.read(|s| (site_for(&s.browser_sites, &host).cloned(), s.runs.get(run).is_some_and(|r| r.untrusted && r.private)));
        if !matches!(u.scheme(), "http" | "https") {
            return Err(format!("{} addresses are not fetched", u.scheme()));
        }
        if let Some(r) = &rule {
            if r.access == "deny" {
                return Err(format!("the owner does not let the browser reach {host}"));
            }
            if let Some(ms) = &r.methods {
                if !ms.iter().any(|m| m.eq_ignore_ascii_case(method.as_str())) {
                    return Err(format!("the owner lets the browser use only {} with {host}", ms.join(", ")));
                }
                return Ok(rule);
            }
        }
        // The Rule of Two at the network: once a task has read private and
        // untrusted content, its browser may fetch, not send.
        if both && !safe(method) {
            return Err(format!("this task has read private and untrusted content, so its browser may fetch pages but not send to {host}"));
        }
        Ok(rule)
    }

    /// The credential a site's rule adds, for the person the run acts for.
    fn site_secret(&self, rule: &BrowserSite, run: &str) -> Option<(String, String)> {
        let c = rule.credential.as_ref()?;
        let name = if c.per_person {
            let person = self.read(|s| s.runs.get(run).map(|r| r.person.clone()))?;
            format!("browser:{}:{person}", rule.host)
        } else {
            c.vault.clone()?
        };
        let secret = self.vault.get(&name).ok().flatten()?;
        Some((c.header.clone(), c.value.replace("{secret}", secret.expose())))
    }

    /// Serve the proxy on a loopback port, for a browser on this host.
    pub async fn serve_egress(self: &Arc<Self>) -> Result<u16> {
        if let Some(p) = self.egress.port() {
            return Ok(p);
        }
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = l.local_addr()?.port();
        if self.egress.port.set(port).is_err() {
            return Ok(self.egress.port().expect("set"));
        }
        let k = self.clone();
        tokio::spawn(async move {
            loop {
                let Ok((s, _)) = l.accept().await else { continue };
                let k = k.clone();
                tokio::spawn(async move { k.egress_connection(s, None).await });
            }
        });
        Ok(port)
    }

    /// One connection from a browser to the proxy. A microVM's connection
    /// is its run's (`run`); one from this host names its run in its proxy
    /// credential.
    pub async fn egress_connection<S>(self: Arc<Self>, io: S, run: Option<String>)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let k = self.clone();
        let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
            let (k, run) = (k.clone(), run.clone());
            async move { Ok::<_, Infallible>(k.proxy(run, req).await) }
        });
        if let Err(e) = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(io), svc).with_upgrades().await {
            tracing::debug!("egress connection: {e}");
        }
    }

    async fn proxy(self: Arc<Self>, run: Option<String>, req: Request<Incoming>) -> Resp {
        let run = match run {
            Some(r) => r,
            None => {
                let token = req
                    .headers()
                    .get("proxy-authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Basic "))
                    .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
                    .and_then(|b| String::from_utf8(b).ok())
                    .and_then(|s| s.split(':').next().map(String::from));
                match token.and_then(|t| self.egress.run_of(&t)) {
                    Some(r) => r,
                    None => {
                        return Response::builder()
                            .status(StatusCode::PROXY_AUTHENTICATION_REQUIRED)
                            .header("proxy-authenticate", "Basic realm=\"magnus\"")
                            .body(Full::new(Bytes::new()))
                            .unwrap()
                    }
                }
            }
        };
        if req.method() == Method::CONNECT {
            let Some(auth) = req.uri().authority().cloned() else { return text(StatusCode::BAD_REQUEST, "CONNECT needs host:port") };
            let host = auth.host().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
            let port = auth.port_u16().unwrap_or(443);
            // A denied site is refused before the tunnel opens.
            let probe = url::Url::parse(&format!("https://{auth}/")).ok();
            if let Some(Err(why)) = probe.as_ref().map(|u| self.browse_decision(&run, u, &Method::GET)) {
                if why.starts_with("the owner does not") {
                    return blocked(&why);
                }
            }
            let k = self.clone();
            tokio::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(up) => {
                        if let Err(e) = k.tunnel(run, host, port, up).await {
                            tracing::debug!("egress tunnel: {e:#}");
                        }
                    }
                    Err(e) => tracing::debug!("egress upgrade: {e}"),
                }
            });
            return Response::new(Full::new(Bytes::new()));
        }
        let Some(u) = req.uri().scheme_str().and_then(|_| url::Url::parse(&req.uri().to_string()).ok()) else {
            return text(StatusCode::BAD_REQUEST, "a proxy request needs an absolute address");
        };
        self.forward(&run, u, req).await
    }

    /// A CONNECT tunnel: end the browser's TLS with a certificate for the
    /// host from the appliance's CA, then serve its requests one by one.
    async fn tunnel(self: Arc<Self>, run: String, host: String, port: u16, up: hyper::upgrade::Upgraded) -> Result<()> {
        struct One(Arc<rustls::sign::CertifiedKey>);
        impl std::fmt::Debug for One {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("One")
            }
        }
        impl rustls::server::ResolvesServerCert for One {
            fn resolve(&self, _: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
                Some(self.0.clone())
            }
        }
        let key = self.egress.leaf(&host)?;
        let mut cfg = rustls::ServerConfig::builder().with_no_client_auth().with_cert_resolver(Arc::new(One(key)));
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(cfg)).accept(TokioIo::new(up)).await?;
        let authority = if port == 443 { host.clone() } else { format!("{host}:{port}") };
        let k = self.clone();
        let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
            let (k, run, authority) = (k.clone(), run.clone(), authority.clone());
            async move {
                let path = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
                Ok::<_, Infallible>(match url::Url::parse(&format!("https://{authority}{path}")) {
                    Ok(u) => k.forward(&run, u, req).await,
                    Err(_) => text(StatusCode::BAD_REQUEST, "not an address"),
                })
            }
        });
        hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(tls), svc).await?;
        Ok(())
    }

    /// Decide one request, add the site's credential and held cookies, send
    /// it, and hold the site's cookies back from the browser.
    async fn forward(&self, run: &str, u: url::Url, req: Request<Incoming>) -> Resp {
        let method = req.method().clone();
        let rule = match self.browse_decision(run, &u, &method) {
            Ok(r) => r,
            Err(why) => return blocked(&why),
        };
        let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
        // A site with a credential is signed in at the boundary: its
        // cookies stay here, and only travel over https.
        let held = rule.as_ref().is_some_and(|r| r.credential.is_some());
        let mut out = reqwest::header::HeaderMap::new();
        for (name, value) in req.headers() {
            let n = name.as_str();
            if HOP.contains(&n) || (held && n == "cookie") {
                continue;
            }
            out.append(name.clone(), value.clone());
        }
        if held && u.scheme() == "https" {
            if let Some((header, value)) = rule.as_ref().and_then(|r| self.site_secret(r, run)) {
                self.egress.remember_secret(run, &value);
                if let Some(raw) = value.split_whitespace().last() {
                    self.egress.remember_secret(run, raw);
                }
                if let (Ok(h), Ok(v)) = (reqwest::header::HeaderName::from_bytes(header.as_bytes()), reqwest::header::HeaderValue::from_str(&value)) {
                    out.insert(h, v);
                }
                self.egress.credentialed.lock().unwrap().insert(run.to_string());
            }
            let cookies: Vec<String> = self.egress.jar.lock().unwrap().get(&(run.to_string(), host.clone())).map(|j| j.values().cloned().collect()).unwrap_or_default();
            if !cookies.is_empty() {
                if let Ok(v) = reqwest::header::HeaderValue::from_str(&cookies.join("; ")) {
                    out.insert(reqwest::header::COOKIE, v);
                }
                self.egress.credentialed.lock().unwrap().insert(run.to_string());
            }
        }
        let body = match Limited::new(req.into_body(), MAX_UP).collect().await {
            Ok(b) => b.to_bytes(),
            Err(_) => return text(StatusCode::PAYLOAD_TOO_LARGE, "the request body is too large"),
        };
        let resp = match self.egress.upstream.request(method, u.clone()).headers(out).body(body).send().await {
            Ok(r) => r,
            Err(e) => {
                let why = format!("{e:#}");
                return if why.contains("private addresses") { blocked(&format!("{host} is on a private network")) } else { text(StatusCode::BAD_GATEWAY, &format!("{host} could not be reached")) };
            }
        };
        let mut b = Response::builder().status(resp.status());
        for (name, value) in resp.headers() {
            let n = name.as_str();
            if HOP.contains(&n) || n == "content-length" {
                continue;
            }
            if held && n == "set-cookie" {
                if let Some(pair) = value.to_str().ok().and_then(|v| v.split(';').next()).map(str::trim).filter(|p| p.contains('=')) {
                    let name = pair.split('=').next().unwrap_or_default().to_string();
                    self.egress.remember_secret(run, pair.split_once('=').map(|(_, v)| v).unwrap_or_default());
                    self.egress.jar.lock().unwrap().entry((run.to_string(), host.clone())).or_default().insert(name, pair.to_string());
                }
                continue;
            }
            b = b.header(name, value);
        }
        let mut stream = resp;
        let mut got = Vec::new();
        loop {
            match stream.chunk().await {
                Ok(Some(c)) => {
                    if got.len() + c.len() > MAX_DOWN {
                        return text(StatusCode::BAD_GATEWAY, "the page is too large");
                    }
                    got.extend_from_slice(&c);
                }
                Ok(None) => break,
                Err(_) => return text(StatusCode::BAD_GATEWAY, &format!("{host} stopped answering")),
            }
        }
        b.body(Full::new(Bytes::from(got))).unwrap_or_else(|_| text(StatusCode::BAD_GATEWAY, "an unreadable answer"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(host: &str, access: &str) -> BrowserSite {
        BrowserSite { host: host.into(), access: access.into(), methods: None, credential: None }
    }

    #[test]
    fn a_host_takes_its_own_rule_then_the_longest_wildcard() {
        let mut sites = BTreeMap::new();
        for s in [site("*.example.com", "allow"), site("*.corp.example.com", "deny"), site("www.example.com", "deny")] {
            sites.insert(s.host.clone(), s);
        }
        assert_eq!(site_for(&sites, "www.example.com").unwrap().host, "www.example.com");
        assert_eq!(site_for(&sites, "a.corp.example.com").unwrap().host, "*.corp.example.com");
        assert_eq!(site_for(&sites, "News.Example.com.").unwrap().host, "*.example.com");
        assert!(site_for(&sites, "example.com").is_none());
        assert!(site_for(&sites, "badexample.com").is_none());
    }

    #[test]
    fn the_ca_is_kept_and_its_key_hash_stays() {
        let dir = tempfile::tempdir().unwrap();
        let a = Egress::open(dir.path(), false, &[]).unwrap();
        let b = Egress::open(dir.path(), false, &[]).unwrap();
        assert_eq!(a.spki(), b.spki());
        assert!(a.ca_pem().contains("BEGIN CERTIFICATE"));
        let leaf = a.leaf("example.com").unwrap();
        assert_eq!(leaf.cert.len(), 2, "the leaf is sent with the CA");
    }
}
