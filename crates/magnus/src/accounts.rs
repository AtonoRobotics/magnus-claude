//! Password sign-in and sessions for people's apps. Sign-in through an
//! identity provider (`oidc`) opens the same sessions.
//!
//! People, password hashes (Argon2id), sessions and the rest live in the
//! appliance's Postgres (`account_*` tables, made by `migrate`), never in
//! the log: a session token is a credential (invariant 2). The log records
//! only that a person signed in or out, with the session id. Every check
//! reads the store, so a removal or disabling from another process (the
//! CLI's own handle) takes effect here at once, and re-adding an id never
//! resurrects its ended sessions (#466, #469).
//!
//! The one exception is the directory itself (`is_local`, `local_person`,
//! `local_people`): resolving a person is sync (`Kernel::person`), and the
//! database is not, so those three read a cache every other call refreshes
//! when it is older than `LOCAL_TTL`. A name or address may lag half a
//! minute on a quiet server; whether anyone may sign in or stay signed in
//! is always read fresh, so the lag never admits anyone.

use crate::config::SignIn;
use anyhow::{Context, Result};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;
use tokio_postgres::Client;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub id: String,
    pub person: String,
    pub created: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    /// What signed in: "app" or "browser".
    #[serde(default)]
    pub client: String,
    /// The paired device this session was opened on, when it came through
    /// the tunnel or was bound at pairing. Absent on a session that is
    /// older than device certificates, and on a LAN session not yet paired.
    #[serde(default)]
    pub device: Option<String>,
}

pub enum SignInError {
    /// Wrong person or password; the caller says nothing more.
    Rejected,
    /// Too many failures; try again after this time.
    Locked(DateTime<Utc>),
    /// The password was right and the person uses an authenticator app:
    /// ask for the code and try again. Uncounted: needing the code says
    /// nothing was wrong, as with the identity provider's `NeedsCode`.
    NeedsCode,
}

/// Whether a person may sign in: invited until they accept, active after,
/// disabled when turned off. Nobody recorded counts as active, so everyone
/// already here stays signed in (#429).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Invited,
    Active,
    Disabled,
}

impl Status {
    fn as_str(&self) -> &'static str {
        match self {
            Status::Invited => "invited",
            Status::Active => "active",
            Status::Disabled => "disabled",
        }
    }

    fn parse(s: &str) -> Status {
        match s {
            "invited" => Status::Invited,
            "disabled" => Status::Disabled,
            _ => Status::Active,
        }
    }
}

/// A live invitation: who it is for, where the link went, and until when
/// it accepts. Stored by the hash of its token; the token itself is shown
/// once, at the invite.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Invite {
    pub person: String,
    pub email: Option<String>,
    pub expires: DateTime<Utc>,
}

/// What a person may administer: admins enter the console, members do not.
/// Nobody recorded is a member. This is the console axis, not the kernel's
/// owner/developer roles (#429).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    fn parse(s: &str) -> Role {
        match s {
            "admin" => Role::Admin,
            _ => Role::Member,
        }
    }
}

/// Days an invitation stays usable.
const INVITE_DAYS: i64 = 7;

/// How long the directory cache serves before a call refreshes it.
const LOCAL_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The accounts tables, for the backup's table-scoped dump.
pub const TABLES: &[&str] = &[
    "account_people",
    "account_passwords",
    "account_totp",
    "account_totp_steps",
    "account_statuses",
    "account_roles",
    "account_invites",
    "account_sessions",
    "account_machine_codes",
    "account_json_import",
];

/// The accounts tables, beside the log. The kernel reads and writes them;
/// no other role reaches them.
pub fn migrate_sql(role: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS public.account_people (
             person text PRIMARY KEY,
             name   text NOT NULL,
             email  text
         );
         CREATE TABLE IF NOT EXISTS public.account_passwords (
             person text PRIMARY KEY,
             phc    text NOT NULL
         );
         CREATE TABLE IF NOT EXISTS public.account_totp (
             person text PRIMARY KEY,
             secret text NOT NULL
         );
         CREATE TABLE IF NOT EXISTS public.account_totp_steps (
             person text PRIMARY KEY,
             step   bigint NOT NULL
         );
         CREATE TABLE IF NOT EXISTS public.account_statuses (
             person text PRIMARY KEY,
             status text NOT NULL CHECK (status IN ('invited', 'active', 'disabled'))
         );
         CREATE TABLE IF NOT EXISTS public.account_roles (
             person text PRIMARY KEY,
             role   text NOT NULL CHECK (role IN ('admin', 'member'))
         );
         CREATE TABLE IF NOT EXISTS public.account_invites (
             hash    text PRIMARY KEY,
             person  text NOT NULL,
             email   text,
             expires timestamptz NOT NULL
         );
         CREATE INDEX IF NOT EXISTS account_invites_person ON public.account_invites (person);
         CREATE TABLE IF NOT EXISTS public.account_sessions (
             hash    text PRIMARY KEY,
             id      text NOT NULL,
             person  text NOT NULL,
             created timestamptz NOT NULL,
             expires timestamptz NOT NULL,
             client  text NOT NULL,
             device  text
         );
         CREATE INDEX IF NOT EXISTS account_sessions_person ON public.account_sessions (person);
         CREATE TABLE IF NOT EXISTS public.account_machine_codes (
             hash  text PRIMARY KEY,
             until timestamptz NOT NULL
         );
         CREATE TABLE IF NOT EXISTS public.account_json_import (
             imported_at timestamptz PRIMARY KEY
         );
         REVOKE ALL ON public.account_people, public.account_passwords, public.account_totp, public.account_totp_steps, public.account_statuses, public.account_roles, public.account_invites, public.account_sessions, public.account_machine_codes, public.account_json_import FROM PUBLIC;
         GRANT SELECT, INSERT, UPDATE, DELETE ON public.account_people, public.account_passwords, public.account_totp, public.account_totp_steps, public.account_statuses, public.account_roles, public.account_invites, public.account_sessions, public.account_machine_codes, public.account_json_import TO {role};"
    )
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).with_context(|| format!("{} does not parse", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

/// Bring the JSON files' contents into the tables, once: `magnus migrate`
/// runs this after creating them, so the cutover keeps everyone signing
/// in. Completion is recorded in the database itself, so later migrates
/// (every deploy runs one) never resurrect a removed person or an ended
/// session from the untouched files; expired sessions, invites and codes
/// are left behind. Returns how many rows it wrote.
pub async fn import_json(client: &mut Client, state_dir: &Path) -> Result<usize> {
    let tx = client.transaction().await?;
    tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext('magnus-accounts-import'))").await?;
    if tx.query_opt("SELECT 1 FROM public.account_json_import LIMIT 1", &[]).await?.is_some() {
        tx.commit().await?;
        return Ok(0);
    }
    let d = state_dir.join("accounts");
    let mut n = 0usize;
    let now = Utc::now();
    let people: BTreeMap<String, LocalPerson> = read_json(&d.join("people.json"))?;
    for (person, p) in &people {
        let person = key(person);
        n += tx.execute("INSERT INTO public.account_people (person, name, email) VALUES ($1, $2, $3) ON CONFLICT (person) DO NOTHING", &[&person, &p.name, &p.email]).await? as usize;
    }
    let passwords: BTreeMap<String, String> = read_json(&d.join("passwords.json"))?;
    for (person, phc) in &passwords {
        let person = key(person);
        n += tx.execute("INSERT INTO public.account_passwords (person, phc) VALUES ($1, $2) ON CONFLICT (person) DO NOTHING", &[&person, phc]).await? as usize;
    }
    let totp: BTreeMap<String, String> = read_json(&d.join("totp.json"))?;
    for (person, secret) in &totp {
        let person = key(person);
        n += tx.execute("INSERT INTO public.account_totp (person, secret) VALUES ($1, $2) ON CONFLICT (person) DO NOTHING", &[&person, secret]).await? as usize;
    }
    let steps: BTreeMap<String, i64> = read_json(&d.join("totp-steps.json"))?;
    for (person, step) in &steps {
        let person = key(person);
        n += tx.execute("INSERT INTO public.account_totp_steps (person, step) VALUES ($1, $2) ON CONFLICT (person) DO NOTHING", &[&person, step]).await? as usize;
    }
    let statuses: BTreeMap<String, Status> = read_json(&d.join("status.json"))?;
    for (person, status) in &statuses {
        let person = key(person);
        n += tx.execute("INSERT INTO public.account_statuses (person, status) VALUES ($1, $2) ON CONFLICT (person) DO NOTHING", &[&person, &status.as_str()]).await? as usize;
    }
    let roles: BTreeMap<String, Role> = read_json(&d.join("roles.json"))?;
    for (person, role) in &roles {
        let person = key(person);
        n += tx.execute("INSERT INTO public.account_roles (person, role) VALUES ($1, $2) ON CONFLICT (person) DO NOTHING", &[&person, &role.as_str()]).await? as usize;
    }
    let invites: BTreeMap<String, Invite> = read_json(&d.join("invites.json"))?;
    for (hash, inv) in invites.iter().filter(|(_, inv)| inv.expires > now) {
        let who = key(&inv.person);
        n += tx
            .execute(
                "INSERT INTO public.account_invites (hash, person, email, expires) VALUES ($1, $2, $3, $4) ON CONFLICT (hash) DO NOTHING",
                &[hash, &who, &inv.email, &inv.expires],
            )
            .await? as usize;
    }
    let sessions: BTreeMap<String, Session> = read_json(&d.join("sessions.json"))?;
    for (hash, s) in sessions.iter().filter(|(_, s)| s.expires > now) {
        let who = key(&s.person);
        n += tx
            .execute(
                "INSERT INTO public.account_sessions (hash, id, person, created, expires, client, device) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (hash) DO NOTHING",
                &[hash, &s.id, &who, &s.created, &s.expires, &s.client, &s.device],
            )
            .await? as usize;
    }
    let codes: BTreeMap<String, DateTime<Utc>> = read_json(&d.join("machine-codes.json"))?;
    for (hash, until) in codes.iter().filter(|(_, until)| **until > now) {
        n += tx.execute("INSERT INTO public.account_machine_codes (hash, until) VALUES ($1, $2) ON CONFLICT (hash) DO NOTHING", &[hash, until]).await? as usize;
    }
    tx.execute("INSERT INTO public.account_json_import (imported_at) VALUES ($1)", &[&now]).await?;
    tx.commit().await?;
    Ok(n)
}

pub struct Accounts {
    cfg: SignIn,
    url: String,
    client: tokio::sync::Mutex<Option<Client>>,
    /// Failed attempts per counter: (count, first failure). In memory, as
    /// before: a restart forgives guesses in flight.
    failures: Mutex<BTreeMap<String, (u32, DateTime<Utc>)>>,
    /// The directory (id to record) and when it was read. See the module docs.
    local: Mutex<(Instant, BTreeMap<String, LocalPerson>)>,
}

fn token_hash(token: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(token.as_bytes()))
}

