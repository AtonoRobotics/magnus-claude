//! Signed appliance updates with one-step rollback (gate item 5).
//!
//! Releases arrive in `[updates] dir` (CI on the build box writes them
//! there): `appliance/latest.json` names a package (a .tar.gz of the
//! programs and their `VERSION`) and its signature, made by `tauri signer
//! sign --app-version`, so the version is inside what was signed. The
//! appliance installs one only when the signature verifies with the release
//! key and binds the version the manifest announces, and only when it is
//! newer than what runs and was not rolled back from.
//!
//! Installing: unpack beside the programs' directory, run the new programs'
//! `check` against this appliance's log and vault (they must read both),
//! suspend running tasks ("update"), wait for their current step, swap the
//! directories (the old one is kept as `<dir>.prev`), and replace this
//! process with the new programs (`exec`, the same process id and
//! arguments). The new release resumes the tasks once it is serving. If it
//! cannot start, it swaps back and runs the old release, which resumes them.
//! The owner can go back one release the same way.
//!
//! The desktop app's packages are served to the apps, which check their own
//! signatures, by release channel: `stable` from `desktop/` (tagged
//! releases), `test` from `desktop/test/` (a build the owner released to
//! testers), `preview` from `desktop/preview/` (master, after each merge)
//! and `pr-<n>` from `desktop/pr-<n>/` (pull request n, under review). The
//! appliance decides which channel an app gets, never the app: the owner's
//! apps get the channel the owner chose, testers' get test, everyone else's
//! get stable. A channel that has no release (a closed pull request's)
//! hands its apps to the next steadier one: preview, then stable.

use crate::config::Config;
use crate::events::Ev;
use crate::kernel::Kernel;
use crate::state::Status;
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The oldest desktop app this appliance serves: older apps are told to
/// update. Raise it with any API change an older app would get wrong.
pub const MIN_APP: &str = "0.5.0";

/// What an install leaves for the release that starts next.
const PENDING: &str = "update.json";

/// How long running tasks get to finish their current step before the
/// switch. Every step survives a restart at any point (the invariants), so
/// waiting only saves repeating a model reply in flight; a minute covers one.
const DRAIN: Duration = Duration::from_secs(60);

/// The directory the running programs were started from. Fixed at the
/// first look: once the directory is moved aside the kernel's link to the
/// running program follows it, and the path is what an update replaces.
pub fn install_dir() -> Result<PathBuf> {
    static D: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    D.get_or_init(|| std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf))).clone().context("the program has no directory")
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut s = dir.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The release that runs: the installed package's `VERSION`, or the
/// version the programs were built as when they were not installed from one.
pub fn version() -> String {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        install_dir()
            .ok()
            .and_then(|d| std::fs::read_to_string(d.join("VERSION")).ok())
            .map(|v| v.trim().to_string())
            .filter(|v| semver::Version::parse(v).is_ok())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
    })
    .clone()
}

pub fn newer(a: &str, than: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(than)) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}

/// The signature must verify with the release key and bind `version`.
pub fn verify(data: &[u8], signature_b64: &str, public_key_b64: &str, version: &str) -> Result<()> {
    let text = |b: &str| -> Result<String> { Ok(String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b.trim())?)?) };
    let pk = minisign_verify::PublicKey::decode(&text(public_key_b64).context("the release key")?).map_err(|e| anyhow::anyhow!("the release key: {e}"))?;
    let sig = minisign_verify::Signature::decode(&text(signature_b64).context("the signature")?).map_err(|e| anyhow::anyhow!("the signature: {e}"))?;
    pk.verify(data, &sig, false).map_err(|_| anyhow::anyhow!("the signature does not verify with the release key"))?;
    let signed = sig.trusted_comment().split('\t').find_map(|f| f.strip_prefix("version:")).context("the signature names no version")?;
    anyhow::ensure!(signed == version, "the package was signed as {signed}, not {version}");
    Ok(())
}

/// `magnus check --config`: read this appliance's log and vault without
/// taking them over, as a new release must before it is switched to.
pub async fn check(cfg: &Config) -> Result<Value> {
    let events = crate::log::Log::read_only(&cfg.database.url).await?.load().await?;
    let mut state = crate::state::State::default();
    for e in &events {
        state.apply(e);
    }
    let vault = crate::vault::Vault::open(&cfg.appliance.state_dir)?;
    let secrets = vault.names()?.len();
    Ok(json!({ "version": version(), "events": events.len(), "secrets": secrets }))
}

