//! The installed desktop app, driven through tauri-driver against a real
//! appliance over HTTPS. Each test pins the certificate and signs in through
//! the identity provider, then does what crosses into the desktop shell:
//! - `desktop/tests/app.mjs`: connect Claude, type a task, approve in the
//!   inbox, see the outcome with its evidence, download the delivered file.
//! - `desktop/tests/app-library.mjs`: download a file from the library.
//! - `desktop/tests/app-people.mjs`: two people sign in on one app, one after
//!   the other, each with their own rights.
//! - `desktop/tests/app-apps.mjs`: the owner adds one of the person's apps
//!   and signs in to it through a loopback address the shell opens.
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

/// The sign-in link crosses into the shell, and so does the kept session:
/// signing out must leave nothing of the first person for the second.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the built desktop app, tauri-driver, WebKitWebDriver and xvfb-run"]
async fn two_people_sign_in_on_one_installed_app_each_with_their_own_rights() {
    let (f, _idp, url, fingerprint) = appliance().await;
    let title = format!("Plan the offsite budget {}", canary("people"));
    f.app.api(PERSON).submit(&title).await;
    let d = Driver::start().await;
    let (ok, log) = d.script("app-people.mjs", &url, &fingerprint, &[("MAGNUS_OTHER", "carol"), ("MAGNUS_TASK_TITLE", &title)]).await;
    assert!(ok, "two people on one app failed:\n{log}");
}

/// The app sign-in's code comes back to a loopback address the shell opens
/// on this computer: only the installed app can catch it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the built desktop app, tauri-driver, WebKitWebDriver and xvfb-run"]
async fn the_owner_adds_an_app_and_signs_in_to_it_from_the_installed_app() {
    let (f, _idp, url, fingerprint) = appliance().await;
    let cal = FakeOAuthApp::start(3600).await;
    let d = Driver::start().await;
    let (ok, log) = d.script("app-apps.mjs", &url, &fingerprint, &[("MAGNUS_APP_URL", &cal.url)]).await;
    assert!(ok, "adding an app failed:\n{log}");
    let (_, list) = f.app.api(PERSON).get("/v1/connectors").await;
    let c = &list["connectors"][0];
    assert_eq!((c["name"].as_str(), c["signed_in"].as_bool()), (Some("calendar"), Some(true)), "{list}");
    let risk = |t: &str| c["tools"].as_array().unwrap().iter().find(|x| x["name"] == t).unwrap()["risk"].clone();
    assert_eq!((risk("list_events"), risk("create_event")), (json!("read"), serde_json::Value::Null), "{list}");
}
