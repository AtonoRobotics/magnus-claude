//! Clients for the person's apps over MCP: streamable HTTP, or stdio for a
//! server the appliance runs itself.
//!
//! Only the boundary calls `call`, after every check, with the credential it
//! resolved from the vault. The kernel lists tools to pin their definitions.

use crate::config::Connector;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL: &str = "2025-06-18";

#[derive(Clone, Debug)]
pub struct AppTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug)]
pub enum CallError {
    /// The request never left: nothing happened at the app.
    NotSent(String),
    /// The request may have reached the app; its outcome is unknown.
    Unknown(String),
}

pub struct CallOutput {
    pub text: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

#[derive(Clone)]
pub struct Mcp {
    http: reqwest::Client,
    /// One running server per stdio connector, started on first use.
    stdio: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<Stdio>>>>>>,
    /// Where stdio servers get a home directory of their own.
    home: PathBuf,
}

/// A stdio server: newline-delimited JSON-RPC on its stdin and stdout.
struct Stdio {
    command: Vec<String>,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    next: u64,
}

const STDIO_TIMEOUT: Duration = Duration::from_secs(120);

struct Session {
    url: String,
    bearer: Option<String>,
    id: Option<String>,
}

impl Mcp {
    pub fn new(state_dir: &std::path::Path) -> Mcp {
        Mcp {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .build()
                .expect("http client"),
            stdio: Default::default(),
            home: state_dir.join("connectors"),
        }
    }

    /// Start a stdio server with nothing of the appliance's environment but
    /// PATH, in a home directory of its own, and introduce ourselves.
    async fn spawn(&self, c: &Connector) -> Result<Stdio, CallError> {
        let home = self.home.join(&c.name);
        std::fs::create_dir_all(&home).map_err(|e| CallError::NotSent(format!("could not make {}: {e}", home.display())))?;
        let mut cmd = tokio::process::Command::new(&c.command[0]);
        cmd.args(&c.command[1..])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &home)
            .current_dir(&home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| CallError::NotSent(format!("could not start {}: {e}", c.command[0])))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped")).lines();
        let mut p = Stdio { command: c.command.clone(), child, stdin, stdout, next: 1 };
        let init = json!({ "protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": { "name": "magnus", "version": env!("CARGO_PKG_VERSION") } });
        p.request("initialize", init).await.map_err(|e| CallError::NotSent(format!("{} did not start: {}", c.name, match e { CallError::NotSent(m) | CallError::Unknown(m) => m })))?;
        p.notify("notifications/initialized").await.map_err(|_| CallError::NotSent(format!("{} stopped while starting", c.name)))?;
        Ok(p)
    }

    /// Send one request to a stdio connector's server, starting it (again)
    /// when it is not running. A request that may have reached it is never
    /// resent.
    async fn stdio_rpc(&self, c: &Connector, method: &str, params: Value) -> Result<Value, CallError> {
        let slot = self.stdio.lock().unwrap().entry(c.name.clone()).or_default().clone();
        let mut g = slot.lock().await;
        let alive = g.as_mut().is_some_and(|p| p.command == c.command && p.child.try_wait().ok().flatten().is_none());
        if !alive {
            *g = Some(self.spawn(c).await?);
        }
        let p = g.as_mut().expect("started");
        let r = p.request(method, params).await;
        if matches!(r, Err(CallError::Unknown(_))) {
            // Its state is unknown: the next request starts it afresh.
            *g = None;
        }
        r
    }

