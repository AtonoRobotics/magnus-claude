//! The appliance configuration file (TOML), written by the installer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub appliance: Appliance,
    pub database: Database,
    pub model: Model,
    #[serde(default)]
    pub adapters: Vec<Adapter>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub verifier: Verifier,
    #[serde(default)]
    pub web: Web,
    #[serde(default)]
    pub sandbox: Option<Sandbox>,
    #[serde(default)]
    pub browser: Option<Browser>,
    /// Local speech to text for voice input (`config/install-whisper.sh`).
    #[serde(default)]
    pub speech: Option<Speech>,
    #[serde(default)]
    pub people: Vec<Person>,
    #[serde(default)]
    pub connectors: Vec<Connector>,
    #[serde(default)]
    pub sign_in: SignIn,
    #[serde(default)]
    pub observability: Observability,
}

/// Developer-view settings (K51).
#[derive(Deserialize, Clone, Debug)]
pub struct Observability {
    /// A running run with no new event for this long counts as stalled (V3).
    #[serde(default = "stall_minutes")]
    pub stall_minutes: u32,
}

impl Default for Observability {
    fn default() -> Self {
        Observability { stall_minutes: stall_minutes() }
    }
}

fn stall_minutes() -> u32 {
    15
}

#[derive(Deserialize, Clone, Debug)]
pub struct Appliance {
    pub state_dir: PathBuf,
    /// Plain HTTP. Passwords are refused here except from this machine.
    pub listen: String,
    /// HTTPS for people's apps on the network. Without `tls_cert` and
    /// `tls_key`, the appliance makes its own certificate on first start and
    /// apps pin it on first connection.
    #[serde(default)]
    pub tls_listen: Option<String>,
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,
    #[serde(default)]
    pub tls_key: Option<PathBuf>,
}

/// Sign-in for people's apps. Tunable values, not decisions (K51).
#[derive(Deserialize, Clone, Debug)]
pub struct SignIn {
    #[serde(default = "session_days")]
    pub session_days: u32,
    #[serde(default = "max_failures")]
    pub max_failures: u32,
    #[serde(default = "lockout_minutes")]
    pub lockout_minutes: u32,
    #[serde(default = "min_password_length")]
    pub min_password_length: usize,
    /// Sign in through an identity provider instead of passwords.
    #[serde(default)]
    pub oidc: Option<Oidc>,
}

impl Default for SignIn {
    fn default() -> Self {
        SignIn { session_days: session_days(), max_failures: max_failures(), lockout_minutes: lockout_minutes(), min_password_length: min_password_length(), oidc: None }
    }
}

/// An OpenID Connect provider (Keycloak, for one). People approve the
/// sign-in on its own page (the device-code grant, RFC 8628); its endpoints
/// come from its discovery document.
#[derive(Deserialize, Clone, Debug)]
pub struct Oidc {
    /// For example `https://identity.example.com/realms/magnus`.
    pub issuer: String,
    /// A public client with the device-code grant turned on.
    pub client_id: String,
    /// The ID token claim that holds a person's id on this appliance.
    #[serde(default = "person_claim")]
    pub person_claim: String,
    #[serde(default = "oidc_scope")]
    pub scope: String,
    /// What the sign-in button names; the issuer's host when unset.
    #[serde(default)]
    pub label: Option<String>,
    /// Sign-ins waiting for approval at once. Starting one needs no
    /// credential, so this bounds what strangers can make the appliance hold.
    #[serde(default = "max_pending")]
    pub max_pending: usize,
}

fn person_claim() -> String {
    "preferred_username".into()
}
fn oidc_scope() -> String {
    "openid".into()
}
fn max_pending() -> usize {
    100
}

fn session_days() -> u32 {
    30
}
fn max_failures() -> u32 {
    5
}
fn lockout_minutes() -> u32 {
    15
}
fn min_password_length() -> usize {
    10
}

