//! The appliance configuration file (TOML), written by the installer.

use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub appliance: Appliance,
    /// The admin console (`magnus-admin`), reachable only from the
    /// organisation's network or VPN.
    #[serde(default)]
    pub admin: Option<Admin>,
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
    pub trash: Trash,
    #[serde(default)]
    pub schedules: Schedules,
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
    /// Brokered connectors' microVMs, booted with the sandbox's Firecracker
    /// and kernel.
    #[serde(default)]
    pub brokered: Brokered,
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
    /// Client certificates for the public hostname. None: the appliance
    /// does not pair devices, and the tunnel is not asked to present one.
    #[serde(default)]
    pub client_certs: Option<ClientCerts>,
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

/// The admin console's reach (admin console spec, Access): its hostname
/// (requests naming anything else are not found), the appliance's addresses
/// on the organisation's network to listen on (never every address, never
/// loopback), the LAN and VPN subnets it takes connections from (never
/// loopback), and the organisation's certificate for that hostname (TLS 1.3
/// only).
#[derive(Deserialize, Clone, Debug)]
pub struct Admin {
    pub hostname: String,
    /// Other names the console answers to, on the same networks: the
    /// machine's name on the VPN and its addresses, so the console opens
    /// however a computer reaches the appliance (owner, 2026-11-02).
    #[serde(default)]
    pub aliases: Vec<String>,
    pub listen: Vec<String>,
    pub allow: Vec<String>,
    pub tls_cert: PathBuf,
    pub tls_key: PathBuf,
    /// The organisation's name, shown in the console's top bar.
    #[serde(default)]
    pub org: Option<String>,
    /// The console's confidential client in the realm, which proves itself
    /// with the console's own key (`magnus-admin jwk` prints its public half).
    #[serde(default = "admin_client")]
    pub client_id: String,
    /// The realm, when not the app's (`sign_in.oidc.issuer`).
    #[serde(default)]
    pub issuer: Option<String>,
    /// Where the console reaches the identity provider when not at the
    /// issuer's address (its internal address on the appliance); the tokens
    /// still name the public issuer.
    #[serde(default)]
    pub idp_internal: Option<String>,
    /// Where the provider's admin API answers, when the server keeps it on
    /// an origin of its own (Keycloak's `hostname-admin`, such as
    /// `https://127.0.0.1:19446`), and the PEM CA its certificate is from.
    /// Unset: the appliance's own `sign_in.oidc.status_admin_url` and
    /// `status_admin_ca`, the same server's; else the issuer's host.
    #[serde(default)]
    pub idp_admin_url: Option<String>,
    #[serde(default)]
    pub idp_admin_ca: Option<PathBuf>,
    /// A session ends after this long unused, and after `session_hours`.
    #[serde(default = "admin_idle_minutes")]
    pub idle_minutes: u32,
    #[serde(default = "admin_session_hours")]
    pub session_hours: u32,
    /// A step-up (a fresh password) counts this long.
    #[serde(default = "admin_step_up_seconds")]
    pub step_up_seconds: u32,
    /// The provider is asked again whether the admin is still one, and
    /// still turned on, at least this often.
    #[serde(default = "admin_refresh_seconds")]
    pub refresh_seconds: u32,
    /// The console's own database role (`postgres://magnus_root:…@…`):
    /// past the walls between people, and only adding to the log, never
    /// changing it. `magnus migrate` creates it.
    #[serde(default)]
    pub database_url: Option<String>,
    /// The console's service account in the realm (admin console spec,
    /// Server and API): how it manages people there, proving itself with
    /// the console's own key, and only after the admin's own check passes.
    #[serde(default = "admin_service_client")]
    pub service_client_id: String,
    /// Where the tunnel (cloudflared) says whether it is connected: its
    /// metrics address's `/ready`, on this machine. None: no tunnel here.
    #[serde(default)]
    pub tunnel_ready: Option<String>,
    /// The names the tunnel serves to the internet (the user API and the
    /// realm), shown under Operations › Network; the console is never one.
    #[serde(default)]
    pub public_hostnames: Vec<String>,
    /// A certificate browsers already trust, for `hostname` (admin console
    /// spec, Access). `"tailscale"`: the machine's tailnet has it issued
    /// (Let's Encrypt, by ACME DNS-01 under the tailnet's domain) into
    /// `trusted_cert` and `trusted_key`, and the console renews it. The
    /// console serves it to browsers naming `hostname`, and `tls_cert` (the
    /// organisation's own CA's) to every other name, and to all of them
    /// until it is issued. None: `tls_cert` is the organisation's, renewed
    /// by it and read again on Operations › Network › Renew.
    #[serde(default)]
    pub trusted_via: Option<String>,
    #[serde(default)]
    pub trusted_cert: Option<PathBuf>,
    #[serde(default)]
    pub trusted_key: Option<PathBuf>,
    /// The `tailscale` program, when not the one on the PATH.
    #[serde(default)]
    pub tailscale: Option<PathBuf>,
}