fn pending_path(cfg: &Config) -> PathBuf {
    cfg.appliance.state_dir.join(PENDING)
}

/// A switch of releases is under way.
pub fn switching(cfg: &Config) -> bool {
    pending_path(cfg).exists()
}

pub fn pending(cfg: &Config) -> Option<Value> {
    std::fs::read(pending_path(cfg)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn write_pending(cfg: &Config, v: &Value) -> Result<()> {
    std::fs::write(pending_path(cfg), serde_json::to_vec(v)?)?;
    Ok(())
}

/// How this process was started: the program's name and its arguments.
fn started_as() -> Result<Value> {
    let exe = std::env::current_exe()?;
    let program = exe.file_name().context("program name")?.to_string_lossy().to_string();
    let program = program.strip_suffix(" (deleted)").unwrap_or(&program).to_string();
    Ok(json!({ "program": program, "args": std::env::args().skip(1).collect::<Vec<_>>() }))
}

/// Replace this process with `program` from the install directory, which
/// now holds the other release (the same process id).
fn exec_installed(how: &Value) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let path = install_dir()?.join(how["program"].as_str().context("program name")?);
    let args: Vec<String> = how["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(String::from)).collect();
    let e = std::process::Command::new(&path).args(&args).exec();
    Err(anyhow::anyhow!("start {}: {e}", path.display()))
}

/// Start this release again in this process (same pid and arguments), as
/// when the log may hold an event its state lacks; if that fails, exit, so
/// that nothing goes on from a state the log does not have.
pub fn restart_in_place() -> ! {
    let e = started_as().and_then(|how| exec_installed(&how));
    tracing::error!("restart in place: {e:?}");
    std::process::exit(1)
}

/// Exchange the running programs with the kept release.
fn swap(dir: &Path, with: &Path, keep_as: &Path) -> Result<()> {
    if keep_as.exists() {
        std::fs::remove_dir_all(keep_as)?;
    }
    std::fs::rename(dir, keep_as).with_context(|| format!("move {} aside", dir.display()))?;
    if let Err(e) = std::fs::rename(with, dir) {
        let _ = std::fs::rename(keep_as, dir);
        return Err(e).with_context(|| format!("put {} in place", with.display()));
    }
    Ok(())
}

/// Before the kernel starts: a release that was switched to and did not
/// come up is switched away from, once.
pub fn on_start(cfg: &Config) -> Result<()> {
    install_dir()?;
    let Some(mut p) = pending(cfg) else {
        return Ok(());
    };
    if p["started"].as_bool() == Some(true) {
        return roll_back_now(cfg, &p, "the new release did not start");
    }
    p["started"] = json!(true);
    write_pending(cfg, &p)
}

/// The release just switched to failed to start: go back and run the old one.
pub fn roll_back_now(cfg: &Config, p: &Value, reason: &str) -> Result<()> {
    if p["rollback"].as_bool() == Some(true) {
        // The release we went back to failed too: stay down for a person.
        let _ = std::fs::remove_file(pending_path(cfg));
        bail!("the release rolled back to did not start either: {reason}");
    }
    let dir = install_dir()?;
    let prev = sibling(&dir, ".prev");
    anyhow::ensure!(prev.is_dir(), "no earlier release to go back to");
    let failed = sibling(&dir, ".failed");
    swap(&dir, &prev, &failed)?;
    // The failed release stays as the one a rollback would reach.
    if prev.exists() {
        std::fs::remove_dir_all(&prev)?;
    }
    std::fs::rename(&failed, &prev)?;
    // The old release starts again exactly as it was started.
    write_pending(cfg, &json!({ "from": p["to"], "to": p["from"], "rollback": true, "reason": reason, "started_as": p["started_as"] }))?;
    tracing::error!("going back to {}: {reason}", p["from"]);
    exec_installed(&p["started_as"])
}

impl Kernel {
    /// Once serving: record how the switch went and resume the suspended tasks.
    pub async fn after_start(self: &Arc<Self>) -> Result<()> {
        if let Some(p) = pending(&self.cfg) {
            let (from, to) = (p["from"].as_str().unwrap_or_default().to_string(), p["to"].as_str().unwrap_or_default().to_string());
            if p["rollback"].as_bool() == Some(true) {
                let reason = p["reason"].as_str().unwrap_or("rolled back").to_string();
                self.emit("kernel", Ev::UpdateRolledBack { from, to, reason }).await?;
            } else {
                self.emit("kernel", Ev::UpdateFinished { from, to }).await?;
            }
            std::fs::remove_file(pending_path(&self.cfg))?;
        } else if let Some((_, to, _)) = self.read(|s| s.updating.clone()) {
            // Stopped between suspending the tasks and the switch.
            self.emit("kernel", Ev::UpdateFailed { to, reason: "the appliance stopped before the switch".into() }).await?;
        }
        self.resume_updated().await
    }

    async fn resume_updated(self: &Arc<Self>) -> Result<()> {
        if self.read(|s| s.kill_switch) {
            return Ok(());
        }
        let runs: Vec<String> = self.read(|s| s.runs.values().filter(|r| r.status == Status::Suspended && r.reason.as_deref() == Some("update")).map(|r| r.id.clone()).collect());
        for run in runs {
            self.emit("kernel", Ev::Resumed { run: run.clone() }).await?;
            self.wake(&run);
        }
        Ok(())
    }

    /// The latest release in the updates directory, if any.
    pub fn latest_release(&self) -> Option<Value> {
        std::fs::read(self.cfg.updates_dir().join("appliance/latest.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// Look for a newer release and install it. Returns what happened, for
    /// the owner; on success this process is replaced and never returns.
    pub async fn update(self: &Arc<Self>, actor: &str) -> Result<Value> {
        let _one = updating().lock().await;
        let current = version();
        let Some(m) = self.latest_release() else {
            return Ok(json!({ "status": "up_to_date", "version": current }));
        };
        let to = m["version"].as_str().unwrap_or_default().to_string();
        if !newer(&to, &current) || self.read(|s| s.skipped_releases.contains(&to)) {
            return Ok(json!({ "status": "up_to_date", "version": current, "latest": to }));
        }
        let new = match self.prepare(&m, &to).await {
            Ok(dir) => dir,
            Err(e) => {
                let reason = format!("{e:#}");
                self.emit(actor, Ev::UpdateFailed { to: to.clone(), reason: reason.clone() }).await?;
                return Ok(json!({ "status": "failed", "version": current, "latest": to, "reason": reason }));
            }
        };
        self.switch(actor, &current, &to, false, |dir| swap(dir, &new, &sibling(dir, ".prev"))).await
    }

    /// Go back one release: the one kept beside the running programs.
    pub async fn roll_back(self: &Arc<Self>, actor: &str) -> Result<Value> {
        let _one = updating().lock().await;
        let dir = install_dir()?;
        let prev = sibling(&dir, ".prev");
        let to = std::fs::read_to_string(prev.join("VERSION")).map(|v| v.trim().to_string()).context("no earlier release is kept")?;
        let current = version();
        self.switch(actor, &current, &to, true, |dir| {
            let tmp = sibling(dir, ".swap");
            swap(dir, &prev, &tmp)?;
            std::fs::rename(&tmp, &prev)?;
            Ok(())
        })
        .await
    }

    /// Verify, unpack and check a release; its programs' directory.
    async fn prepare(&self, m: &Value, to: &str) -> Result<PathBuf> {
        let file = m["file"].as_str().context("the release names no file")?;
        anyhow::ensure!(!file.contains('/') && !file.contains(".."), "not a release file name");
        let bytes = std::fs::read(self.cfg.updates_dir().join("appliance").join(file)).with_context(|| format!("read {file}"))?;
        verify(&bytes, m["signature"].as_str().unwrap_or_default(), &self.cfg.updates.public_key, to)?;
        let dir = install_dir()?;
        let new = sibling(&dir, ".new");
        if new.exists() {
            std::fs::remove_dir_all(&new)?;
        }
        std::fs::create_dir_all(&new)?;
        tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..])).unpack(&new).context("unpack the release")?;
        let v = std::fs::read_to_string(new.join("VERSION")).context("the release has no VERSION")?;
        anyhow::ensure!(v.trim() == to, "the release holds {}, not {to}", v.trim());
        let exe = std::env::current_exe()?;
        let prog = new.join(exe.file_name().context("program name")?);
        anyhow::ensure!(prog.is_file(), "the release has no {}", prog.display());
        let out = tokio::process::Command::new(&prog).args(["check", "--config"]).arg(&self.cfg.path).output().await.context("run the new release's check")?;
        anyhow::ensure!(out.status.success(), "the new release cannot read this appliance: {}", String::from_utf8_lossy(&out.stderr).trim());
        Ok(new)
    }

    /// Suspend the running tasks, let them finish their step, swap the
    /// programs and start them in this process.
    async fn switch(self: &Arc<Self>, actor: &str, from: &str, to: &str, rollback: bool, put: impl FnOnce(&Path) -> Result<()>) -> Result<Value> {
        self.emit(actor, Ev::UpdateStarted { from: from.into(), to: to.into(), rollback }).await?;
        self.suspend_for_update().await?;
        let start = std::time::Instant::now();
        while self.any_driving() && start.elapsed() < DRAIN {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let dir = install_dir()?;
        let r = started_as().and_then(|how| {
            write_pending(&self.cfg, &json!({ "from": from, "to": to, "rollback": rollback, "reason": if rollback { "the owner went back one release" } else { "" }, "started_as": how }))?;
            put(&dir)?;
            Ok(how)
        });
        let how = match r {
            Ok(how) => how,
            Err(e) => {
                let _ = std::fs::remove_file(pending_path(&self.cfg));
                let reason = format!("{e:#}");
                self.emit("kernel", Ev::UpdateFailed { to: to.into(), reason: reason.clone() }).await?;
                self.resume_updated().await?;
                return Ok(json!({ "status": "failed", "version": from, "latest": to, "reason": reason }));
            }
        };
        tracing::info!("switching from {from} to {to}");
        exec_installed(&how)?;
        unreachable!()
    }

    /// Every running task waits for the switch.
    pub async fn suspend_for_update(&self) -> Result<()> {
        let running: Vec<String> = self.read(|s| s.runs.values().filter(|r| r.status == Status::Running).map(|r| r.id.clone()).collect());
        for run in running {
            self.suspend(&run, "update", None).await?;
        }
        Ok(())
    }
}

fn updating() -> &'static tokio::sync::Mutex<()> {
    static M: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Look for a release every `check_minutes`, and install it when automatic.
pub fn schedule(k: Arc<Kernel>) {
    let c = k.cfg.updates.clone();
    if c.check_minutes == 0 || !c.automatic {
        return;
    }
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(c.check_minutes as u64 * 60));
        loop {
            t.tick().await;
            match k.update("kernel:update").await {
                Ok(v) if v["status"] == "failed" => tracing::error!("update: {}", v["reason"]),
                Ok(_) => {}
                Err(e) => tracing::error!("update: {e:#}"),
            }
        }
    });
}

