//! Release channels of the desktop app (DECISIONS.md, "Release channels"),
//! from publishing to serving: `release/publish.sh` puts a pull request's
//! build in development, master's in test and a tag in production, and
//! never moves a channel backwards; the appliance decides which channel each
//! app gets, never the app: the owner's apps development, testers' test,
//! everyone else's production.

use magnus_invariants::*;
use reqwest::StatusCode;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn release(script: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release").join(script)
}

/// A build's two installers, as CI collects them, in `dist`.
fn build(dist: &Path, version: &str) {
    let _ = std::fs::remove_dir_all(dist);
    std::fs::create_dir_all(dist).unwrap();
    for f in [format!("Magnus_{version}_amd64.AppImage"), format!("Magnus_{version}_x64-setup.exe")] {
        std::fs::write(dist.join(f), format!("the {version} build")).unwrap();
    }
}

/// `release/publish.sh desktop` as CI runs it; its exit code.
fn publish(key: &Path, updates: &Path, tmp: &Path, version: &str, channel: &str) -> i32 {
    let (dist, notes) = (tmp.join("dist"), tmp.join("notes"));
    build(&dist, version);
    std::fs::write(&notes, format!("Build {version}.\n")).unwrap();
    let out = Command::new(release("publish.sh"))
        .args(["desktop", version])
        .args([updates, &notes, &dist])
        .arg(channel)
        .env("MAGNUS_RELEASE_KEY", key)
        .output()
        .unwrap();
    assert!(out.status.code() != Some(1), "publish {version} to {channel}: {}", String::from_utf8_lossy(&out.stderr));
    out.status.code().unwrap()
}

/// What an app at `version` is offered, asking as the apps do: with the
/// session of `who` if signed in (apps from before channels ask without).
async fn offered(app: &Appliance, who: Option<&str>, version: &str) -> (StatusCode, Value) {
    let mut r = reqwest::Client::new().get(format!("{}/v1/updates/desktop/latest.json", app.base)).header("x-magnus-app", version);
    if let Some(w) = who {
        r = r.bearer_auth(app.api(w).token);
    }
    let r = r.send().await.unwrap();
    let st = r.status();
    (st, r.json().await.unwrap_or(Value::Null))
}

