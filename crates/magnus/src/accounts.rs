//! Password sign-in and sessions for people's apps. Sign-in through an
//! identity provider (`oidc`) opens the same sessions.
//!
//! Password hashes (Argon2id) and session token hashes live in the state
//! directory, never in the log: a session token is a credential (invariant 2).
//! The log records only that a person signed in or out, with the session id.

use crate::config::SignIn;
use crate::vault::write_private;
use anyhow::{Context, Result};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

/// Days an invitation stays usable.
const INVITE_DAYS: i64 = 7;

pub struct Accounts {
    dir: PathBuf,
    cfg: SignIn,
    /// Sessions by the SHA-256 of their token.
    sessions: Mutex<BTreeMap<String, Session>>,
    /// Failed attempts per person: (count, first failure).
    failures: Mutex<BTreeMap<String, (u32, DateTime<Utc>)>>,
    /// Last accepted authenticator step per person: a code signs in once,
    /// and concurrent uses of it admit exactly one (#441).
    totp_steps: Mutex<BTreeMap<String, i64>>,
    /// Serializes local-people changes (invites, roles, statuses):
    /// concurrent accepts of one token admit exactly one, concurrent
    /// invites never resurrect a consumed one, and two switches never
    /// strand the organisation with no admin (#450, #429). In-process
    /// only, like every lock here; the CLI is the rare second writer, as
    /// with the passwords file.
    people: Mutex<()>,
}

