//! What keeps the shared CI runner moving (owner, 2026-11-04: "Why does
//! every test need to run on every PR? Most of them are small changes"):
//! a pull request runs only the tests its changed paths reach
//! (`release/ci-select.sh`), every test has a time limit
//! (`.config/nextest.toml`), a browser sign-in that never comes back fails
//! its test in three minutes, and what killed tests leave on the test
//! Keycloak is cleared (`release/ci-clean.sh`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// What `release/ci-select.sh` prints for these changed paths.
fn select(paths: &[&str]) -> String {
    let mut child = Command::new(repo().join("release/ci-select.sh")).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(paths.join("\n").as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn a_pull_request_runs_only_the_tests_its_changes_reach() {
    // Documents: no tests, and the check still passes.
    assert_eq!(select(&["README.md", "docs/a.md", "DECISIONS.md"]), "MAGNUS_SELECT=none\n");
    assert_eq!(select(&[]), "MAGNUS_SELECT=none\n");
    // One test file: that test, without the installers.
    assert_eq!(select(&["tests/scratch/tests/chat.rs", "PROGRESS.md"]), "MAGNUS_SELECT=tests\nMAGNUS_FILTER=binary_id(=magnus-scratch::chat)\n");
    // A deleted test file: the scratch tests (a filter naming it would fail).
    assert_eq!(select(&["tests/scratch/tests/gone_now.rs"]), "MAGNUS_SELECT=tests\nMAGNUS_FILTER=package(=magnus-scratch)\n");
    // The rules script: its test.
    assert_eq!(select(&["release/pr-rules.sh"]), "MAGNUS_SELECT=tests\nMAGNUS_FILTER=binary_id(=magnus-scratch::pr_rules)\n");
    // A connector: its crate and its scratch tests.
    let gmail = select(&["brokered/gmail/src/lib.rs"]);
    assert!(gmail.contains("MAGNUS_SELECT=tests") && gmail.contains("package(=magnus-gmail)") && gmail.contains("brokered|gmail"), "{gmail}");
    // The kernel: every suite, still without the installers.
    assert_eq!(select(&["crates/magnus/src/lib.rs", "tests/scratch/tests/chat.rs"]), "MAGNUS_SELECT=tests\nMAGNUS_FILTER=all()\n");
    // The app, the build, the workflow, the installed-app tests, or a path
    // the selection does not know: everything, installers included.
    for p in ["desktop/ui/index.html", "Cargo.lock", ".github/workflows/desktop.yml", ".config/nextest.toml", "tests/scratch/tests/desktop_app.rs", "sandbox/image.sh", "release/publish.sh", "somewhere-new/x"] {
        assert_eq!(select(&[p, "README.md"]), "MAGNUS_SELECT=full\n", "{p}");
    }
}

/// The terms of the filter `select` printed (none for no tests).
fn terms(selected: &str) -> Vec<String> {
    selected.lines().find_map(|l| l.strip_prefix("MAGNUS_FILTER=")).map(|f| f.split(" | ").map(String::from).collect()).unwrap_or_default()
}

/// Every Markdown file a tracked source compiles in with `include_str!`,
/// with the source that names it, both as paths from the root.
fn compiled_markdown() -> Vec<(String, String)> {
    let root = repo().canonicalize().unwrap();
    let out = Command::new("git").arg("-C").arg(&root).args(["ls-files", "*.rs"]).output().unwrap();
    assert!(out.status.success());
    let mut found = Vec::new();
    for src in String::from_utf8(out.stdout).unwrap().lines() {
        let Ok(text) = std::fs::read_to_string(root.join(src)) else { continue };
        for part in text.split("include_str!(\"").skip(1) {
            let Some(arg) = part.split('"').next().filter(|a| a.ends_with(".md")) else { continue };
            let Ok(md) = root.join(src).parent().unwrap().join(arg).canonicalize() else { continue };
            found.push((md.strip_prefix(&root).unwrap().to_string_lossy().into_owned(), src.to_string()));
        }
    }
    found
}

/// The agent's and the chat's instructions are compiled into the kernel
/// (`run/mod.rs`), so a change to one reaches what a change to the kernel
/// does; the selection took them for documents and ran nothing (#190).
#[test]
fn a_compiled_prompt_runs_the_tests_of_the_code_that_compiles_it() {
    let compiled = compiled_markdown();
    for prompt in ["crates/magnus/instructions/agent.md", "crates/magnus/instructions/chat.md", "crates/magnus/instructions/character.md"] {
        assert!(compiled.iter().any(|(md, _)| md == prompt), "{prompt} is not found compiled in: {compiled:?}");
    }
    for (md, src) in &compiled {
        assert_eq!(select(&[md.as_str(), "README.md"]), select(&[src.as_str()]), "{md}, compiled in by {src}");
    }
}

/// The Google connectors share one client (`brokered/google`), so a change
/// to it runs every connector whose Cargo.toml depends on it, with that
/// connector's own tests; Google Chat's were left out (#190).
#[test]
fn a_change_to_the_shared_google_client_runs_every_connector_built_on_it() {
    let shared = terms(&select(&["brokered/google/src/lib.rs"]));
    let mut consumers = Vec::new();
    for dir in std::fs::read_dir(repo().join("brokered")).unwrap() {
        let dir = dir.unwrap().path();
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else { continue };
        if manifest.lines().any(|l| l.trim_start().starts_with("magnus-brokered-google ") || l.trim_start().starts_with("magnus-brokered-google=")) {
            consumers.push(dir.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    consumers.sort();
    assert!(consumers.contains(&"chat".to_string()) && consumers.contains(&"calendar".to_string()), "{consumers:?}");
    let missing: Vec<String> = consumers.iter().flat_map(|c| terms(&select(&[&format!("brokered/{c}/src/main.rs")]))).filter(|t| !shared.contains(t)).collect();
    assert!(missing.is_empty(), "a change to the shared Google client does not run {missing:?}; it runs {shared:?}");
}

#[test]
fn every_test_has_a_time_limit() {
    let config: toml::Value = toml::from_str(&std::fs::read_to_string(repo().join(".config/nextest.toml")).unwrap()).unwrap();
    let limit = |t: &toml::Value| -> u64 {
        let period: u64 = t["period"].as_str().unwrap().trim_end_matches('s').parse().unwrap();
        period * t["terminate-after"].as_integer().expect("a slow test is killed, not only reported") as u64
    };
    for profile in ["default", "ci"] {
        assert_eq!(limit(&config["profile"][profile]["slow-timeout"]), 300, "{profile}");
    }
    for o in config["profile"]["default"]["overrides"].as_array().unwrap() {
        assert!(limit(&o["slow-timeout"]) <= 600, "{o}");
    }
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    assert!(workflow.contains("cargo nextest run --profile ci"));
}

#[tokio::test]
#[should_panic(expected = "did not come back to the app's loopback address")]
async fn a_browser_that_never_comes_back_fails_its_test_and_does_not_hold_the_run() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    magnus_scratch::loopback_within(l, std::time::Duration::from_secs(1)).await;
}

fn decide(seen: &Path, now: u64, realms: &[&str]) -> Vec<String> {
    let mut child = Command::new(repo().join("release/ci-clean.sh"))
        .args(["--decide", seen.to_str().unwrap(), &now.to_string()])
        .env_remove("MAGNUS_CLEAN_AGE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(realms.join("\n").as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().lines().map(String::from).collect()
}

#[test]
fn only_test_realms_left_for_three_hours_are_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let seen = dir.path().join("seen");
    let (old, new) = ("t0123456789abcdef0123456789abcdef", "tfedcba9876543210fedcba9876543210");
    // First seen: nothing is deleted, whatever its name.
    assert!(decide(&seen, 1_000, &["master", "magnus", "test", old]).is_empty());
    // Under three hours later: still kept.
    assert!(decide(&seen, 1_000 + 10_799, &[old, new]).is_empty());
    // Three hours after it was first seen: the old test realm only; never
    // master or a realm that is not a test's.
    assert_eq!(decide(&seen, 1_000 + 10_800, &["master", "test", old, new]), vec![old.to_string()]);
    // A realm its test deleted is forgotten.
    assert!(decide(&seen, 1_000 + 20_000, &[]).is_empty());
    assert!(decide(&seen, 1_000 + 20_001, &[new]).is_empty());
}

/// An AppImage stopped mid-run leaves its extraction, about 260 MB, and 296
/// of them filled the build box with the Docker build cache (#203). Those
/// unpacked three hours ago or more go, unless a process still runs from one;
/// newer ones and anything else in the directory stay.
#[test]
fn appimage_extractions_left_for_three_hours_are_removed() {
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now();
    let make = |name: &str| {
        let d = dir.path().join(name);
        std::fs::create_dir_all(d.join("usr/bin")).unwrap();
        std::fs::write(d.join("AppRun"), "").unwrap();
        d
    };
    let unpacked = |d: &Path, hours_ago: u64| std::fs::File::open(d).unwrap().set_modified(now - std::time::Duration::from_secs(hours_ago * 3600)).unwrap();
    let old = make("appimage_extracted_0123456789abcdef0123456789abcdef");
    let new = make("appimage_extracted_fedcba9876543210fedcba9876543210");
    let other = make("appimage_extracted_by_someone");
    let running = make("appimage_extracted_00000000000000000000000000000000");
    std::fs::copy("/bin/sleep", running.join("usr/bin/sleep")).unwrap();
    for (d, hours) in [(&old, 4), (&new, 2), (&other, 4), (&running, 4)] {
        unpacked(d, hours);
    }
    let mut app = Command::new(running.join("usr/bin/sleep")).arg("60").spawn().unwrap();
    let secs = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let out = Command::new(repo().join("release/ci-clean.sh")).args(["--extractions", dir.path().to_str().unwrap(), &secs.to_string()]).env_remove("MAGNUS_CLEAN_AGE").output().unwrap();
    let _ = app.kill();
    let _ = app.wait();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // Only the one four hours old that nothing runs from.
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{}\n", old.display()));
    let script = std::fs::read_to_string(repo().join("release/ci-clean.sh")).unwrap();
    assert!(script.contains("for d in $(extractions_due /tmp \"$now\"); do\n    rm -rf \"$d\""), "the run removes what the decision names in /tmp");
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    assert!(workflow.contains("run: release/ci-clean.sh"), "release-checks clears what earlier runs left");
}

/// The build cache goes once no build has used it for three hours; images
/// stay, since another run may be using one (#203).
#[test]
fn docker_build_cache_unused_for_three_hours_is_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.path().join("docker-calls");
    let docker = bin.join("docker");
    std::fs::write(&docker, "#!/bin/sh\necho \"$*\" >> \"$DOCKER_LOG\"\necho 'Total:\t1.2GB'\n").unwrap();
    std::fs::set_permissions(&docker, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let out = Command::new(repo().join("release/ci-clean.sh"))
        .arg("--build-cache")
        .env_remove("MAGNUS_CLEAN_AGE")
        .env("DOCKER_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "Docker build cache unused for 3h or more: Total:\t1.2GB\n");
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "builder prune --all --force --filter until=10800s\n", "only the build cache, by its age");
}

/// With 1.3 GB free, Chromium crashed in four tests and the update test could
/// not unpack its release (#203). Short of disk, a run fails as it starts and
/// says why; every workflow checks before it builds.
#[test]
fn a_run_short_of_disk_fails_at_its_start_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let run = |need: &str| Command::new(repo().join("release/ci-disk.sh")).arg(dir.path()).env("MAGNUS_CI_FREE_GB", need).output().unwrap();
    let out = run("0");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = run("1000000000");
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(!out.status.success() && err.contains("GB free on") && err.contains("a run needs 1000000000 GB"), "{err}");
    let script = std::fs::read_to_string(repo().join("release/ci-disk.sh")).unwrap();
    assert!(script.contains("need=${MAGNUS_CI_FREE_GB:-25}"), "the threshold the decision names");
    for (file, first_build) in [("desktop.yml", "name: Version"), ("injection.yml", "name: Sandbox image"), ("appliance.yml", "name: Sandbox image")] {
        let workflow = std::fs::read_to_string(repo().join(".github/workflows").join(file)).unwrap();
        let check = workflow.find("run: release/ci-disk.sh").unwrap_or_else(|| panic!("{file} does not check its disk"));
        assert!(check < workflow.find(first_build).unwrap(), "{file} builds before it checks its disk");
    }
    let desktop = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    assert!(desktop.find("run: release/ci-clean.sh").unwrap() < desktop.find("run: release/ci-disk.sh").unwrap(), "the check comes after what earlier runs left is cleared");
}

/// A runner whose cargo has no `tauri` subcommand gets one, on the PATH the
/// later steps keep. The second run does not install it again.
#[test]
fn a_runner_without_cargo_tauri_installs_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("cargo-home");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.path().join("installs");
    let cargo = bin.join("cargo");
    std::fs::write(
        &cargo,
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = tauri ]; then exec "$CARGO_HOME/bin/cargo-tauri" "$@"; fi
if [ "${1:-}" = install ]; then
  echo install >> "$INSTALL_LOG"
  dest="$CARGO_HOME/bin/cargo-tauri"
  printf '%s\n' '#!/bin/sh' 'exit 0' > "$dest"
  chmod +x "$dest"
  exit 0
fi
echo "unexpected: $*" >&2
exit 1
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cargo, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let run = || {
        let out = std::process::Command::new(repo().join("release/ensure-tauri.sh"))
            .env("CARGO_HOME", &home)
            .env("INSTALL_LOG", &log)
            .env("PATH", &path)
            .env("HOME", dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(run(), format!("{}\n", home.join("bin").display()));
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "install\n");
    assert_eq!(run(), format!("{}\n", home.join("bin").display()));
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "install\n", "a runner that already has cargo-tauri is left alone");
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    assert!(workflow.contains("release/ensure-tauri.sh"), "release-checks installs tauri before it builds");
}

/// build-box-beta has rustup and not the gnu Windows target, so the
/// installer step never reached the tests (master runs 41958146895 and
/// 41955875225). The second run does not add it again.
#[test]
fn a_runner_without_the_windows_target_installs_it() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.path().join("adds");
    let rustup = bin.join("rustup");
    std::fs::write(
        &rustup,
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = target ] && [ "${2:-}" = list ]; then
  [ -f "$ADDS" ] && echo x86_64-pc-windows-gnu
  exit 0
fi
if [ "${1:-}" = target ] && [ "${2:-}" = add ] && [ "${3:-}" = x86_64-pc-windows-gnu ]; then
  echo add >> "$ADDS"
  exit 0
fi
echo "unexpected: $*" >&2
exit 1
"#,
    )
    .unwrap();
    std::fs::set_permissions(&rustup, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let run = || {
        let out = std::process::Command::new(repo().join("release/ensure-windows-target.sh")).env("ADDS", &log).env("PATH", &path).output().unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    };
    run();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "add\n");
    run();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "add\n", "a runner that already has the target is left alone");
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    assert!(workflow.contains("release/ensure-windows-target.sh"), "release-checks installs the Windows target before it builds");
}

