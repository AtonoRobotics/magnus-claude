//! Google Drive's contract test (connectors spec, Brokered API
//! connectors: "Every brokered connector runs contract tests in CI against
//! provider test accounts before its manifest is published"): the image the
//! catalog names, built from this checkout, in its own Firecracker microVM,
//! against Drive itself: with no account, that Drive is reached and
//! an unknown token is said as a sign-in to do again; with the owner's
//! Google test account, every tool, once it is given:
//!
//! - `MAGNUS_GOOGLE_TEST_REFRESH_TOKEN`: a refresh token of the test
//!   account's, signed in with Alpha Vector's Google client
//!   (`catalog/registrations.toml`) for the consent screen's scopes;
//! - `MAGNUS_GOOGLE_CLIENT_SECRET`: that client's secret.
//!
//! Without them it says so. It tells no one: its one share is to anyone
//! with the link, taken back at once, of a file that says only that the
//! test wrote it; every file and folder it makes it deletes.

use magnus::apps::{CallError, CallOutput};
use magnus::brokered::Brokered;
use magnus::config;
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    magnus_invariants::appliance::workspace_root()
}

/// An access token for the test account, from Google.
async fn access_token(refresh: &str, secret: &str) -> String {
    let r: toml::Table = std::fs::read_to_string(root().join("catalog/registrations.toml")).unwrap().parse().unwrap();
    let client = r["google"]["client_id"].as_str().unwrap();
    let v: Value = reqwest::Client::new()
        .post("https://oauth2.googleapis.com/token")
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", client), ("client_secret", secret)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["access_token"].as_str().unwrap_or_else(|| panic!("Google refused the test account's refresh token (a Testing app's lasts seven days): {v}")).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn drive_s_tools_do_what_they_say_against_drive() {
    let mf: toml::Table = std::fs::read_to_string(root().join("catalog/google-drive.toml")).unwrap().parse().unwrap();
    let images = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(root().join("brokered/build.sh")).arg("magnus-drive").arg(images.path()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let digest = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert_eq!(mf["endpoint"].as_str(), Some(digest.as_str()), "catalog/google-drive.toml names another image than the one built from this commit");

    let guest = |var: &str, file: &str| std::env::var(var).map(PathBuf::from).unwrap_or_else(|_| root().join("sandbox/out").join(file));
    let sandbox = config::Sandbox {
        firecracker: std::env::var("MAGNUS_FIRECRACKER").unwrap_or("/usr/local/bin/firecracker".into()).into(),
        kernel_image: guest("MAGNUS_GUEST_KERNEL", "vmlinux"),
        rootfs: guest("MAGNUS_GUEST_ROOTFS", "rootfs.ext4"),
        jailer: None,
        vcpus: 1,
        memory_mib: 256,
        workspace_mib: 64,
    };
    let state = tempfile::tempdir().unwrap();
    let cfg = config::Brokered { images: Some(images.path().into()), ..Default::default() };
    let b = Brokered::new(Some(sandbox), cfg, state.path());
    let mut c: config::Connector = toml::from_str(&format!("name = \"google-drive\"\ntransport = \"brokered\"\nurl = \"{digest}\"\naudience = \"public\"\nlabel = \"private+untrusted\"")).unwrap();
    c.hosts = mf["hosts"].as_array().unwrap().iter().map(|h| h.as_str().unwrap().to_string()).collect();
    c.scopes = mf["scopes"].as_array().unwrap().iter().map(|h| h.as_str().unwrap().to_string()).collect();

    // Every tool it lists is classed in the manifest, and the other way round.
    let mut listed: Vec<String> = b.list_tools(&c).await.unwrap().into_iter().map(|t| t.name).collect();
    let mut classed: Vec<String> = mf["tools"].as_table().unwrap().keys().cloned().collect();
    listed.sort();
    classed.sort();
    assert_eq!(listed, classed);

    // Drive itself, reached from the microVM over TLS it checks: a token
    // Google does not know is said as a sign-in to do again.
    let r = b.call(&c, "contract", Some("ya29.not-a-token"), "list_recent_files", &json!({}), &|| Ok(())).await.unwrap();
    assert!(r.is_error && r.text.contains("Google Drive rejected the sign-in"), "{}", r.text);

    let (Ok(refresh), Ok(secret)) = (std::env::var("MAGNUS_GOOGLE_TEST_REFRESH_TOKEN"), std::env::var("MAGNUS_GOOGLE_CLIENT_SECRET")) else {
        eprintln!("the rest of Drive's contract test needs the owner's Google test account (MAGNUS_GOOGLE_TEST_REFRESH_TOKEN, MAGNUS_GOOGLE_CLIENT_SECRET); not given, so it was not run");
        b.stop(None, None).await;
        return;
    };

    let token = access_token(&refresh, &secret).await;
    let call = |tool: &'static str, args: Value| {
        let (b, c, token) = (&b, &c, token.clone());
        async move {
            let r: Result<CallOutput, CallError> = b.call(c, "contract", Some(&token), tool, &args, &|| Ok(())).await;
            let r = r.unwrap_or_else(|e| panic!("{tool}: {e:?}"));
            assert!(!r.is_error, "{tool}: {}", r.text);
            r.structured.unwrap_or_else(|| panic!("{tool}: no structured answer"))
        }
    };

    // A text file: made, found, read, copied, filed in a folder with new
    // text, read again, shared with anyone with the link and taken back,
    // trashed and taken back, then each deleted for good.
    let name = format!("Magnus contract test {}", uuid::Uuid::new_v4());
    let f = call("create_file", json!({ "name": name, "text": "Written by Magnus's contract test." })).await;
    let id = f["file_id"].as_str().unwrap().to_string();
    assert_eq!(call("get_file_metadata", json!({ "file_id": id })).await["name"], name.as_str());
    assert!(call("list_recent_files", json!({ "max_results": 20 })).await["files"].as_array().unwrap().iter().any(|f| f["file_id"] == id.as_str()));
    let mut found = false;
    for _ in 0..10 {
        found = call("search_files", json!({ "query": format!("name = '{name}'") })).await["files"].as_array().unwrap().iter().any(|f| f["file_id"] == id.as_str());
        if found {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    assert!(found, "search_files did not find {name}");
    assert_eq!(call("read_file_content", json!({ "file_id": id })).await["text"], "Written by Magnus's contract test.");
    let copy = call("copy_file", json!({ "file_id": id, "name": format!("{name} copy") })).await;
    let folder = call("create_folder", json!({ "name": name })).await;
    let moved = call("update_file", json!({ "file_id": id, "name": format!("{name} moved"), "text": "Rewritten.", "move_to_folder_id": folder["file_id"] })).await;
    assert_eq!(moved["folder_ids"], json!([folder["file_id"]]), "{moved}");
    assert_eq!(call("read_file_content", json!({ "file_id": id })).await["text"], "Rewritten.");
    let link = call("share_file", json!({ "file_id": id, "anyone_with_link": true, "role": "reader" })).await;
    let perms = call("get_file_permissions", json!({ "file_id": id })).await;
    assert!(perms["permissions"].as_array().unwrap().iter().any(|p| p["type"] == "anyone"), "{perms}");
    call("unshare_file", json!({ "file_id": id, "permission_id": link["permission_id"] })).await;
    assert!(call("trash_file", json!({ "file_id": id })).await["trashed"] == true);
    assert!(call("untrash_file", json!({ "file_id": id })).await["trashed"] == false);
    // A Google document from text, downloaded as PDF.
    let doc = call("create_file", json!({ "name": format!("{name} doc"), "text": "A document.", "convert_to": "document" })).await;
    let pdf = call("download_file_content", json!({ "file_id": doc["file_id"] })).await;
    assert_eq!(pdf["mime_type"], "application/pdf", "{pdf}");
    for gone in [&doc["file_id"], &copy["file_id"], &json!(id), &folder["file_id"]] {
        assert_eq!(call("delete_file", json!({ "file_id": gone })).await["deleted"], true);
    }
    let after = b.call(&c, "contract", Some(&token), "get_file_metadata", &json!({ "file_id": id }), &|| Ok(())).await.unwrap();
    assert!(after.is_error && after.text.contains("could not find"), "{}", after.text);
    b.stop(None, None).await;
}