/// Whether `c` names a release channel of the desktop app.
pub fn valid_channel(c: &str) -> bool {
    c == "stable" || c == "test" || c == "preview" || c.strip_prefix("pr-").is_some_and(|n| (1..=9).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The channel a version of the app was built for, by the pre-release CI
/// gives it: `<v>-review.<run>.pr.<n>` (and the earlier `<v>-pr.<n>.<run>`)
/// is pull request n's, `<v>-preview.<run>` master's, anything else stable.
pub fn channel_of(version: &str) -> String {
    let pre = version.split('+').next().and_then(|v| v.split_once('-')).map(|(_, p)| p).unwrap_or("");
    let ids: Vec<&str> = pre.split('.').collect();
    if let Some(n) = ids.iter().position(|i| *i == "pr").and_then(|i| ids.get(i + 1)) {
        if valid_channel(&format!("pr-{n}")) {
            return format!("pr-{n}");
        }
    }
    if ids[0] == "preview" { "preview" } else { "stable" }.to_string()
}

fn channel_dir(cfg: &Config, c: &str) -> PathBuf {
    let desktop = cfg.updates_dir().join("desktop");
    if c == "stable" {
        desktop
    } else {
        desktop.join(c)
    }
}

fn channel_release(cfg: &Config, c: &str) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(channel_dir(cfg, c).join("latest.json")).ok()?).ok()
}

