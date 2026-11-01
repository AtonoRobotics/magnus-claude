//! The admin console (owner's "Magnus Admin Console Spec", built as
//! written, amended by the owner's later rulings): the root control surface
//! for the organisation's appliance, served by its own process,
//! `magnus-admin`, not the user API, reachable only from the organisation's
//! network or its VPN (`net`). Requests must name the console's hostname;
//! anything else is not found. Every page is hardened: a strict
//! Content-Security-Policy, nothing from anywhere but the appliance.
//!
//! Its parts so far: the process, its listener and its shell (`net`), and
//! signing in (`auth`, `session`). The audit log and each section follow as
//! their own pull requests.

pub mod auth;
pub mod net;
pub mod session;

use crate::config::Config;
use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use std::sync::Arc;

/// What every page may load: only the appliance's own files, no inline
/// script, never framed.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; manifest-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// The console's settings, checked, and what it keeps while it runs.
pub struct Console {
    pub hostname: String,
    pub org: String,
    pub idp: session::Idp,
    pub key: auth::Key,
    pub store: auth::Store,
    pub idle_minutes: i64,
    pub session_hours: i64,
    pub step_up_seconds: i64,
    pub refresh_seconds: i64,
}

/// `magnus-admin --config <file>`, or `magnus-admin jwk --config <file>`
/// to print the console's public key, for its client in the realm.
pub fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("MAGNUS_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.iter().position(|a| a == "--config").and_then(|i| args.get(i + 1)) else {
        eprintln!("usage: magnus-admin --config <file>");
        std::process::exit(2);
    };
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let jwk = args.get(1).map(String::as_str) == Some("jwk");
    let r = rt.block_on(async {
        let cfg = Config::load(std::path::Path::new(path)).with_context(|| format!("load {path}"))?;
        if jwk {
            println!("{}", auth::Key::load_or_make(&key_dir(&cfg))?.jwks());
            return Ok(());
        }
        serve(cfg).await
    });
    if let Err(e) = r {
        eprintln!("magnus-admin: {e:#}");
        std::process::exit(1);
    }
}

/// Serve the console until the process ends. It refuses to start on an
/// address, a subnet or a certificate that would break its reach rules.
pub async fn serve(cfg: Config) -> Result<()> {
    let a = cfg.admin.clone().context("magnus.toml has no [admin] section: the admin console needs its hostname, listen addresses, allowed subnets and certificate")?;
    let addrs = net::listen_addrs(&a.listen)?;
    let allow = net::allowlist(&a.allow)?;
    anyhow::ensure!(valid_host(&a.hostname), "admin.hostname {:?} is not a hostname", a.hostname);
    let tls = net::tls(&a.tls_cert, &a.tls_key)?;
    let oidc = cfg.sign_in.oidc.as_ref();
    let issuer = a.issuer.clone().or_else(|| oidc.map(|o| o.issuer.clone())).context("the admin console signs in through the organisation's identity provider: set sign_in.oidc or admin.issuer")?;
    let roles_claim = oidc.and_then(|o| o.roles_claim.clone()).unwrap_or_else(|| "/realm_access/roles".into());
    let console = Arc::new(Console {
        hostname: a.hostname.to_ascii_lowercase(),
        org: a.org.clone().unwrap_or_else(|| "Magnus".into()),
        idp: session::Idp::new(issuer, a.client_id.clone(), roles_claim, a.idp_internal.clone()),
        key: auth::Key::load_or_make(&key_dir(&cfg))?,
        store: Default::default(),
        idle_minutes: a.idle_minutes.into(),
        session_hours: a.session_hours.into(),
        step_up_seconds: a.step_up_seconds.into(),
        refresh_seconds: a.refresh_seconds.into(),
    });
    let l = net::Listener::bind(&addrs, allow, tls).await?;
    axum::serve(l, router(console)).await?;
    Ok(())
}

/// Where the console keeps its own key: the appliance's state, readable by
/// its owner only.
fn key_dir(cfg: &Config) -> std::path::PathBuf {
    cfg.appliance.state_dir.join("admin")
}

fn valid_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= 253 && h.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !l.starts_with('-') && !l.ends_with('-'))
}

pub fn router(console: Arc<Console>) -> Router {
    // Every API route but health is an admin's alone.
    let api = Router::new()
        .route("/api/v1/session", get(session::me).delete(session::logout))
        .route("/api/v1/session/step-up", axum::routing::post(session::confirm))
        .route("/api/v1/{*rest}", axum::routing::any(api_not_found))
        .layer(axum::middleware::from_fn_with_state(console.clone(), session::guard));
    Router::new()
        .route("/", get(shell))
        .route("/assets/{file}", get(asset))
        .route("/auth/login", get(session::login))
        .route("/auth/step-up", get(session::step_up))
        .route("/auth/callback", get(session::callback))
        .route("/api/v1/health", get(health))
        .merge(api)
        .fallback(not_found)
        .layer(axum::middleware::from_fn_with_state(console.clone(), host_check))
        .layer(axum::middleware::from_fn(harden))
        .with_state(console)
}

/// Only requests naming the console's hostname: anything else is not found,
/// so the console is not even confirmed to be here.
async fn host_check(State(c): State<Arc<Console>>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default().to_ascii_lowercase();
    let name = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host.as_str(), |(n, _)| n);
    if name != c.hostname {
        return not_found().await;
    }
    next.run(req).await
}

/// Every answer: the strict policy, no sniffing, no referrer, never framed,
/// and HTTPS remembered.
async fn harden(req: Request, next: Next) -> Response {
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    for (k, v) in [
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::STRICT_TRANSPORT_SECURITY, "max-age=31536000"),
        (header::CACHE_CONTROL, "no-store"),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    h.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    h.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    r
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn api_not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
}

async fn health() -> Response {
    Json(json!({ "ok": true, "version": crate::update::version() })).into_response()
}

const SHELL: &str = include_str!("../../../../desktop/admin/index.html");

async fn shell(State(c): State<Arc<Console>>) -> Response {
    let page = SHELL.replace("{{org}}", &html_escape(&c.org));
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response()
}

/// The console's own files and the desktop app's styles, which it shares
/// (the same components and themes), all from the binary.
async fn asset(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    let (body, kind): (&'static str, &str) = match file.as_str() {
        "app.css" => (include_str!("../../../../desktop/ui/app.css"), "text/css; charset=utf-8"),
        "theme.js" => (include_str!("../../../../desktop/ui/theme.js"), "text/javascript; charset=utf-8"),
        "admin.css" => (include_str!("../../../../desktop/admin/admin.css"), "text/css; charset=utf-8"),
        "admin.js" => (include_str!("../../../../desktop/admin/admin.js"), "text/javascript; charset=utf-8"),
        _ => return not_found().await,
    };
    ([(header::CONTENT_TYPE, kind)], body).into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_hostname_is_a_hostname() {
        assert!(super::valid_host("admin.acme.internal") && super::valid_host("admin"));
        for bad in ["", "admin..acme", "-admin.acme", "admin.acme/", "admin acme", "10.0.0.1:8443"] {
            assert!(!super::valid_host(bad), "{bad}");
        }
    }
}
