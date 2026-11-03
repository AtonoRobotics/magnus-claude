//! For brokered connectors' tests: a server image built from this checkout
//! (`brokered/build.sh`).

use magnus_invariants::appliance::workspace_root;
use std::path::Path;

/// A server image, built: where it is and its digest (`sha256:<hex>`).
pub struct Image {
    pub dir: tempfile::TempDir,
    pub digest: String,
}

impl Image {
    /// The image's file, named by its digest.
    pub fn file(&self) -> std::path::PathBuf {
        self.dir.path().join(format!("{}.cpio", self.digest.trim_start_matches("sha256:")))
    }
}

/// Build `package`'s server image as a release does.
pub fn image(package: &str) -> Image {
    build(package, None)
}

/// Build it with its own target directory, as another machine would.
pub fn image_elsewhere(package: &str, target: &Path) -> Image {
    build(package, Some(target))
}

fn build(package: &str, target: Option<&Path>) -> Image {
    let dir = tempfile::tempdir().unwrap();
    let mut c = std::process::Command::new(workspace_root().join("brokered/build.sh"));
    c.arg(package).arg(dir.path());
    if let Some(t) = target {
        c.env("CARGO_TARGET_DIR", t);
    }
    let out = c.output().expect("run brokered/build.sh");
    assert!(out.status.success(), "building {package}'s image failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let digest = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(digest.starts_with("sha256:"), "{digest}");
    Image { dir, digest }
}
