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
    assert!(runs_on.contains("magnus-linux") && !runs_on.contains("magnus-light"), "{runs_on}");
}
