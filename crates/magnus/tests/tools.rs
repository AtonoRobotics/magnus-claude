//! Integration tests for the browser sidecar and the sandbox desktop, using
//! real Chromium and a real Firecracker VM.

use magnus::{boundary::computer_command, browser::Browser, config, sandbox::Sandboxes};
use serde_json::json;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

async fn page_server() -> String {
    use axum::{response::Html, routing::get, Router};
    let app = Router::new()
        .route("/", get(|| async { Html("<html><head><title>Shop</title></head><body><h1>Welcome</h1><a href='/next'>Go next</a><input placeholder='Your name'></body></html>") }))
        .route("/next", get(|| async { Html("<html><body><p>Second page</p></body></html>") }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    url
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_navigates_clicks_types_and_screenshots() {
    let chromium = std::env::var("MAGNUS_CHROMIUM").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/ms-playwright/chromium-1234/chrome-linux64/chrome")
    });
    let dir = tempfile::tempdir().unwrap();
    let b = Browser::new(
        Some(config::Browser { command: vec!["node".into(), root().join("browser/sidecar.js").to_string_lossy().into()], chromium }),
        dir.path(),
    );
    let url = page_server().await;
    let v = b.request("r1", json!({ "op": "navigate", "url": url })).await.unwrap();
    let snap = v["text"].as_str().unwrap();
    assert!(snap.contains("Title: Shop") && snap.contains("Welcome"), "{snap}");
    assert!(snap.contains("[e1] link \"Go next\""), "{snap}");
    let v = b.request("r1", json!({ "op": "type", "target": "e2", "text": "Dana" })).await.unwrap();
    assert!(v["text"].as_str().unwrap().contains("= \"Dana\""), "{}", v["text"]);
    let v = b.request("r1", json!({ "op": "click", "target": "Go next" })).await.unwrap();
    assert!(v["text"].as_str().unwrap().contains("Second page"), "{}", v["text"]);
    let v = b.request("r1", json!({ "op": "screenshot" })).await.unwrap();
    assert!(v["image_b64"].as_str().unwrap().starts_with("iVBOR"), "not a PNG");
    // Runs are isolated: a second run starts blank.
    let v = b.request("r2", json!({ "op": "snapshot" })).await.unwrap();
    assert!(v["text"].as_str().unwrap().contains("about:blank"));
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_cannot_read_the_appliances_files_or_execute_a_navigation_script() {
    let chromium = std::env::var("MAGNUS_CHROMIUM").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/ms-playwright/chromium-1234/chrome-linux64/chrome")
    });
    let dir = tempfile::tempdir().unwrap();
    let canary = dir.path().join("private.txt");
    std::fs::write(&canary, "PRIVATE-HOST-FILE-CANARY").unwrap();
    let b = Browser::new(
        Some(config::Browser { command: vec!["node".into(), root().join("browser/sidecar.js").to_string_lossy().into()], chromium }),
        dir.path(),
    );
    let url = page_server().await;
    b.request("r1", json!({ "op": "navigate", "url": url })).await.unwrap();
    for forbidden in [
        url::Url::from_file_path(&canary).unwrap().to_string(),
        "javascript:document.body.textContent='SCRIPT-RAN'".into(),
        "data:text/html,INLINE-DOCUMENT".into(),
        "chrome://version".into(),
    ] {
        let result = b.request("r1", json!({ "op": "navigate", "url": forbidden })).await;
        assert!(result.is_err(), "the browser accepted {forbidden}: {result:?}");
    }
    let v = b.request("r1", json!({ "op": "snapshot" })).await.unwrap();
    let text = v["text"].as_str().unwrap();
    assert!(text.contains("Welcome") && !text.contains("PRIVATE-HOST-FILE-CANARY") && !text.contains("SCRIPT-RAN"), "{text}");
    b.close("r1").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sandbox_desktop_takes_screenshots_and_types() {
    let out = root().join("sandbox/out");
    let dir = tempfile::tempdir().unwrap();
    let s = Sandboxes::new(
        Some(config::Sandbox {
            firecracker: "/usr/local/bin/firecracker".into(),
            kernel_image: out.join("vmlinux"),
            rootfs: out.join("rootfs.ext4"),
            jailer: None,
            vcpus: 2,
            memory_mib: 1024,
            workspace_mib: 512,
        }),
        dir.path(),
    );
    let cmd = computer_command("computer_screenshot", &json!({})).unwrap();
    let v = s.exec("r1", &cmd, 60).await.unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    let png = s.read("r1", "/tmp/magnus-shot.png").await.unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    let v = s.exec("r1", "export DISPLAY=:0; (xterm &); sleep 1.5", 30).await.unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    let cmd = computer_command("computer_type", &json!({ "text": "echo hi > /workspace/typed.txt\n" })).unwrap();
    let v = s.exec("r1", &cmd, 60).await.unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let typed = s.read("r1", "/workspace/typed.txt").await.unwrap();
    assert_eq!(String::from_utf8_lossy(&typed).trim(), "hi");
    s.destroy("r1").await;
}