/// The channel an app on `c` gets its release from: `c` while it has one,
/// else the next steadier channel that does.
pub fn resolve_channel(cfg: &Config, c: &str) -> Option<String> {
    let steadier: &[&str] = if c.starts_with("pr-") { &["preview", "stable"] } else if c == "preview" || c == "test" { &["stable"] } else { &[] };
    std::iter::once(c).chain(steadier.iter().copied()).find(|c| channel_release(cfg, c).is_some()).map(String::from)
}

/// The desktop app's release manifest for an app on `channel`, from the
/// channel `resolve_channel` gives, with each package's address on this
/// appliance (`origin` is the address the app used).
pub fn desktop_manifest(cfg: &Config, origin: &str, channel: &str) -> Option<Value> {
    let from = resolve_channel(cfg, channel)?;
    let mut m = channel_release(cfg, &from)?;
    if let Some(platforms) = m["platforms"].as_object_mut() {
        for p in platforms.values_mut() {
            if let Some(f) = p["file"].as_str().map(String::from) {
                p["url"] = json!(format!("{origin}/v1/updates/desktop/{from}/files/{f}"));
            }
        }
    }
    Some(m)
}

/// The channel the app asking gets: the owner's choice for the owner's apps
/// (until they choose, the channel their app's version was built for), test
/// for testers' (the `tester` role), and stable for everyone else's. An app that asks without a session gets the
/// channel its version was built for: the apps from before channels ask so,
/// and once updated they ask with their session.
pub fn assigned_channel(k: &Kernel, h: &HeaderMap) -> String {
    let built_for = channel_of(h.get("x-magnus-app").and_then(|v| v.to_str().ok()).unwrap_or(""));
    if crate::api::credential(h).is_none() {
        return built_for;
    }
    match crate::api::who(k, h) {
        Ok(p) if p.is_owner() => k.read(|s| s.update_channel.clone()).unwrap_or(built_for),
        Ok(p) if p.has_role("tester") => "test".into(),
        _ => "stable".into(),
    }
}

