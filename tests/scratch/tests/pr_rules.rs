//! The pull request rules CI checks (`release/pr-rules.sh`; CLAUDE.md, "How
//! work lands"): one open at a time in the order opened, into master only,
//! holding master's current commit; a large change is warned about, not
//! refused. After pull requests sat open for days while master moved on,
//! and their work went stale (owner, 2026-10-31).

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
    let out = Command::new(script()).args([&pr.to_string(), head, base, master]).env("MAGNUS_OPEN_PRS", open).env_remove("MAGNUS_PR_LINES").current_dir(repo).output().unwrap();
    (out.status.code().unwrap(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn one_pull_request_at_a_time_into_master_holding_master() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    git(repo, &["init", "-q", "-b", "master"]);
    let base = commit(repo, "base", 1);
    git(repo, &["checkout", "-q", "-b", "fix", &base]);
    let fix = commit(repo, "src.rs", 10);

    // From current master, the only one open: it passes.
    let (code, out) = rules(repo, 43, &fix, "master", &base, "43");
    assert_eq!(code, 0, "{out}");

    // One opened before it is still open: it waits, and says on which.
    let (code, out) = rules(repo, 43, &fix, "master", &base, "40 43 44");
    assert_eq!(code, 4, "{out}");
    assert!(out.contains("#40") && !out.contains("#44"), "{out}");
    // The earliest open one is not held back by those after it.
    assert_eq!(rules(repo, 40, &fix, "master", &base, "40 43 44").0, 0);

    // Stacked on another branch: refused.
    let (code, out) = rules(repo, 43, &fix, "feature", &base, "43");
    assert_eq!(code, 4, "{out}");
    assert!(out.contains("merges into feature"), "{out}");

    // Master moves on: refused until master is merged in.
    git(repo, &["checkout", "-q", "master"]);
    let moved = commit(repo, "other.rs", 1);
    let (code, out) = rules(repo, 43, &fix, "master", &moved, "43");
    assert_eq!(code, 4, "{out}");
    assert!(out.contains("merge master into it"), "{out}");
    git(repo, &["checkout", "-q", "fix"]);
    git(repo, &["merge", "-q", "--no-edit", "master"]);
    let merged = git(repo, &["rev-parse", "HEAD"]);
    assert_eq!(rules(repo, 43, &merged, "master", &moved, "43").0, 0);

    // A large change is warned about, not refused; tests and documents
    // do not count.
    commit(repo, "big.rs", 900);
    let big = git(repo, &["rev-parse", "HEAD"]);
    let (code, out) = rules(repo, 43, &big, "master", &moved, "43");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("::warning::") && out.contains("910 lines"), "{out}");
    std::fs::create_dir_all(repo.join("tests")).unwrap();
    git(repo, &["checkout", "-q", "-b", "tests-only", &merged]);
    commit(repo, "tests/t.rs", 900);
    commit(repo, "NOTES.md", 900);
    let docs = git(repo, &["rev-parse", "HEAD"]);
    let (code, out) = rules(repo, 43, &docs, "master", &moved, "43");
    assert!(code == 0 && !out.contains("::warning::"), "{out}");
}