fn session_of(r: &tokio_postgres::Row) -> Session {
    Session { id: r.get("id"), person: r.get("person"), created: r.get("created"), expires: r.get("expires"), client: r.get("client"), device: r.get("device") }
}

/// A person the appliance itself keeps: config-independent identity for
/// the embedded directory (#429). The id is the key, not repeated here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalPerson {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

fn valid_person_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Person ids are lowercase-canonical: sign-in lowercases at entry, so
/// every read and write goes through here and `Ada` never becomes a
/// second, unreachable account beside `ada` (#470).
fn key(person: &str) -> String {
    person.to_lowercase()
}

impl Accounts {
    /// Open the store, connecting at once (so a dead database fails here,
    /// not at the first sign-in) and warming the directory cache.
    pub async fn open(db_url: &str, cfg: SignIn) -> Result<Accounts> {
        let client = crate::log::connect(db_url).await?;
        let local = Self::read_local(&client).await?;
        Ok(Accounts { cfg, url: db_url.into(), client: tokio::sync::Mutex::new(Some(client)), failures: Mutex::new(BTreeMap::new()), local: Mutex::new((Instant::now(), local)) })
    }

    async fn client(&self) -> Result<tokio::sync::MutexGuard<'_, Option<Client>>> {
        let mut g = self.client.lock().await;
        if g.as_ref().is_none_or(|c| c.is_closed()) {
            *g = Some(crate::log::connect(&self.url).await?);
        }
        Ok(g)
    }

    /// The directory as it stands, refreshing the cache first when it is
    /// older than `LOCAL_TTL`. Every async call starts here.
    async fn fresh_local(&self, c: &Client) -> Result<()> {
        if self.local.lock().unwrap().0.elapsed() > LOCAL_TTL {
            self.refresh_local(c).await?;
        }
        Ok(())
    }

    async fn refresh_local(&self, c: &Client) -> Result<()> {
        let map = Self::read_local(c).await?;
        *self.local.lock().unwrap() = (Instant::now(), map);
        Ok(())
    }

    async fn read_local(c: &Client) -> Result<BTreeMap<String, LocalPerson>> {
        let mut map = BTreeMap::new();
        for r in c.query("SELECT person, name, email FROM public.account_people ORDER BY person", &[]).await.context("read the directory")? {
            let person: String = r.get("person");
            map.insert(person, LocalPerson { name: r.get("name"), email: r.get("email") });
        }
        Ok(map)
    }

    /// Whether the person has a password set.
    pub async fn has_password(&self, person: &str) -> bool {
        let person = key(person);
        let mut g = match self.client().await {
            Ok(g) => g,
            Err(_) => return false,
        };
        let c = match g.as_mut() {
            Some(c) => c,
            None => return false,
        };
        c.query_opt("SELECT 1 FROM public.account_passwords WHERE person = $1", &[&person]).await.map(|r| r.is_some()).unwrap_or(false)
    }

    /// Whether the person enrolled an authenticator app.
    pub async fn has_totp(&self, person: &str) -> bool {
        let person = key(person);
        let mut g = match self.client().await {
            Ok(g) => g,
            Err(_) => return false,
        };
        let c = match g.as_mut() {
            Some(c) => c,
            None => return false,
        };
        c.query_opt("SELECT 1 FROM public.account_totp WHERE person = $1", &[&person]).await.map(|r| r.is_some()).unwrap_or(false)
    }

    /// Whether the person may sign in: active unless recorded otherwise.
    pub async fn status(&self, person: &str) -> Status {
        let person = key(person);
        let mut g = match self.client().await {
            Ok(g) => g,
            Err(_) => return Status::Active,
        };
        let c = match g.as_mut() {
            Some(c) => c,
            None => return Status::Active,
        };
        let _ = self.fresh_local(c).await;
        c.query_opt("SELECT status FROM public.account_statuses WHERE person = $1", &[&person]).await.ok().flatten().map(|r| Status::parse(r.get::<_, String>(0).as_str())).unwrap_or(Status::Active)
    }

    /// Turn a person on or off. Disabling ends their live sessions at once
    /// and returns the ids of those ended, for the caller to record.
    /// Refuses to disable the organisation's last enabled admin.
    pub async fn set_status(&self, person: &str, status: Status) -> Result<Vec<String>> {
        let person = key(person);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let tx = c.transaction().await?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext('magnus-accounts'))").await?;
        if status == Status::Disabled && Self::sole_enabled_admin_tx(&tx, &person).await? {
            anyhow::bail!("This is the organisation's last admin: make someone else an admin first.");
        }
        tx.execute("INSERT INTO public.account_statuses (person, status) VALUES ($1, $2) ON CONFLICT (person) DO UPDATE SET status = EXCLUDED.status", &[&person, &status.as_str()]).await?;
        let mut ended = vec![];
        if status == Status::Disabled {
            for r in tx.query("DELETE FROM public.account_sessions WHERE person = $1 RETURNING id", &[&person]).await? {
                ended.push(r.get("id"));
            }
        }
        tx.commit().await?;
        self.refresh_local(c).await?;
        Ok(ended)
    }

    /// What the person may administer: member unless recorded an admin.
    pub async fn role(&self, person: &str) -> Role {
        let person = key(person);
        let mut g = match self.client().await {
            Ok(g) => g,
            Err(_) => return Role::Member,
        };
        let c = match g.as_mut() {
            Some(c) => c,
            None => return Role::Member,
        };
        let _ = self.fresh_local(c).await;
        c.query_opt("SELECT role FROM public.account_roles WHERE person = $1", &[&person]).await.ok().flatten().map(|r| Role::parse(r.get::<_, String>(0).as_str())).unwrap_or(Role::Member)
    }

    /// Whether `person` is an enabled admin with no other: demoting or
    /// disabling them would strand the organisation. Unrecorded people are
    /// members, so an unmanaged switch never guards.
    async fn sole_enabled_admin_tx(tx: &tokio_postgres::Transaction<'_>, person: &str) -> Result<bool> {
        let role: Option<String> = tx.query_opt("SELECT role FROM public.account_roles WHERE person = $1", &[&person]).await?.map(|r| r.get(0));
        if role.as_deref() != Some("admin") {
            return Ok(false);
        }
        let status: Option<String> = tx.query_opt("SELECT status FROM public.account_statuses WHERE person = $1", &[&person]).await?.map(|r| r.get(0));
        if status.as_deref().is_some_and(|s| s != "active") {
            return Ok(false);
        }
        let others: i64 = tx
            .query_one(
                "SELECT COUNT(*) FROM public.account_roles r LEFT JOIN public.account_statuses s ON s.person = r.person
                 WHERE r.role = 'admin' AND (s.status IS NULL OR s.status = 'active') AND r.person <> $1",
                &[&person],
            )
            .await?
            .get(0);
        Ok(others == 0)
    }

    /// Whether the appliance itself keeps this person (as opposed to the
    /// config or an outside directory). From the directory cache.
    pub fn is_local(&self, person: &str) -> bool {
        let person = key(person);
        self.local.lock().unwrap().1.contains_key(person.as_str())
    }

    /// `is_local`, read from the store instead of the cache: for the
    /// sign-in routing decision, which must see a person the CLI added
    /// seconds ago, not the cache from before (#466).
    pub async fn is_local_fresh(&self, person: &str) -> bool {
        let mut g = match self.client().await {
            Ok(g) => g,
            Err(_) => return self.is_local(person),
        };
        let c = match g.as_mut() {
            Some(c) => c,
            None => return self.is_local(person),
        };
        let _ = self.fresh_local(c).await;
        c.query_opt("SELECT 1 FROM public.account_people WHERE person = $1", &[&person]).await.map(|r| r.is_some()).unwrap_or_else(|_| self.is_local(person))
    }

    /// The appliance's record for a local person, if there is one. From
    /// the directory cache.
    pub fn local_person(&self, person: &str) -> Option<LocalPerson> {
        let person = key(person);
        self.local.lock().unwrap().1.get(person.as_str()).cloned()
    }

    /// Every local person by id. From the directory cache.
    pub fn local_people(&self) -> Vec<(String, LocalPerson)> {
        self.local.lock().unwrap().1.clone().into_iter().collect()
    }

    /// Refresh the directory cache from the store. Sign-in calls this
    /// before resolving the person, so someone the CLI added seconds ago
    /// resolves (#466).
    pub async fn refresh_people(&self) -> Result<()> {
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        self.refresh_local(c).await
    }

    /// Add a person to the appliance's own directory. Refuses a second
    /// person with the id, and an id outside 1 to 64 letters, digits,
    /// dots, dashes and underscores.
    pub async fn add_person(&self, id: &str, name: &str, email: Option<&str>) -> Result<()> {
        let id = key(id);
        anyhow::ensure!(valid_person_id(&id), "a person id is 1 to 64 letters, digits, dots, dashes and underscores");
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let n = c.execute("INSERT INTO public.account_people (person, name, email) VALUES ($1, $2, $3) ON CONFLICT (person) DO NOTHING", &[&id, &name, &email]).await?;
        anyhow::ensure!(n == 1, "{id} is already a person here");
        self.refresh_local(c).await?;
        Ok(())
    }

    /// Remove a local person and every local trace: the record, password,
    /// authenticator, used codes, status, role, invites and sessions.
    /// Refuses an unknown id, and the organisation's last enabled admin.
    /// The sessions die in the store, so no handle keeps them (#469).
    pub async fn remove_person(&self, person: &str) -> Result<Vec<String>> {
        let person = key(person);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let tx = c.transaction().await?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext('magnus-accounts'))").await?;
        let n = tx.execute("DELETE FROM public.account_people WHERE person = $1", &[&person]).await?;
        anyhow::ensure!(n == 1, "{person} is not a person here");
        if Self::sole_enabled_admin_tx(&tx, &person).await? {
            anyhow::bail!("This is the organisation's last admin: make someone else an admin first.");
        }
        for table in ["account_passwords", "account_totp", "account_totp_steps", "account_statuses", "account_roles"] {
            tx.execute(&format!("DELETE FROM public.{table} WHERE person = $1"), &[&person]).await?;
        }
        tx.execute("DELETE FROM public.account_invites WHERE person = $1", &[&person]).await?;
        let mut ended = vec![];
        for r in tx.query("DELETE FROM public.account_sessions WHERE person = $1 RETURNING id", &[&person]).await? {
            ended.push(r.get("id"));
        }
        tx.commit().await?;
        self.failures.lock().unwrap().remove(person.as_str());
        self.failures.lock().unwrap().remove(Self::console_counter(&person).as_str());
        self.refresh_local(c).await?;
        Ok(ended)
    }

    /// Set a person's role. Refuses to demote the organisation's last
    /// enabled admin.
    pub async fn set_role(&self, person: &str, role: Role) -> Result<()> {
        let person = key(person);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let tx = c.transaction().await?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext('magnus-accounts'))").await?;
        if role == Role::Member && Self::sole_enabled_admin_tx(&tx, &person).await? {
            anyhow::bail!("This is the organisation's last admin: make someone else an admin first.");
        }
        tx.execute("INSERT INTO public.account_roles (person, role) VALUES ($1, $2) ON CONFLICT (person) DO UPDATE SET role = EXCLUDED.role", &[&person, &role.as_str()]).await?;
        tx.commit().await?;
        self.refresh_local(c).await?;
        Ok(())
    }

    /// Set a person's password. A password completes an invitation by any
    /// path, so someone invited becomes active; anyone disabled stays
    /// disabled until explicitly turned back on.
    pub async fn set_password(&self, person: &str, password: &str) -> Result<()> {
        let person = key(person);
        anyhow::ensure!(
            password.chars().count() >= self.cfg.min_password_length,
            "a password needs at least {} characters",
            self.cfg.min_password_length
        );
        let salt = SaltString::generate(&mut OsRng);
        let phc = Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|e| anyhow::anyhow!("hash: {e}"))?.to_string();
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let tx = c.transaction().await?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext('magnus-accounts'))").await?;
        tx.execute("INSERT INTO public.account_passwords (person, phc) VALUES ($1, $2) ON CONFLICT (person) DO UPDATE SET phc = EXCLUDED.phc", &[&person, &phc]).await?;
        tx.execute("UPDATE public.account_statuses SET status = 'active' WHERE person = $1 AND status = 'invited'", &[&person]).await?;
        tx.commit().await?;
        self.refresh_local(c).await?;
        Ok(())
    }

    /// Enroll a person's authenticator app: a fresh 160-bit secret is
    /// stored, and the `otpauth://` URI to enroll is returned.
    /// Re-enrolling replaces the secret.
    pub async fn enroll_totp(&self, person: &str) -> Result<String> {
        let person = key(person);
        let mut raw = [0u8; 20];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let secret = crate::totp::base32_encode(&raw);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        c.execute("INSERT INTO public.account_totp (person, secret) VALUES ($1, $2) ON CONFLICT (person) DO UPDATE SET secret = EXCLUDED.secret", &[&person, &secret]).await?;
        self.refresh_local(c).await?;
        let label = percent_encoding::utf8_percent_encode(&person, percent_encoding::NON_ALPHANUMERIC);
        Ok(format!("otpauth://totp/Magnus:{label}?secret={}&issuer=Magnus&digits=6&period=30", secret.trim_end_matches('=')))
    }

    /// A one-time code for the owner's backup password, valid ten minutes.
    /// Only someone with a shell on the appliance's machine (as its user)
    /// can make one, so a relay or tunnel that delivers remote connections
    /// from loopback cannot carry the backup.
    pub async fn new_machine_code(&self) -> Result<String> {
        use rand::Rng;
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let now = Utc::now();
        c.execute("DELETE FROM public.account_machine_codes WHERE until <= $1", &[&now]).await?;
        let code: String = (0..10).map(|_| char::from(b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"[rand::rngs::OsRng.gen_range(0..32)])).collect();
        c.execute("INSERT INTO public.account_machine_codes (hash, until) VALUES ($1, $2)", &[&token_hash(&code), &(now + Duration::minutes(10))]).await?;
        self.refresh_local(c).await?;
        Ok(code)
    }

    /// Invite a person: a single-use token, good for 7 days, is stored by
    /// its hash and returned to pass on. The person is local or in the
    /// config; someone with neither a password nor a record is marked
    /// invited, anyone else keeps their status, so an invite never
    /// revokes working access. Callers pass a real person.
    pub async fn invite(&self, person: &str, email: Option<&str>) -> Result<String> {
        let person = key(person);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let tx = c.transaction().await?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext('magnus-accounts'))").await?;
        let now = Utc::now();
        tx.execute("DELETE FROM public.account_invites WHERE expires <= $1", &[&now]).await?;
        let mut raw = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let token = hex::encode(raw);
        tx.execute("DELETE FROM public.account_invites WHERE person = $1", &[&person]).await?;
        tx.execute(
            "INSERT INTO public.account_invites (hash, person, email, expires) VALUES ($1, $2, $3, $4)",
            &[&token_hash(&token), &person, &email, &(now + Duration::days(INVITE_DAYS))],
        )
        .await?;
        let known = tx.query_opt("SELECT 1 FROM public.account_statuses WHERE person = $1", &[&person]).await?.is_some()
            || tx.query_opt("SELECT 1 FROM public.account_passwords WHERE person = $1", &[&person]).await?.is_some();
        if !known {
            tx.execute("INSERT INTO public.account_statuses (person, status) VALUES ($1, 'invited') ON CONFLICT (person) DO NOTHING", &[&person]).await?;
        }
        tx.commit().await?;
        self.refresh_local(c).await?;
        Ok(token)
    }

    /// Accept an invitation: the token with the new password activates the
    /// person. The person it activated, or None for a bad, used or expired
    /// token (all answered alike).
    pub async fn accept(&self, token: &str, password: &str) -> Result<Option<String>> {
        self.accept_at(token, password, Utc::now()).await
    }

    /// `accept` at `at`, for tests.
    pub async fn accept_at(&self, token: &str, password: &str, at: DateTime<Utc>) -> Result<Option<String>> {
        // The password is judged before the token is touched, so a short one
        // does not burn the invitation.
        anyhow::ensure!(password.chars().count() >= self.cfg.min_password_length, "a password needs at least {} characters", self.cfg.min_password_length);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let tx = c.transaction().await?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext('magnus-accounts'))").await?;
        tx.execute("DELETE FROM public.account_invites WHERE expires <= $1", &[&at]).await?;
        // A disabling stands against an outstanding invitation: the token
        // stays live for a re-enabling, but accepting refuses alike and
        // consumes nothing (#449).
        let held: Option<String> = tx.query_opt("SELECT person FROM public.account_invites WHERE hash = $1", &[&token_hash(token.trim())]).await?.map(|r| r.get(0));
        if let Some(person) = &held {
            let status: Option<String> = tx.query_opt("SELECT status FROM public.account_statuses WHERE person = $1", &[person]).await?.map(|r| r.get(0));
            if status.as_deref() == Some("disabled") {
                tx.commit().await?;
                return Ok(None);
            }
        }
        // Consumed by the delete itself, so racing accepts admit exactly
        // one (#450).
        let consumed: Option<String> = tx.query_opt("DELETE FROM public.account_invites WHERE hash = $1 RETURNING person", &[&token_hash(token.trim())]).await?.map(|r| r.get(0));
        let Some(person) = consumed else {
            tx.commit().await?;
            return Ok(None);
        };
        let salt = SaltString::generate(&mut OsRng);
        let phc = Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|e| anyhow::anyhow!("hash: {e}"))?.to_string();
        tx.execute("INSERT INTO public.account_passwords (person, phc) VALUES ($1, $2) ON CONFLICT (person) DO UPDATE SET phc = EXCLUDED.phc", &[&person, &phc]).await?;
        tx.execute("INSERT INTO public.account_statuses (person, status) VALUES ($1, 'active') ON CONFLICT (person) DO UPDATE SET status = 'active'", &[&person]).await?;
        tx.commit().await?;
        self.refresh_local(c).await?;
        Ok(Some(person))
    }

    /// Until when a person may not try another password, after too many
    /// wrong ones: the appliance's own and the identity provider's alike.
    pub fn locked(&self, person: &str) -> Option<DateTime<Utc>> {
        let person = key(person);
        let now = Utc::now();
        let window = Duration::minutes(self.cfg.lockout_minutes as i64);
        let mut f = self.failures.lock().unwrap();
        let (n, first) = f.get(person.as_str()).copied()?;
        if now - first > window {
            f.remove(person.as_str());
            return None;
        }
        (n >= self.cfg.max_failures).then_some(first + window)
    }

    /// Count a wrong password. Only for people of this appliance, so
    /// guessing names cannot grow the table.
    pub fn failed(&self, person: &str) {
        let person = key(person);
        let mut f = self.failures.lock().unwrap();
        let e = f.entry(person).or_insert((0, Utc::now()));
        e.0 += 1;
    }

    pub fn succeeded(&self, person: &str) {
        let person = key(person);
        self.failures.lock().unwrap().remove(person.as_str());
    }

    /// Take back one counted attempt: one counted before its answer came,
    /// whose answer said nothing about the password (the provider was down).
    pub fn forgive(&self, person: &str) {
        let person = key(person);
        let mut f = self.failures.lock().unwrap();
        if let Some(e) = f.get_mut(person.as_str()) {
            e.0 = e.0.saturating_sub(1);
        }
    }

    /// The counter that guards the owner's backup password: its own, so
    /// wrong passwords sent to the identity provider from anywhere cannot
    /// lock the owner out of the backup on the appliance's machine.
    pub fn backup_counter(person: &str) -> String {
        format!("backup:{person}")
    }

    /// The counter for attempts made over the network. Every counter is a
    /// scope and the whole person id, so no two people's counters meet,
    /// whatever their ids.
    pub fn network_counter(person: &str) -> String {
        format!("net:{person}")
    }

    /// The counter for attempts made on the appliance's own machine: its own,
    /// so wrong passwords sent from the network cannot lock the person out
    /// of the appliance itself.
    pub fn local_counter(person: &str) -> String {
        format!("local:{person}")
    }

    /// The counter for console sign-in attempts: its own, so guessing at
    /// the console cannot lock the person's apps, and guessing at the apps
    /// cannot lock the console (#429).
    pub fn console_counter(person: &str) -> String {
        format!("console:{person}")
    }

    /// End every session of `person`; the ids of those ended.
    pub async fn end_all(&self, person: &str) -> Result<Vec<String>> {
        let person = key(person);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let mut ended = vec![];
        for r in c.query("DELETE FROM public.account_sessions WHERE person = $1 RETURNING id", &[&person]).await? {
            ended.push(r.get("id"));
        }
        self.refresh_local(c).await?;
        Ok(ended)
    }

    /// The people with a live session.
    pub async fn signed_in(&self) -> std::collections::BTreeSet<String> {
        let mut g = match self.client().await {
            Ok(g) => g,
            Err(_) => return std::collections::BTreeSet::new(),
        };
        let c = match g.as_mut() {
            Some(c) => c,
            None => return std::collections::BTreeSet::new(),
        };
        let _ = self.fresh_local(c).await;
        c.query("SELECT DISTINCT person FROM public.account_sessions WHERE expires > now()", &[]).await.map(|rows| rows.into_iter().map(|r| r.get(0)).collect()).unwrap_or_default()
    }

    /// Take a one-time code printed on the appliance's machine
    /// (`magnus sign-in-code`): true once, if it is current.
    pub async fn take_machine_code(&self, code: &str) -> Result<bool> {
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let now = Utc::now();
        c.execute("DELETE FROM public.account_machine_codes WHERE until <= $1", &[&now]).await?;
        let n = c.execute("DELETE FROM public.account_machine_codes WHERE hash = $1 AND until > $2", &[&token_hash(code.trim()), &now]).await?;
        self.refresh_local(c).await?;
        Ok(n == 1)
    }

    /// Check a password. The caller opens the session.
    pub async fn sign_in(&self, person: &str, password: &str) -> Result<std::result::Result<(), SignInError>> {
        self.sign_in_counted(person, password, &Self::network_counter(person)).await
    }

    /// Check a password, counting wrong ones on `counter`.
    pub async fn sign_in_counted(&self, person: &str, password: &str, counter: &str) -> Result<std::result::Result<(), SignInError>> {
        let person = key(person);
        let counter = key(counter);
        if let Some(until) = self.locked(&counter) {
            return Ok(Err(SignInError::Locked(until)));
        }
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let phc: Option<String> = c.query_opt("SELECT phc FROM public.account_passwords WHERE person = $1", &[&person]).await?.map(|r| r.get(0));
        if !Self::password_ok(phc.as_deref(), password) || phc.is_none() {
            if phc.is_some() {
                self.failed(&counter);
            }
            return Ok(Err(SignInError::Rejected));
        }
        self.succeeded(&counter);
        Ok(Ok(()))
    }

    fn password_ok(phc: Option<&str>, password: &str) -> bool {
        // Verify against a throwaway hash when the person is unknown, so
        // timing does not tell which names exist.
        static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let dummy = DUMMY.get_or_init(|| {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default().hash_password(salt.as_str().as_bytes(), &salt).map(|h| h.to_string()).unwrap_or_default()
        });
        PasswordHash::new(phc.unwrap_or(dummy)).map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok()).unwrap_or(false)
    }

    /// Check a password and, where the person enrolled one, the code from
    /// their authenticator app. A missing code answers `NeedsCode`
    /// (uncounted); a wrong code rejects and counts, like a wrong password.
    pub async fn sign_in_counted_with_code(&self, person: &str, password: &str, code: Option<&str>, counter: &str) -> Result<std::result::Result<(), SignInError>> {
        self.sign_in_counted_with_code_at(person, password, code, counter, Utc::now()).await
    }

    /// `sign_in_counted_with_code` at `at`, for tests.
    pub async fn sign_in_counted_with_code_at(&self, person: &str, password: &str, code: Option<&str>, counter: &str, at: DateTime<Utc>) -> Result<std::result::Result<(), SignInError>> {
        let person = key(person);
        let counter = key(counter);
        if let Some(until) = self.locked(&counter) {
            return Ok(Err(SignInError::Locked(until)));
        }
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let phc: Option<String> = c.query_opt("SELECT phc FROM public.account_passwords WHERE person = $1", &[&person]).await?.map(|r| r.get(0));
        if !Self::password_ok(phc.as_deref(), password) || phc.is_none() {
            if phc.is_some() {
                self.failed(&counter);
            }
            return Ok(Err(SignInError::Rejected));
        }
        // After the password, so timing does not tell the status apart, and
        // before the code, so no NeedsCode betrays a right password for
        // someone turned off. Uncounted: the password was not wrong.
        let status: Option<String> = c.query_opt("SELECT status FROM public.account_statuses WHERE person = $1", &[&person]).await?.map(|r| r.get(0));
        if status.as_deref().is_some_and(|s| s != "active") {
            return Ok(Err(SignInError::Rejected));
        }
        // The password was right, so the person is known: a wrong code
        // counts on their counter, as a wrong password does.
        let secret: Option<String> = c.query_opt("SELECT secret FROM public.account_totp WHERE person = $1", &[&person]).await?.map(|r| r.get(0));
        let Some(secret) = secret else {
            self.succeeded(&counter);
            return Ok(Ok(()));
        };
        match code.map(str::trim).filter(|c| !c.is_empty()) {
            None => Ok(Err(SignInError::NeedsCode)),
            Some(one) => match crate::totp::verify_step(&secret, one, at) {
                Some(matched) => {
                    // Recorded by compare-and-set, so concurrent uses of
                    // the code admit exactly one; a used or passed step
                    // rejects and counts, like a wrong code (#441).
                    let n = c
                        .execute(
                            "INSERT INTO public.account_totp_steps (person, step) VALUES ($1, $2)
                             ON CONFLICT (person) DO UPDATE SET step = EXCLUDED.step WHERE account_totp_steps.step < EXCLUDED.step",
                            &[&person, &matched],
                        )
                        .await?;
                    if n != 1 {
                        self.failed(&counter);
                        return Ok(Err(SignInError::Rejected));
                    }
                    self.succeeded(&counter);
                    Ok(Ok(()))
                }
                None => {
                    self.failed(&counter);
                    Ok(Err(SignInError::Rejected))
                }
            },
        }
    }

    /// Open a session for a person whose identity is already established.
    /// `device` is the paired computer, when this sign-in presented one.
    pub async fn open_session(&self, person: &str, client: &str, device: Option<&str>) -> Result<(Session, String)> {
        let person = key(person);
        let now = Utc::now();
        let mut raw = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let token = format!("mgs_{}", hex::encode(raw));
        let session =
            Session { id: crate::kernel::new_id("sess"), person, created: now, expires: now + Duration::days(self.cfg.session_days as i64), client: client.to_string(), device: device.map(str::to_string) };
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        c.execute(
            "INSERT INTO public.account_sessions (hash, id, person, created, expires, client, device) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[&token_hash(&token), &session.id, &session.person, &session.created, &session.expires, &session.client, &session.device],
        )
        .await?;
        self.refresh_local(c).await?;
        Ok((session, token))
    }

    /// The live session a token belongs to: unexpired, its row still
    /// there, and its person still active. Read from the store on every
    /// call, so a removal or disabling from any process revokes here at
    /// once, and a re-added id never inherits ended sessions (#469).
    pub async fn session(&self, token: &str) -> Option<Session> {
        let mut g = self.client().await.ok()?;
        let c = g.as_mut()?;
        let _ = self.fresh_local(c).await;
        c.query_opt(
            "SELECT s.id, s.person, s.created, s.expires, s.client, s.device FROM public.account_sessions s
             LEFT JOIN public.account_statuses t ON t.person = s.person
             WHERE s.hash = $1 AND s.expires > now() AND (t.status IS NULL OR t.status = 'active')",
            &[&token_hash(token)],
        )
        .await
        .ok()
        .flatten()
        .map(|r| session_of(&r))
    }

    /// A person's live sessions, newest first, each with whether it is the
    /// one `token` belongs to.
    pub async fn sessions_of(&self, person: &str, token: &str) -> Vec<(Session, bool)> {
        let person = key(person);
        let mut g = match self.client().await {
            Ok(g) => g,
            Err(_) => return vec![],
        };
        let c = match g.as_mut() {
            Some(c) => c,
            None => return vec![],
        };
        let _ = self.fresh_local(c).await;
        let mine = token_hash(token);
        c.query("SELECT hash, id, person, created, expires, client, device FROM public.account_sessions WHERE person = $1 AND expires > now() ORDER BY created DESC", &[&person])
            .await
            .map(|rows| rows.into_iter().map(|r| (session_of(&r), r.get::<_, String>("hash") == mine)).collect())
            .unwrap_or_default()
    }

    /// End one of a person's sessions by its id: that device is signed out.
    pub async fn end(&self, person: &str, id: &str) -> Result<Option<Session>> {
        let person = key(person);
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let gone =
            c.query_opt("DELETE FROM public.account_sessions WHERE person = $1 AND id = $2 RETURNING id, person, created, expires, client, device", &[&person, &id]).await?.map(|r| session_of(&r));
        self.refresh_local(c).await?;
        Ok(gone)
    }

    /// Remember which paired computer a session belongs to. Pairing does
    /// this for the sign-in that asked for the certificate, which is on the
    /// LAN and so was not bound on its own.
    pub async fn bind_session_device(&self, token: &str, device: &str) -> Result<()> {
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        c.execute("UPDATE public.account_sessions SET device = $1 WHERE hash = $2", &[&device, &token_hash(token)]).await?;
        self.refresh_local(c).await?;
        Ok(())
    }

    /// End every session bound to a device, and return their ids. Revoking
    /// the device's certificate does this, so a stolen computer's saved
    /// sign-in stops working with the certificate.
    pub async fn end_device(&self, device: &str) -> Result<Vec<Session>> {
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let ended: Vec<Session> = c
            .query("DELETE FROM public.account_sessions WHERE device = $1 RETURNING id, person, created, expires, client, device", &[&device])
            .await?
            .into_iter()
            .map(|r| session_of(&r))
            .collect();
        self.refresh_local(c).await?;
        Ok(ended)
    }

    pub async fn sign_out(&self, token: &str) -> Result<Option<Session>> {
        let mut g = self.client().await?;
        let c = g.as_mut().context("accounts connection")?;
        let gone = c.query_opt("DELETE FROM public.account_sessions WHERE hash = $1 RETURNING id, person, created, expires, client, device", &[&token_hash(token)]).await?.map(|r| session_of(&r));
        self.refresh_local(c).await?;
        Ok(gone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SignIn;

    fn cfg() -> SignIn {
        SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None }
    }

    fn server_url() -> String {
        std::env::var("MAGNUS_TEST_PG").unwrap_or_else(|_| "postgres://postgres:magnus_dev@127.0.0.1:54340/postgres".into())
    }

    /// A fresh database per test, dropped when it ends. Mirrors
    /// `magnus_invariants::pg::TestDb`, which this crate cannot reuse
    /// (that crate depends on this one).
    struct TestDb {
        name: String,
    }

    /// A migrated fresh database and a client on it as the server user.
    async fn db() -> (TestDb, String) {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let name = format!("a_{id}");
        let (client, conn) = tokio_postgres::connect(&server_url(), tokio_postgres::NoTls).await.expect("connect to the test Postgres");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        client.batch_execute(&format!("CREATE DATABASE {name}")).await.expect("create test database");
        let mut u = url::Url::parse(&server_url()).unwrap();
        u.set_path(&format!("/{name}"));
        let url = u.to_string();
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        client.batch_execute(&migrate_sql("postgres")).await.unwrap();
        (TestDb { name }, url)
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            let name = self.name.clone();
            // Drop runs inside the test's runtime, which cannot be blocked
            // on; the cleanup gets a thread and runtime of its own.
            let _ = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async move {
                    let (client, conn) = tokio_postgres::connect(&server_url(), tokio_postgres::NoTls).await.expect("connect to the test Postgres");
                    tokio::spawn(async move {
                        let _ = conn.await;
                    });
                    client.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).await.expect("drop test database");
                })
            })
            .join();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn revoking_a_device_ends_its_sessions_and_a_session_without_one_still_loads() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        let (_, token) = accounts.open_session("ada", "app", None).await.unwrap();
        accounts.bind_session_device(&token, "dev_this").await.unwrap();
        let (_, other) = accounts.open_session("ada", "app", Some("dev_other")).await.unwrap();
        let ended = accounts.end_device("dev_this").await.unwrap();
        assert_eq!(ended.len(), 1);
        assert!(accounts.session(&token).await.is_none(), "the revoked device's session was still live");
        assert_eq!(accounts.session(&other).await.unwrap().device.as_deref(), Some("dev_other"));
        let (_, plain) = accounts.open_session("ada", "app", None).await.unwrap();
        assert!(accounts.session(&plain).await.unwrap().device.is_none(), "a session with no device did not load");
    }

    /// Enrolling a second factor asks for the code afterwards: the right
    /// password alone answers `NeedsCode`, the password with the current
    /// code signs in, and a wrong code is rejected and counted, like a
    /// wrong password (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_factor_asks_for_the_code_and_counts_a_wrong_one() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, SignIn { session_days: 1, max_failures: 3, lockout_minutes: 15, min_password_length: 10, oidc: None }).await.unwrap();
        accounts.set_password("ada", "a-long-password").await.unwrap();
        assert!(!accounts.has_totp("ada").await);
        let uri = accounts.enroll_totp("ada").await.unwrap();
        assert!(uri.starts_with("otpauth://totp/"), "not an enrollment URI: {uri}");
        assert!(accounts.has_totp("ada").await, "enrollment did not stick");
        let at = chrono::Utc::now();
        let secret = uri.split("secret=").nth(1).and_then(|s| s.split('&').next()).context("the enrollment URI carries no secret").unwrap();
        let secret = crate::totp::base32_decode(secret).unwrap();
        let code = crate::totp::generate(&secret, (at.timestamp().div_euclid(30)) as u64, 6);
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", None, &counter, at).await, Ok(Err(SignInError::NeedsCode))), "the password alone signed in past the second factor");
        assert!(accounts.locked(&counter).is_none(), "needing the code counted as a failure");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some("000000"), &counter, at).await, Ok(Err(SignInError::Rejected))), "a wrong code did not reject");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&code), &counter, at).await, Ok(Ok(()))), "the current code did not sign in");
        for _ in 0..3 {
            let _ = accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some("000000"), &counter, at).await;
        }
        assert!(accounts.locked(&counter).is_some(), "wrong codes did not lock the counter");
    }

    /// A code signs in once: replaying it within the window rejects and
    /// counts, while the next step's code still works; eight tasks racing
    /// the same code admit exactly one (#441).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_code_signs_in_once() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, SignIn { session_days: 1, max_failures: 100, lockout_minutes: 15, min_password_length: 10, oidc: None }).await.unwrap();
        accounts.set_password("ada", "a-long-password").await.unwrap();
        let uri = accounts.enroll_totp("ada").await.unwrap();
        let secret = uri.split("secret=").nth(1).and_then(|s| s.split('&').next()).unwrap();
        let secret = crate::totp::base32_decode(secret).unwrap();
        let counter = Accounts::network_counter("ada");
        let at = chrono::Utc::now();
        let step = at.timestamp().div_euclid(30) as u64;
        let code = crate::totp::generate(&secret, step, 6);
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&code), &counter, at).await, Ok(Ok(()))), "the first use of the code did not sign in");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&code), &counter, at).await, Ok(Err(SignInError::Rejected))), "replaying the code signed in again");
        // Eight tasks race the next step's code: every one of them verifies
        // it (it is live), but the step admits exactly one.
        let accounts = std::sync::Arc::new(accounts);
        let race_code = crate::totp::generate(&secret, step + 1, 6);
        let mut tasks = vec![];
        for _ in 0..8 {
            let (a, c, counter) = (accounts.clone(), race_code.clone(), counter.clone());
            tasks.push(tokio::spawn(async move { matches!(a.sign_in_counted_with_code("ada", "a-long-password", Some(&c), &counter).await, Ok(Ok(()))) }));
        }
        let mut admitted = 0u32;
        for t in tasks {
            admitted += t.await.unwrap() as u32;
        }
        assert_eq!(admitted, 1, "the raced code did not admit exactly one task");
        let next = crate::totp::generate(&secret, step + 2, 6);
        let later = chrono::DateTime::from_timestamp((step + 2) as i64 * 30 + 5, 0).unwrap();
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&next), &counter, later).await, Ok(Ok(()))), "the next step's code did not sign in");
    }

    /// An invitation activates on accept: invited cannot sign in, the token
    /// with a password activates once, and a second use, a bad token or an
    /// expired one answers nothing (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn invite_accept_activates_and_consumes() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        let token = accounts.invite("ada", Some("ada@example.com")).await.unwrap();
        assert_eq!(accounts.status("ada").await, Status::Invited, "an invite did not mark the person invited");
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "whatever-password", None, &counter).await, Ok(Err(SignInError::Rejected))), "an invited person signed in");
        assert_eq!(accounts.accept(&token, "a-long-password").await.unwrap(), Some("ada".to_string()), "accepting the invite did not activate");
        assert_eq!(accounts.status("ada").await, Status::Active, "accepting left the person invited");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter).await, Ok(Ok(()))), "the accepted person cannot sign in");
        assert_eq!(accounts.accept(&token, "another-long-password").await.unwrap(), None, "the invite worked twice");
        assert_eq!(accounts.accept("deadbeef", "another-long-password").await.unwrap(), None, "a bad token accepted");
        let stale = accounts.invite("ada", None).await.unwrap();
        let past = chrono::Utc::now() + chrono::Duration::days(8);
        assert_eq!(accounts.accept_at(&stale, "another-long-password", past).await.unwrap(), None, "an expired invite accepted");
        let kept = accounts.invite("ada", None).await.unwrap();
        assert!(accounts.accept(&kept, "short").await.is_err(), "a short password accepted");
        assert_eq!(accounts.accept(&kept, "another-long-password").await.unwrap(), Some("ada".to_string()), "the short password burned the invite");
        // A password set directly completes the invitation too — but never a
        // disabling.
        accounts.invite("bea", None).await.unwrap();
        accounts.set_password("bea", "bea-long-password").await.unwrap();
        assert_eq!(accounts.status("bea").await, Status::Active, "a set password left the invitee invited");
        accounts.set_status("bea", Status::Disabled).await.unwrap();
        accounts.set_password("bea", "bea-newer-password").await.unwrap();
        assert_eq!(accounts.status("bea").await, Status::Disabled, "a set password re-enabled a disabled person");
    }

    /// Turning a person off refuses sign-in and ends their live sessions;
    /// turning them back on restores sign-in (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn disabled_refuses_and_ends_sessions() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        accounts.set_password("ada", "a-long-password").await.unwrap();
        let (session, token) = accounts.open_session("ada", "app", None).await.unwrap();
        let counter = Accounts::network_counter("ada");
        let ended = accounts.set_status("ada", Status::Disabled).await.unwrap();
        assert_eq!(ended, vec![session.id.clone()], "disabling did not end the live session");
        assert!(accounts.session(&token).await.is_none(), "the session survived disabling");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter).await, Ok(Err(SignInError::Rejected))), "a disabled person signed in");
        accounts.set_status("ada", Status::Active).await.unwrap();
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter).await, Ok(Ok(()))), "re-enabling did not restore sign-in");
    }

    /// Inviting someone already active (a resend) hands them a token without
    /// deactivating them (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn invite_keeps_an_active_person_active() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        accounts.set_password("ada", "a-long-password").await.unwrap();
        assert_eq!(accounts.status("ada").await, Status::Active, "a person with no record is not active");
        let token = accounts.invite("ada", None).await.unwrap();
        assert_eq!(accounts.status("ada").await, Status::Active, "inviting deactivated an active person");
        assert_eq!(accounts.accept(&token, "a-new-long-password").await.unwrap(), Some("ada".to_string()), "the resend token did not accept");
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-new-long-password", None, &counter).await, Ok(Ok(()))), "the resent password did not take");
    }

    /// A disabling from another handle (the CLI's shape: its own `Accounts`
    /// on the same database) ends this handle's live sessions for good:
    /// re-enabling restores sign-in, never the ended sessions (#448, #469).
    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_elsewhere_revokes_live_sessions() {
        let (_db, url) = db().await;
        let server = Accounts::open(&url, cfg()).await.unwrap();
        server.set_password("ada", "a-long-password").await.unwrap();
        let (_, token) = server.open_session("ada", "app", None).await.unwrap();
        assert!(server.session(&token).await.is_some(), "the session did not resolve before disabling");
        let cli = Accounts::open(&url, cfg()).await.unwrap();
        cli.set_status("ada", Status::Disabled).await.unwrap();
        assert!(server.session(&token).await.is_none(), "the session survived a disabling from another handle");
        cli.set_status("ada", Status::Active).await.unwrap();
        assert!(server.session(&token).await.is_none(), "re-enabling resurrected the ended session");
        let counter = Accounts::network_counter("ada");
        assert!(matches!(server.sign_in_counted_with_code("ada", "a-long-password", None, &counter).await, Ok(Ok(()))), "re-enabling did not restore sign-in");
    }

    /// Removing a person from another handle ends this handle's sessions
    /// for good: re-adding the id never resurrects the stale tokens, and
    /// the store holds no session for them (#469).
    #[tokio::test(flavor = "multi_thread")]
    async fn removal_elsewhere_ends_sessions_and_readding_resurrects_nothing() {
        let (_db, url) = db().await;
        let server = Accounts::open(&url, cfg()).await.unwrap();
        server.add_person("ada", "Ada", None).await.unwrap();
        server.set_password("ada", "a-long-password").await.unwrap();
        let (_, token) = server.open_session("ada", "app", None).await.unwrap();
        assert!(server.session(&token).await.is_some(), "the session did not resolve before removal");
        let cli = Accounts::open(&url, cfg()).await.unwrap();
        cli.remove_person("ada").await.unwrap();
        assert!(server.session(&token).await.is_none(), "the session survived a removal from another handle");
        cli.add_person("ada", "A New Ada", None).await.unwrap();
        assert!(server.session(&token).await.is_none(), "re-adding the id resurrected the ended session");
        assert!(server.sessions_of("ada", &token).await.is_empty(), "the store still holds the ended session");
    }

    /// An outstanding invitation cannot undo a disabling: accept refuses
    /// alike without consuming the token, which stays usable if the person
    /// is turned back on (#449).
    #[tokio::test(flavor = "multi_thread")]
    async fn an_invite_does_not_reenable() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        let token = accounts.invite("ada", None).await.unwrap();
        accounts.set_status("ada", Status::Disabled).await.unwrap();
        assert_eq!(accounts.accept(&token, "a-long-password").await.unwrap(), None, "accepting re-enabled a disabled person");
        assert_eq!(accounts.status("ada").await, Status::Disabled, "accepting flipped the status");
        accounts.set_status("ada", Status::Active).await.unwrap();
        assert_eq!(accounts.accept(&token, "a-long-password").await.unwrap(), Some("ada".to_string()), "the kept token did not survive the refusal");
    }

    /// Racing accepts of one invitation admit exactly one, round after
    /// round: the token is single-use under concurrency too (#450).
    #[tokio::test(flavor = "multi_thread")]
    async fn racing_accepts_admit_exactly_one() {
        let (_db, url) = db().await;
        let accounts = std::sync::Arc::new(Accounts::open(&url, cfg()).await.unwrap());
        for round in 0..10 {
            let person = format!("ada{round}");
            let token = accounts.invite(&person, None).await.unwrap();
            let mut tasks = vec![];
            for i in 0..8 {
                let (a, t) = (accounts.clone(), token.clone());
                tasks.push(tokio::spawn(async move {
                    let pw = format!("a-long-password-{i}");
                    matches!(a.accept(&t, &pw).await, Ok(Some(_)))
                }));
            }
            let mut admitted = 0u32;
            for t in tasks {
                admitted += t.await.unwrap() as u32;
            }
            assert_eq!(admitted, 1, "round {round}: racing accepts did not admit exactly one");
        }
    }

    /// Local roles default to member, set explicitly, and read back (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn roles_default_member_and_set_explicitly() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        assert_eq!(accounts.role("ada").await, Role::Member, "an unrecorded person is not a member");
        accounts.set_role("ada", Role::Admin).await.unwrap();
        assert_eq!(accounts.role("ada").await, Role::Admin, "setting admin did not stick");
        accounts.set_role("bea", Role::Member).await.unwrap();
        assert_eq!(accounts.role("bea").await, Role::Member, "setting member did not stick");
    }

    /// The last enabled admin keeps the role and the sign-in: demoting or
    /// disabling them refuses, a disabled admin record does not count, and a
    /// second admin unlocks both (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn last_admin_keeps_role_and_signin() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        accounts.set_password("ada", "a-long-password").await.unwrap();
        accounts.set_password("bea", "bea-long-password").await.unwrap();
        accounts.set_role("ada", Role::Admin).await.unwrap();
        let e = accounts.set_role("ada", Role::Member).await.unwrap_err();
        assert!(format!("{e:#}").contains("last admin"), "demoting the last admin refused wrongly: {e:#}");
        assert_eq!(accounts.role("ada").await, Role::Admin, "the refused demotion landed");
        let e = accounts.set_status("ada", Status::Disabled).await.unwrap_err();
        assert!(format!("{e:#}").contains("last admin"), "disabling the last admin refused wrongly: {e:#}");
        assert_eq!(accounts.status("ada").await, Status::Active, "the refused disabling landed");
        // A disabled admin on record does not free the switch: nobody
        // enabled would be left to administer.
        accounts.set_role("bea", Role::Admin).await.unwrap();
        accounts.set_status("bea", Status::Disabled).await.unwrap();
        assert!(accounts.set_role("ada", Role::Member).await.is_err(), "a disabled admin record freed the last enabled admin's role");
        accounts.set_status("bea", Status::Active).await.unwrap();
        accounts.set_role("ada", Role::Member).await.unwrap();
        accounts.set_status("ada", Status::Disabled).await.unwrap();
        assert_eq!(accounts.role("ada").await, Role::Member, "a second admin did not unlock the demotion");
        assert_eq!(accounts.status("ada").await, Status::Disabled, "a second admin did not unlock the disabling");
    }

    /// With no admin records the switch is unmanaged: disabling works, so
    /// the guard never locks an appliance that never assigned roles (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn no_admin_records_leaves_the_switch_unguarded() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        accounts.set_password("ada", "a-long-password").await.unwrap();
        accounts.set_status("ada", Status::Disabled).await.unwrap();
        assert_eq!(accounts.status("ada").await, Status::Disabled, "disabling with no admin records refused");
    }

    /// Local people round-trip and remove cleanly: adding records them,
    /// removing drops every local trace (person, password, code, status,
    /// role, invites, used steps), never touching another's; adding twice
    /// or with a wild id refuses, as does removing the last admin (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn local_people_round_trip_and_remove_cleanly() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        assert!(!accounts.is_local("ada"), "nobody local yet");
        accounts.add_person("ada", "Ada", Some("ada@example.com")).await.unwrap();
        assert!(accounts.is_local("ada"), "adding did not stick");
        let ada = accounts.local_person("ada").unwrap();
        assert_eq!((ada.name.as_str(), ada.email.as_deref()), ("Ada", Some("ada@example.com")), "the record came back wrong");
        assert!(accounts.add_person("ada", "Ada", None).await.is_err(), "adding twice worked");
        let long = "a".repeat(101);
        for wild in ["../x", "", "a/b", long.as_str()] {
            assert!(accounts.add_person(wild, "X", None).await.is_err(), "a wild id worked: {wild}");
        }
        accounts.set_password("ada", "a-long-password").await.unwrap();
        let uri = accounts.enroll_totp("ada").await.unwrap();
        let secret = uri.split("secret=").nth(1).and_then(|s| s.split('&').next()).unwrap();
        let now = chrono::Utc::now();
        let code = crate::totp::generate(&crate::totp::base32_decode(secret).unwrap(), (now.timestamp() / 30) as u64, 6);
        accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&code), "console", now).await.unwrap();
        let ada_token = accounts.invite("ada", None).await.unwrap();
        let bea_token = accounts.invite("bea", None).await.unwrap();
        accounts.set_role("ada", Role::Admin).await.unwrap();
        accounts.add_person("bea", "Bea", None).await.unwrap();
        accounts.set_role("bea", Role::Admin).await.unwrap();
        assert_eq!(accounts.accept(&bea_token, "bea-long-password").await.unwrap(), Some("bea".to_string()), "bea's invite did not work");
        accounts.remove_person("ada").await.unwrap();
        assert!(!accounts.is_local("ada"), "removing left the person");
        assert!(!accounts.has_password("ada").await, "removing left the password");
        assert!(!accounts.has_totp("ada").await, "removing left the code");
        assert_eq!(accounts.status("ada").await, Status::Active, "removing left the status");
        assert_eq!(accounts.role("ada").await, Role::Member, "removing left the role");
        assert_eq!(accounts.accept(&ada_token, "another-long-password").await.unwrap(), None, "removing left the invite");
        assert!(accounts.has_password("bea").await, "removing ada burned bea's account");
        let left: i64 = {
            let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(async move {
                let _ = conn.await;
            });
            client.query_one("SELECT COUNT(*) FROM public.account_totp_steps WHERE person = 'ada'", &[]).await.unwrap().get(0)
        };
        assert_eq!(left, 0, "removing left used steps");
        let e = accounts.remove_person("bea").await.unwrap_err();
        assert!(format!("{e:#}").contains("last admin"), "removing the last admin refused wrongly: {e:#}");
        // A second admin unlocks the removal.
        accounts.add_person("ada", "Ada", None).await.unwrap();
        accounts.set_role("ada", Role::Admin).await.unwrap();
        accounts.remove_person("bea").await.unwrap();
        assert!(!accounts.is_local("bea"), "a second admin did not unlock the removal");
        assert!(accounts.remove_person("ghost").await.is_err(), "removing nobody worked");
    }

    /// Without an enrolled second factor, the code path behaves exactly as
    /// the password path: the feature is off until enrolled (#429).
    #[tokio::test(flavor = "multi_thread")]
    async fn no_second_factor_behaves_as_passwords_alone() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        accounts.set_password("ada", "a-long-password").await.unwrap();
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter).await, Ok(Ok(()))), "a password without a second factor stopped signing in");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "wrong-password", None, &counter).await, Ok(Err(SignInError::Rejected))), "a wrong password stopped rejecting");
        assert!(matches!(accounts.sign_in_counted_with_code("nobody", "whatever-password", None, &counter).await, Ok(Err(SignInError::Rejected))), "an unknown name stopped rejecting");
    }

    /// Each counter is its scope then the whole person id, so no person's
    /// network counter is another's local or backup one, whatever their ids
    /// (review of #154: `local:carol` from the network locked carol locally).
    #[tokio::test(flavor = "multi_thread")]
    async fn one_persons_counter_is_never_anothers() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        for _ in 0..5 {
            accounts.failed(&Accounts::network_counter("local:carol"));
            accounts.failed(&Accounts::network_counter("backup:carol"));
        }
        assert!(accounts.locked(&Accounts::network_counter("local:carol")).is_some());
        assert!(accounts.locked(&Accounts::local_counter("carol")).is_none(), "local:carol's network failures locked carol on this machine");
        assert!(accounts.locked(&Accounts::backup_counter("carol")).is_none(), "backup:carol's network failures locked carol's backup password");
    }

    /// Person ids are lowercase-canonical: sign-in lowercases at entry,
    /// so creating `Ada` keeps `ada`, and any casing signs in, holds the
    /// role and status, and collides on re-adding (#470).
    #[tokio::test(flavor = "multi_thread")]
    async fn person_ids_are_lowercase_canonical() {
        let (_db, url) = db().await;
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        accounts.add_person("Ada", "Ada", None).await.unwrap();
        assert!(accounts.is_local("ada"), "adding Ada did not keep ada");
        assert!(accounts.is_local("ADA"), "the casing did not resolve");
        assert!(accounts.add_person("ada", "ada", None).await.is_err(), "ada re-added beside Ada");
        accounts.set_password("ADA", "a-long-password").await.unwrap();
        accounts.set_role("aDa", Role::Admin).await.unwrap();
        for who in ["ada", "Ada", "ADA"] {
            assert!(matches!(accounts.sign_in(who, "a-long-password").await.unwrap(), Ok(())), "{who} cannot sign in");
            assert_eq!(accounts.role(who).await, Role::Admin, "{who} lost the role");
        }
        let token = accounts.invite("Bo", None).await.unwrap();
        assert_eq!(accounts.accept(&token, "bo-long-password").await.unwrap(), Some("bo".to_string()), "the invite did not canonicalize");
        assert!(matches!(accounts.sign_in("BO", "bo-long-password").await.unwrap(), Ok(())), "BO cannot sign in");
        accounts.set_status("bO", Status::Disabled).await.unwrap();
        assert_eq!(accounts.status("bo").await, Status::Disabled, "the disabling missed");
    }

    /// `magnus migrate` brings the JSON files' contents into the tables
    /// once, so the cutover keeps everyone signing in; a rerun changes
    /// nothing (#466).
    #[tokio::test(flavor = "multi_thread")]
    async fn migrate_imports_the_json_files_once() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("accounts");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("people.json"), r#"{"ada":{"name":"Ada","email":"ada@example.com"},"Bea":{"name":"Bea"}}"#).unwrap();
        let salt = SaltString::generate(&mut OsRng);
        let phc = Argon2::default().hash_password(b"a-long-password", &salt).unwrap().to_string();
        std::fs::write(d.join("passwords.json"), serde_json::to_string(&BTreeMap::from([("ada".to_string(), phc)])).unwrap()).unwrap();
        std::fs::write(d.join("status.json"), r#"{"ada":"active"}"#).unwrap();
        std::fs::write(d.join("roles.json"), r#"{"ada":"admin"}"#).unwrap();
        let (_db, url) = db().await;
        let (mut client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let n = import_json(&mut client, dir.path()).await.unwrap();
        assert_eq!(n, 5, "the import wrote {n} rows, not the two people, password, status and role");
        let accounts = Accounts::open(&url, cfg()).await.unwrap();
        assert!(accounts.is_local("ada"), "the person did not survive the import");
        assert!(accounts.local_people().iter().all(|(id, _)| *id == id.to_lowercase()), "the import kept an uppercase key");
        assert_eq!(accounts.role("ada").await, Role::Admin, "the role did not survive the import");
        assert!(matches!(accounts.sign_in("ada", "a-long-password").await.unwrap(), Ok(())), "the password did not survive the import");
        assert_eq!(import_json(&mut client, dir.path()).await.unwrap(), 0, "a rerun changed rows");
        // A removal after the import stays removed: the next migrate never
        // resurrects the person from the untouched files (review P1: every
        // deploy runs migrate).
        accounts.set_role("bea", Role::Admin).await.unwrap();
        accounts.remove_person("ada").await.unwrap();
        assert_eq!(import_json(&mut client, dir.path()).await.unwrap(), 0, "a migrate after a removal changed rows");
        assert!(!accounts.is_local("ada"), "the migrate resurrected the removed person");
        assert!(matches!(accounts.sign_in("ada", "a-long-password").await.unwrap(), Err(_)), "the resurrected password signs in");
    }
}
