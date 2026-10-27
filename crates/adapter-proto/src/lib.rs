//! The model adapter contract, `magnus.adapter/1`.
//!
//! An adapter is a separate process that owns everything provider-specific:
//! authentication (API keys, OAuth link flows, token refresh), the model
//! catalog, request and stream translation, error classification and cost.
//! The kernel speaks only this contract.
//!
//! Transport: the adapter listens on a Unix socket given as `--socket <path>`.
//! Each request is one connection: the kernel writes one JSON line
//! `{"contract": "magnus.adapter/1", "method": ..., "params": ...}` and reads
//! JSON event lines until `done`, `error`, or a method's single result.
//! A connection per request lets the kernel move new requests to a replaced
//! adapter without restarting itself.
//!
//! Credentials: the kernel resolves the adapter's credential from the vault and
//! passes it in `credential`. An adapter that refreshes a token returns the
//! new value in a `credential` event; the kernel stores it in the vault. The
//! kernel never logs either.
//!
//! State: each assistant turn may carry an opaque `state` value (signed
//! thinking, encrypted reasoning, a response id). The kernel stores it and
//! sends it back with that turn, and never reads it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub const CONTRACT: &str = "magnus.adapter/1";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Envelope {
    pub contract: String,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    /// Identify the adapter. Answered with one `hello` event.
    Hello {},
    /// List models. Answered with one `catalog` event or an `error`.
    Catalog {
        settings: Value,
        credential: Option<String>,
    },
    /// Run one model turn. Answered with a stream ending in `done` or `error`.
    Complete(CompleteParams),
    /// Start a sign-in. Answered with an `auth_step` event or an `error`.
    AuthBegin { settings: Value },
    /// Finish a sign-in with what the person brought back (callback URL, code).
    /// Answered with `auth_done` carrying the credential to store, or `error`.
    AuthComplete {
        settings: Value,
        flow: Value,
        input: Value,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CompleteParams {
    pub model: String,
    /// Adapter configuration from the owner, minus the credential name.
    pub settings: Value,
    pub credential: Option<String>,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
    /// Opaque provider state from the adapter that produced this assistant turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text { text: String },
    Image { media_type: String, data_b64: String },
    Document { media_type: String, data_b64: String, name: String },
    ToolCall { id: String, name: String, input: Value },
    ToolResult { call_id: String, content: Vec<Block>, is_error: bool },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// The provider could not serve the request (connection failure, 5xx,
    /// overload). A retry may help.
    Unavailable,
    /// The provider's response could not be understood.
    Malformed,
    /// A rate, quota or subscription limit. `retry_after_s` says when, if known.
    UsageLimit,
    /// The credential is missing, invalid or expired and cannot be refreshed.
    /// The person must sign in again.
    AuthExpired,
    /// The provider rejected the request itself (for example, context too long).
    InvalidRequest,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Cost of this turn in US dollars, from the adapter's price list.
    pub cost_usd: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    /// True when the id names one fixed model version that will not change.
    pub fixed_version: bool,
    #[serde(default)]
    pub context_window: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Hello {
        contract: String,
        adapter: String,
        version: String,
    },
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    Usage(Usage),
    /// The model id the provider reports actually serving this turn.
    ServedModel {
        id: String,
    },
    State {
        state: Value,
    },
    /// A refreshed credential for the vault.
    Credential {
        value: String,
    },
    Done {
        stop: String,
    },
    Error {
        class: ErrorClass,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_s: Option<f64>,
    },
    Catalog {
        models: Vec<ModelInfo>,
    },
    /// What the person must do to sign in: open `url`, or enter `user_code`.
    /// `flow` is opaque and comes back in `auth_complete`.
    AuthStep {
        flow: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_code: Option<String>,
        instructions: String,
    },
    AuthDone {
        credential: String,
    },
}

/// Send events back to the kernel on one request's connection.
pub struct Sink {
    w: tokio::net::unix::OwnedWriteHalf,
}

impl Sink {
    pub async fn send(&mut self, e: &Event) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(e)?;
        line.push(b'\n');
        self.w.write_all(&line).await?;
        Ok(())
    }

    pub async fn error(&mut self, class: ErrorClass, message: impl Into<String>) -> anyhow::Result<()> {
        self.send(&Event::Error { class, message: message.into(), retry_after_s: None }).await
    }
}

/// Run an adapter: listen on `socket` and hand each request to `handle`.
pub async fn serve<H, F>(socket: &Path, handle: H) -> anyhow::Result<()>
where
    H: Fn(Request, Sink) -> F + Clone + Send + 'static,
    F: std::future::Future<Output = anyhow::Result<()>> + Send,
{
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let handle = handle.clone();
        tokio::spawn(async move {
            let (r, w) = stream.into_split();
            let mut sink = Sink { w };
            let mut line = String::new();
            if BufReader::new(r).read_line(&mut line).await.is_err() {
                return;
            }
            match serde_json::from_str::<Envelope>(&line) {
                Ok(env) if env.contract == CONTRACT => {
                    if let Err(e) = handle(env.request, sink).await {
                        eprintln!("adapter request failed: {e:#}");
                    }
                }
                Ok(env) => {
                    let _ = sink
                        .error(ErrorClass::InvalidRequest, format!("unsupported contract {}", env.contract))
                        .await;
                }
                Err(e) => {
                    let _ = sink.error(ErrorClass::InvalidRequest, format!("bad request: {e}")).await;
                }
            }
        });
    }
}

/// The socket path from `--socket <path>` on the command line.
pub fn socket_arg() -> anyhow::Result<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--socket")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: {} --socket <path>", args[0]))
}

/// Kernel side: send one request and receive its events.
pub struct Call {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    _w: tokio::net::unix::OwnedWriteHalf,
}

impl Call {
    pub async fn open(socket: &Path, request: Request) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket).await?;
        let (r, mut w) = stream.into_split();
        let mut line = serde_json::to_vec(&Envelope { contract: CONTRACT.into(), request })?;
        line.push(b'\n');
        w.write_all(&line).await?;
        Ok(Call { lines: BufReader::new(r).lines(), _w: w })
    }

    /// The next event, or `None` when the adapter closed the connection.
    pub async fn next(&mut self) -> anyhow::Result<Option<Event>> {
        match self.lines.next_line().await? {
            Some(l) => Ok(Some(serde_json::from_str(&l)?)),
            None => Ok(None),
        }
    }
}
