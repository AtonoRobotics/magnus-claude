//! Release channels of the desktop app (DECISIONS.md, "Release channels"),
//! from publishing to serving: `release/publish.sh` puts each tested build
//! in its channel (stable, preview, `pr-<n>`) and never moves a channel
//! backwards; the appliance decides which channel each app gets, never the
//! app: the owner's apps get the channel the owner chose, everyone else's
//! get stable, and a channel with no build (a closed pull request's) hands
//! its apps to the next steadier one; `release/retire.sh` removes pull
//! requests' channels; only the owner sees and chooses channels, and only
//! the owner releases a build to testers, whose apps get the test channel.

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

fn retire(updates: &Path, args: &[&str]) {
    let out = Command::new(release("retire.sh")).arg(updates).args(args).output().unwrap();
    assert!(out.status.success(), "retire {args:?}: {}", String::from_utf8_lossy(&out.stderr));
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

/// The owner chooses the channel their apps update from.
async fn choose(app: &Appliance, channel: &str) {
    let (st, v) = app.api("alice").put("/v1/updates/channel", serde_json::json!({ "channel": channel })).await;
    assert_eq!(st, StatusCode::OK, "choose {channel}: {v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_decides_each_apps_channel_and_testers_get_only_what_the_owner_releases() {
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

    // Nothing published: nothing offered.
    assert_eq!(offered_version(&app, Some("alice"), "0.7.0").await, None);
    assert_eq!(offered_version(&app, None, "0.7.1-review.11.pr.7").await, None);

    // A tag, master and a pull request, each to its channel.
    let w = tmp.path().join("work");
    assert_eq!(publish(&key, &updates, &w, "0.7.0", "stable"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-preview.10", "preview"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.11.pr.7", "pr-7"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.12.pr.7", "pr-7"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.13.pr.8", "pr-8"), 0);
    // Until the owner chooses, their apps get the channel their version was
    // built for; then whatever they choose. Everyone else's get stable.
    assert_eq!(offered_version(&app, Some("alice"), "0.7.1-review.6.pr.8").await.as_deref(), Some("0.7.1-review.13.pr.8"));
    for (c, v) in [("stable", "0.7.0"), ("preview", "0.7.1-preview.10"), ("pr-7", "0.7.1-review.12.pr.7"), ("pr-8", "0.7.1-review.13.pr.8")] {
        choose(&app, c).await;
        assert_eq!(offered_version(&app, Some("alice"), "0.7.0").await.as_deref(), Some(v), "{c}");
        assert_eq!(offered_version(&app, Some("bob"), "0.7.1-review.12.pr.7").await.as_deref(), Some("0.7.0"), "bob, with the owner on {c}");
    }

    // A slower run of pull request 7 finishing last does not move it back.
    choose(&app, "pr-7").await;
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.9.pr.7", "pr-7"), 0);
    assert_eq!(offered_version(&app, Some("alice"), "0.7.0").await.as_deref(), Some("0.7.1-review.12.pr.7"));
    // A pull request's channel keeps only its latest build's packages.
    let mut kept: Vec<String> = std::fs::read_dir(updates.join("desktop/pr-7")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    kept.sort();
    assert_eq!(
        kept,
        ["Magnus_0.7.1-review.12.pr.7_amd64.AppImage", "Magnus_0.7.1-review.12.pr.7_amd64.AppImage.sig", "Magnus_0.7.1-review.12.pr.7_x64-setup.exe", "Magnus_0.7.1-review.12.pr.7_x64-setup.exe.sig", "latest.json"]
    );
    // No such channel.
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.14.pr.x", "pr-x"), 2);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.14.pr.9", "../pr-9"), 2);

    // The packages are where the manifest says, signed for their version.
    let (_, man) = offered(&app, Some("alice"), "0.7.0").await;
    let linux = &man["platforms"]["linux-x86_64"];
    let url = linux["url"].as_str().unwrap();
    assert_eq!(url, format!("{}/v1/updates/desktop/pr-7/files/Magnus_0.7.1-review.12.pr.7_amd64.AppImage", app.base));
    let body = reqwest::get(url).await.unwrap().text().await.unwrap();
    assert_eq!(body, "the 0.7.1-review.12.pr.7 build");
    let sig = String::from_utf8(base64_decode(linux["signature"].as_str().unwrap())).unwrap();
    assert!(sig.contains("version:0.7.1-review.12.pr.7"), "the signature does not bind the version: {sig}");
    // Stable's packages are also where apps before channels were told.
    let old = reqwest::get(format!("{}/v1/updates/desktop/files/Magnus_0.7.0_amd64.AppImage", app.base)).await.unwrap();
    assert_eq!(old.text().await.unwrap(), "the 0.7.0 build");

    // Apps before channels name only their version: each gets the channel
    // its version was built for.
    for (v, offer) in [("0.7.0", "0.7.0"), ("0.7.1-preview.3", "0.7.1-preview.10"), ("0.7.1-pr.7.5", "0.7.1-review.12.pr.7"), ("0.7.1-review.6.pr.8", "0.7.1-review.13.pr.8"), ("", "0.7.0")] {
        assert_eq!(offered_version(&app, None, v).await.as_deref(), Some(offer), "an app at {v:?}");
    }

    // Only the owner sees the channels, steadiest first, and chooses.
    let (st, list) = app.api("alice").get("/v1/updates/channels").await;
    assert_eq!(st, StatusCode::OK, "{list}");
    let names: Vec<&str> = list["channels"].as_array().unwrap().iter().map(|c| c["channel"].as_str().unwrap()).collect();
    assert_eq!(names, ["stable", "preview", "pr-7", "pr-8"]);
    assert_eq!(list["channels"][2]["version"], "0.7.1-review.12.pr.7");
    assert_eq!((list["chosen"].as_str(), list["served"].as_str()), (Some("pr-7"), Some("pr-7")), "{list}");
    assert_eq!(app.api("bob").get("/v1/updates/channels").await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.api("bob").put("/v1/updates/channel", serde_json::json!({ "channel": "pr-7" })).await.0, StatusCode::FORBIDDEN);
    assert_eq!(offered_version(&app, Some("bob"), "0.7.0").await.as_deref(), Some("0.7.0"), "bob is still on stable");
    for bad in ["pr-x", "files", "../stable", ""] {
        assert_eq!(app.api("alice").put("/v1/updates/channel", serde_json::json!({ "channel": bad })).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }

    // Testers get the test channel, which has a build only once the owner
    // releases one to them: the same signed packages, from the channel named.
    let (st, _) = app.api("alice").put("/v1/people/carol/roles", serde_json::json!({ "roles": ["tester"] })).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(offered_version(&app, Some("carol"), "0.7.0").await.as_deref(), Some("0.7.0"), "no test release yet: stable");
    for who in ["bob", "carol"] {
        assert_eq!(app.api(who).post("/v1/updates/test", serde_json::json!({ "from": "pr-7" })).await.0, StatusCode::FORBIDDEN, "{who}");
    }
    assert_eq!(app.api("alice").post("/v1/updates/test", serde_json::json!({ "from": "test" })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(app.api("alice").post("/v1/updates/test", serde_json::json!({ "from": "pr-99" })).await.0, StatusCode::NOT_FOUND);
    let (st, v) = app.api("alice").post("/v1/updates/test", serde_json::json!({ "from": "pr-7" })).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, released) = offered(&app, Some("carol"), "0.7.0").await;
    assert_eq!(released["version"], "0.7.1-review.12.pr.7");
    assert_eq!(released["platforms"], {
        let mut m = man["platforms"].clone();
        for p in m.as_object_mut().unwrap().values_mut() {
            p["url"] = serde_json::json!(p["url"].as_str().unwrap().replace("/pr-7/", "/test/"));
        }
        m
    });
    let body = reqwest::get(released["platforms"]["linux-x86_64"]["url"].as_str().unwrap()).await.unwrap().text().await.unwrap();
    assert_eq!(body, "the 0.7.1-review.12.pr.7 build");
    assert_eq!(offered_version(&app, Some("bob"), "0.7.0").await.as_deref(), Some("0.7.0"), "bob is not a tester");
    let mut kept: Vec<String> = std::fs::read_dir(updates.join("desktop/test")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    kept.sort();
    assert_eq!(kept, ["Magnus_0.7.1-review.12.pr.7_amd64.AppImage", "Magnus_0.7.1-review.12.pr.7_x64-setup.exe", "latest.json"]);
    // The next release to testers replaces it; the owner may follow test too.
    let (st, _) = app.api("alice").post("/v1/updates/test", serde_json::json!({ "from": "preview" })).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(offered_version(&app, Some("carol"), "0.7.1-review.12.pr.7").await.as_deref(), Some("0.7.1-preview.10"));
    assert!(!updates.join("desktop/test/Magnus_0.7.1-review.12.pr.7_amd64.AppImage").exists(), "the earlier test release's packages stay");
    choose(&app, "test").await;
    assert_eq!(offered_version(&app, Some("alice"), "0.7.0").await.as_deref(), Some("0.7.1-preview.10"));
    let db = app.db.admin().await;
    let approved: i64 = db.query_one("SELECT count(*) FROM public.events WHERE kind = 'test_release_approved' AND actor = 'person:alice'", &[]).await.unwrap().get(0);
    assert_eq!(approved, 2, "each release to testers is in the log, with who approved it");

    // Pull request 8 closes without merging; master's next build retires
    // every channel whose pull request is not open (7 is merged into it).
    choose(&app, "pr-8").await;
    retire(&updates, &["pr-8"]);
    assert_eq!(offered_version(&app, Some("alice"), "0.7.1-review.13.pr.8").await.as_deref(), Some("0.7.1-preview.10"), "a closed pull request's apps move to preview");
    let (_, list) = app.api("alice").get("/v1/updates/channels").await;
    assert_eq!((list["follows"].as_str(), list["served"].as_str()), (Some("pr-8"), Some("preview")), "{list}");
    assert_eq!(publish(&key, &updates, &w, "0.7.1-preview.15", "preview"), 0);
    assert_eq!(publish(&key, &updates, &w, "0.7.1-review.16.pr.9", "pr-9"), 0);
    retire(&updates, &["--except", "pr-9"]);
    let (_, list) = app.api("alice").get("/v1/updates/channels").await;
    let names: Vec<&str> = list["channels"].as_array().unwrap().iter().map(|c| c["channel"].as_str().unwrap()).collect();
    assert_eq!(names, ["stable", "test", "preview", "pr-9"]);
    choose(&app, "pr-7").await;
    assert_eq!(offered_version(&app, Some("alice"), "0.7.1-review.12.pr.7").await.as_deref(), Some("0.7.1-preview.15"));
    retire(&updates, &["pr-7"]);

    // A test release outlives the channel it came from.
    assert_eq!(offered_version(&app, Some("carol"), "0.7.0").await.as_deref(), Some("0.7.1-preview.10"));

    // With no preview either, a pull request's apps fall to stable.
    std::fs::remove_dir_all(updates.join("desktop/preview")).unwrap();
    assert_eq!(offered_version(&app, Some("alice"), "0.7.1-review.12.pr.7").await.as_deref(), Some("0.7.0"));

    // No app names its own channel, and nothing outside the channels is served.
    for p in [
        "/v1/updates/desktop/pr-9/latest.json",
        "/v1/updates/desktop/stable/latest.json",
        "/v1/updates/desktop/files/latest.json",
        "/v1/updates/desktop/stable/files/latest.json",
        "/v1/updates/desktop/pr-9/files/..%2Flatest.json",
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
