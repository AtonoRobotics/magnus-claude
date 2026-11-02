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
pub mod kernel;
pub mod net;
pub mod ops;
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
    pub kernel: kernel::Kernel,
    /// The appliance's state, whose disk Health watches.
    pub state_dir: std::path::PathBuf,
}

impl Console {
    /// Write one console action to the log (admin console spec: every
    /// action, with who, what, when, the source address, and for a change
    /// its before and after), before the action answers.
    pub async fn audit(&self, who: (&str, &str), action: &str, target: Option<&str>, source: std::net::SocketAddr, before: Option<serde_json::Value>, after: Option<serde_json::Value>) -> Result<i64> {
        self.kernel
            .audit(&json!({ "admin": who.0, "name": who.1, "action": action, "target": target, "source": source.ip().to_string(), "before": before, "after": after }))
            .await
            .context("record it in the audit log")
    }
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
        kernel: kernel::Kernel { path: crate::control::socket_path(&cfg.appliance.state_dir) },
        state_dir: cfg.appliance.state_dir.clone(),
    });
    let l = net::Listener::bind(&addrs, allow, tls).await?;
    // tap_io gives the listener axum's connect-info: each request's source,
    // for the audit log.
    use axum::serve::ListenerExt;
    let l = l.tap_io(|_| {});
    axum::serve(l, router(console).into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;
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
        .route("/api/v1/overview", get(ops::overview))
        .route("/api/v1/health", get(ops::health))
        .route("/api/v1/kill-switch", get(ops::switch_state))
        .route("/api/v1/kill-switch/engage", axum::routing::post(ops::engage))
        .route("/api/v1/kill-switch/release", axum::routing::post(ops::release))
        .route("/api/v1/audit", get(audit_list))
        .route("/api/v1/audit.csv", get(audit_csv))
        .route("/api/v1/{*rest}", axum::routing::any(api_not_found))
        .layer(axum::middleware::from_fn_with_state(console.clone(), session::guard));
    Router::new()
        .route("/", get(shell))
        .route("/assets/{file}", get(asset))
        .route("/auth/login", get(session::login))
        .route("/auth/step-up", get(session::step_up))
        .route("/auth/callback", get(session::callback))
        .route("/status", get(status))
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

/// The audit log, filtered by actor, action, target and time (`since`,
/// `until`), newest first.
async fn audit_list(State(c): State<Arc<Console>>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> Response {
    match audit_rows(&c, q).await {
        Ok(v) => Json(json!({ "entries": v })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": "kernel", "message": format!("{e:#}") }))).into_response(),
    }
}

async fn audit_rows(c: &Console, q: Option<String>) -> Result<Vec<serde_json::Value>> {
    let path = format!("/v1/audit{}", q.map(|q| format!("?{q}")).unwrap_or_default());
    Ok(c.kernel.call(axum::http::Method::GET, &path, None).await?["entries"].as_array().cloned().unwrap_or_default())
}

/// The same, as CSV.
async fn audit_csv(State(c): State<Arc<Console>>, axum::extract::RawQuery(q): axum::extract::RawQuery) -> Response {
    let rows = match audit_rows(&c, q).await {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
    };
    let cell = |v: &serde_json::Value| {
        let s = match v {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        // A cell a spreadsheet would run as a formula is kept as text.
        let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s };
        format!("\"{}\"", s.replace('"', "\"\""))
    };
    let mut out = String::from("seq,at,admin,name,action,target,source,before,after\r\n");
    for r in &rows {
        let cols = ["seq", "at", "admin", "name", "action", "target", "source", "before", "after"].map(|f| cell(&r[f]));
        out.push_str(&cols.join(","));
        out.push_str("\r\n");
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"magnus-audit.csv\"")], out).into_response()
}

async fn api_not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
}

/// Whether the console answers, for the status dot before anyone signs
/// in: nothing more, not even its version.
async fn status() -> Response {
    Json(json!({ "ok": true })).into_response()
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