/// The release key is a file on the build box. A runner without it gets a
/// throwaway key and must not publish. Requiring `magnus-light` as well
/// matched no runner.
#[test]
fn a_runner_without_the_release_key_signs_with_a_throwaway_and_does_not_publish() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".magnus-release")).unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cargo = bin.join("cargo");
    std::fs::write(
        &cargo,
        r#"#!/bin/sh
set -eu
[ "${1:-}" = tauri ] && [ "${2:-}" = signer ] && [ "${3:-}" = generate ] || { echo "unexpected: $*" >&2; exit 1; }
w=""
while [ $# -gt 0 ]; do
  if [ "$1" = -w ]; then w=$2; shift; fi
  shift
done
printf '%s\n' 'throwaway' > "$w"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cargo, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let script = repo().join("release/signing-key.sh");
    let run = |key: &std::path::Path, dest: &std::path::Path| {
        let out = std::process::Command::new(&script).arg(dest).env("HOME", &home).env("PATH", &path).env("MAGNUS_RELEASE_KEY", key).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let real = home.join(".magnus-release/update.key");
    std::fs::write(&real, "release-key").unwrap();
    let dest = dir.path().join("throw");
    assert_eq!(run(&real, &dest), format!("{}\n1\n", real.display()));
    assert!(!dest.join("key").exists(), "the release key was replaced");
    let missing = dir.path().join("absent");
    let made = dir.path().join("made");
    assert_eq!(run(&missing, &made), format!("{}\n0\n", made.join("key").display()));
    assert_eq!(std::fs::read_to_string(made.join("key")).unwrap(), "throwaway\n");
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    assert!(workflow.contains("release/signing-key.sh"));
    assert!(workflow.contains("env.MAGNUS_PUBLISH == '1'"), "publishing waits for the release key");
    let runs_on = workflow.split("runs-on:").nth(1).unwrap().lines().next().unwrap();
    let pull_requests = runs_on.split("||").next().unwrap();
    assert!(pull_requests.contains("pull_request") && pull_requests.contains("magnus-linux") && !pull_requests.contains("magnus-light"), "{runs_on}");
}

/// Master run 42097633195 landed on build-box-beta, which has the release key
/// now and no appliance: it published the test build into a directory no app
/// reads, then failed installing on the appliance (#204). A run publishes only
/// with the key and the appliance; master and tags fail at once without them,
/// and run on the build box, the one runner with magnus-linux and magnus-light.
#[test]
fn only_the_runner_with_the_appliance_publishes_and_master_and_tags_run_there() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("update.key");
    let config = dir.path().join("magnus.toml");
    let run = |must: &str| {
        let out = std::process::Command::new(repo().join("release/can-publish.sh"))
            .arg(&config)
            .arg(must)
            .env("MAGNUS_RELEASE_KEY", &key)
            .output()
            .unwrap();
        (out.status.success(), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
    };
    assert_eq!(run("0"), (true, "0\n".into(), String::new()), "no key, no appliance");
    let (ok, _, err) = run("1");
    assert!(!ok && err.contains("no release key") && err.contains("no appliance"), "{err}");
    std::fs::write(&key, "release-key").unwrap();
    assert_eq!(run("0"), (true, "0\n".into(), String::new()), "the key without the appliance (strix-beta)");
    let (ok, _, err) = run("1");
    assert!(!ok && !err.contains("no release key") && err.contains("no appliance"), "{err}");
    std::fs::write(&config, "").unwrap();
    assert_eq!(run("0"), (true, "1\n".into(), String::new()));
    assert_eq!(run("1"), (true, "1\n".into(), String::new()));

    let desktop = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    assert!(desktop.contains("release/can-publish.sh \"${{ vars.MAGNUS_APPLIANCE_CONFIG }}\" \"${{ github.event_name == 'push' && '1' || '0' }}\""));
    let runs_on = desktop.split("runs-on:").nth(1).unwrap().lines().next().unwrap();
    let pushes = runs_on.split("||").nth(1).expect("master and tags have their own runner");
    assert!(pushes.contains("magnus-linux") && pushes.contains("magnus-light"), "{runs_on}");
    let appliance = std::fs::read_to_string(repo().join(".github/workflows/appliance.yml")).unwrap();
    let runs_on = appliance.split("runs-on:").nth(1).unwrap().lines().next().unwrap();
    assert!(runs_on.contains("magnus-linux") && runs_on.contains("magnus-light"), "{runs_on}");
    let check = appliance.find("release/can-publish.sh \"${{ vars.MAGNUS_APPLIANCE_CONFIG }}\" 1").expect("the release checks its runner");
    assert!(check < appliance.find("dev/services.sh").unwrap(), "the release checks its runner before it builds");
}

/// Pull requests run beside each other, so none publishes to the installed
/// apps: two open ones replaced each other's build in the owner's app, and
/// a pull request tested on beta published nothing. Master publishes every
/// merge to the owner's app (development) and the testers' (test), and a
/// master build holds everything merged (2026-11-06).
#[test]
fn only_master_publishes_to_the_installed_apps() {
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    let steps: Vec<&str> = workflow.split("      - name: ").skip(1).collect();
    for step in &steps {
        let name = step.lines().next().unwrap_or_default();
        if step.lines().any(|l| !l.trim_start().starts_with('#') && l.contains("release/publish.sh")) {
            let cond = step.lines().find(|l| l.trim_start().starts_with("if:")).unwrap_or_else(|| panic!("{name} publishes on every event"));
            assert!(!cond.contains("'pull_request'"), "{name} publishes from a pull request: {cond}");
        }
    }
    let master = steps.iter().find(|s| s.contains("refs/heads/master") && s.contains("dist test")).expect("master publishes to test");
    assert!(master.contains("dist development"), "master does not publish to the owner's app");
    // A pull request's build still there from before is given up or held by
    // where it landed, or master would be refused there for good (review of #219).
    assert!(master.contains("MAGNUS_GIVEN_UP") && master.contains("MAGNUS_LANDED") && master.contains("GH_TOKEN"), "master does not reconcile a pull request's build in development");
}

/// The two update AppImages compile after the suite. Beside it, they took
/// the cores the browser tests need (run 41949527809).
#[test]
fn the_update_builds_wait_until_the_suite_has_finished() {
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    let suite = workflow.find("Unit, invariant and scratch tests").expect("suite step");
    let builds = workflow.find("Start the update test's two builds").expect("update builds");
    assert!(builds > suite, "the update builds start during the suite");
}

/// build-box-beta has rustup and not the Windows cross target (run 41957162011).
#[test]
fn the_windows_installer_adds_its_cross_target_first() {
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    let add = workflow.find("rustup target add x86_64-pc-windows-gnu").expect("rustup target");
    let mingw = workflow.find("x86_64-w64-mingw32-dlltool").expect("mingw");
    let build = workflow.find("cargo tauri build --runner cargo --target x86_64-pc-windows-gnu").expect("windows build");
    assert!(add < build && mingw < build, "the Windows build runs before its target and mingw are installed");
}

/// build-box-beta has no WebKitWebDriver, so the installed-app tests all
/// stopped at once (run 41959422399).
#[test]
fn the_installed_app_tests_install_webkit_webdriver_first() {
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    let driver = workflow.find("webkit2gtk-driver").expect("webkit driver");
    let clip = workflow.find("xclip").expect("xclip");
    let tests = workflow.find("name: Installed-app tests").expect("installed-app tests");
    assert!(driver < tests && clip < tests, "the installed-app tests run before WebKitWebDriver and xclip are installed");
}

/// The `run:` text of the desktop workflow's step with this name.
fn step_run(name: &str) -> String {
    let workflow = std::fs::read_to_string(repo().join(".github/workflows/desktop.yml")).unwrap();
    let step = workflow.split("      - name: ").find(|s| s.lines().next() == Some(name)).unwrap_or_else(|| panic!("no step {name}"));
    let mut lines = step.lines().skip_while(|l| !l.trim_start().starts_with("run:"));
    let run = lines.next().unwrap_or_else(|| panic!("{name} has no run"));
    let inline = run.trim_start().trim_start_matches("run:").trim();
    if inline != "|" {
        return format!("{inline}\n");
    }
    let indent = run.len() - run.trim_start().len();
    lines.take_while(|l| l.trim().is_empty() || l.len() - l.trim_start().len() > indent).map(|l| format!("{}\n", l.get(indent + 2..).unwrap_or(""))).collect()
}

/// The installed-app drivers step installed nothing: it tested `[ -n "" ]`
/// and ran `apt-get install -y` with no packages, and the xclip check
/// replaced the list instead of adding to it (#189). Run as CI runs it, with
/// only these commands on the PATH: a runner without WebKitWebDriver and
/// xclip asks for both in one install, and one with both installs nothing.
#[test]
fn the_installed_app_drivers_install_every_missing_package() {
    let run_step = |present: &[&str]| {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = dir.path().join("sudo");
        let stub = |name: &str, body: &str| {
            let p = bin.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        };
        stub("sudo", r#"echo "sudo $*" >> "$SUDO_LOG""#);
        stub("cargo", r#"echo "cargo $*" >> "$SUDO_LOG""#);
        for c in present {
            stub(c, "exit 0");
        }
        let script = dir.path().join("step.sh");
        std::fs::write(&script, step_run("Installed-app drivers")).unwrap();
        let out = Command::new("/bin/bash")
            .args(["--noprofile", "--norc", "-eo", "pipefail"])
            .arg(&script)
            .current_dir(repo())
            .env("PATH", &bin)
            .env("HOME", dir.path())
            .env("CARGO_HOME", dir.path().join("cargo-home"))
            .env("SUDO_LOG", &log)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        std::fs::read_to_string(&log).unwrap_or_default()
    };
    assert_eq!(
        run_step(&["tauri-driver"]),
        "sudo -n apt-get update -qq\nsudo -n apt-get install -y webkit2gtk-driver xclip\n",
        "a runner without WebKitWebDriver and xclip gets both"
    );
    assert_eq!(run_step(&["WebKitWebDriver", "xclip", "tauri-driver"]), "", "a runner with both installs nothing");
    // A change to the script runs everything, this test and the step included.
    assert_eq!(select(&["release/ensure-packages.sh"]), "MAGNUS_SELECT=full\n");
}

/// Master's runs finish: a merge waits for master's run in progress instead
/// of cancelling it, so with pull requests merging beside each other master
/// is still tested in full and the owner's app still gets its build (#228;
/// 6947618's, 8b78dea's and 99d84fe's runs were cancelled by the next merge).
/// A pull request's new push still cancels its own earlier run.
#[test]
fn a_merge_does_not_cancel_masters_run() {
    for w in ["desktop", "injection", "appliance"] {
        let workflow = std::fs::read_to_string(repo().join(format!(".github/workflows/{w}.yml"))).unwrap();
        let cancel = workflow.lines().find(|l| l.trim_start().starts_with("cancel-in-progress:")).unwrap_or_else(|| panic!("{w}.yml names no cancel-in-progress"));
        let v = cancel.split_once(':').unwrap().1.trim();
        assert!(v == "false" || v == "${{ github.event_name == 'pull_request' }}", "{w}.yml cancels master's run on the next merge: {cancel}");
    }
}
