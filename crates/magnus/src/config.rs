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
    pub backup: Backup,
    #[serde(default)]
    pub updates: Updates,
    /// The file this was read from: a new release is checked against it.
    #[serde(skip)]
    pub path: PathBuf,
    /// Removable modules, on or off for every model, and per model.
    #[serde(default)]
    pub modules: Modules,
    #[serde(default)]
    pub web: Web,
    #[serde(default)]
    pub sandbox: Option<Sandbox>,
    #[serde(default)]
    pub browser: Option<Browser>,
    /// Local speech to text for voice input (`config/install-whisper.sh`).
    #[serde(default)]
    pub speech: Option<Speech>,
    /// The read-aloud voice, run on the appliance (`config/install-voice.sh`).
    #[serde(default)]
    pub read_aloud: Option<ReadAloud>,
    #[serde(default)]
    pub knowledge: Knowledge,
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
    /// Where the ID token carries the person's groups (a JSON pointer, for
    /// Keycloak `/groups`). When set, the provider's groups replace the
    /// config's at each sign-in (E2).
    #[serde(default)]
    pub groups_claim: Option<String>,
    /// Where it carries their roles (Keycloak realm roles: `/realm_access/roles`).
    #[serde(default)]
    pub roles_claim: Option<String>,
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

/// Backups of the event log and the vault (K10).
#[derive(Deserialize, Clone, Debug)]
pub struct Backup {
    /// Where backups are written; `<state_dir>/backups` unless set. Set it
    /// to another disk: a backup on the appliance's own disk is lost with it.
    #[serde(default)]
    pub dir: Option<std::path::PathBuf>,
    /// The key backups are encrypted with; `<state_dir>/backup.key` unless
    /// set, made on the first backup. Keep a copy off the appliance.
    #[serde(default)]
    pub key_file: Option<std::path::PathBuf>,
    /// Hours between scheduled backups; 0 turns the schedule off.
    #[serde(default = "every_hours")]
    pub every_hours: u32,
    /// Days a backup is kept: the record keeps key-store backups no longer
    /// than the legal deletion deadline, and the GDPR gives a month to act
    /// on an erasure request (Art. 12(3)), so a deleted secret leaves every
    /// backup within it.
    #[serde(default = "keep_days")]
    pub keep_days: u32,
}

/// Daily: with a month kept, a month of daily restore points.
fn every_hours() -> u32 {
    24
}
fn keep_days() -> u32 {
    30
}

impl Default for Backup {
    fn default() -> Self {
        Backup { dir: None, key_file: None, every_hours: every_hours(), keep_days: keep_days() }
    }
}

/// Signed updates of the appliance and of the desktop app (gate item 5).
#[derive(Deserialize, Clone, Debug)]
pub struct Updates {
    /// Where release packages arrive (`appliance/` and `desktop/`, each with
    /// its `latest.json`); `<state_dir>/updates` unless set. The appliance
    /// installs its own from here and serves the desktop app's to the apps.
    #[serde(default)]
    pub dir: Option<std::path::PathBuf>,
    /// The release key (minisign, as `tauri signer` writes it); packages
    /// signed by anything else are refused. The project's key unless set.
    #[serde(default = "release_key")]
    pub public_key: String,
    /// Install a newer appliance release as soon as it is found.
    #[serde(default = "yes")]
    pub automatic: bool,
    /// Minutes between looks for a new release; 0 looks only when asked.
    #[serde(default = "check_minutes")]
    pub check_minutes: u32,
}

fn release_key() -> String {
    include_str!("../../../release/update.pub").trim().to_string()
}
fn yes() -> bool {
    true
}
/// Hourly: a look reads one local file, and a release reaches the
/// appliance within the hour it is published.
fn check_minutes() -> u32 {
    60
}

impl Default for Updates {
    fn default() -> Self {
        Updates { dir: None, public_key: release_key(), automatic: true, check_minutes: check_minutes() }
    }
}

