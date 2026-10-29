//! The installed desktop app, driven through tauri-driver against a real
//! appliance over HTTPS. Each test pins the certificate and signs in through
//! the identity provider, then does what crosses into the desktop shell:
//! - `desktop/tests/app.mjs`: connect Claude, type a task, approve in the
//!   inbox, see the outcome with its evidence, download the delivered file.
//! - `desktop/tests/app-library.mjs`: download a file from the library.
//!
//! The Windows app runs on another machine: `the_windows_app_passes_against_this_appliance`
//! serves the appliance on the LAN, and `desktop/tests/remote.mjs` runs the
//! same scripts there (`.github/workflows/desktop.yml`).
//!
//! Needs the built app (`cargo tauri build` in desktop/src-tauri, or
//! MAGNUS_DESKTOP_APP), `tauri-driver`, `WebKitWebDriver`
//! (MAGNUS_WEBKIT_DRIVER) and `xvfb-run`. Run with `--ignored`.

use magnus_invariants::appliance::workspace_root;
use magnus_invariants::{canary, eventually, PATIENCE};
use magnus_scratch::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Stdio;

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
}

fn app_bin() -> PathBuf {
    let app = std::env::var("MAGNUS_DESKTOP_APP").map(PathBuf::from).unwrap_or_else(|_| workspace_root().join("desktop/src-tauri/target/release/magnus-desktop"));
    assert!(app.is_file(), "build the app first: {}", app.display());
    app
}

/// tauri-driver under a virtual display, for a fresh user: its own home,
/// settings and Downloads, and no keyring, so the session goes to the
/// private file.
struct Driver {
    wd: String,
    home: tempfile::TempDir,
    _driver: tokio::process::Child,
}

