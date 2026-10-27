//! The kernel side of the model adapter contract (`magnus.adapter/1`).
//!
//! Each adapter is a separate process the kernel starts and supervises. To
//! replace or update one, the kernel starts the new process, checks its
//! contract version, and points new requests at its socket; requests already
//! in flight finish on the old one, which is then stopped. The kernel keeps
//! running throughout.

use crate::config;
use crate::vault::Vault;
use anyhow::{anyhow, Context, Result};
use magnus_adapter_proto::{self as proto, Block, CompleteParams, ErrorClass, Event, Message, Request, ToolSpec, Usage};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;

pub struct Running {
    pub cfg: config::Adapter,
    pub socket: PathBuf,
    pub adapter: String,
    pub version: String,
    child: Mutex<Child>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut c = self.child.lock().unwrap();
        let _ = c.kill();
        let _ = c.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

pub struct Adapters {
    dir: PathBuf,
    vault: Arc<Vault>,
    running: RwLock<HashMap<String, Arc<Running>>>,
    configs: RwLock<HashMap<String, config::Adapter>>,
    generation: std::sync::atomic::AtomicU64,
}

#[derive(Debug, Clone)]
pub struct AdapterError {
    pub class: ErrorClass,
    pub message: String,
    pub retry_after_s: Option<f64>,
}

impl AdapterError {
    fn unavailable(m: impl Into<String>) -> Self {
        AdapterError { class: ErrorClass::Unavailable, message: m.into(), retry_after_s: None }
    }
}

pub struct TurnOut {
    pub blocks: Vec<Block>,
    pub state: Option<Value>,
    pub usage: Usage,
    pub served_model: String,
    pub stop: String,
}

impl Adapters {
    pub fn new(state_dir: &std::path::Path, vault: Arc<Vault>, cfgs: &[config::Adapter]) -> Adapters {
        let dir = state_dir.join("adapters");
        let _ = std::fs::create_dir_all(&dir);
        Adapters {
            dir,
            vault,
            running: RwLock::new(HashMap::new()),
            configs: RwLock::new(cfgs.iter().map(|c| (c.id.clone(), c.clone())).collect()),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    async fn launch(&self, cfg: &config::Adapter) -> Result<Arc<Running>> {
        anyhow::ensure!(!cfg.command.is_empty(), "adapter {} has no command", cfg.id);
        let gen = self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let socket = self.dir.join(format!("{}-{}-{gen}.sock", cfg.id, std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let log = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(format!("{}.log", cfg.id)))?;
        let mut cmd = Command::new(&cfg.command[0]);
        cmd.args(&cfg.command[1..]).arg("--socket").arg(&socket);
        cmd.env_clear().env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.stdin(Stdio::null()).stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log));
        let child = crate::spawn::spawn(cmd).with_context(|| format!("start adapter {}", cfg.id))?;
        let mut running = Running { cfg: cfg.clone(), socket: socket.clone(), adapter: String::new(), version: String::new(), child: Mutex::new(child) };
        let start = std::time::Instant::now();
        loop {
            if socket.exists() {
                if let Ok(mut call) = proto::Call::open(&socket, Request::Hello {}).await {
                    match call.next().await {
                        Ok(Some(Event::Hello { contract, adapter, version })) => {
                            anyhow::ensure!(contract == proto::CONTRACT, "adapter {} speaks {contract}, not {}", cfg.id, proto::CONTRACT);
                            running.adapter = adapter;
                            running.version = version;
                            return Ok(Arc::new(running));
                        }
                        Ok(other) => anyhow::bail!("adapter {} answered hello with {other:?}", cfg.id),
                        Err(_) => {}
                    }
                }
            }
            if let Ok(Some(st)) = running.child.lock().unwrap().try_wait() {
                anyhow::bail!("adapter {} exited at start ({st})", cfg.id);
            }
            anyhow::ensure!(start.elapsed() < Duration::from_secs(15), "adapter {} did not start within 15s", cfg.id);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    /// The running process for `id`, started (or restarted) if needed.
    pub async fn get(&self, id: &str) -> Result<Arc<Running>> {
        if let Some(r) = self.running.read().await.get(id) {
            if r.child.lock().unwrap().try_wait().ok().flatten().is_none() {
                return Ok(r.clone());
            }
        }
        let cfg = self.configs.read().await.get(id).cloned().ok_or_else(|| anyhow!("no adapter named {id}"))?;
        let mut w = self.running.write().await;
        if let Some(r) = w.get(id) {
            if r.child.lock().unwrap().try_wait().ok().flatten().is_none() {
                return Ok(r.clone());
            }
        }
        let r = self.launch(&cfg).await?;
        w.insert(id.to_string(), r.clone());
        Ok(r)
    }

    /// Replace or add an adapter without restarting the kernel. The new process
    /// must pass the contract handshake before it takes any request.
    pub async fn replace(&self, cfg: config::Adapter) -> Result<(String, String)> {
        let r = self.launch(&cfg).await?;
        let info = (r.adapter.clone(), r.version.clone());
        self.configs.write().await.insert(cfg.id.clone(), cfg.clone());
        let old = self.running.write().await.insert(cfg.id.clone(), r);
        if let Some(old) = old {
            // Let in-flight requests on the old socket finish, then stop it.
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(600)).await;
                drop(old);
            });
        }
        Ok(info)
    }

