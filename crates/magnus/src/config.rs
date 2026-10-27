//! The appliance configuration file (TOML), written by the installer.

use serde::Deserialize;
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
    pub people: Vec<Person>,
    #[serde(default)]
    pub connectors: Vec<Connector>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Appliance {
    pub state_dir: PathBuf,
    pub listen: String,
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
    pub token: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Connector {
    pub name: String,
    #[serde(default = "mcp_http")]
    pub transport: String,
    pub url: String,
    #[serde(default)]
    pub credential: Option<String>,
    pub audience: String,
    pub label: String,
    #[serde(default)]
    pub access_groups: Vec<String>,
    #[serde(default = "header")]
    pub idempotency: String,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolClass>,
}

fn mcp_http() -> String {
    "mcp_http".into()
}
/// Apps are assumed not to deduplicate unless the owner says they honor an
/// `Idempotency-Key` header, so a crash never causes a blind resend.
fn header() -> String {
    "none".into()
}

#[derive(Deserialize, Clone, Debug)]
pub struct ToolClass {
    pub risk: String,
    /// Overrides the app's inbound label for this tool's results.
    #[serde(default)]
    pub label: Option<String>,
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&text)?;
        for c in &cfg.connectors {
            anyhow::ensure!(
                !c.name.contains("__") && c.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'),
                "connector name {:?} must be letters, digits, '-' or '_' and must not contain '__'",
                c.name
            );
            for (t, class) in &c.tools {
                anyhow::ensure!(crate::tools::Risk::parse(&class.risk).is_some(), "tool {}.{t}: unknown risk class {}", c.name, class.risk);
            }
            anyhow::ensure!(crate::tools::Label::parse(&c.label).is_some(), "connector {}: unknown label {}", c.name, c.label);
            anyhow::ensure!(crate::tools::Audience::parse(&c.audience).is_some(), "connector {}: unknown audience {}", c.name, c.audience);
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
        self.people.iter().find(|p| sha2::Sha256::digest(p.token.as_bytes()) == want)
    }
}

impl Person {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }
    pub fn is_owner(&self) -> bool {
        self.has_role("owner")
    }
}