impl Config {
    pub fn updates_dir(&self) -> std::path::PathBuf {
        self.updates.dir.clone().unwrap_or_else(|| self.appliance.state_dir.join("updates"))
    }
    pub fn backup_dir(&self) -> std::path::PathBuf {
        self.backup.dir.clone().unwrap_or_else(|| self.appliance.state_dir.join("backups"))
    }
    pub fn backup_key_file(&self) -> std::path::PathBuf {
        self.backup.key_file.clone().unwrap_or_else(|| self.appliance.state_dir.join("backup.key"))
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Verifier {
    #[serde(default)]
    pub enabled: bool,
    /// Its model, `adapter/model`; the run's own when unset.
    #[serde(default)]
    pub model: Option<String>,
}

/// Removable modules (each compensates for a model weakness and is kept
/// only while its ablation shows it helps): the default for every model,
/// then per model (`adapter/model`). The verifier's default is
/// `[verifier] enabled`.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Modules {
    #[serde(default)]
    pub lost_reset: Option<bool>,
    #[serde(default)]
    pub escalation: Option<bool>,
    #[serde(default)]
    pub memory_pass: Option<bool>,
    #[serde(default)]
    pub lost: crate::run::lost::Thresholds,
    #[serde(default)]
    pub model: Vec<ModelModules>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ModelModules {
    /// `adapter/model`.
    pub model: String,
    #[serde(default)]
    pub verifier: Option<bool>,
    #[serde(default)]
    pub lost_reset: Option<bool>,
    #[serde(default)]
    pub escalation: Option<bool>,
    #[serde(default)]
    pub memory_pass: Option<bool>,
    /// The stronger model a lost run escalates to, `adapter/model`.
    #[serde(default)]
    pub escalate_to: Option<String>,
}

impl Config {
    /// Each removable module on or off for a model, and where it escalates:
    /// per-model settings over the defaults. Lost-reset and escalation are
    /// on unless turned off: they act only on a run that is lost.
    pub fn modules_for(&self, adapter: &str, model: &str) -> (std::collections::BTreeMap<String, bool>, Option<String>) {
        let key = format!("{adapter}/{model}");
        let per = self.modules.model.iter().find(|m| m.model == key);
        let pick = |name: &str, default: bool| {
            let p = per.and_then(|m| match name {
                "verifier" => m.verifier,
                "lost_reset" => m.lost_reset,
                "memory_pass" => m.memory_pass,
                _ => m.escalation,
            });
            (name.to_string(), p.unwrap_or(default))
        };
        let on = [
            pick("verifier", self.verifier.enabled),
            pick("lost_reset", self.modules.lost_reset.unwrap_or(true)),
            pick("escalation", self.modules.escalation.unwrap_or(true)),
            pick("memory_pass", self.modules.memory_pass.unwrap_or(true)),
        ]
        .into_iter()
        .collect();
        (on, per.and_then(|m| m.escalate_to.clone()))
    }
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
pub struct ReadAloud {
    /// The `speak` program: text on stdin, MP3 on stdout.
    pub speak: PathBuf,
    /// Kokoro's model and voices files.
    pub model: PathBuf,
    pub voices: PathBuf,
    /// One of Kokoro's voices, such as `af_heart`.
    pub voice: String,
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

#[derive(Deserialize, serde::Serialize, Clone, Debug)]
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
    /// `oauth`: the person signs in to the app (MCP authorization); the
    /// credential holds the tokens. Otherwise the credential is a bearer token.
    #[serde(default)]
    pub auth: Option<String>,
    /// A client registered with the app by hand, for an app that does not
    /// register clients itself.
    #[serde(default)]
    pub client_id: Option<String>,
    /// The company's record of what it holds (an ERP, a CRM): a knowledge
    /// entry whose quote is found in its answer is verified.
    #[serde(default)]
    pub system_of_record: bool,
}

/// The company knowledge base. Without an embedding server it is searched
/// by its words (Postgres full-text search).
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Knowledge {
    #[serde(default)]
    pub embeddings: Option<Embeddings>,
}

/// An embedding server with the OpenAI-compatible `/embeddings` call (a
/// local one such as Ollama, llama.cpp or TEI keeps the text on site).
#[derive(Deserialize, Clone, Debug)]
pub struct Embeddings {
    /// The API's base address, e.g. `http://127.0.0.1:11434/v1`.
    pub url: String,
    pub model: String,
    /// The vault secret holding its API key, if it needs one.
    #[serde(default)]
    pub credential: Option<String>,
}

fn mcp_http() -> String {
    "mcp_http".into()
}
/// Apps are assumed not to deduplicate unless the owner says they honor an
/// `Idempotency-Key` header, so a crash never causes a blind resend.
fn header() -> String {
    "none".into()
}

#[derive(Deserialize, serde::Serialize, Clone, Debug)]
pub struct ToolClass {
    pub risk: String,
    /// Overrides the app's inbound label for this tool's results.
    #[serde(default)]
    pub label: Option<String>,
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(path)?;
        let mut cfg: Config = toml::from_str(&text)?;
        cfg.path = path.canonicalize()?;
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