    pub async fn list(&self) -> Vec<(String, Vec<String>, Option<(String, String)>)> {
        let cfgs = self.configs.read().await.clone();
        let running = self.running.read().await;
        let mut v: Vec<_> = cfgs
            .values()
            .map(|c| (c.id.clone(), c.command.clone(), running.get(&c.id).map(|r| (r.adapter.clone(), r.version.clone()))))
            .collect();
        v.sort();
        v
    }

    fn split_settings(cfg: &config::Adapter) -> (Value, Option<String>) {
        let mut s = cfg.settings.clone();
        let cred = s.remove("credential").and_then(|v| v.as_str().map(String::from));
        (Value::Object(s), cred)
    }

    fn credential(&self, name: &Option<String>) -> Option<String> {
        name.as_ref().and_then(|n| self.vault.get(n).ok().flatten()).map(|s| s.expose().to_string())
    }

    fn store_refreshed(&self, name: &Option<String>, value: &str) {
        if let Some(n) = name {
            if let Err(e) = self.vault.put(n, value) {
                tracing::error!("could not store a refreshed credential for {n}: {e}");
            }
        }
    }

    pub async fn complete(
        &self,
        id: &str,
        model: &str,
        system: &str,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        max_tokens: u32,
        mut on_text: impl FnMut(&str),
    ) -> Result<TurnOut, AdapterError> {
        let r = self.get(id).await.map_err(|e| AdapterError::unavailable(format!("adapter {id}: {e:#}")))?;
        let (settings, cred_name) = Self::split_settings(&r.cfg);
        let credential = self.credential(&cred_name);
        let params = CompleteParams {
            model: model.into(),
            settings,
            credential,
            system: system.into(),
            messages,
            tools,
            max_tokens,
        };
        let mut call = proto::Call::open(&r.socket, Request::Complete(params))
            .await
            .map_err(|e| AdapterError::unavailable(format!("adapter {id} unreachable: {e}")))?;
        let mut text = String::new();
        let mut calls = vec![];
        let (mut state, mut usage, mut served) = (None, None, None);
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(600), call.next())
                .await
                .map_err(|_| AdapterError::unavailable("adapter stream stalled for 10 minutes"))?
                .map_err(|e| AdapterError { class: ErrorClass::Malformed, message: format!("adapter stream: {e}"), retry_after_s: None })?;
            match ev {
                None => return Err(AdapterError::unavailable(format!("adapter {id} closed the stream early"))),
                Some(Event::Text { text: t }) => {
                    on_text(&t);
                    text.push_str(&t);
                }
                Some(Event::ToolCall { id, name, input }) => calls.push(Block::ToolCall { id, name, input }),
                Some(Event::State { state: s }) => state = Some(s),
                Some(Event::Usage(u)) => usage = Some(u),
                Some(Event::ServedModel { id }) => served = Some(id),
                Some(Event::Credential { value }) => self.store_refreshed(&cred_name, &value),
                Some(Event::Error { class, message, retry_after_s }) => return Err(AdapterError { class, message, retry_after_s }),
                Some(Event::Done { stop }) => {
                    let mut blocks = vec![];
                    if !text.is_empty() {
                        blocks.push(Block::Text { text });
                    }
                    blocks.extend(calls);
                    let served_model = served.ok_or_else(|| AdapterError {
                        class: ErrorClass::Malformed,
                        message: "the adapter did not report the served model".into(),
                        retry_after_s: None,
                    })?;
                    return Ok(TurnOut {
                        blocks,
                        state,
                        usage: usage.unwrap_or(Usage { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, cost_usd: 0.0 }),
                        served_model,
                        stop,
                    });
                }
                Some(other) => tracing::warn!("adapter {id} sent an unexpected event: {other:?}"),
            }
        }
    }

    /// Start a sign-in for adapter `id`: returns what the person must do.
    pub async fn auth_begin(&self, id: &str) -> Result<Event> {
        let r = self.get(id).await?;
        let (settings, _) = Self::split_settings(&r.cfg);
        let mut call = proto::Call::open(&r.socket, Request::AuthBegin { settings }).await?;
        call.next().await?.ok_or_else(|| anyhow!("adapter closed the connection"))
    }

    /// Finish a sign-in; on success the credential goes straight to the vault.
    pub async fn auth_complete(&self, id: &str, flow: Value, input: Value) -> Result<std::result::Result<(), String>> {
        let r = self.get(id).await?;
        let (settings, cred_name) = Self::split_settings(&r.cfg);
        let name = cred_name.ok_or_else(|| anyhow!("adapter {id} has no credential name in its settings"))?;
        let mut call = proto::Call::open(&r.socket, Request::AuthComplete { settings, flow, input }).await?;
        match call.next().await? {
            Some(Event::AuthDone { credential }) => {
                self.vault.put(&name, &credential)?;
                Ok(Ok(()))
            }
            Some(Event::Error { message, .. }) => Ok(Err(message)),
            other => Ok(Err(format!("unexpected answer: {other:?}"))),
        }
    }

    pub async fn catalog(&self, id: &str) -> Result<Event> {
        let r = self.get(id).await?;
        let (settings, cred_name) = Self::split_settings(&r.cfg);
        let credential = self.credential(&cred_name);
        let mut call = proto::Call::open(&r.socket, Request::Catalog { settings, credential }).await?;
        call.next().await?.ok_or_else(|| anyhow!("adapter closed the connection"))
    }
}