    async fn rpc(&self, s: &Session, id: Option<u64>, method: &str, params: Value, extra: &[(&str, &str)]) -> Result<Option<Value>, CallError> {
        let mut body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if let Some(id) = id {
            body["id"] = json!(id);
        }
        let mut req = self
            .http
            .post(&s.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", PROTOCOL)
            .json(&body);
        if let Some(b) = &s.bearer {
            req = req.bearer_auth(b);
        }
        if let Some(sid) = &s.id {
            req = req.header("mcp-session-id", sid);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_connect() || e.is_builder() {
                CallError::NotSent(format!("could not connect: {e}"))
            } else {
                CallError::Unknown(format!("request failed after sending: {e}"))
            }
        })?;
        let status = resp.status();
        if id.is_none() {
            return Ok(None);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            // An HTTP-level rejection means the server did not run the tool.
            return Err(CallError::NotSent(format!("app returned HTTP {status}: {}", text.chars().take(300).collect::<String>())));
        }
        let sse = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.starts_with("text/event-stream"))
            .unwrap_or(false);
        let text = resp.text().await.map_err(|e| CallError::Unknown(format!("response lost: {e}")))?;
        let msg: Value = if sse {
            text.split("\n\n")
                .filter_map(|frame| {
                    let data: String = frame.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim).collect();
                    serde_json::from_str::<Value>(&data).ok()
                })
                .find(|v| v.get("id") == Some(&json!(id)))
                .ok_or_else(|| CallError::Unknown("no response in event stream".into()))?
        } else {
            serde_json::from_str(&text).map_err(|_| CallError::Unknown("response is not JSON".into()))?
        };
        if let Some(err) = msg.get("error") {
            return Err(CallError::NotSent(format!("app error: {}", err["message"].as_str().unwrap_or("unknown"))));
        }
        Ok(Some(msg["result"].clone()))
    }

    async fn open(&self, url: &str, bearer: Option<&str>) -> Result<Session, CallError> {
        let mut s = Session { url: url.into(), bearer: bearer.map(String::from), id: None };
        let mut req = self
            .http
            .post(url)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": { "protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": { "name": "magnus", "version": env!("CARGO_PKG_VERSION") } }
            }));
        if let Some(b) = bearer {
            req = req.bearer_auth(b);
        }
        let resp = req.send().await.map_err(|e| CallError::NotSent(format!("could not connect: {e}")))?;
        if !resp.status().is_success() {
            return Err(CallError::NotSent(format!("app refused initialize: HTTP {}", resp.status())));
        }
        s.id = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(String::from);
        let _ = resp.text().await;
        let _ = self.rpc(&s, None, "notifications/initialized", json!({}), &[]).await;
        Ok(s)
    }

    /// Everything an app returns passes here before anyone sees it, so an app
    /// that echoes the credential it was called with leaks it nowhere.
    fn clean(s: &str, bearer: Option<&str>) -> String {
        match bearer {
            Some(b) => crate::vault::redact(s, b),
            None => s.to_string(),
        }
    }

    fn clean_value(v: Value, bearer: Option<&str>) -> Value {
        match bearer {
            Some(b) => serde_json::from_str(&crate::vault::redact(&v.to_string(), b)).unwrap_or(Value::Null),
            None => v,
        }
    }

    fn clean_err(e: CallError, bearer: Option<&str>) -> CallError {
        match e {
            CallError::NotSent(m) => CallError::NotSent(Self::clean(&m, bearer)),
            CallError::Unknown(m) => CallError::Unknown(Self::clean(&m, bearer)),
        }
    }

    pub async fn list_tools(&self, c: &Connector, bearer: Option<&str>) -> Result<Vec<AppTool>> {
        let tools = self.list_tools_raw(c, bearer).await.map_err(|e| anyhow!("{}", Self::clean(&e.to_string(), bearer)))?;
        Ok(tools
            .into_iter()
            .map(|t| AppTool {
                name: Self::clean(&t.name, bearer),
                description: Self::clean(&t.description, bearer),
                input_schema: Self::clean_value(t.input_schema, bearer),
            })
            .collect())
    }

    async fn list_tools_raw(&self, c: &Connector, bearer: Option<&str>) -> Result<Vec<AppTool>> {
        let r = if c.transport == "mcp_stdio" {
            self.stdio_rpc(c, "tools/list", json!({})).await.map_err(|e| anyhow!("{e:?}"))?
        } else {
            let s = self.open(&c.url, bearer).await.map_err(|e| anyhow!("{e:?}"))?;
            self.rpc(&s, Some(1), "tools/list", json!({}), &[]).await.map_err(|e| anyhow!("{e:?}"))?.unwrap_or_default()
        };
        Ok(r["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| AppTool {
                name: t["name"].as_str().unwrap_or_default().into(),
                description: t["description"].as_str().unwrap_or_default().into(),
                input_schema: t.get("inputSchema").cloned().unwrap_or(json!({ "type": "object" })),
            })
            .collect())
    }

    pub async fn call(&self, c: &Connector, bearer: Option<&str>, tool: &str, args: &Value, key: Option<&str>) -> Result<CallOutput, CallError> {
        match self.call_raw(c, bearer, tool, args, key).await {
            Ok(o) => Ok(CallOutput {
                text: Self::clean(&o.text, bearer),
                structured: o.structured.map(|v| Self::clean_value(v, bearer)),
                is_error: o.is_error,
            }),
            Err(e) => Err(Self::clean_err(e, bearer)),
        }
    }

    async fn call_raw(&self, c: &Connector, bearer: Option<&str>, tool: &str, args: &Value, key: Option<&str>) -> Result<CallOutput, CallError> {
        let mut params = json!({ "name": tool, "arguments": args });
        let mut extra = vec![];
        if let Some(k) = key {
            params["_meta"] = json!({ "io.magnus/idempotency-key": k });
            extra.push(("idempotency-key", k));
        }
        let r = if c.transport == "mcp_stdio" {
            self.stdio_rpc(c, "tools/call", params).await?
        } else {
            let s = self.open(&c.url, bearer).await?;
            self.rpc(&s, Some(2), "tools/call", params, &extra).await?.unwrap_or_default()
        };
        let text = r["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(CallOutput {
            text,
            structured: r.get("structuredContent").cloned().filter(|v| !v.is_null()),
            is_error: r["isError"].as_bool().unwrap_or(false),
        })
    }
}