#[derive(Deserialize, Clone, Debug)]
pub struct Database {
    /// The kernel's own role: INSERT and SELECT on the log, nothing else.
    pub url: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Model {
    pub adapter: String,
    pub name: String,
    #[serde(default = "default_retries")]
    pub max_retries: u32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_retries() -> u32 {
    3
}

fn default_max_tokens() -> u32 {
    16000
}

#[derive(Deserialize, Clone, Debug)]
pub struct Adapter {
    pub id: String,
    pub command: Vec<String>,
    /// Provider settings passed to the adapter. `credential` names the vault
    /// secret the kernel resolves and passes along; it is not sent itself.
    #[serde(default)]
    pub settings: serde_json::Map<String, Value>,
}

#[derive(Deserialize, Clone, Debug, serde::Serialize)]
pub struct Budget {
    pub max_model_calls: u32,
    pub max_actions: u32,
    pub max_usd: f64,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { max_model_calls: 300, max_actions: 500, max_usd: 50.0 }
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Verifier {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Web {
    /// Allow fetching loopback and private addresses. Off on a real appliance.
    #[serde(default)]
    pub allow_private_networks: bool,
    /// Optional search endpoint (SearXNG JSON API).
    #[serde(default)]
    pub search_url: Option<String>,
}

/// The browser sidecar: Chromium on the host, behind the boundary.
#[derive(Deserialize, Clone, Debug)]
pub struct Browser {
    pub command: Vec<String>,
    pub chromium: PathBuf,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Speech {
    /// whisper.cpp's `whisper-cli`.
    pub whisper: PathBuf,
    /// A ggml model file.
    pub model: PathBuf,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Sandbox {
    pub firecracker: PathBuf,
    pub kernel_image: PathBuf,
    pub rootfs: PathBuf,
    #[serde(default)]
    pub jailer: Option<PathBuf>,
    #[serde(default = "one")]
    pub vcpus: u32,
    #[serde(default = "mem")]
    pub memory_mib: u32,
    #[serde(default = "disk")]
    pub workspace_mib: u32,
}

fn one() -> u32 {
    1
}
fn mem() -> u32 {
    1024
}
fn disk() -> u32 {
    2048
}

#[derive(Deserialize, Clone, Debug)]
pub struct Person {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// An API token for scripts; people's apps sign in (provider or password) instead.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

/// One of the person's apps, reached over MCP: from this file, or added by
/// the owner in the app (`Ev::ConnectorSet`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Connector {
    pub name: String,
    /// What the app shows; the name when absent.
    #[serde(default)]
    pub title: Option<String>,
    /// `mcp_http` (streamable HTTP at `url`) or `mcp_stdio` (`command`, run
    /// on the appliance).
    #[serde(default = "mcp_http")]
    pub transport: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub command: Vec<String>,
    /// A vault entry shared by everyone who uses the app.
    #[serde(default)]
    pub credential: Option<String>,
    /// How each person signs in to the app; their tokens go to the vault.
    #[serde(default)]
    pub auth: Option<ConnectorAuth>,
    pub audience: String,
    pub label: String,
    #[serde(default)]
    pub access_groups: Vec<String>,
    #[serde(default = "header")]
    pub idempotency: String,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolClass>,
}

/// A person's sign-in to an app.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConnectorAuth {
    /// `device` (RFC 8628), `loopback` (authorization code with PKCE,
    /// redirected to the person's own machine), or `api_key`.
    pub kind: String,
    #[serde(default)]
    pub client_id: String,
    /// The vault entry of a client secret, for providers whose desktop
    /// clients carry one.
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub device_authorization_endpoint: Option<String>,
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Extra parameters for the authorization request (`access_type`, …).
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

fn mcp_http() -> String {
    "mcp_http".into()
}
/// Apps are assumed not to deduplicate unless the owner says they honor an
/// `Idempotency-Key` header, so a crash never causes a blind resend.
fn header() -> String {
    "none".into()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolClass {
    pub risk: String,
    /// Overrides the app's inbound label for this tool's results.
    #[serde(default)]
    pub label: Option<String>,
    /// The input field holding the text of a message this tool sends; the
    /// kernel signs it as Magnus, for the person (E6).
    #[serde(default)]
    pub sends: Option<String>,
}

impl ToolClass {
    pub fn validate(&self, what: &str) -> Result<(), String> {
        if crate::tools::Risk::parse(&self.risk).is_none() {
            return Err(format!("{what}: unknown risk class {}", self.risk));
        }
        if let Some(l) = &self.label {
            if crate::tools::Label::parse(l).is_none() {
                return Err(format!("{what}: unknown label {l}"));
            }
        }
        if self.sends.as_ref().is_some_and(|f| f.trim().is_empty()) {
            return Err(format!("{what}: sends names no input field"));
        }
        Ok(())
    }
}

/// An endpoint the appliance talks to: https, or http on this machine.
pub fn endpoint_ok(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    let local = matches!(u.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback()) || matches!(u.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback()) || u.host_str() == Some("localhost");
    u.scheme() == "https" || (u.scheme() == "http" && local)
}

impl Connector {
    pub fn title(&self) -> String {
        self.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| self.name.clone())
    }

    /// Whether each person signs in for themselves.
    pub fn per_person(&self) -> bool {
        self.auth.is_some()
    }

    pub fn validate(&self) -> Result<(), String> {
        let c = self;
        if c.name.is_empty() || c.name.contains("__") || !c.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
            return Err(format!("connector name {:?} must be letters, digits, '-' or '_' and must not contain '__'", c.name));
        }
        match c.transport.as_str() {
            "mcp_http" if !(c.url.starts_with("http://") || c.url.starts_with("https://")) || url::Url::parse(&c.url).is_err() => {
                return Err(format!("connector {}: url must be an http or https address", c.name))
            }
            "mcp_stdio" if c.command.is_empty() || c.command[0].trim().is_empty() => return Err(format!("connector {}: mcp_stdio needs a command", c.name)),
            "mcp_http" | "mcp_stdio" => {}
            other => return Err(format!("connector {}: unknown transport {other}", c.name)),
        }
        for (t, class) in &c.tools {
            class.validate(&format!("tool {}.{t}", c.name))?;
        }
        if crate::tools::Label::parse(&c.label).is_none() {
            return Err(format!("connector {}: unknown label {}", c.name, c.label));
        }
        if crate::tools::Audience::parse(&c.audience).is_none() {
            return Err(format!("connector {}: unknown audience {}", c.name, c.audience));
        }
        if let Some(a) = &c.auth {
            if c.credential.is_some() {
                return Err(format!("connector {}: a shared credential and per-person sign-in exclude each other", c.name));
            }
            let need = |v: &Option<String>, what: &str| match v {
                Some(u) if endpoint_ok(u) => Ok(()),
                Some(u) => Err(format!("connector {}: {what} {u} must be https", c.name)),
                None => Err(format!("connector {}: {} sign-in needs {what}", c.name, a.kind)),
            };
            match a.kind.as_str() {
                "device" => {
                    need(&a.device_authorization_endpoint, "device_authorization_endpoint")?;
                    need(&a.token_endpoint, "token_endpoint")?;
                }
                "loopback" => {
                    need(&a.authorization_endpoint, "authorization_endpoint")?;
                    need(&a.token_endpoint, "token_endpoint")?;
                }
                "api_key" => {}
                other => return Err(format!("connector {}: unknown sign-in kind {other} (device, loopback or api_key)", c.name)),
            }
            if a.kind != "api_key" && a.client_id.trim().is_empty() {
                return Err(format!("connector {}: {} sign-in needs the client ID of the app registered with the provider", c.name, a.kind));
            }
        }
        Ok(())
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&text)?;
        for c in &cfg.connectors {
            c.validate().map_err(|e| anyhow::anyhow!(e))?;
        }
        Ok(cfg)
    }

    pub fn person(&self, id: &str) -> Option<&Person> {
        self.people.iter().find(|p| p.id == id)
    }

    pub fn person_by_token(&self, token: &str) -> Option<&Person> {
        use sha2::Digest;
        // Compare digests so the comparison time does not depend on the token.
        let want = sha2::Sha256::digest(token.as_bytes());
        self.people.iter().find(|p| p.token.as_ref().is_some_and(|t| sha2::Sha256::digest(t.as_bytes()) == want))
    }
}

impl Person {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }
    pub fn is_owner(&self) -> bool {
        self.has_role("owner")
    }
    /// The developer view: raw log, runs, experiments (K10, K45).
    pub fn is_developer(&self) -> bool {
        self.has_role("developer")
    }
}
