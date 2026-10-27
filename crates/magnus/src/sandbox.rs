//! One Firecracker microVM per run, with no network device, controlled over
//! vsock (K16). The workspace disk outlives a stopped VM so a resumed run
//! boots a fresh VM on the same disk (K20); it is deleted with the run (K19).

use crate::config::Sandbox as Cfg;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const GUEST_PORT: u32 = 52;

pub struct Vm {
    child: Mutex<Child>,
    vsock: PathBuf,
}

impl Drop for Vm {
    fn drop(&mut self) {
        let mut c = self.child.lock().unwrap();
        let _ = c.kill();
        let _ = c.wait();
    }
}

pub struct Sandboxes {
    cfg: Option<Cfg>,
    root: PathBuf,
    vms: tokio::sync::Mutex<HashMap<String, Arc<Vm>>>,
}

/// The Firecracker configuration for a run's VM. It never has a network
/// interface; `launch` refuses one even if a caller adds it.
pub fn vm_config(cfg: &Cfg, workspace: &Path, vsock: &Path) -> Value {
    json!({
        "boot-source": {
            "kernel_image_path": cfg.kernel_image,
            "boot_args": "console=ttyS0 reboot=k panic=1 pci=off quiet loglevel=3 init=/sbin/magnus-guest"
        },
        "drives": [
            { "drive_id": "rootfs", "path_on_host": cfg.rootfs, "is_root_device": true, "is_read_only": true },
            { "drive_id": "workspace", "path_on_host": workspace, "is_root_device": false, "is_read_only": false }
        ],
        "machine-config": { "vcpu_count": cfg.vcpus, "mem_size_mib": cfg.memory_mib },
        "vsock": { "guest_cid": 3, "uds_path": vsock }
    })
}

impl Sandboxes {
    pub fn new(cfg: Option<Cfg>, state_dir: &Path) -> Sandboxes {
        Sandboxes { cfg, root: state_dir.join("runs"), vms: tokio::sync::Mutex::new(HashMap::new()) }
    }

    pub fn available(&self) -> bool {
        self.cfg.is_some()
    }

    fn dir(&self, run: &str) -> PathBuf {
        self.root.join(run)
    }

    async fn launch(&self, run: &str) -> Result<Arc<Vm>> {
        let cfg = self.cfg.as_ref().ok_or_else(|| anyhow!("no sandbox is configured on this appliance"))?;
        let dir = self.dir(run);
        std::fs::create_dir_all(&dir)?;
        let workspace = dir.join("workspace.ext4");
        if !workspace.exists() {
            let f = std::fs::File::create(&workspace)?;
            f.set_len(cfg.workspace_mib as u64 * 1024 * 1024)?;
            drop(f);
            let out = Command::new("mkfs.ext4").args(["-q", "-F", "-L", "workspace"]).arg(&workspace).output()
                .or_else(|_| Command::new("/usr/sbin/mkfs.ext4").args(["-q", "-F", "-L", "workspace"]).arg(&workspace).output())
                .context("run mkfs.ext4")?;
            anyhow::ensure!(out.status.success(), "mkfs.ext4 failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        let vsock = dir.join("vsock.sock");
        let api = dir.join("api.sock");
        let _ = std::fs::remove_file(&vsock);
        let _ = std::fs::remove_file(&api);
        let config = vm_config(cfg, &workspace, &vsock);
        anyhow::ensure!(config.get("network-interfaces").is_none(), "refusing a sandbox with a network device");
        let config_path = dir.join("vm.json");
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config)?)?;
        let console = std::fs::File::create(dir.join("console.log"))?;
        let mut cmd = Command::new(&cfg.firecracker);
        cmd.arg("--api-sock").arg(&api).arg("--config-file").arg(&config_path).arg("--level").arg("Error");
        cmd.stdin(Stdio::null()).stdout(Stdio::from(console.try_clone()?)).stderr(Stdio::from(console));
        let child = crate::spawn::spawn(cmd).context("start firecracker")?;
        let vm = Arc::new(Vm { child: Mutex::new(child), vsock });
        // Wait for the guest agent.
        let start = std::time::Instant::now();
        loop {
            if let Ok(v) = request(&vm.vsock, &json!({ "op": "ping" }), Duration::from_secs(2)).await {
                if v["ok"] == true {
                    return Ok(vm);
                }
            }
            if let Ok(Some(st)) = vm.child.lock().unwrap().try_wait() {
                let log = std::fs::read_to_string(dir.join("console.log")).unwrap_or_default();
                return Err(anyhow!("the sandbox VM exited during boot ({st}): {}", log.chars().rev().take(800).collect::<String>().chars().rev().collect::<String>()));
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err(anyhow!("the sandbox VM did not come up within 30s"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn vm(&self, run: &str) -> Result<Arc<Vm>> {
        let mut vms = self.vms.lock().await;
        if let Some(vm) = vms.get(run) {
            if vm.child.lock().unwrap().try_wait().ok().flatten().is_none() {
                return Ok(vm.clone());
            }
        }
        let vm = self.launch(run).await?;
        vms.insert(run.to_string(), vm.clone());
        Ok(vm)
    }

    pub async fn exec(&self, run: &str, command: &str, timeout_s: u64) -> Result<Value> {
        let vm = self.vm(run).await?;
        request(&vm.vsock, &json!({ "op": "exec", "command": command, "timeout_s": timeout_s }), Duration::from_secs(timeout_s + 15)).await
    }

    pub async fn write(&self, run: &str, path: &str, data: &[u8]) -> Result<Value> {
        let vm = self.vm(run).await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(data);
        request(&vm.vsock, &json!({ "op": "write", "path": path, "data_b64": b64 }), Duration::from_secs(60)).await
    }

    pub async fn read(&self, run: &str, path: &str) -> Result<Vec<u8>> {
        let vm = self.vm(run).await?;
        let v = request(&vm.vsock, &json!({ "op": "read", "path": path }), Duration::from_secs(60)).await?;
        if let Some(e) = v["error"].as_str() {
            return Err(anyhow!("{e}"));
        }
        Ok(base64::engine::general_purpose::STANDARD.decode(v["data_b64"].as_str().unwrap_or_default())?)
    }

    /// Stop the run's VM and keep its disk (suspend).
    pub async fn stop(&self, run: &str) {
        if let Some(vm) = self.vms.lock().await.remove(run) {
            let _ = request(&vm.vsock, &json!({ "op": "sync" }), Duration::from_secs(5)).await;
            drop(vm);
        }
    }

    /// Stop the VM and delete the workspace (the run ended).
    pub async fn destroy(&self, run: &str) {
        self.stop(run).await;
        let _ = std::fs::remove_dir_all(self.dir(run));
    }
}

async fn request(vsock: &Path, req: &Value, timeout: Duration) -> Result<Value> {
    tokio::time::timeout(timeout, async {
        let s = UnixStream::connect(vsock).await?;
        let (r, mut w) = s.into_split();
        w.write_all(format!("CONNECT {GUEST_PORT}\n").as_bytes()).await?;
        let mut r = BufReader::new(r);
        let mut ack = String::new();
        r.read_line(&mut ack).await?;
        anyhow::ensure!(ack.starts_with("OK"), "guest agent not listening");
        w.write_all(format!("{req}\n").as_bytes()).await?;
        let mut line = String::new();
        r.read_line(&mut line).await?;
        anyhow::ensure!(!line.is_empty(), "guest agent closed the connection");
        Ok(serde_json::from_str(&line)?)
    })
    .await
    .map_err(|_| anyhow!("the sandbox did not answer in time"))?
}
