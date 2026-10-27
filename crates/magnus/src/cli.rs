//! `magnus migrate --config <file> --admin-url <url>`: create the event log
//! and the kernel's database role.
//! `magnus serve --config <file>`: run the appliance.

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
            _ => {
                eprintln!("usage: magnus migrate --config <file> --admin-url <url>\n       magnus serve --config <file>\n       magnus conformance --config <file> --adapter <id> [--model <id>]");
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

async fn migrate(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    let admin = arg(args, "--admin-url").context("--admin-url is required")?;
    crate::log::migrate(&admin, &cfg.database.url).await
}

async fn serve(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    let listen = cfg.appliance.listen.clone();
    let k = crate::kernel::Kernel::boot(cfg).await?;
    tokio::spawn(k.clone().sweep());
    let listener = tokio::net::TcpListener::bind(&listen).await.with_context(|| format!("listen on {listen}"))?;
    tracing::info!("magnus listening on {listen}");
    axum::serve(listener, crate::api::router(k)).await?;
    Ok(())
}

/// Run an adapter's conformance suite with the appliance's vault, for release
/// checks. Prints the report; exits 1 unless every check passes.
async fn conformance(args: &[String]) -> Result<()> {
    let cfg = config(args)?;
    let id = arg(args, "--adapter").context("--adapter is required")?;
    let model = arg(args, "--model").unwrap_or_else(|| cfg.model.name.clone());
    let vault = std::sync::Arc::new(crate::vault::Vault::open(&cfg.appliance.state_dir)?);
    let a = cfg.adapters.iter().find(|a| a.id == id).context("no such adapter in the config")?.clone();
    let adapters = crate::adapters::Adapters::new(&cfg.appliance.state_dir, vault, &cfg.adapters);
    let r = adapters.launch(&a).await?;
    let report = adapters.conformance(&r, &model).await;
    println!("{}", serde_json::to_string_pretty(&report)?);
    anyhow::ensure!(report.passed, "conformance failed");
    Ok(())
}