fn token_hash(token: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(token.as_bytes()))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).with_context(|| format!("{} does not parse", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

/// Where the password hashes live, for the `password` command.
pub fn dir(state_dir: &Path) -> PathBuf {
    state_dir.join("accounts")
}

/// A one-time code for the owner's backup password, valid ten minutes. Only
/// someone with a shell on the appliance's machine (as its user) can make
/// one, so a relay or tunnel that delivers remote connections from loopback
/// cannot carry the backup.
pub fn new_machine_code(state_dir: &Path) -> Result<String> {
    use rand::Rng;
    let d = dir(state_dir);
    std::fs::create_dir_all(&d)?;
    let path = d.join("machine-codes.json");
    let mut all: BTreeMap<String, DateTime<Utc>> = read_json(&path)?;
    let now = Utc::now();
    all.retain(|_, until| *until > now);
    let code: String = (0..10).map(|_| char::from(b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"[rand::rngs::OsRng.gen_range(0..32)])).collect();
    all.insert(token_hash(&code), now + Duration::minutes(10));
    crate::vault::write_private(&path, &serde_json::to_vec(&all)?)?;
    Ok(code)
}

/// Enroll a person's authenticator app: a fresh 160-bit secret is stored
/// beside the passwords, and the `otpauth://` URI to enroll is returned.
/// Re-enrolling replaces the secret. Used by the CLI on the appliance
/// itself; the console enrolls through the Users screen (#429).
pub fn enroll_totp(state_dir: &Path, person: &str) -> Result<String> {
    let d = dir(state_dir);
    std::fs::create_dir_all(&d)?;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    let path = d.join("totp.json");
    let mut all: BTreeMap<String, String> = read_json(&path)?;
    let mut raw = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let secret = crate::totp::base32_encode(&raw);
    all.insert(person.to_string(), secret.clone());
    write_private(&path, &serde_json::to_vec_pretty(&all)?)?;
    let label = percent_encoding::utf8_percent_encode(person, percent_encoding::NON_ALPHANUMERIC);
    Ok(format!("otpauth://totp/Magnus:{label}?secret={}&issuer=Magnus&digits=6&period=30", secret.trim_end_matches('=')))
}

/// Set a person's password. Used by the CLI on the appliance itself. A
/// password completes an invitation by any path, so someone invited becomes
/// active; anyone disabled stays disabled until explicitly turned back on.
pub fn set_password(state_dir: &Path, cfg: &SignIn, person: &str, password: &str) -> Result<()> {
    anyhow::ensure!(
        password.chars().count() >= cfg.min_password_length,
        "a password needs at least {} characters",
        cfg.min_password_length
    );
    let d = dir(state_dir);
    std::fs::create_dir_all(&d)?;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    let path = d.join("passwords.json");
    let mut all: BTreeMap<String, String> = read_json(&path)?;
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|e| anyhow::anyhow!("hash: {e}"))?.to_string();
    all.insert(person.to_string(), phc);
    write_private(&path, &serde_json::to_vec_pretty(&all)?)?;
    let spath = d.join("status.json");
    let mut statuses: BTreeMap<String, Status> = read_json(&spath)?;
    if statuses.get(person) == Some(&Status::Invited) {
        statuses.insert(person.to_string(), Status::Active);
        write_private(&spath, &serde_json::to_vec_pretty(&statuses)?)?;
    }
    Ok(())
}

impl Accounts {
    pub fn open(state_dir: &Path, cfg: SignIn) -> Result<Accounts> {
        let d = dir(state_dir);
        std::fs::create_dir_all(&d)?;
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
        let now = Utc::now();
        let mut sessions: BTreeMap<String, Session> = read_json(&d.join("sessions.json"))?;
        sessions.retain(|_, s| s.expires > now);
        let totp_steps: BTreeMap<String, i64> = read_json(&d.join("totp-steps.json"))?;
        Ok(Accounts { dir: d, cfg, sessions: Mutex::new(sessions), failures: Mutex::new(BTreeMap::new()), totp_steps: Mutex::new(totp_steps), people: Mutex::new(()) })
    }

    fn save(&self, sessions: &BTreeMap<String, Session>) -> Result<()> {
        write_private(&self.dir.join("sessions.json"), &serde_json::to_vec_pretty(sessions)?)
    }

    /// Whether the person has a password set (read fresh: the CLI may have set it).
    pub fn has_password(&self, person: &str) -> bool {
        read_json::<BTreeMap<String, String>>(&self.dir.join("passwords.json")).map(|m| m.contains_key(person)).unwrap_or(false)
    }

    /// Whether the person enrolled an authenticator app (read fresh: the
    /// CLI may have enrolled it).
    pub fn has_totp(&self, person: &str) -> bool {
        read_json::<BTreeMap<String, String>>(&self.dir.join("totp.json")).map(|m| m.contains_key(person)).unwrap_or(false)
    }

    /// Whether the person may sign in: active unless recorded otherwise
    /// (read fresh: the CLI may have changed it).
    pub fn status(&self, person: &str) -> Status {
        read_json::<BTreeMap<String, Status>>(&self.dir.join("status.json")).map(|m| m.get(person).copied().unwrap_or(Status::Active)).unwrap_or(Status::Active)
    }

    /// Turn a person on or off. Disabling ends their live sessions at once
    /// and returns the ids of those ended, for the caller to record.
    /// Refuses to disable the organisation's last enabled admin.
    pub fn set_status(&self, person: &str, status: Status) -> Result<Vec<String>> {
        let _g = self.people.lock().unwrap();
        if status == Status::Disabled && self.sole_enabled_admin(person)? {
            anyhow::bail!("This is the organisation's last admin: make someone else an admin first.");
        }
        let path = self.dir.join("status.json");
        let mut all: BTreeMap<String, Status> = read_json(&path)?;
        all.insert(person.to_string(), status);
        write_private(&path, &serde_json::to_vec_pretty(&all)?)?;
        if status == Status::Disabled {
            return self.end_all(person);
        }
        Ok(vec![])
    }

    /// What the person may administer: member unless recorded an admin
    /// (read fresh: the CLI may have changed it).
    pub fn role(&self, person: &str) -> Role {
        read_json::<BTreeMap<String, Role>>(&self.dir.join("roles.json")).map(|m| m.get(person).copied().unwrap_or(Role::Member)).unwrap_or(Role::Member)
    }

    /// Whether `person` is an enabled admin with no other: demoting or
    /// disabling them would strand the organisation. Unrecorded people are
    /// members, so an unmanaged switch never guards.
    fn sole_enabled_admin(&self, person: &str) -> Result<bool> {
        let roles: BTreeMap<String, Role> = read_json(&self.dir.join("roles.json"))?;
        let statuses: BTreeMap<String, Status> = read_json(&self.dir.join("status.json"))?;
        let enabled = |p: &str| roles.get(p) == Some(&Role::Admin) && statuses.get(p).copied().unwrap_or(Status::Active) == Status::Active;
        Ok(enabled(person) && !roles.keys().any(|p| p != person && enabled(p)))
    }

    /// Set a person's role. Refuses to demote the organisation's last
    /// enabled admin.
    pub fn set_role(&self, person: &str, role: Role) -> Result<()> {
        let _g = self.people.lock().unwrap();
        if role == Role::Member && self.sole_enabled_admin(person)? {
            anyhow::bail!("This is the organisation's last admin: make someone else an admin first.");
        }
        let path = self.dir.join("roles.json");
        let mut all: BTreeMap<String, Role> = read_json(&path)?;
        all.insert(person.to_string(), role);
        write_private(&path, &serde_json::to_vec_pretty(&all)?)
    }

    /// Invite a person: a single-use token, good for 7 days, is stored by
    /// its hash and returned to pass on; the person must already be in the
    /// config (the local user store lifts that in a later #429 slice).
    /// Someone with neither a password nor a record is marked invited;
    /// anyone else keeps their status, so an invite never revokes working
    /// access.
    pub fn invite(&self, person: &str, email: Option<&str>) -> Result<String> {
        let _g = self.people.lock().unwrap();
        let d = &self.dir;
        std::fs::create_dir_all(d)?;
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        let path = d.join("invites.json");
        let mut all: BTreeMap<String, Invite> = read_json(&path)?;
        let now = Utc::now();
        all.retain(|_, inv| inv.expires > now);
        let mut raw = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let token = hex::encode(raw);
        all.retain(|_, inv| inv.person != person);
        all.insert(token_hash(&token), Invite { person: person.to_string(), email: email.map(str::to_string), expires: now + Duration::days(INVITE_DAYS) });
        write_private(&path, &serde_json::to_vec_pretty(&all)?)?;
        let passwords: BTreeMap<String, String> = read_json(&d.join("passwords.json"))?;
        let spath = d.join("status.json");
        let mut statuses: BTreeMap<String, Status> = read_json(&spath)?;
        if !statuses.contains_key(person) && !passwords.contains_key(person) {
            statuses.insert(person.to_string(), Status::Invited);
            write_private(&spath, &serde_json::to_vec_pretty(&statuses)?)?;
        }
        Ok(token)
    }

    /// Accept an invitation: the token with the new password activates the
    /// person. The person it activated, or None for a bad, used or expired
    /// token (all answered alike).
    pub fn accept(&self, cfg: &SignIn, token: &str, password: &str) -> Result<Option<String>> {
        self.accept_at(cfg, token, password, Utc::now())
    }

    /// `accept` at `at`, for tests.
    pub fn accept_at(&self, cfg: &SignIn, token: &str, password: &str, at: DateTime<Utc>) -> Result<Option<String>> {
        // The password is judged before the token is touched, so a short one
        // does not burn the invitation.
        anyhow::ensure!(password.chars().count() >= cfg.min_password_length, "a password needs at least {} characters", cfg.min_password_length);
        let _g = self.people.lock().unwrap();
        let d = &self.dir;
        let path = d.join("invites.json");
        let mut all: BTreeMap<String, Invite> = read_json(&path)?;
        all.retain(|_, inv| inv.expires > at);
        // A disabling stands against an outstanding invitation: the token
        // stays live for a re-enabling, but accepting refuses alike and
        // consumes nothing (#449).
        if all.get(&token_hash(token.trim())).is_some_and(|inv| self.status(&inv.person) == Status::Disabled) {
            return Ok(None);
        }
        let Some(inv) = all.remove(&token_hash(token.trim())) else {
            write_private(&path, &serde_json::to_vec_pretty(&all)?)?;
            return Ok(None);
        };
        write_private(&path, &serde_json::to_vec_pretty(&all)?)?;
        // `set_password` takes the state dir; this dir is its `accounts`
        // child, so the parent is always there.
        let state = self.dir.parent().expect("the accounts dir has its state dir above it");
        set_password(state, cfg, &inv.person, password)?;
        let spath = d.join("status.json");
        let mut statuses: BTreeMap<String, Status> = read_json(&spath)?;
        statuses.insert(inv.person.clone(), Status::Active);
        write_private(&spath, &serde_json::to_vec_pretty(&statuses)?)?;
        Ok(Some(inv.person))
    }

    /// Until when a person may not try another password, after too many
    /// wrong ones: the appliance's own and the identity provider's alike.
    pub fn locked(&self, person: &str) -> Option<DateTime<Utc>> {
        let now = Utc::now();
        let window = Duration::minutes(self.cfg.lockout_minutes as i64);
        let mut f = self.failures.lock().unwrap();
        let (n, first) = f.get(person).copied()?;
        if now - first > window {
            f.remove(person);
            return None;
        }
        (n >= self.cfg.max_failures).then_some(first + window)
    }

    /// Count a wrong password. Only for people of this appliance, so
    /// guessing names cannot grow the table.
    pub fn failed(&self, person: &str) {
        let mut f = self.failures.lock().unwrap();
        let e = f.entry(person.to_string()).or_insert((0, Utc::now()));
        e.0 += 1;
    }

    pub fn succeeded(&self, person: &str) {
        self.failures.lock().unwrap().remove(person);
    }

    /// Take back one counted attempt: one counted before its answer came,
    /// whose answer said nothing about the password (the provider was down).
    pub fn forgive(&self, person: &str) {
        let mut f = self.failures.lock().unwrap();
        if let Some(e) = f.get_mut(person) {
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

    /// End every session of a person (the provider turned them off).
    /// End every session of `person`; the ids of those ended.
    pub fn end_all(&self, person: &str) -> Result<Vec<String>> {
        let mut s = self.sessions.lock().unwrap();
        let ended: Vec<String> = s.values().filter(|x| x.person == person).map(|x| x.id.clone()).collect();
        if !ended.is_empty() {
            s.retain(|_, x| x.person != person);
            self.save(&s)?;
        }
        Ok(ended)
    }

    /// The people with a live session.
    pub fn signed_in(&self) -> std::collections::BTreeSet<String> {
        let now = Utc::now();
        self.sessions.lock().unwrap().values().filter(|x| x.expires > now).map(|x| x.person.clone()).collect()
    }

    /// Take a one-time code printed on the appliance's machine
    /// (`magnus sign-in-code`): true once, if it is current.
    pub fn take_machine_code(&self, code: &str) -> Result<bool> {
        let _g = self.sessions.lock().unwrap();
        let path = self.dir.join("machine-codes.json");
        let mut all: BTreeMap<String, DateTime<Utc>> = read_json(&path)?;
        let now = Utc::now();
        all.retain(|_, until| *until > now);
        let found = all.remove(&token_hash(code.trim())).is_some();
        crate::vault::write_private(&path, &serde_json::to_vec(&all)?)?;
        Ok(found)
    }

    /// Check a password. The caller opens the session.
    pub fn sign_in(&self, person: &str, password: &str) -> Result<std::result::Result<(), SignInError>> {
        self.sign_in_counted(person, password, &Self::network_counter(person))
    }

    /// Check a password, counting wrong ones on `counter`.
    pub fn sign_in_counted(&self, person: &str, password: &str, counter: &str) -> Result<std::result::Result<(), SignInError>> {
        if let Some(until) = self.locked(counter) {
            return Ok(Err(SignInError::Locked(until)));
        }
        let all: BTreeMap<String, String> = read_json(&self.dir.join("passwords.json"))?;
        if !Self::password_ok(&all, person, password) {
            if all.contains_key(person) {
                self.failed(counter);
            }
            return Ok(Err(SignInError::Rejected));
        }
        self.succeeded(counter);
        Ok(Ok(()))
    }

    fn password_ok(all: &BTreeMap<String, String>, person: &str, password: &str) -> bool {
        // Verify against a throwaway hash when the person is unknown, so
        // timing does not tell which names exist.
        static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let dummy = DUMMY.get_or_init(|| {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default().hash_password(salt.as_str().as_bytes(), &salt).map(|h| h.to_string()).unwrap_or_default()
        });
        let phc = all.get(person).unwrap_or(dummy);
        PasswordHash::new(phc).map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok()).unwrap_or(false) && all.contains_key(person)
    }

    /// Check a password and, where the person enrolled one, the code from
    /// their authenticator app. A missing code answers `NeedsCode`
    /// (uncounted); a wrong code rejects and counts, like a wrong password.
    pub fn sign_in_counted_with_code(&self, person: &str, password: &str, code: Option<&str>, counter: &str) -> Result<std::result::Result<(), SignInError>> {
        self.sign_in_counted_with_code_at(person, password, code, counter, Utc::now())
    }

    /// `sign_in_counted_with_code` at `at`, for tests.
    pub fn sign_in_counted_with_code_at(&self, person: &str, password: &str, code: Option<&str>, counter: &str, at: DateTime<Utc>) -> Result<std::result::Result<(), SignInError>> {
        if let Some(until) = self.locked(counter) {
            return Ok(Err(SignInError::Locked(until)));
        }
        let all: BTreeMap<String, String> = read_json(&self.dir.join("passwords.json"))?;
        if !Self::password_ok(&all, person, password) {
            if all.contains_key(person) {
                self.failed(counter);
            }
            return Ok(Err(SignInError::Rejected));
        }
        // After the password, so timing does not tell the status apart, and
        // before the code, so no NeedsCode betrays a right password for
        // someone turned off. Uncounted: the password was not wrong.
        if self.status(person) != Status::Active {
            return Ok(Err(SignInError::Rejected));
        }
        // The password was right, so the person is known: a wrong code
        // counts on their counter, as a wrong password does.
        let secrets: BTreeMap<String, String> = read_json(&self.dir.join("totp.json"))?;
        let Some(secret) = secrets.get(person) else {
            self.succeeded(counter);
            return Ok(Ok(()));
        };
        match code.map(str::trim).filter(|c| !c.is_empty()) {
            None => Ok(Err(SignInError::NeedsCode)),
            Some(c) => match crate::totp::verify_step(secret, c, at) {
                Some(matched) => {
                    // Check and record under one lock, so concurrent uses of
                    // the code admit exactly one; a used or passed step
                    // rejects and counts, like a wrong code.
                    let mut used = self.totp_steps.lock().unwrap();
                    if used.get(person).is_some_and(|&last| matched <= last) {
                        self.failed(counter);
                        return Ok(Err(SignInError::Rejected));
                    }
                    used.insert(person.to_string(), matched);
                    write_private(&self.dir.join("totp-steps.json"), &serde_json::to_vec_pretty(&*used)?)?;
                    self.succeeded(counter);
                    Ok(Ok(()))
                }
                None => {
                    self.failed(counter);
                    Ok(Err(SignInError::Rejected))
                }
            },
        }
    }

    /// Open a session for a person whose identity is already established.
    /// `device` is the paired computer, when this sign-in presented one.
    pub fn open_session(&self, person: &str, client: &str, device: Option<&str>) -> Result<(Session, String)> {
        let now = Utc::now();
        let mut raw = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let token = format!("mgs_{}", hex::encode(raw));
        let session = Session {
            id: crate::kernel::new_id("sess"),
            person: person.to_string(),
            created: now,
            expires: now + Duration::days(self.cfg.session_days as i64),
            client: client.to_string(),
            device: device.map(str::to_string),
        };
        let mut s = self.sessions.lock().unwrap();
        s.insert(token_hash(&token), session.clone());
        self.save(&s)?;
        Ok((session, token))
    }

    /// The live session a token belongs to: unexpired, and its person
    /// still active. Status reads the file fresh, so a disabling from
    /// another process (the CLI's own handle) revokes here too, without
    /// reloading the session map (#448).
    pub fn session(&self, token: &str) -> Option<Session> {
        let found = self.sessions.lock().unwrap().get(&token_hash(token)).filter(|x| x.expires > Utc::now()).cloned()?;
        (self.status(&found.person) == Status::Active).then_some(found)
    }

    /// A person's live sessions, newest first, each with whether it is the
    /// one `token` belongs to.
    pub fn sessions_of(&self, person: &str, token: &str) -> Vec<(Session, bool)> {
        let now = Utc::now();
        let mine = token_hash(token);
        let s = self.sessions.lock().unwrap();
        let mut out: Vec<(Session, bool)> = s.iter().filter(|(_, x)| x.person == person && x.expires > now).map(|(h, x)| (x.clone(), *h == mine)).collect();
        out.sort_by(|a, b| b.0.created.cmp(&a.0.created));
        out
    }

    /// End one of a person's sessions by its id: that device is signed out.
    pub fn end(&self, person: &str, id: &str) -> Result<Option<Session>> {
        let mut s = self.sessions.lock().unwrap();
        let Some(h) = s.iter().find(|(_, x)| x.person == person && x.id == id).map(|(h, _)| h.clone()) else { return Ok(None) };
        let gone = s.remove(&h);
        self.save(&s)?;
        Ok(gone)
    }

    /// Remember which paired computer a session belongs to. Pairing does
    /// this for the sign-in that asked for the certificate, which is on the
    /// LAN and so was not bound on its own.
    pub fn bind_session_device(&self, token: &str, device: &str) -> Result<()> {
        let mut s = self.sessions.lock().unwrap();
        if let Some(session) = s.get_mut(&token_hash(token)) {
            session.device = Some(device.to_string());
            self.save(&s)?;
        }
        Ok(())
    }

    /// End every session bound to a device, and return their ids. Revoking
    /// the device's certificate does this, so a stolen computer's saved
    /// sign-in stops working with the certificate.
    pub fn end_device(&self, device: &str) -> Result<Vec<Session>> {
        let mut s = self.sessions.lock().unwrap();
        let ended: Vec<Session> = s.values().filter(|x| x.device.as_deref() == Some(device)).cloned().collect();
        if !ended.is_empty() {
            s.retain(|_, x| x.device.as_deref() != Some(device));
            self.save(&s)?;
        }
        Ok(ended)
    }

    pub fn sign_out(&self, token: &str) -> Result<Option<Session>> {
        let mut s = self.sessions.lock().unwrap();
        let gone = s.remove(&token_hash(token));
        if gone.is_some() {
            self.save(&s)?;
        }
        Ok(gone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SignIn;

    fn open(dir: &std::path::Path) -> Accounts {
        Accounts::open(
            dir,
            SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None },
        )
        .unwrap()
    }

    #[test]
    fn revoking_a_device_ends_its_sessions_and_an_old_session_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = open(dir.path());
        let (_, token) = accounts.open_session("ada", "app", None).unwrap();
        accounts.bind_session_device(&token, "dev_this").unwrap();
        let (_, other) = accounts.open_session("ada", "app", Some("dev_other")).unwrap();
        let ended = accounts.end_device("dev_this").unwrap();
        assert_eq!(ended.len(), 1);
        assert!(accounts.session(&token).is_none(), "the revoked device's session was still live");
        assert_eq!(accounts.session(&other).unwrap().device.as_deref(), Some("dev_other"));
        let old = r#"{"h":{"id":"sess_old","person":"ada","created":"2030-01-01T00:00:00Z","expires":"2030-02-01T00:00:00Z","client":"app"}}"#;
        let map: BTreeMap<String, Session> = serde_json::from_str(old).unwrap();
        assert!(map["h"].device.is_none(), "a session from before device certificates did not load");
    }

    /// Enrolling a second factor asks for the code afterwards: the right
    /// password alone answers `NeedsCode`, the password with the current
    /// code signs in, and a wrong code is rejected and counted, like a
    /// wrong password (#429).
    #[test]
    fn a_second_factor_asks_for_the_code_and_counts_a_wrong_one() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 3, lockout_minutes: 15, min_password_length: 10, oidc: None };
        set_password(dir.path(), &cfg, "ada", "a-long-password").unwrap();
        let accounts = Accounts::open(dir.path(), cfg).unwrap();
        assert!(!accounts.has_totp("ada"));
        let uri = enroll_totp(dir.path(), "ada").unwrap();
        assert!(uri.starts_with("otpauth://totp/"), "not an enrollment URI: {uri}");
        assert!(accounts.has_totp("ada"), "enrollment did not stick");
        let at = chrono::Utc::now();
        let secret = uri.split("secret=").nth(1).and_then(|s| s.split('&').next()).context("the enrollment URI carries no secret").unwrap();
        let secret = crate::totp::base32_decode(secret).unwrap();
        let code = crate::totp::generate(&secret, (at.timestamp().div_euclid(30)) as u64, 6);
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", None, &counter, at), Ok(Err(SignInError::NeedsCode))), "the password alone signed in past the second factor");
        assert!(accounts.locked(&counter).is_none(), "needing the code counted as a failure");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some("000000"), &counter, at), Ok(Err(SignInError::Rejected))), "a wrong code did not reject");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&code), &counter, at), Ok(Ok(()))), "the current code did not sign in");
        for _ in 0..3 {
            let _ = accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some("000000"), &counter, at);
        }
        assert!(accounts.locked(&counter).is_some(), "wrong codes did not lock the counter");
    }

    /// A code signs in once: replaying it within the window rejects and
    /// counts, while the next step's code still works; eight threads racing
    /// the same code admit exactly one (#441).
    #[test]
    fn a_code_signs_in_once() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 100, lockout_minutes: 15, min_password_length: 10, oidc: None };
        set_password(dir.path(), &cfg, "ada", "a-long-password").unwrap();
        let accounts = Accounts::open(dir.path(), cfg).unwrap();
        let uri = enroll_totp(dir.path(), "ada").unwrap();
        let secret = uri.split("secret=").nth(1).and_then(|s| s.split('&').next()).unwrap();
        let secret = crate::totp::base32_decode(secret).unwrap();
        let counter = Accounts::network_counter("ada");
        let at = chrono::Utc::now();
        let step = at.timestamp().div_euclid(30) as u64;
        let code = crate::totp::generate(&secret, step, 6);
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&code), &counter, at), Ok(Ok(()))), "the first use of the code did not sign in");
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&code), &counter, at), Ok(Err(SignInError::Rejected))), "replaying the code signed in again");
        // Eight threads race the next step's code: every one of them verifies
        // it (it is live), but the step admits exactly one.
        let race_code = crate::totp::generate(&secret, step + 1, 6);
        let admitted = std::sync::Mutex::new(0u32);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    if matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", Some(&race_code), &counter), Ok(Ok(()))) {
                        *admitted.lock().unwrap() += 1;
                    }
                });
            }
        });
        assert_eq!(*admitted.lock().unwrap(), 1, "the raced code did not admit exactly one thread");
        let next = crate::totp::generate(&secret, step + 2, 6);
        let later = chrono::DateTime::from_timestamp((step + 2) as i64 * 30 + 5, 0).unwrap();
        assert!(matches!(accounts.sign_in_counted_with_code_at("ada", "a-long-password", Some(&next), &counter, later), Ok(Ok(()))), "the next step's code did not sign in");
    }

    /// An invitation activates on accept: invited cannot sign in, the token
    /// with a password activates once, and a second use, a bad token or an
    /// expired one answers nothing (#429).
    #[test]
    fn invite_accept_activates_and_consumes() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        let accounts = Accounts::open(dir.path(), cfg.clone()).unwrap();
        let token = accounts.invite("ada", Some("ada@example.com")).unwrap();
        assert_eq!(accounts.status("ada"), Status::Invited, "an invite did not mark the person invited");
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "whatever-password", None, &counter), Ok(Err(SignInError::Rejected))), "an invited person signed in");
        assert_eq!(accounts.accept(&cfg, &token, "a-long-password").unwrap(), Some("ada".to_string()), "accepting the invite did not activate");
        assert_eq!(accounts.status("ada"), Status::Active, "accepting left the person invited");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter), Ok(Ok(()))), "the accepted person cannot sign in");
        assert_eq!(accounts.accept(&cfg, &token, "another-long-password").unwrap(), None, "the invite worked twice");
        assert_eq!(accounts.accept(&cfg, "deadbeef", "another-long-password").unwrap(), None, "a bad token accepted");
        let stale = accounts.invite("ada", None).unwrap();
        let past = chrono::Utc::now() + chrono::Duration::days(8);
        assert_eq!(accounts.accept_at(&cfg, &stale, "another-long-password", past).unwrap(), None, "an expired invite accepted");
        let kept = accounts.invite("ada", None).unwrap();
        assert!(accounts.accept(&cfg, &kept, "short").is_err(), "a short password accepted");
        assert_eq!(accounts.accept(&cfg, &kept, "another-long-password").unwrap(), Some("ada".to_string()), "the short password burned the invite");
        // A password set directly completes the invitation too — but never a
        // disabling.
        let cfg2 = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        accounts.invite("bea", None).unwrap();
        set_password(dir.path(), &cfg2, "bea", "bea-long-password").unwrap();
        assert_eq!(accounts.status("bea"), Status::Active, "a set password left the invitee invited");
        accounts.set_status("bea", Status::Disabled).unwrap();
        set_password(dir.path(), &cfg2, "bea", "bea-newer-password").unwrap();
        assert_eq!(accounts.status("bea"), Status::Disabled, "a set password re-enabled a disabled person");
    }

    /// Turning a person off refuses sign-in and ends their live sessions;
    /// turning them back on restores sign-in (#429).
    #[test]
    fn disabled_refuses_and_ends_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        set_password(dir.path(), &cfg, "ada", "a-long-password").unwrap();
        let accounts = Accounts::open(dir.path(), cfg).unwrap();
        let (session, token) = accounts.open_session("ada", "app", None).unwrap();
        let counter = Accounts::network_counter("ada");
        let ended = accounts.set_status("ada", Status::Disabled).unwrap();
        assert_eq!(ended, vec![session.id.clone()], "disabling did not end the live session");
        assert!(accounts.session(&token).is_none(), "the session survived disabling");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter), Ok(Err(SignInError::Rejected))), "a disabled person signed in");
        accounts.set_status("ada", Status::Active).unwrap();
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter), Ok(Ok(()))), "re-enabling did not restore sign-in");
    }

    /// Inviting someone already active (a resend) hands them a token without
    /// deactivating them (#429).
    #[test]
    fn invite_keeps_an_active_person_active() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        set_password(dir.path(), &cfg, "ada", "a-long-password").unwrap();
        let accounts = Accounts::open(dir.path(), cfg.clone()).unwrap();
        assert_eq!(accounts.status("ada"), Status::Active, "a person with no record is not active");
        let token = accounts.invite("ada", None).unwrap();
        assert_eq!(accounts.status("ada"), Status::Active, "inviting deactivated an active person");
        assert_eq!(accounts.accept(&cfg, &token, "a-new-long-password").unwrap(), Some("ada".to_string()), "the resend token did not accept");
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-new-long-password", None, &counter), Ok(Ok(()))), "the resent password did not take");
    }

    /// A disabling from another handle (the CLI's shape: its own `Accounts`
    /// on the same directory) revokes this handle's live sessions too: the
    /// server never reloads the session file, so resolution checks the
    /// status file, which is always fresh (#448).
    #[test]
    fn disabling_elsewhere_revokes_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        set_password(dir.path(), &cfg, "ada", "a-long-password").unwrap();
        let server = Accounts::open(dir.path(), cfg.clone()).unwrap();
        let (_, token) = server.open_session("ada", "app", None).unwrap();
        assert!(server.session(&token).is_some(), "the session did not resolve before disabling");
        let cli = Accounts::open(dir.path(), cfg).unwrap();
        cli.set_status("ada", Status::Disabled).unwrap();
        assert!(server.session(&token).is_none(), "the session survived a disabling from another handle");
        cli.set_status("ada", Status::Active).unwrap();
        assert!(server.session(&token).is_some(), "re-enabling did not restore the live session");
    }

    /// An outstanding invitation cannot undo a disabling: accept refuses
    /// alike without consuming the token, which stays usable if the person
    /// is turned back on (#449).
    #[test]
    fn an_invite_does_not_reenable() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        let accounts = Accounts::open(dir.path(), cfg.clone()).unwrap();
        let token = accounts.invite("ada", None).unwrap();
        accounts.set_status("ada", Status::Disabled).unwrap();
        assert_eq!(accounts.accept(&cfg, &token, "a-long-password").unwrap(), None, "accepting re-enabled a disabled person");
        assert_eq!(accounts.status("ada"), Status::Disabled, "accepting flipped the status");
        accounts.set_status("ada", Status::Active).unwrap();
        assert_eq!(accounts.accept(&cfg, &token, "a-long-password").unwrap(), Some("ada".to_string()), "the kept token did not survive the refusal");
    }

    /// Racing accepts of one invitation admit exactly one, round after
    /// round: the token is single-use under concurrency too (#450). A
    /// regression guard — thread scheduling here never overlapped the
    /// unlocked window locally, so the guarantee itself rests on the
    /// mutex `accept` holds across read, removal and write.
    #[test]
    fn racing_accepts_admit_exactly_one() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        let accounts = Accounts::open(dir.path(), cfg.clone()).unwrap();
        for round in 0..10 {
            let person = format!("ada{round}");
            let token = accounts.invite(&person, None).unwrap();
            let admitted = std::sync::Mutex::new(0u32);
            let (accounts, cfg, token, admitted) = (&accounts, &cfg, &token, &admitted);
            std::thread::scope(|s| {
                for i in 0..8 {
                    s.spawn(move || {
                        let pw = format!("a-long-password-{i}");
                        if matches!(accounts.accept(cfg, token, &pw), Ok(Some(_))) {
                            *admitted.lock().unwrap() += 1;
                        }
                    });
                }
            });
            assert_eq!(*admitted.lock().unwrap(), 1, "round {round}: racing accepts did not admit exactly one");
        }
    }

    /// Local roles default to member, set explicitly, and read back (#429).
    #[test]
    fn roles_default_member_and_set_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        let accounts = Accounts::open(dir.path(), cfg).unwrap();
        assert_eq!(accounts.role("ada"), Role::Member, "an unrecorded person is not a member");
        accounts.set_role("ada", Role::Admin).unwrap();
        assert_eq!(accounts.role("ada"), Role::Admin, "setting admin did not stick");
        accounts.set_role("bea", Role::Member).unwrap();
        assert_eq!(accounts.role("bea"), Role::Member, "setting member did not stick");
    }

    /// The last enabled admin keeps the role and the sign-in: demoting or
    /// disabling them refuses, a disabled admin record does not count, and a
    /// second admin unlocks both (#429).
    #[test]
    fn last_admin_keeps_role_and_signin() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        set_password(dir.path(), &cfg, "ada", "a-long-password").unwrap();
        set_password(dir.path(), &cfg, "bea", "bea-long-password").unwrap();
        let accounts = Accounts::open(dir.path(), cfg).unwrap();
        accounts.set_role("ada", Role::Admin).unwrap();
        let e = accounts.set_role("ada", Role::Member).unwrap_err();
        assert!(format!("{e:#}").contains("last admin"), "demoting the last admin refused wrongly: {e:#}");
        assert_eq!(accounts.role("ada"), Role::Admin, "the refused demotion landed");
        let e = accounts.set_status("ada", Status::Disabled).unwrap_err();
        assert!(format!("{e:#}").contains("last admin"), "disabling the last admin refused wrongly: {e:#}");
        assert_eq!(accounts.status("ada"), Status::Active, "the refused disabling landed");
        // A disabled admin on record does not free the switch: nobody
        // enabled would be left to administer.
        accounts.set_role("bea", Role::Admin).unwrap();
        accounts.set_status("bea", Status::Disabled).unwrap();
        assert!(accounts.set_role("ada", Role::Member).is_err(), "a disabled admin record freed the last enabled admin's role");
        accounts.set_status("bea", Status::Active).unwrap();
        accounts.set_role("ada", Role::Member).unwrap();
        accounts.set_status("ada", Status::Disabled).unwrap();
        assert_eq!(accounts.role("ada"), Role::Member, "a second admin did not unlock the demotion");
        assert_eq!(accounts.status("ada"), Status::Disabled, "a second admin did not unlock the disabling");
    }

    /// With no admin records the switch is unmanaged: disabling works, so
    /// the guard never locks an appliance that never assigned roles (#429).
    #[test]
    fn no_admin_records_leaves_the_switch_unguarded() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None };
        set_password(dir.path(), &cfg, "ada", "a-long-password").unwrap();
        let accounts = Accounts::open(dir.path(), cfg).unwrap();
        accounts.set_status("ada", Status::Disabled).unwrap();
        assert_eq!(accounts.status("ada"), Status::Disabled, "disabling with no admin records refused");
    }

    /// Without an enrolled second factor, the code path behaves exactly as
    /// the password path: the feature is off until enrolled (#429).
    #[test]
    fn no_second_factor_behaves_as_passwords_alone() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = open(dir.path());
        set_password(dir.path(), &SignIn { session_days: 1, max_failures: 5, lockout_minutes: 15, min_password_length: 10, oidc: None }, "ada", "a-long-password").unwrap();
        let counter = Accounts::network_counter("ada");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "a-long-password", None, &counter), Ok(Ok(()))), "a password without a second factor stopped signing in");
        assert!(matches!(accounts.sign_in_counted_with_code("ada", "wrong-password", None, &counter), Ok(Err(SignInError::Rejected))), "a wrong password stopped rejecting");
        assert!(matches!(accounts.sign_in_counted_with_code("nobody", "whatever-password", None, &counter), Ok(Err(SignInError::Rejected))), "an unknown name stopped rejecting");
    }

    /// Each counter is its scope then the whole person id, so no person's
    /// network counter is another's local or backup one, whatever their ids
    /// (review of #154: `local:carol` from the network locked carol locally).
    #[test]
    fn one_persons_counter_is_never_anothers() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = open(dir.path());
        for _ in 0..5 {
            accounts.failed(&Accounts::network_counter("local:carol"));
            accounts.failed(&Accounts::network_counter("backup:carol"));
        }
        assert!(accounts.locked(&Accounts::network_counter("local:carol")).is_some());
        assert!(accounts.locked(&Accounts::local_counter("carol")).is_none(), "local:carol's network failures locked carol on this machine");
        assert!(accounts.locked(&Accounts::backup_counter("carol")).is_none(), "backup:carol's network failures locked carol's backup password");
    }
}