/// The channels that have a release, steadiest first: stable, test,
/// preview, then each pull request's by number.
pub fn desktop_channels(cfg: &Config) -> Vec<Value> {
    let mut prs: Vec<u64> = std::fs::read_dir(cfg.updates_dir().join("desktop"))
        .map(|d| d.filter_map(|e| e.ok()?.file_name().to_str()?.strip_prefix("pr-")?.parse().ok()).collect())
        .unwrap_or_default();
    prs.sort_unstable();
    ["stable".to_string(), "test".to_string(), "preview".to_string()]
        .into_iter()
        .chain(prs.into_iter().map(|n| format!("pr-{n}")))
        .filter(|c| valid_channel(c))
        .filter_map(|c| {
            let m = channel_release(cfg, &c)?;
            Some(json!({ "channel": c, "version": m["version"], "notes": m["notes"], "pub_date": m["pub_date"] }))
        })
        .collect()
}

use crate::api::{err, internal, owner, Transport};
use axum::{
    extract::{Path as UrlPath, Request, State as AxState},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};

type K = AxState<Arc<Kernel>>;

pub fn router() -> Router<Arc<Kernel>> {
    Router::new()
        .route("/v1/version", get(|| async { Json(json!({ "version": version(), "min_app": MIN_APP })) }))
        .route("/v1/updates", get(status))
        .route("/v1/updates/check", post(check_now))
        .route("/v1/updates/rollback", post(roll_back))
        .route("/v1/updates/channels", get(channels))
        .route("/v1/updates/channel", put(choose_channel))
        .route("/v1/updates/test", post(release_to_testers))
        .route("/v1/updates/desktop/latest.json", get(desktop_latest))
        .route("/v1/updates/desktop/files/{name}", get(stable_file))
        .route("/v1/updates/desktop/{channel}/files/{name}", get(desktop_file))
}

