//! Each run's browser in a microVM of its own: Chromium and the sidecar in
//! an image with no network device. Chromium's proxy is a forwarder in the
//! guest that reaches the kernel over vsock (port 1080), where the egress
//! proxy decides every request for this run. The kernel drives the sidecar
//! through the guest agent. Nothing in the VM holds a credential.
//!
//! On a development host without a VM image, the sidecar runs on the host
//! and Chromium reaches the same proxy with a per-run proxy credential.

use crate::kernel::Kernel;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

/// The vsock port the guest's forwarder connects to on the host.
pub const EGRESS_PORT: u32 = 1080;
/// Where the forwarder listens in the guest, as Chromium's proxy.
pub const GUEST_PROXY: &str = "http://127.0.0.1:3128";

impl Kernel {
    /// Chromium's launch arguments: its only network is the proxy, loopback
    /// included, and the only certificates it accepts beyond the system's
    /// are the egress proxy's.
    pub fn chromium_args(&self, proxy: &str) -> Vec<String> {
        vec![format!("--proxy-server={proxy}"), "--proxy-bypass-list=<-loopback>".into(), format!("--ignore-certificate-errors-spki-list={}", self.egress.spki())]
    }

    /// One request to a run's browser.
    pub async fn browse(self: &Arc<Self>, run: &str, mut req: Value) -> Result<Value> {
        if self.browser.in_vm() {
            let vm = self.browser_vm(run).await?;
            let v = self.sandboxes.guest(&vm, &json!({ "op": "browser", "request": req }), std::time::Duration::from_secs(120)).await?;
            if let Some(e) = v["error"].as_str() {
                return Err(anyhow!("{e}"));
            }
            return Ok(v);
        }
        let port = self.serve_egress().await?;
        let proxy = format!("http://127.0.0.1:{port}");
        req["proxy"] = json!({ "server": proxy, "username": self.egress.token(run), "password": "magnus" });
        self.browser.request(run, req, &self.chromium_args(&proxy)).await
    }

    /// The run's browser VM, booted on first use with its egress listener.
    async fn browser_vm(self: &Arc<Self>, run: &str) -> Result<Arc<crate::sandbox::Vm>> {
        let rootfs = self.cfg.browser.as_ref().and_then(|b| b.rootfs.clone()).ok_or_else(|| anyhow!("no browser image is configured"))?;
        let k = self.clone();
        let run_id = run.to_string();
        // Each connection from this VM's forwarder is this run's.
        let on_egress = move |s: tokio::net::UnixStream| {
            let (k, run) = (k.clone(), run_id.clone());
            tokio::spawn(async move { k.egress_connection(s, Some(run)).await });
        };
        let (vm, fresh) = self.sandboxes.browser_vm(run, &rootfs, EGRESS_PORT, on_egress).await?;
        if fresh {
            let v = self
                .sandboxes
                .guest(&vm, &json!({ "op": "browser_start", "args": self.chromium_args(GUEST_PROXY) }), std::time::Duration::from_secs(60))
                .await?;
            if let Some(e) = v["error"].as_str() {
                return Err(anyhow!("the browser did not start in its VM: {e}"));
            }
        }
        Ok(vm)
    }

    /// The run ended: close its browser, and forget its proxy credential,
    /// held cookies and secrets.
    pub async fn browse_close(&self, run: &str) {
        if self.browser.in_vm() {
            self.sandboxes.browser_vm_stop(run).await;
        } else {
            self.browser.close(run).await;
        }
        self.egress.forget(run);
    }
}
