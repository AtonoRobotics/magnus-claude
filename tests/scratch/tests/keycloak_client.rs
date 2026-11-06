//! `config/keycloak-client.py`, which `release/identity-on-appliance.sh` uses
//! to register the appliance in its own Keycloak (#290), run against the real
//! test Keycloak (`dev/keycloak.sh`) on a realm of its own.

use magnus_invariants::appliance::workspace_root;
use magnus_scratch::{admin_login, server_url, Keycloak};

fn script(args: &[&str]) -> std::process::Output {
    std::process::Command::new("python3")
        .arg("-I")
        .arg(workspace_root().join("config/keycloak-client.py"))
        .args(args)
        .output()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_appliance_client_gets_the_device_grant_the_typed_password_and_the_browser_with_pkce() {
    let kc = Keycloak::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (username, password) = admin_login();
    let admin = dir.path().join("admin.json");
    std::fs::write(&admin, serde_json::json!({ "username": username, "password": password }).to_string()).unwrap();
    let url = server_url();
    let out = script(&[
        "--admin-url", &url,
        "--admin-file", admin.to_str().unwrap(),
        "--ca", "/etc/ssl/certs/ca-certificates.crt",
        "--realm", &kc.realm,
        "--client-id", "magnus-appliance-under-test",
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    // DECISIONS.md (2026-10-30): what sign-in needs of the client.
    let c = kc.client("magnus-appliance-under-test").await;
    assert_eq!(c["publicClient"], true, "{c}");
    assert_eq!(c["attributes"]["oauth2.device.authorization.grant.enabled"], "true", "{c}");
    assert_eq!(c["directAccessGrantsEnabled"], true, "a password typed in the app: {c}");
    assert_eq!(c["standardFlowEnabled"], true, "Keycloak's page in the browser: {c}");
    assert_eq!(c["redirectUris"], serde_json::json!(["http://127.0.0.1/*"]), "{c}");
    assert_eq!(c["attributes"]["pkce.code.challenge.method"], "S256", "{c}");
}

#[test]
fn it_names_no_keycloak_of_its_own() {
    // Its old defaults pointed at a retired deployment's files; where Keycloak
    // is and who administers it now come from the caller (no hardcoded value).
    let out = script(&["--realm", "magnus"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    for flag in ["--admin-url", "--admin-file", "--ca"] {
        assert!(err.contains(flag), "{flag} must be required: {err}");
    }
}