async fn offered_version(app: &Appliance, who: Option<&str>, version: &str) -> Option<String> {
    let (st, m) = offered(app, who, version).await;
    match st {
        StatusCode::OK => Some(m["version"].as_str().unwrap().to_string()),
        StatusCode::NO_CONTENT => None,
        st => panic!("{who:?} at {version}: {st} {m}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_gets_development_testers_get_test_and_everyone_else_production() {
    let tmp = tempfile::tempdir().unwrap();
    let key = tmp.path().join("test.key");
    let made = Command::new("cargo").args(["tauri", "signer", "generate", "--ci", "-p", "", "-f", "-w"]).arg(&key).output().unwrap();
    assert!(made.status.success(), "a test release key: {}", String::from_utf8_lossy(&made.stderr));

    let m = FakeModel::start(steps(vec![])).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    let updates = app.dir.path().join("updates");
    let path = app.dir.path().join("magnus.toml");
    let mut cfg: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    cfg.insert("updates".into(), toml::Value::Table(toml::toml! { dir = (updates.display().to_string()) }));
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    app.restart().await;
    let (st, _) = app.api("alice").put("/v1/people/carol/roles", serde_json::json!({ "roles": ["tester"] })).await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Nothing published: nothing offered.
    for who in ["alice", "bob", "carol"] {
        assert_eq!(offered_version(&app, Some(who), "0.7.0").await, None, "{who}");
    }

    // A pull request, master and a tag, each to its channel: the owner's
    // apps get development, the tester's test, everyone else's production.
    let w = tmp.path().join("work");
    assert_eq!(publish(&key, &updates, &w, "0.7.0", "production"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-test.10", "test"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.11.pr.7", "development"), 0);
    let offers = |who| offered_version(&app, Some(who), "0.7.0");
    assert_eq!(offers("alice").await.as_deref(), Some("0.7.1-review.11.pr.7"));
    assert_eq!(offers("carol").await.as_deref(), Some("0.7.1-test.10"));
    assert_eq!(offers("bob").await.as_deref(), Some("0.7.0"));

    // Each later build replaces its channel's; a slower run finishing last
    // does not move a channel back.
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.13.pr.8", "development"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.12.pr.7", "development"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-test.14", "test"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-test.9", "test"), 0);
    assert_eq!(offers("alice").await.as_deref(), Some("0.7.1-review.13.pr.8"));
    assert_eq!(offers("carol").await.as_deref(), Some("0.7.1-test.14"));
    // Development and test keep only their latest build's packages.
    for (c, v) in [("development", "0.7.1-review.13.pr.8"), ("test", "0.7.1-test.14")] {
        let mut kept: Vec<String> = std::fs::read_dir(updates.join("desktop").join(c)).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        kept.sort();
        let want: Vec<String> = [format!("Magnus_{v}_amd64.AppImage"), format!("Magnus_{v}_amd64.AppImage.sig"), format!("Magnus_{v}_x64-setup.exe"), format!("Magnus_{v}_x64-setup.exe.sig"), "latest.json".into()].into();
        assert_eq!(kept, want, "{c}");
    }
    for c in ["stable", "pr-7", "../test"] {
        assert_eq!(publish(&key, &updates, &w, "0.7.1-review.15.pr.7", c), 2, "{c}");
    }

    // A tag newer than a channel's build reaches its apps too: no app is
    // behind production. The channel's next build takes over again.
    assert_eq!(publish(&key, &updates, &w, "0.7.1", "production"), 0);
    for who in ["alice", "carol", "bob"] {
        assert_eq!(offers(who).await.as_deref(), Some("0.7.1"), "{who}");
    }
    assert_eq!(publish(&key, &updates, &w, "0.7.2-review.16.pr.9", "development"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.2-test.17", "test"), 0);
    assert_eq!(offers("alice").await.as_deref(), Some("0.7.2-review.16.pr.9"));
    assert_eq!(offers("carol").await.as_deref(), Some("0.7.2-test.17"));
    assert_eq!(offers("bob").await.as_deref(), Some("0.7.1"));

    // The packages are where the manifest says, signed for their version.
    for (who, files, v) in [("alice", "development/files", "0.7.2-review.16.pr.9"), ("carol", "test/files", "0.7.2-test.17"), ("bob", "files", "0.7.1")] {
        let (_, man) = offered(&app, Some(who), "0.7.0").await;
        let linux = &man["platforms"]["linux-x86_64"];
        let url = linux["url"].as_str().unwrap();
        assert_eq!(url, format!("{}/v1/updates/desktop/{files}/Magnus_{v}_amd64.AppImage", app.base));
        assert_eq!(reqwest::get(url).await.unwrap().text().await.unwrap(), format!("the {v} build"));
        let sig = String::from_utf8(base64_decode(linux["signature"].as_str().unwrap())).unwrap();
        assert!(sig.contains(&format!("version:{v}")), "the signature does not bind the version: {sig}");
    }

    // Apps before channels ask without a session and get the channel their
    // version was built for.
    for (v, offer) in [("0.7.0", "0.7.1"), ("0.7.1-review.107.pr.31", "0.7.2-review.16.pr.9"), ("0.7.1-test.3", "0.7.2-test.17"), ("", "0.7.1")] {
        assert_eq!(offered_version(&app, None, v).await.as_deref(), Some(offer), "an app at {v:?}");
    }

    // No app names its own channel, and nothing outside the channels is served.
    for p in [
        "/v1/updates/desktop/test/latest.json",
        "/v1/updates/desktop/development/latest.json",
        "/v1/updates/desktop/files/latest.json",
        "/v1/updates/desktop/test/files/latest.json",
        "/v1/updates/desktop/pr-9/files/Magnus_0.7.2-review.16.pr.9_amd64.AppImage",
        "/v1/updates/desktop/test/files/..%2Flatest.json",
        "/v1/updates/desktop/..%2F..%2Fappliance/latest.json",
    ] {
        let st = reqwest::get(format!("{}{p}", app.base)).await.unwrap().status();
        assert_eq!(st, StatusCode::NOT_FOUND, "{p}");
    }
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}
