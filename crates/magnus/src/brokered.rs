//! Brokered connectors (connectors spec, Brokered API connectors): MCP
//! servers Alpha Vector builds over a provider's public API, run here.
//!
//! - Packaging: a connector's endpoint is its server image's digest
//!   (`sha256:<hex>`); the image is an initramfs whose `/init` is the server
//!   (`brokered/build.sh`).

use anyhow::{anyhow, Result};

/// The image digest a brokered connector's endpoint names: its hex.
pub fn digest(endpoint: &str) -> Result<&str> {
    endpoint
        .strip_prefix("sha256:")
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)))
        .ok_or_else(|| anyhow!("a brokered connector's endpoint is its server image's digest, sha256:<64 hex digits>, not {endpoint:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_named_by_its_full_digest() {
        assert!(digest(&format!("sha256:{}", "0a".repeat(32))).is_ok());
        for e in ["https://a.example/mcp", "sha256:abc", &format!("sha256:{}", "0A".repeat(32)), &format!("sha512:{}", "0a".repeat(32))] {
            assert!(digest(e).is_err(), "{e}");
        }
    }
}