/// Versions both ways: every answer names this release, and an app older
/// than `MIN_APP` (it says its own in `x-magnus-app`) is told to update.
pub async fn app_version(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let exempt = path == "/v1/version" || path == "/v1/health" || path.starts_with("/v1/updates/desktop/");
    let app = req.headers().get("x-magnus-app").and_then(|v| v.to_str().ok()).map(String::from);
    let mut r = match app {
        Some(a) if !exempt && path.starts_with("/v1/") && newer(MIN_APP, &a) => (
            StatusCode::UPGRADE_REQUIRED,
            Json(json!({ "error": format!("This app ({a}) is older than this appliance accepts. Update the app to {MIN_APP} or later."), "update_required": true, "min_app": MIN_APP, "version": version() })),
        )
            .into_response(),
        _ => next.run(req).await,
    };
    if let Ok(v) = HeaderValue::from_str(&version()) {
        r.headers_mut().insert("x-magnus-appliance", v);
    }
    r
}

async fn status(AxState(k): K, h: HeaderMap) -> Response {
    if let Err(r) = owner(&k, &h) {
        return r;
    }
    let latest = k.latest_release().map(|m| json!({ "version": m["version"], "notes": m["notes"] }));
    let prev = install_dir().ok().and_then(|d| std::fs::read_to_string(sibling(&d, ".prev").join("VERSION")).ok()).map(|v| v.trim().to_string());
    let (updating, last, skipped) = k.read(|s| (s.updating.clone(), s.last_update.clone(), s.skipped_releases.clone()));
    Json(json!({
        "version": version(),
        "min_app": MIN_APP,
        "automatic": k.cfg.updates.automatic,
        "latest": latest,
        "updating": updating.map(|(from, to, rollback)| json!({ "from": from, "to": to, "rollback": rollback })),
        "last": last,
        "skipped": skipped,
        "previous": prev,
    }))
    .into_response()
}

/// Install the latest release now. The switch happens after the answer.
async fn check_now(AxState(k): K, h: HeaderMap) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let current = version();
    let to = k.latest_release().and_then(|m| m["version"].as_str().map(String::from));
    match to {
        Some(to) if newer(&to, &current) && !k.read(|s| s.skipped_releases.contains(&to)) => {
            let actor = format!("person:{}", p.id);
            tokio::spawn(async move {
                if let Err(e) = k.update(&actor).await {
                    tracing::error!("update: {e:#}");
                }
            });
            (StatusCode::ACCEPTED, Json(json!({ "status": "installing", "version": current, "latest": to }))).into_response()
        }
        to => Json(json!({ "status": "up_to_date", "version": current, "latest": to })).into_response(),
    }
}

async fn roll_back(AxState(k): K, h: HeaderMap) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(to) = install_dir().ok().and_then(|d| std::fs::read_to_string(sibling(&d, ".prev").join("VERSION")).ok()) else {
        return err(StatusCode::CONFLICT, "no earlier release is kept on this appliance");
    };
    let actor = format!("person:{}", p.id);
    tokio::spawn(async move {
        if let Err(e) = k.roll_back(&actor).await {
            tracing::error!("roll back: {e:#}");
        }
    });
    (StatusCode::ACCEPTED, Json(json!({ "status": "rolling_back", "version": version(), "to": to.trim() }))).into_response()
}

/// The channels with a release, and the one the owner's apps follow (asked
/// from one of them: `follows` is what it gets), for the owner to choose.
async fn channels(AxState(k): K, h: HeaderMap) -> Response {
    if let Err(r) = owner(&k, &h) {
        return r;
    }
    let follows = assigned_channel(&k, &h);
    let served = resolve_channel(&k.cfg, &follows);
    Json(json!({ "channels": desktop_channels(&k.cfg), "chosen": k.read(|s| s.update_channel.clone()), "follows": follows, "served": served })).into_response()
}