impl Driver {
    async fn start() -> Driver {
        let native = std::env::var("MAGNUS_WEBKIT_DRIVER").map(PathBuf::from).ok().or_else(|| which("WebKitWebDriver")).expect("WebKitWebDriver (set MAGNUS_WEBKIT_DRIVER)");
        let driver = which("tauri-driver").or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo/bin/tauri-driver"))).expect("tauri-driver");
        let home = tempfile::tempdir().unwrap();
        let (config, downloads) = (home.path().join(".config"), home.path().join("Downloads"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(config.join("user-dirs.dirs"), format!("XDG_DOWNLOAD_DIR=\"{}\"\n", downloads.display())).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let native_port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = tokio::process::Command::new("xvfb-run")
            .args(["-a", "-s", "-screen 0 1280x900x24"])
            .arg(&driver)
            .args(["--port", &port.to_string(), "--native-port", &native_port.to_string(), "--native-driver"])
            .arg(&native)
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", &config)
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
            .env("NO_AT_BRIDGE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("start tauri-driver");
        let wd = format!("http://127.0.0.1:{port}");
        eventually("tauri-driver", PATIENCE, || {
            let wd = wd.clone();
            async move { reqwest::get(format!("{wd}/status")).await.ok().map(|_| ()) }
        })
        .await;
        Driver { wd, home, _driver: child }
    }

    fn config(&self) -> PathBuf {
        self.home.path().join(".config")
    }

    fn downloads(&self) -> PathBuf {
        self.home.path().join("Downloads")
    }

    /// Run an installed-app script against the appliance at `url`; returns
    /// whether it passed and what it printed.
    async fn script(&self, name: &str, url: &str, fingerprint: &str, envs: &[(&str, &str)]) -> (bool, String) {
        let out = tokio::process::Command::new("node")
            .arg(workspace_root().join("desktop/tests").join(name))
            .env("MAGNUS_WEBDRIVER", &self.wd)
            .env("MAGNUS_APP", app_bin())
            .env("MAGNUS_URL", url)
            .env("MAGNUS_FINGERPRINT", fingerprint)
            .env("MAGNUS_PERSON", PERSON)
            .env("MAGNUS_PROVIDER", IDP_LABEL)
            .envs(envs.iter().copied())
            .envs(std::env::var("MAGNUS_SHOTS").ok().map(|s| ("MAGNUS_SHOTS", s)))
            .output()
            .await
            .unwrap();
        (out.status.success(), format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
}

/// An appliance serving HTTPS with sign-in through the identity provider.
async fn appliance() -> (Flow, FakeIdp, String, String) {
    let mut f = Flow::start().await;
    let idp = FakeIdp::start(PERSON).await;
    idp.configure(&mut f.app).await;
    let (url, fingerprint) = f.with_tls().await;
    (f, idp, url, fingerprint)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the built desktop app, tauri-driver, WebKitWebDriver and xvfb-run"]
async fn the_linux_app_pins_signs_in_through_the_provider_approves_and_shows_evidence() {
    let (f, _idp, url, fingerprint) = appliance().await;
    let d = Driver::start().await;
    let key = canary("anthropic-key");
    let (ok, log) = d.script("app.mjs", &url, &fingerprint, &[("MAGNUS_API_KEY", &key), ("MAGNUS_TASK", TASK)]).await;
    assert!(ok, "the app flow failed:\n{log}\n{}", f.report().await);
    f.check_outcome(&key).await;

    // The app kept the session in a file only this user can read, and the
    // pinned fingerprint beside the address; nothing else secret.
    let dir = d.config().join("magnus");
    let session = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).find(|e| e.file_name().to_string_lossy().starts_with("session-")).expect("a session file");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(session.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    let settings = std::fs::read_to_string(dir.join("app.json")).unwrap();
    assert!(settings.contains(&fingerprint), "{settings}");
    assert!(!settings.contains("mgs_") && !settings.contains(&key), "a secret reached the settings: {settings}");
}

/// Found on 0.4.0 (2026-10-29): the library listed files the app could not
/// save, because the shell fetched bytes only from task outputs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the built desktop app, tauri-driver, WebKitWebDriver and xvfb-run"]
async fn the_installed_app_downloads_a_file_from_the_library() {
    let (f, _idp, url, fingerprint) = appliance().await;
    let (name, content) = ("kitchen-notes.txt", format!("Tiles from the corner shop. {}\n", canary("library")));
    use base64::Engine;
    let (st, v) = f.app.api(PERSON).post("/v1/files", json!({ "name": name, "media_type": "text/plain", "data_b64": base64::engine::general_purpose::STANDARD.encode(&content) })).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    let d = Driver::start().await;
    let (ok, log) = d.script("app-library.mjs", &url, &fingerprint, &[("MAGNUS_FILE", name), ("MAGNUS_FILE_TEXT", &content)]).await;
    assert!(ok, "the library download failed:\n{log}");
    assert_eq!(std::fs::read_to_string(Path::new(&d.downloads()).join(name)).unwrap(), content);
}

/// The installed Windows app, on another machine, against an appliance here.
/// The appliance listens on MAGNUS_LAN_HOST at the first of MAGNUS_LAN_PORTS,
/// the identity provider at the second, and the third says how to reach them
/// (`GET /run`) and takes each script's result (`POST /result`), as
/// `desktop/tests/remote.mjs` sends it. Waits for every script in
/// `remote.mjs`'s list; the CI job's time limit bounds the wait.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Windows app running desktop/tests/remote.mjs against this machine"]
async fn the_windows_app_passes_against_this_appliance() {
    use axum::{extract::State, routing::{get, post}, Json, Router};
    use std::sync::{Arc, Mutex};
    let host = std::env::var("MAGNUS_LAN_HOST").expect("MAGNUS_LAN_HOST: this machine's address on the LAN");
    let ports: Vec<String> = std::env::var("MAGNUS_LAN_PORTS").expect("MAGNUS_LAN_PORTS: appliance,provider,coordinator").split(',').map(|p| p.trim().to_string()).collect();
    assert_eq!(ports.len(), 3, "MAGNUS_LAN_PORTS needs three ports: appliance,provider,coordinator");

    let mut f = Flow::start().await;
    let idp = FakeIdp::start_on(PERSON, &format!("{host}:{}", ports[1])).await;
    idp.configure(&mut f.app).await;
    let (url, fingerprint) = f.with_tls_on(&format!("{host}:{}", ports[0])).await;
    let (name, content) = (format!("kitchen-notes-{}.txt", canary("n")), format!("Tiles from the corner shop. {}\n", canary("library")));
    use base64::Engine;
    let (st, v) = f.app.api(PERSON).post("/v1/files", json!({ "name": name, "media_type": "text/plain", "data_b64": base64::engine::general_purpose::STANDARD.encode(&content) })).await;
    assert_eq!(st.as_u16(), 201, "{v}");
    let key = canary("anthropic-key");
    let run = json!({
        "MAGNUS_URL": url, "MAGNUS_FINGERPRINT": fingerprint, "MAGNUS_PERSON": PERSON, "MAGNUS_PROVIDER": IDP_LABEL,
        "MAGNUS_API_KEY": key, "MAGNUS_TASK": TASK, "MAGNUS_FILE": name, "MAGNUS_FILE_TEXT": content,
    });

    type Results = Arc<Mutex<Vec<serde_json::Value>>>;
    let results: Results = Default::default();
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let router = Router::new()
        .route("/run", get(move || { let run = run.clone(); async move { Json(run) } }))
        .route("/result", post(|State((r, tx)): State<(Results, tokio::sync::mpsc::UnboundedSender<()>)>, Json(v): Json<serde_json::Value>| async move {
            let last = v["last"].as_bool().unwrap_or(false);
            r.lock().unwrap().push(v);
            if last { let _ = tx.send(()); }
            "ok"
        }))
        .with_state((results.clone(), done_tx));
    let l = tokio::net::TcpListener::bind(format!("{host}:{}", ports[2])).await.unwrap();
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    eprintln!("waiting for the Windows app at http://{host}:{}", ports[2]);
    done_rx.recv().await.unwrap();

    let results = results.lock().unwrap().clone();
    let failed: Vec<String> = results.iter().filter(|r| r["ok"] != true).map(|r| format!("{}:\n{}", r["script"], r["log"].as_str().unwrap_or_default())).collect();
    assert!(!results.is_empty() && failed.is_empty(), "the Windows app failed:\n{}\n{}", failed.join("\n"), f.report().await);
    f.check_outcome(&key).await;
}