/// Hash of a tool definition, for pinning (B10).
pub fn definition_hash(t: &AppTool) -> String {
    use sha2::Digest;
    let canon = crate::boundary::canonical(&json!({ "name": t.name, "description": t.description, "input_schema": t.input_schema }));
    hex::encode(sha2::Sha256::digest(canon.as_bytes()))
}

impl Stdio {
    async fn send(&mut self, msg: &Value) -> Result<(), CallError> {
        let mut line = serde_json::to_vec(msg).map_err(|e| CallError::NotSent(e.to_string()))?;
        line.push(b'\n');
        self.stdin.write_all(&line).await.map_err(|e| CallError::NotSent(format!("the server is not running: {e}")))?;
        self.stdin.flush().await.map_err(|e| CallError::Unknown(format!("the request may not have reached the server: {e}")))
    }

    async fn notify(&mut self, method: &str) -> Result<(), CallError> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method })).await
    }

    /// One request and its answer; lines that are not that answer (the
    /// server's notifications, its requests to us) are skipped.
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.next;
        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await?;
        let answer = tokio::time::timeout(STDIO_TIMEOUT, async {
            loop {
                match self.stdout.next_line().await {
                    Ok(Some(line)) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&line) {
                            if v.get("id") == Some(&json!(id)) && (v.get("result").is_some() || v.get("error").is_some()) {
                                return Ok(v);
                            }
                        }
                    }
                    Ok(None) => return Err(CallError::Unknown("the server stopped before answering".into())),
                    Err(e) => return Err(CallError::Unknown(format!("reading the answer failed: {e}"))),
                }
            }
        })
        .await
        .map_err(|_| CallError::Unknown("the server did not answer in time".into()))??;
        if let Some(err) = answer.get("error") {
            return Err(CallError::NotSent(format!("app error: {}", err["message"].as_str().unwrap_or("unknown"))));
        }
        Ok(answer["result"].clone())
    }
}