/// The owner chooses the channel their own apps update from.
async fn choose_channel(AxState(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(channel) = b["channel"].as_str().filter(|c| valid_channel(c)) else {
        return err(StatusCode::BAD_REQUEST, "channel: stable, preview or pr-<n>");
    };
    match k.emit(&format!("person:{}", p.id), Ev::UpdateChannelSet { channel: channel.into() }).await {
        Ok(_) => Json(json!({ "chosen": channel })).into_response(),
        Err(e) => internal(e),
    }
}

/// The owner releases a build to testers: `from` channel's release, the same
/// signed packages, becomes the test channel's (the one before it goes).
async fn release_to_testers(AxState(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(from) = b["from"].as_str().filter(|c| valid_channel(c) && *c != "test") else {
        return err(StatusCode::BAD_REQUEST, "from: stable, preview or pr-<n>");
    };
    let Some(release) = channel_release(&k.cfg, from) else {
        return err(StatusCode::NOT_FOUND, "that channel has no build");
    };
    let version = release["version"].as_str().unwrap_or_default().to_string();
    let copy = || -> Result<()> {
        let (src, dst) = (channel_dir(&k.cfg, from), channel_dir(&k.cfg, "test"));
        std::fs::create_dir_all(&dst)?;
        let mut files = vec![];
        for p in release["platforms"].as_object().into_iter().flat_map(|m| m.values()) {
            let f = p["file"].as_str().context("a package without a file")?;
            if f.contains('/') || f.contains("..") {
                bail!("a package outside its channel: {f}");
            }
            std::fs::copy(src.join(f), dst.join(f))?;
            files.push(f.to_string());
        }
        let mut m = release.clone();
        m["pub_date"] = json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        std::fs::write(dst.join("latest.json.tmp"), serde_json::to_vec_pretty(&m)?)?;
        std::fs::rename(dst.join("latest.json.tmp"), dst.join("latest.json"))?;
        for e in std::fs::read_dir(&dst)? {
            let name = e?.file_name().to_string_lossy().to_string();
            if name.starts_with("Magnus_") && !files.contains(&name) {
                std::fs::remove_file(dst.join(&name))?;
            }
        }
        Ok(())
    };
    if let Err(e) = copy() {
        return internal(e);
    }
    match k.emit(&format!("person:{}", p.id), Ev::TestReleaseApproved { version: version.clone(), from: from.into() }).await {
        Ok(_) => Json(json!({ "test": version, "from": from })).into_response(),
        Err(e) => internal(e),
    }
}

/// The apps ask with their session if they have one; without, an app too old
/// to sign in can still update. The appliance decides the channel.
async fn desktop_latest(AxState(k): K, Extension(t): Extension<Transport>, h: HeaderMap) -> Response {
    let channel = assigned_channel(&k, &h);
    let Some(host) = h.get("host").and_then(|v| v.to_str().ok()) else {
        return err(StatusCode::BAD_REQUEST, "no host");
    };
    let scheme = if t == Transport::Tls { "https" } else { "http" };
    match desktop_manifest(&k.cfg, &format!("{scheme}://{host}"), &channel) {
        Some(m) => Json(m).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Stable's packages at the address apps before channels were given.
async fn stable_file(k: K, UrlPath(name): UrlPath<String>) -> Response {
    desktop_file(k, UrlPath(("stable".into(), name))).await
}

async fn desktop_file(AxState(k): K, UrlPath((channel, name)): UrlPath<(String, String)>) -> Response {
    if !valid_channel(&channel) || name.contains('/') || name.contains("..") || name == "latest.json" {
        return err(StatusCode::NOT_FOUND, "no such file");
    }
    match tokio::fs::read(channel_dir(&k.cfg, &channel).join(&name)).await {
        Ok(b) => ([("content-type", "application/octet-stream")], b).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err(StatusCode::NOT_FOUND, "no such file"),
        Err(e) => internal(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_names_the_channel_it_was_built_for() {
        for (v, c) in [
            ("0.7.0", "stable"),
            ("0.7.1-preview.120", "preview"),
            ("0.7.1-review.121.pr.31", "pr-31"),
            ("0.7.1-pr.31.88", "pr-31"),
            ("0.7.1-review.121.pr.x", "stable"),
            ("0.7.1-rc.1", "stable"),
            ("", "stable"),
        ] {
            assert_eq!(channel_of(v), c, "{v}");
        }
        for c in ["stable", "preview", "pr-1", "pr-123456789"] {
            assert!(valid_channel(c), "{c}");
        }
        for c in ["", "files", "pr-", "pr-1234567890", "pr-1/..", "../desktop", "Stable"] {
            assert!(!valid_channel(c), "{c}");
        }
    }
}
