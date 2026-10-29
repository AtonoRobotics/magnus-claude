//! Backups of the event log and the vault (K10): one encrypted file each
//! time, made on a schedule and on the owner's request, each one restored
//! in memory as soon as it is written (every event replayed, every secret
//! decrypted) so a backup that would not restore is known at once. Kept no
//! longer than `keep_days` (the record: key-store backups no longer than the
//! legal deletion deadline). `magnus restore` puts one back into an empty
//! database and state directory.
//!
//! The file is XChaCha20-Poly1305 under the backup key (`[backup] key_file`),
//! which is not the vault's key. Whoever holds the file and that key holds
//! the appliance's history and secrets, so the key belongs off the appliance.

use crate::config::Config;
use crate::events::Ev;
use crate::kernel::Kernel;
use anyhow::{bail, Context, Result};
use base64::Engine;
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAGIC: &[u8] = b"MAGNUS-BACKUP-1\n";
const FORMAT: &str = "magnus-backup/1";
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn key(cfg: &Config, create: bool) -> Result<XChaCha20Poly1305> {
    let path = cfg.backup_key_file();
    if !path.exists() {
        if !create {
            bail!("no backup key at {}: put the key kept with the backups there", path.display());
        }
        let mut k = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        crate::vault::write_private(&path, &k)?;
    }
    let k = std::fs::read(&path).with_context(|| format!("read the backup key {}", path.display()))?;
    anyhow::ensure!(k.len() == 32, "the backup key is not 32 bytes");
    XChaCha20Poly1305::new_from_slice(&k).map_err(|_| anyhow::anyhow!("backup key"))
}

fn seal(cipher: &XChaCha20Poly1305, plain: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = cipher.encrypt(XNonce::from_slice(&nonce), plain).map_err(|_| anyhow::anyhow!("encrypt the backup"))?;
    Ok([MAGIC, &nonce[..], &ct].concat())
}

fn open(cipher: &XChaCha20Poly1305, file: &[u8]) -> Result<Value> {
    let body = file.strip_prefix(MAGIC).context("not a Magnus backup")?;
    anyhow::ensure!(body.len() > 24, "the backup is truncated");
    let plain = cipher.decrypt(XNonce::from_slice(&body[..24]), &body[24..]).map_err(|_| anyhow::anyhow!("the backup does not decrypt with this key"))?;
    let v: Value = serde_json::from_slice(&plain)?;
    anyhow::ensure!(v["format"] == FORMAT, "unknown backup format {}", v["format"]);
    Ok(v)
}

/// Restore a backup in memory: every event parses and replays in order,
/// and every vault secret decrypts with the backed-up vault key.
pub fn check(v: &Value) -> Result<(usize, i64)> {
    let mut state = crate::state::State::default();
    let mut last = 0i64;
    let events = v["events"].as_array().context("no events")?;
    for e in events {
        let seq = e["seq"].as_i64().context("an event without seq")?;
        anyhow::ensure!(seq > last, "events out of order at {seq}");
        last = seq;
        let ev: Ev = serde_json::from_value(e["body"].clone()).with_context(|| format!("event {seq} does not parse"))?;
        let at = chrono::DateTime::parse_from_rfc3339(e["at"].as_str().unwrap_or_default()).with_context(|| format!("event {seq} has no time"))?.with_timezone(&chrono::Utc);
        state.apply(&crate::log::Event { seq, at, actor: e["actor"].as_str().unwrap_or_default().into(), ev });
    }
    anyhow::ensure!(v["through_seq"].as_i64() == Some(last), "the backup ends at {last}, not at {}", v["through_seq"]);
    let vk = B64.decode(v["vault"]["key"].as_str().unwrap_or_default())?;
    let cipher = XChaCha20Poly1305::new_from_slice(&vk).map_err(|_| anyhow::anyhow!("the backed-up vault key is corrupt"))?;
    let secrets: std::collections::BTreeMap<String, String> = serde_json::from_str(v["vault"]["secrets"].as_str().unwrap_or("{}"))?;
    for (name, b64) in &secrets {
        let blob = B64.decode(b64)?;
        anyhow::ensure!(blob.len() > 24, "secret {name} is corrupt");
        cipher.decrypt(XNonce::from_slice(&blob[..24]), &blob[24..]).map_err(|_| anyhow::anyhow!("secret {name} does not decrypt"))?;
    }
    Ok((events.len(), last))
}

