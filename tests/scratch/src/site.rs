//! A website for the browser tests, over HTTPS (a certificate for
//! `localhost` from a CA of its own, which the appliance is told to trust)
//! and plain HTTP. It records every request it receives, and its pages show
//! what a careless site might: the Authorization and Cookie headers it got,
//! and the cookies the page's own script can read.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, Request, Response};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Hit {
    pub scheme: &'static str,
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
}

#[derive(Clone)]
pub struct TestSite {
    pub https: u16,
    pub http: u16,
    /// The CA certificate (PEM) that signed the site's certificate.
    pub ca_pem: String,
    /// The session cookie the site sets on /account.
    pub session: String,
    hits: Arc<Mutex<Vec<Hit>>>,
}

fn page(req: &Request<Incoming>, session: &str) -> Response<Full<Bytes>> {
    let h = |k: &str| req.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("none").to_string();
    let html = |b: String| Response::builder().header("content-type", "text/html; charset=utf-8").body(Full::new(Bytes::from(b))).unwrap();
    match (req.method().as_str(), req.uri().path()) {
        ("GET", "/account") => {
            let mut r = html(format!(
                "<html><body><h1>Your account</h1><p>authorization: {}</p><p>cookies sent: {}</p><p id=\"js\">page cookies: not read</p>\
                 <script>document.getElementById('js').textContent = 'page cookies: [' + document.cookie + ']';</script></body></html>",
                h("authorization"),
                h("cookie")
            ));
            r.headers_mut().insert("set-cookie", format!("session={session}; Path=/").parse().unwrap());
            r
        }
        ("GET", "/form") => html("<html><body><form method=\"post\" action=\"/submit\"><input name=\"q\" aria-label=\"Query\"><button type=\"submit\">Go</button></form></body></html>".into()),
        ("POST", "/submit") => html("<html><body><p>submitted</p></body></html>".into()),
        ("GET", "/page") => html("<html><body><p>Plain page from the test site.</p></body></html>".into()),
        _ => Response::builder().status(404).body(Full::new(Bytes::from_static(b"not found"))).unwrap(),
    }
}

impl TestSite {
    pub async fn start() -> TestSite {
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca = rcgen::CertificateParams::default();
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca.distinguished_name.push(rcgen::DnType::CommonName, "Test site CA");
        let ca_cert = ca.self_signed(&ca_key).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap().signed_by(&key, &ca_cert, &ca_key).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![leaf.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
        let https = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let site = TestSite {
            https: https.local_addr().unwrap().port(),
            http: http.local_addr().unwrap().port(),
            ca_pem: ca_cert.pem(),
            session: format!("SESS-{}", uuid::Uuid::new_v4().simple()),
            hits: Arc::new(Mutex::new(vec![])),
        };
        let serve = |site: TestSite, scheme: &'static str| {
            hyper::service::service_fn(move |req: Request<Incoming>| {
                let site = site.clone();
                async move {
                    let headers = req.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())).collect();
                    site.hits.lock().unwrap().push(Hit { scheme, method: req.method().to_string(), path: req.uri().path().to_string(), headers });
                    let resp = page(&req, &site.session);
                    let _ = req.into_body().collect().await;
                    Ok::<_, std::convert::Infallible>(resp)
                }
            })
        };
        let s = site.clone();
        tokio::spawn(async move {
            while let Ok((c, _)) = https.accept().await {
                let (acceptor, svc) = (acceptor.clone(), serve(s.clone(), "https"));
                tokio::spawn(async move {
                    if let Ok(tls) = acceptor.accept(c).await {
                        let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(tls), svc).await;
                    }
                });
            }
        });
        let s = site.clone();
        tokio::spawn(async move {
            while let Ok((c, _)) = http.accept().await {
                let svc = serve(s.clone(), "http");
                tokio::spawn(async move {
                    let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(c), svc).await;
                });
            }
        });
        site
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    pub fn https_url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.https)
    }

    pub fn http_url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.http)
    }
}
