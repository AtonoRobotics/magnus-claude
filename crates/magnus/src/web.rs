//! Web access for the agent: fetch and search, run by the kernel behind the
//! boundary. Everything returned is labeled untrusted.

use anyhow::{anyhow, Result};
use std::net::IpAddr;
use std::time::Duration;

const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_TEXT: usize = 60_000;

pub struct Web {
    http: reqwest::Client,
    allow_private: bool,
    search_url: Option<String>,
}

fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || v.octets()[0] == 100 && (64..128).contains(&v.octets()[1]),
        IpAddr::V6(v) => v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
    }
}

impl Web {
    pub fn new(cfg: &crate::config::Web) -> Web {
        Web {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::limited(5))
                .user_agent("Mozilla/5.0 (compatible; Magnus/0.1)")
                .build()
                .expect("http client"),
            allow_private: cfg.allow_private_networks,
            search_url: cfg.search_url.clone(),
        }
    }

    pub fn can_search(&self) -> bool {
        self.search_url.is_some()
    }

    async fn check_host(&self, url: &url::Url) -> Result<()> {
        anyhow::ensure!(matches!(url.scheme(), "http" | "https"), "only http and https URLs can be fetched");
        if self.allow_private {
            return Ok(());
        }
        let host = url.host_str().ok_or_else(|| anyhow!("URL has no host"))?;
        let port = url.port_or_known_default().unwrap_or(443);
        let addrs = tokio::net::lookup_host((host, port)).await?;
        for a in addrs {
            anyhow::ensure!(!private_ip(a.ip()), "{host} resolves to a private address; fetching it is not allowed");
        }
        Ok(())
    }

    /// Returns (text, content type).
    pub async fn fetch(&self, raw: &str) -> Result<(String, String)> {
        let url = url::Url::parse(raw).map_err(|e| anyhow!("not a URL: {e}"))?;
        self.check_host(&url).await?;
        let resp = self.http.get(url.clone()).send().await.map_err(|e| anyhow!("fetch failed: {e}"))?;
        let status = resp.status();
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let mut bytes = Vec::new();
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| anyhow!("download failed: {e}"))?;
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_BYTES {
                break;
            }
        }
        let mut text = if ctype.contains("html") {
            html2text::from_read(&bytes[..], 100).unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned())
        } else if ctype.starts_with("text/") || ctype.contains("json") || ctype.contains("xml") {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            format!("[{} bytes of {ctype}; not shown as text]", bytes.len())
        };
        if text.len() > MAX_TEXT {
            let mut cut = MAX_TEXT;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str("\n[truncated]");
        }
        Ok((format!("HTTP {status} {url}\n\n{text}"), ctype))
    }

    pub async fn search(&self, query: &str) -> Result<String> {
        let base = self.search_url.as_ref().ok_or_else(|| anyhow!("no search service is configured"))?;
        let mut url = url::Url::parse(base)?;
        url.query_pairs_mut().append_pair("q", query).append_pair("format", "json");
        let v: serde_json::Value = self.http.get(url).send().await?.json().await?;
        let mut out = String::new();
        for r in v["results"].as_array().into_iter().flatten().take(10) {
            out.push_str(&format!(
                "- {}\n  {}\n  {}\n",
                r["title"].as_str().unwrap_or_default(),
                r["url"].as_str().unwrap_or_default(),
                r["content"].as_str().unwrap_or_default()
            ));
        }
        if out.is_empty() {
            out = "no results".into();
        }
        Ok(out)
    }
}