impl Kernel {
    /// Back up now: write the file, restore it in memory, record both, and
    /// drop backups older than `keep_days`.
    pub async fn back_up(&self, actor: &str) -> Result<Value> {
        let _one = backing_up().lock().await;
        let rows = self.log.dump().await?;
        let through = rows.last().map(|r| r["seq"].as_i64().unwrap_or(0)).unwrap_or(0);
        let (vault_key, secrets) = self.vault.raw()?;
        let made = chrono::Utc::now();
        let doc = json!({
            "format": FORMAT, "made_at": made, "through_seq": through, "events": rows,
            "vault": { "key": B64.encode(vault_key), "secrets": secrets },
        });
        let plain = serde_json::to_vec(&doc)?;
        let cipher = key(&self.cfg, true)?;
        let bytes = seal(&cipher, &plain)?;
        let dir = self.cfg.backup_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("magnus-{}.backup", made.format("%Y%m%dT%H%M%S%.3fZ")));
        crate::vault::write_private(&path, &bytes)?;
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        self.emit(actor, Ev::BackupMade { file: file.clone(), through_seq: through, events: rows.len(), bytes: bytes.len() }).await?;
        // The restore test, on the file as written.
        let checked = std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| open(&cipher, &b)).and_then(|v| check(&v));
        let (ok, detail) = match &checked {
            Ok((n, last)) => (true, format!("restored in memory: {n} events through {last}, every secret decrypts")),
            Err(e) => (false, format!("{e:#}")),
        };
        self.emit("kernel", Ev::BackupChecked { file: file.clone(), ok, detail: detail.clone() }).await?;
        let removed = prune(&dir, self.cfg.backup.keep_days)?;
        Ok(json!({ "file": file, "through_seq": through, "events": rows.len(), "bytes": bytes.len(), "restored": ok, "detail": detail, "removed": removed }))
    }
}

fn backing_up() -> &'static tokio::sync::Mutex<()> {
    static M: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Backups past their keeping time, by the time in their name.
fn prune(dir: &Path, keep_days: u32) -> Result<Vec<String>> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(keep_days as i64);
    let mut removed = vec![];
    for e in std::fs::read_dir(dir)? {
        let name = e?.file_name().to_string_lossy().to_string();
        let Some(ts) = name.strip_prefix("magnus-").and_then(|n| n.strip_suffix(".backup")) else { continue };
        let Ok(t) = chrono::NaiveDateTime::parse_from_str(ts, "%Y%m%dT%H%M%S%.3fZ") else { continue };
        if t.and_utc() < cutoff {
            std::fs::remove_file(dir.join(&name))?;
            removed.push(name);
        }
    }
    Ok(removed)
}

/// Back up every `every_hours`, from the appliance's start.
pub fn schedule(k: Arc<Kernel>) {
    let every = k.cfg.backup.every_hours;
    if every == 0 {
        return;
    }
    tokio::spawn(async move {
        let mut t = tokio::time::interval(std::time::Duration::from_secs(every as u64 * 3600));
        t.tick().await;
        loop {
            t.tick().await;
            if let Err(e) = k.back_up("kernel:backup").await {
                tracing::error!("scheduled backup: {e:#}");
            }
        }
    });
}

/// `magnus restore`: a backup into an empty event log and an empty vault.
pub async fn restore(cfg: &Config, admin_url: &str, from: &Path) -> Result<(usize, i64)> {
    let cipher = key(cfg, false)?;
    let v = open(&cipher, &std::fs::read(from).with_context(|| format!("read {}", from.display()))?)?;
    let (n, last) = check(&v)?;
    let vault_dir = cfg.appliance.state_dir.join("vault");
    if vault_dir.join("secrets.json").exists() {
        bail!("{} already holds secrets; restore into an empty state directory", vault_dir.display());
    }
    crate::log::migrate(admin_url, &cfg.database.url).await?;
    let admin = crate::log::connect(admin_url).await?;
    let mut db = admin;
    let tx = db.transaction().await?;
    let have: i64 = tx.query_one("SELECT count(*) FROM public.events", &[]).await?.get(0);
    if have > 0 {
        bail!("the event log already holds {have} events; restore into an empty one");
    }
    for e in v["events"].as_array().into_iter().flatten() {
        let at = chrono::DateTime::parse_from_rfc3339(e["at"].as_str().unwrap_or_default())?.with_timezone(&chrono::Utc);
        tx.execute(
            "INSERT INTO public.events (seq, at, kind, actor, run_id, body) OVERRIDING SYSTEM VALUE VALUES ($1, $2, $3, $4, $5, $6)",
            &[&e["seq"].as_i64(), &at, &e["kind"].as_str(), &e["actor"].as_str(), &e["run_id"].as_str(), &e["body"]],
        )
        .await?;
    }
    if last > 0 {
        tx.execute("SELECT setval(pg_get_serial_sequence('public.events', 'seq'), $1)", &[&last]).await?;
    }
    tx.commit().await?;
    std::fs::create_dir_all(&vault_dir)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&vault_dir, std::fs::Permissions::from_mode(0o700))?;
    crate::vault::write_private(&vault_dir.join("key"), &B64.decode(v["vault"]["key"].as_str().unwrap_or_default())?)?;
    crate::vault::write_private(&vault_dir.join("secrets.json"), v["vault"]["secrets"].as_str().unwrap_or("{}").as_bytes())?;
    Ok((n, last))
}

pub fn list(dir: &PathBuf) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            (n.starts_with("magnus-") && n.ends_with(".backup")).then(|| (n, e.metadata().map(|m| m.len()).unwrap_or(0)))
        })
        .collect();
    out.sort();
    out
}
