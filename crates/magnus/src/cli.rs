//! `magnus migrate --config <file> --admin-url <url>`: create the event log
//! and the kernel's database role.
//! `magnus serve --config <file>`: run the appliance.
//! `magnus password --config <file> --person <id>`: set a person's sign-in
//! password, read from standard input.

use crate::config::Config;
use anyhow::{Context, Result};
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

pub fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("MAGNUS_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().collect();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let r = rt.block_on(async {
        match args.get(1).map(String::as_str) {
            Some("migrate") => migrate(&args).await,
            Some("serve") => serve(&args).await,
            Some("conformance") => conformance(&args).await,
            Some("password") => password(&args),
            Some("restore") => restore(&args).await,
            Some("check") => check(&args).await,
            Some("version") => {
                println!("{}", crate::update::version());
                Ok(())
            }
            _ => {
                eprintln!("usage: magnus migrate --config <file> --admin-url <url>\n       magnus serve --config <file>\n       magnus password --config <file> --person <id>   (password on stdin)\n       magnus conformance --config <file> --adapter <id> [--model <id>]\n       magnus restore --config <file> --admin-url <url> --from <backup file>\n       magnus check --config <file>   (reads the log and the vault, as a new release must)\n       magnus version");
                std::process::exit(2);
            }
        }
    });
    if let Err(e) = r {
        eprintln!("magnus: {e:#}");
        std::process::exit(1);
    }
}

fn config(args: &[String]) -> Result<Config> {
    let path = PathBuf::from(arg(args, "--config").context("--config is required")?);
    Config::load(&path).with_context(|| format!("load {}", path.display()))
}

/// Put a backup back into an empty event log and state directory (K10).
async fn restore(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    let admin = arg(args, "--admin-url").context("--admin-url is required")?;
    let from = PathBuf::from(arg(args, "--from").context("--from is required")?);
    let (n, last) = crate::backup::restore(&cfg, &admin, &from).await?;
    println!("restored {n} events through {last} and the vault from {}", from.display());
    Ok(())
}

/// A new release proves it can read this appliance before it is switched to.
async fn check(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    println!("{}", crate::update::check(&cfg).await?);
    Ok(())
}

async fn migrate(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    let admin = arg(args, "--admin-url").context("--admin-url is required")?;
    crate::log::migrate(&admin, &cfg.database.url).await
}

async fn serve(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    crate::update::on_start(&cfg)?;
    // A panic while starting is a failed start too, not a process that is
    // gone with the switch unrecorded.
    use futures::FutureExt;
    let r = match std::panic::AssertUnwindSafe(run(cfg.clone())).catch_unwind().await {
        Ok(r) => r,
        Err(p) => Err(anyhow::anyhow!("panicked: {}", p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default())),
    };
    // A release just switched to that cannot serve goes back to the one before.
    if let Err(e) = &r {
        if let Some(p) = crate::update::pending(&cfg) {
            crate::update::roll_back_now(&cfg, &p, &format!("{e:#}"))?;
        }
    }
    r
}

async fn run(cfg: Config) -> Result<()> {
    use crate::api::{router, Transport};
    use std::net::SocketAddr;
    let listen = cfg.appliance.listen.clone();
    let tls = match &cfg.appliance.tls_listen {
        Some(addr) => {
            let (server, fingerprint) = crate::tls::load(&cfg.appliance.state_dir, cfg.appliance.tls_cert.as_deref(), cfg.appliance.tls_key.as_deref())?;
            Some((addr.clone(), server, fingerprint))
        }
        None => None,
    };
    let k = crate::kernel::Kernel::boot(cfg).await?;
    let plain = tokio::net::TcpListener::bind(&listen).await.with_context(|| format!("listen on {listen}"))?;
    // The bound address, which differs from `listen` when it names port 0.
    let bound = plain.local_addr()?;
    let tls = match tls {
        Some((addr, server, fingerprint)) => Some((crate::tls::Listener::bind(&addr, server).await?, addr, fingerprint)),
        None => None,
    };
    // Serving: an update that brought this release here is done.
    k.after_start().await?;
    crate::backup::schedule(k.clone());
    crate::trash::schedule(k.clone());
    crate::update::schedule(k.clone());
    tokio::spawn(k.clone().sweep());
    tracing::info!("magnus {} listening on http://{bound}", crate::update::version());
    let plain = axum::serve(plain, router(k.clone(), Transport::Plain).into_make_service_with_connect_info::<SocketAddr>());
    match tls {
        Some((l, addr, fingerprint)) => {
            use axum::serve::ListenerExt;
            // tap_io gives the TLS listener axum's connect-info for its peer address.
            let l = l.tap_io(|_| {});
            tracing::info!("magnus listening on https://{addr}, certificate SHA-256 {fingerprint}");
            let secure = axum::serve(l, router(k, Transport::Tls).into_make_service_with_connect_info::<SocketAddr>());
            tokio::try_join!(async { plain.await }, async { secure.await })?;
        }
        None => plain.await?,
    }
    Ok(())
}

/// Set a person's password from standard input (one line).
fn password(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    let person = arg(args, "--person").context("--person is required")?;
    anyhow::ensure!(cfg.person(&person).is_some(), "{person} is not a person in the config");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let pw = line.trim_end_matches(['\r', '\n']);
    crate::accounts::set_password(&cfg.appliance.state_dir, &cfg.sign_in, &person, pw)?;
    eprintln!("password set for {person}");
    Ok(())
}

/// Run an adapter's conformance suite with the appliance's vault, for release
/// checks. Prints the report; exits 1 unless every check passes.
async fn conformance(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    let id = arg(args, "--adapter").context("--adapter is required")?;
    let model = match arg(args, "--model") {
        Some(m) => m,
        None if cfg.model.adapter == id => cfg.model.name.clone(),
        None => anyhow::bail!("--model is required: the configured model belongs to adapter {}", cfg.model.adapter),
    };
    let vault = std::sync::Arc::new(crate::vault::Vault::open(&cfg.appliance.state_dir)?);
    let a = cfg.adapters.iter().find(|a| a.id == id).context("no such adapter in the config")?.clone();
    let adapters = crate::adapters::Adapters::new(&cfg.appliance.state_dir, vault, &cfg.adapters);
    let r = adapters.launch(&a).await?;
    let report = adapters.conformance(&r, &model).await;
    println!("{}", serde_json::to_string_pretty(&report)?);
    anyhow::ensure!(report.passed, "conformance failed");
    Ok(())
}