fn admin_service_client() -> String {
    "magnus-admin-svc".into()
}
fn admin_step_up_seconds() -> u32 {
    300
}
fn admin_refresh_seconds() -> u32 {
    240
}

fn admin_client() -> String {
    "magnus-admin".into()
}
fn admin_idle_minutes() -> u32 {
    30
}
fn admin_session_hours() -> u32 {
    12
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
    /// How long a sign-in in the person's browser may take: finding a
    /// password or passkey, two-factor. The app waits as long, from what the
    /// appliance tells it.
    #[serde(default = "sign_in_minutes")]
    pub sign_in_minutes: u32,
    /// A confidential client whose service account may only view users
    /// (Keycloak: `realm-management` → `view-users`). The appliance asks it
    /// who uses a code from an authenticator app, since the password grant
    /// answers the same to a wrong password and a missing code.
    #[serde(default)]
    pub status_client: Option<String>,
    /// The vault secret holding that client's secret.
    #[serde(default)]
    pub status_credential: Option<String>,
    /// Where Keycloak's admin API answers, when not at the issuer's host: a
    /// server can keep it on an address of its own (`hostname-admin`), such
    /// as `https://127.0.0.1:19446`. Unset: the issuer's host.
    #[serde(default)]
    pub status_admin_url: Option<String>,
    /// A PEM certificate authority to trust for that address, when a
    /// private one issued its certificate.
    #[serde(default)]
    pub status_admin_ca: Option<std::path::PathBuf>,
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
fn sign_in_minutes() -> u32 {
    10
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

/// Cloudflare's managed CA issues one certificate per paired computer.
/// The token in the vault is zone-scoped and can write client certificates;
/// it is not an all-accounts token.
#[derive(Deserialize, Clone, Debug)]
pub struct ClientCerts {
    /// The public hostname, without a scheme: `magnus.pyrallon.ai`.
    pub hostname: String,
    /// The zone id of that hostname. Not a secret; it is in the dashboard.
    pub zone_id: String,
    /// The vault secret whose text is the Cloudflare API token.
    #[serde(default = "client_cert_credential")]
    pub credential: String,
    /// How long a certificate is valid. Cloudflare accepts 1 to 3650 days.
    #[serde(default = "client_cert_days")]
    pub days: u32,
}

fn client_cert_credential() -> String {
    "cloudflare_mtls".into()
}
fn client_cert_days() -> u32 {
    365
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

impl Budget {
    /// Each limit the lower of the two.
    pub fn within(&self, cap: &Budget) -> Budget {
        Budget { max_model_calls: self.max_model_calls.min(cap.max_model_calls), max_actions: self.max_actions.min(cap.max_actions), max_usd: self.max_usd.min(cap.max_usd) }
    }
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

/// Deleted items wait in the trash, restorable, for `keep_days`, then their
/// content is removed. A month, as backups are kept: the same date answers
/// "when is it gone?", within the GDPR's month to act on erasure.
#[derive(Deserialize, Clone, Debug)]
pub struct Trash {
    #[serde(default = "keep_days")]
    pub keep_days: u32,
}

impl Default for Trash {
    fn default() -> Self {
        Trash { keep_days: keep_days() }
    }
}

/// Recurring tasks.
#[derive(Deserialize, Clone, Debug)]
pub struct Schedules {
    /// A schedule pauses itself, and tells its person, after this many of
    /// its runs fail in a row (or cannot start), so a broken one stops
    /// spending until someone looks.
    #[serde(default = "pause_after_failures")]
    pub pause_after_failures: u32,
    /// The shortest interval a schedule may repeat at.
    #[serde(default = "min_every_minutes")]
    pub min_every_minutes: u32,
}

impl Default for Schedules {
    fn default() -> Self {
        Schedules { pause_after_failures: pause_after_failures(), min_every_minutes: min_every_minutes() }
    }
}

fn pause_after_failures() -> u32 {
    3
}

fn min_every_minutes() -> u32 {
    5
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
    /// Seconds one piece may take before it is stopped.
    #[serde(default = "speak_timeout")]
    pub timeout_s: u64,
}

fn speak_timeout() -> u64 {
    60
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

/// Brokered connectors (connectors spec, Brokered API connectors): each
/// person's instance of one is a microVM booted from its server image.
#[derive(Deserialize, Clone, Debug)]
pub struct Brokered {
    /// Where the server images are, each named by its digest:
    /// `<sha256 hex>.cpio`.
    #[serde(default)]
    pub images: Option<PathBuf>,
    /// Seconds an instance waits unused before it is stopped ("stopped
    /// when idle").
    #[serde(default = "brokered_idle")]
    pub idle_s: u64,
    #[serde(default = "brokered_memory")]
    pub memory_mib: u32,
    /// Where the egress proxy reaches a manifest host (`host:port`) instead
    /// of the address DNS gives: a pinned address, or a stand-in for the
    /// provider in a test. What a microVM may ask for is still only its
    /// manifest's hosts.
    #[serde(default)]
    pub resolve: std::collections::BTreeMap<String, String>,
    /// Certificates (DER files) servers trust beside the public roots: an
    /// organisation whose egress inspects TLS, or a test's stand-in.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
}

/// Five minutes: the steps of one run call an app minutes apart at most
/// (each waits on a model's answer), so a run keeps its instance, which
/// boots in about a second, while a person who has stopped working holds
/// no microVM for long.
fn brokered_idle() -> u64 {
    300
}

/// The image is unpacked into the microVM's memory and is a few megabytes;
/// the rest is for what one call carries (a message with its attachments).
fn brokered_memory() -> u32 {
    128
}

impl Default for Brokered {
    fn default() -> Self {
        Brokered { images: None, idle_s: brokered_idle(), memory_mib: brokered_memory(), resolve: Default::default(), roots: vec![] }
    }
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
    /// Groups it is blocked for (connectors spec, Org policy: "allowed or
    /// blocked per group"): their people neither see it in the Library nor
    /// reach it, whatever `access_groups` says.
    #[serde(default)]
    pub blocked_groups: Vec<String>,
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
    /// The access asked for at sign-in, when the app's manifest names it
    /// (else what the app's metadata offers).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The provider whose registration (Alpha Vector's client, set by the
    /// owner) signs people in to it.
    #[serde(default)]
    pub registration: Option<String>,
    /// Its name and one sentence on what Magnus can do with it, from its
    /// manifest.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    /// From the catalog, not added by the owner.
    #[serde(default)]
    pub catalog: bool,
    /// A brokered connector (`transport = "brokered"`, `url` its server
    /// image's digest, `sha256:<hex>`): the provider API hosts its microVM
    /// may reach, `host` (port 443) or `host:port`, and the provider's
    /// authorization server people sign in at.
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    /// The loopback port a sign-in must come back to, when the provider
    /// matches the redirect exactly (Slack). None: any loopback port.
    #[serde(default)]
    pub loopback: Option<u16>,
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
        anyhow::ensure!(cfg.backup.keep_days >= 1, "[backup] keep_days must be at least 1: with 0 every backup is deleted as soon as it is made");
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
            if c.transport == crate::brokered::BROKERED {
                for r in &cfg.brokered.roots {
                    std::fs::read(r).map_err(|e| anyhow::anyhow!("[brokered] roots: {}: {e}", r.display()))?;
                }
                crate::brokered::digest(&c.url).map_err(|e| anyhow::anyhow!("connector {}: {e}", c.name))?;
                anyhow::ensure!(!c.hosts.is_empty(), "connector {}: a brokered connector names the provider hosts it may reach", c.name);
                // Its calls carry no idempotency key to its server, so one
                // whose outcome is unknown cannot be sent again blindly: the
                // person is asked (`none`).
                anyhow::ensure!(c.idempotency != "header", "connector {}: a brokered connector's calls carry no idempotency key, so it cannot have idempotency = \"header\"; leave it out, and a call whose outcome is unknown is put to the person", c.name);
            }
        }
        Ok(cfg)
    }

    pub fn person(&self, id: &str) -> Option<&Person> {
        self.people.iter().find(|p| p.id == id)
    }

    /// Whether a password set on the appliance signs this person in. With an
    /// identity provider it is the owner's backup alone, so turning someone
    /// off at the provider shuts them out (owner, 2026-10-30: "The back up
    /// password is only for me"), and only on the appliance's own machine
    /// (the API checks where); the owner is read from this file, never from
    /// the provider. Without one, it is everyone's way in.
    pub fn may_use_password(&self, id: &str) -> bool {
        self.person(id).is_some_and(|p| self.sign_in.oidc.is_none() || p.is_owner())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn load(connector: &str) -> anyhow::Result<Config> {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("magnus.toml");
        let text = format!("[appliance]\nstate_dir = \"/s\"\nlisten = \"127.0.0.1:0\"\n[database]\nurl = \"postgres://x\"\n[model]\nname = \"m\"\nadapter = \"a\"\nmax_tokens = 1\n[[connectors]]\nname = \"mail\"\naudience = \"public\"\nlabel = \"private+untrusted\"\n{connector}");
        std::fs::write(&p, text).unwrap();
        Config::load(&p)
    }

    /// A brokered connector's calls carry no idempotency key, so it is
    /// never taken to deduplicate: one that says it does is refused, and
    /// one that says nothing does not.
    #[test]
    fn a_brokered_connector_cannot_claim_to_deduplicate() {
        let brokered = format!("transport = \"brokered\"\nurl = \"sha256:{}\"\nhosts = [\"api.example\"]\n", "0a".repeat(32));
        let e = load(&format!("{brokered}idempotency = \"header\"\n")).unwrap_err();
        assert!(format!("{e:#}").contains("carry no idempotency key"), "{e:#}");
        assert_eq!(load(&brokered).unwrap().connectors[0].idempotency, "none");
        // A remote app may say it honors the header.
        assert!(load("url = \"https://a.example/mcp\"\nidempotency = \"header\"\n").is_ok());
    }
}
