//! Release channels of the desktop app (DECISIONS.md, "Release channels"),
//! from publishing to serving: `release/publish.sh` puts a pull request's
//! build in development, master's in test and a tag in production, and
//! never moves a channel backwards, in version or in content (a build
//! without the work of the one before is refused); the appliance decides
//! which channel each app gets, never the app: the owner's apps
//! development, testers' test, everyone else's production.

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

/// `release/publish.sh desktop` as CI runs it, in this repository's
/// checkout; its exit code.
fn publish(key: &Path, updates: &Path, tmp: &Path, version: &str, channel: &str) -> i32 {
    publish_from(key, updates, tmp, version, channel, Path::new(env!("CARGO_MANIFEST_DIR")), None)
}

/// The same, built from the checkout `repo`; `head` is a pull request's
/// head commit when the checkout is its merge with master.
fn publish_from(key: &Path, updates: &Path, tmp: &Path, version: &str, channel: &str, repo: &Path, head: Option<&str>) -> i32 {
    let (dist, notes) = (tmp.join("dist"), tmp.join("notes"));
    build(&dist, version);
    std::fs::write(&notes, format!("Build {version}.\n")).unwrap();
    let out = Command::new(release("publish.sh"))
        .args(["desktop", version])
        .args([updates, &notes, &dist])
        .arg(channel)
        .env("MAGNUS_RELEASE_KEY", key)
        .env_remove("MAGNUS_COMMIT")
        .envs(head.map(|h| ("MAGNUS_COMMIT", h)))
        .current_dir(repo)
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

    // A tag does not take an app off its channel's build, newer or not: it
    // need not hold that build's work (a pull request not yet merged). The
    // channel's next build moves it forward.
    assert_eq!(publish(&key, &updates, &w, "0.7.1", "production"), 0);
    assert_eq!(offers("alice").await.as_deref(), Some("0.7.1-review.13.pr.8"));
    assert_eq!(offers("carol").await.as_deref(), Some("0.7.1-test.14"));
    assert_eq!(offers("bob").await.as_deref(), Some("0.7.1"));
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

/// git in `repo`; its output.
fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@example.com"]).args(args).current_dir(repo).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A commit on the current branch of `repo`; its id.
fn commit(repo: &Path, file: &str) -> String {
    std::fs::write(repo.join(file), file).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-qm", file]);
    git(repo, &["rev-parse", "HEAD"])
}

/// What the owner saw (2026-10-31): their app ran pull request A's build,
/// then got pull request B's, numbered later but built without A's work.
/// A channel's next build must hold the work of the one there; one that
/// does not is refused, whatever its version.
#[test]
fn a_build_without_the_work_already_in_its_channel_is_never_published() {
    let tmp = tempfile::tempdir().unwrap();
    let key = tmp.path().join("test.key");
    let made = Command::new("cargo").args(["tauri", "signer", "generate", "--ci", "-p", "", "-f", "-w"]).arg(&key).output().unwrap();
    assert!(made.status.success(), "a test release key: {}", String::from_utf8_lossy(&made.stderr));
    let (updates, w, repo) = (tmp.path().join("updates"), tmp.path().join("work"), tmp.path().join("repo"));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "master"]);
    let base = commit(&repo, "base");
    git(&repo, &["checkout", "-q", "-b", "a"]);
    let a = commit(&repo, "a");
    git(&repo, &["checkout", "-q", "-b", "b", &base]);
    let b = commit(&repo, "b");
    let channel = |c: &str| -> (String, String) {
        let dir = if c == "production" { updates.join("desktop") } else { updates.join("desktop").join(c) };
        let m: Value = serde_json::from_slice(&std::fs::read(dir.join("latest.json")).unwrap()).unwrap();
        (m["version"].as_str().unwrap().into(), m["commit"].as_str().unwrap().into())
    };

    // A's build is in development; B's, later but without A's work, is not.
    git(&repo, &["checkout", "-q", &a]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.1-review.20.pr.1", "development", &repo, None), 0);
    git(&repo, &["checkout", "-q", &b]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.1-review.21.pr.2", "development", &repo, None), 3);
    assert_eq!(channel("development"), ("0.7.1-review.20.pr.1".into(), a.clone()));

    // A merges. B, built as CI builds a pull request (its merge with
    // master), holds A's work and goes out, recorded by its own head.
    git(&repo, &["checkout", "-q", "master"]);
    git(&repo, &["merge", "-q", "--no-ff", "--no-edit", "a"]);
    let merged = git(&repo, &["rev-parse", "HEAD"]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.1-test.22", "test", &repo, None), 0);
    git(&repo, &["checkout", "-q", "--detach", "master"]);
    git(&repo, &["merge", "-q", "--no-ff", "--no-edit", "b"]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.1-review.23.pr.2", "development", &repo, Some(&b)), 0);
    assert_eq!(channel("development"), ("0.7.1-review.23.pr.2".into(), b.clone()));

    // A's old branch, rebuilt later, lacks B: refused. So is a test build
    // or a tag of a commit older than the one its channel holds.
    git(&repo, &["checkout", "-q", &a]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.1-review.24.pr.1", "development", &repo, None), 3);
    git(&repo, &["checkout", "-q", &base]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.1-test.25", "test", &repo, None), 3);
    assert_eq!(channel("test"), ("0.7.1-test.22".into(), merged.clone()));
    git(&repo, &["checkout", "-q", &merged]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.1", "production", &repo, None), 0);
    git(&repo, &["checkout", "-q", &base]);
    assert_eq!(publish_from(&key, &updates, &w, "0.7.2", "production", &repo, None), 3);
    assert_eq!(channel("production"), ("0.7.1".into(), merged));
    assert_eq!(channel("development"), ("0.7.1-review.23.pr.2".into(), b));
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}
