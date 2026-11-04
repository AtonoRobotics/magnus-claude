//! The pull request rules CI checks (`release/pr-rules.sh`; CLAUDE.md, "How
//! work lands"): several may be open, an agent pull request past the cap
//! is the one that fails, into master only; a branch behind master and a
//! large change are warned about, not refused. The one-at-a-time gate
//! blocked every pull request behind a single unfinished one (owner,
//! 2026-11-04). `release/pr-stale.sh --decide` is the age at which a quiet
//! pull request is commented on, labelled, and closed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/pr-rules.sh")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@example.com"]).args(args).current_dir(repo).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn commit(repo: &Path, file: &str, lines: usize) -> String {
    std::fs::write(repo.join(file), "x\n".repeat(lines)).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-qm", file]);
    git(repo, &["rev-parse", "HEAD"])
}

/// The rules for pull request `pr` at `head` into `base`, with these open;
/// its exit code and what it printed.
fn rules(repo: &Path, pr: u32, head: &str, base: &str, master: &str, open: &str) -> (i32, String) {
    let out = Command::new(script())
        .args([&pr.to_string(), head, base, master])
        .env("MAGNUS_OPEN_PRS", open)
        .env_remove("MAGNUS_PR_LINES")
        .env_remove("MAGNUS_PR_AGENT")
        .env_remove("MAGNUS_PR_CAP")
        .env_remove("MAGNUS_PR_WARN")
        .current_dir(repo)
        .output()
        .unwrap();
    (out.status.code().unwrap(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn several_pull_requests_may_be_open_and_only_a_new_agent_one_past_the_cap_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    git(repo, &["init", "-q", "-b", "master"]);
    let base = commit(repo, "base", 1);
    git(repo, &["checkout", "-q", "-b", "fix", &base]);
    let fix = commit(repo, "src.rs", 10);

    // From current master, with others already open, under the cap: it passes.
    let open = "40:agent 43:agent 44:human";
    let (code, out) = rules(repo, 43, &fix, "master", &base, open);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("::warning::"), "{out}");

    // Seven agent pull requests. The newest fails and names one that does
    // not; the sixth, and a person's pull request, pass with the same list.
    let seven = "1:agent 2:agent 3:agent 4:agent 5:agent 6:agent 7:agent 8:human";
    let (code, out) = rules(repo, 7, &fix, "master", &base, seven);
    assert_eq!(code, 4, "{out}");
    assert!(out.contains("#1") && out.contains("cap of 6") && !out.contains("#8"), "{out}");
    assert_eq!(rules(repo, 6, &fix, "master", &base, seven).0, 0);
    let (code, out) = rules(repo, 8, &fix, "master", &base, seven);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("cap of 6"), "{out}");

    // Past the warning line, within the cap: a warning, not a failure.
    let five = "1:agent 2:agent 3:agent 4:agent 5:agent";
    let (code, out) = rules(repo, 5, &fix, "master", &base, five);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("::warning::") && out.contains("5 of 5"), "{out}");
    let (code, out) = rules(repo, 4, &fix, "master", &base, five);
    assert!(code == 0 && !out.contains("::warning::"), "{out}");

    // Stacked on another branch: refused.
    let (code, out) = rules(repo, 43, &fix, "feature", &base, "43:agent");
    assert_eq!(code, 4, "{out}");
    assert!(out.contains("merges into feature"), "{out}");

    // Master moves on: a warning, not a failure. Merging it in clears it.
    git(repo, &["checkout", "-q", "master"]);
    let moved = commit(repo, "other.rs", 1);
    let (code, out) = rules(repo, 43, &fix, "master", &moved, "43:agent");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("::warning::") && out.contains("merge master into it"), "{out}");
    git(repo, &["checkout", "-q", "fix"]);
    git(repo, &["merge", "-q", "--no-edit", "master"]);
    let merged = git(repo, &["rev-parse", "HEAD"]);
    let (code, out) = rules(repo, 43, &merged, "master", &moved, "43:agent");
    assert!(code == 0 && !out.contains("::warning::"), "{out}");

    // A large change is warned about, not refused; tests and documents
    // do not count.
    commit(repo, "big.rs", 900);
    let big = git(repo, &["rev-parse", "HEAD"]);
    let (code, out) = rules(repo, 43, &big, "master", &moved, "43:agent");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("::warning::") && out.contains("910 lines"), "{out}");
    std::fs::create_dir_all(repo.join("tests")).unwrap();
    git(repo, &["checkout", "-q", "-b", "tests-only", &merged]);
    commit(repo, "tests/t.rs", 900);
    commit(repo, "NOTES.md", 900);
    let docs = git(repo, &["rev-parse", "HEAD"]);
    let (code, out) = rules(repo, 43, &docs, "master", &moved, "43:agent");
    assert!(code == 0 && !out.contains("::warning::"), "{out}");
}

fn stale(hours: u32, hold: &str) -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/pr-stale.sh");
    let out = Command::new(script)
        .args(["--decide", &hours.to_string(), hold])
        .env_remove("MAGNUS_STALE_HOURS")
        .env_remove("MAGNUS_STALE_LABEL_HOURS")
        .env_remove("MAGNUS_STALE_CLOSE_HOURS")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn a_quiet_pull_request_is_warned_then_labelled_then_closed() {
    assert_eq!(stale(23, ""), "");
    assert_eq!(stale(24, ""), "comment\n");
    assert_eq!(stale(72, ""), "comment\nlabel\n");
    assert_eq!(stale(120, ""), "comment\nlabel\nclose\n");
    // hold stops the close and nothing earlier.
    assert_eq!(stale(120, "hold"), "comment\nlabel\n");
    assert_eq!(stale(23, "hold"), "");
}
