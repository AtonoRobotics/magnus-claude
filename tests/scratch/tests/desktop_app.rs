//! The real desktop app on Linux, driven through tauri-driver against a real
//! appliance over HTTPS (`desktop/tests/app.mjs`): pin the certificate, sign
//! in through the identity provider, connect Claude, type a task, approve in
//! the inbox, see the outcome with its evidence, download the file.
//!
//! Needs the built app (`cargo tauri build` in desktop/src-tauri, or
//! MAGNUS_DESKTOP_APP), `tauri-driver`, `WebKitWebDriver`
//! (MAGNUS_WEBKIT_DRIVER) and `xvfb-run`. Run with `--ignored`.

use magnus_invariants::appliance::workspace_root;
use magnus_invariants::{canary, eventually, PATIENCE};
use magnus_scratch::*;
use std::path::PathBuf;
use std::process::Stdio;

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the built desktop app, tauri-driver, WebKitWebDriver and xvfb-run"]
async fn the_linux_app_pins_signs_in_through_the_provider_approves_and_shows_evidence() {
    let app_bin = std::env::var("MAGNUS_DESKTOP_APP").map(PathBuf::from).unwrap_or_else(|_| workspace_root().join("desktop/src-tauri/target/release/magnus-desktop"));
    assert!(app_bin.is_file(), "build the app first: {}", app_bin.display());
    let native = std::env::var("MAGNUS_WEBKIT_DRIVER").map(PathBuf::from).ok().or_else(|| which("WebKitWebDriver")).expect("WebKitWebDriver (set MAGNUS_WEBKIT_DRIVER)");
    let driver = which("tauri-driver").or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo/bin/tauri-driver"))).expect("tauri-driver");

    let mut f = Flow::start().await;
    let idp = FakeIdp::start(PERSON).await;
    idp.configure(&mut f.app).await;
    let (url, fingerprint) = f.with_tls().await;

    // The app runs as a fresh user: its own home, settings and Downloads,
    // and no keyring, so the session goes to the private file.
    let home = tempfile::tempdir().unwrap();
    let (config, downloads) = (home.path().join(".config"), home.path().join("Downloads"));
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(config.join("user-dirs.dirs"), format!("XDG_DOWNLOAD_DIR=\"{}\"\n", downloads.display())).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let native_port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut td = tokio::process::Command::new("xvfb-run")
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

    let key = canary("anthropic-key");
    let out = tokio::process::Command::new("node")
        .arg(workspace_root().join("desktop/tests/app.mjs"))
        .env("MAGNUS_WEBDRIVER", &wd)
        .env("MAGNUS_APP", &app_bin)
        .env("MAGNUS_URL", &url)
        .env("MAGNUS_FINGERPRINT", &fingerprint)
        .env("MAGNUS_PERSON", PERSON)
        .env("MAGNUS_PROVIDER", IDP_LABEL)
        .env("MAGNUS_API_KEY", &key)
        .env("MAGNUS_TASK", TASK)
        .env("MAGNUS_DOWNLOADS", &downloads)
        .envs(std::env::var("MAGNUS_SHOTS").ok().map(|s| ("MAGNUS_SHOTS", s)))
        .output()
        .await
        .unwrap();
    let _ = td.start_kill();
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the app flow failed:\n{log}\n{}", f.report().await);
    f.check_outcome(&key).await;

    // The app kept the session in a file only this user can read, and the
    // pinned fingerprint beside the address; nothing else secret.
    let dir = config.join("magnus");
    let session = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).find(|e| e.file_name().to_string_lossy().starts_with("session-")).expect("a session file");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(session.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    let settings = std::fs::read_to_string(dir.join("app.json")).unwrap();
    assert!(settings.contains(&fingerprint), "{settings}");
    assert!(!settings.contains("mgs_") && !settings.contains(&key), "a secret reached the settings: {settings}");
}
