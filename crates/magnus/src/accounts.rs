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
}

pub enum SignInError {
    /// Wrong person or password; the caller says nothing more.
    Rejected,
    /// Too many failures; try again after this time.
    Locked(DateTime<Utc>),
}

pub struct Accounts {
    dir: PathBuf,
    cfg: SignIn,
    /// Sessions by the SHA-256 of their token.
    sessions: Mutex<BTreeMap<String, Session>>,
    /// Failed attempts per person: (count, first failure).
    failures: Mutex<BTreeMap<String, (u32, DateTime<Utc>)>>,
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

/// Set a person's password. Used by the CLI on the appliance itself.
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
    write_private(&path, &serde_json::to_vec_pretty(&all)?)
}

impl Accounts {
    pub fn open(state_dir: &Path, cfg: SignIn) -> Result<Accounts> {
        let d = dir(state_dir);
        std::fs::create_dir_all(&d)?;
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
        let now = Utc::now();
        let mut sessions: BTreeMap<String, Session> = read_json(&d.join("sessions.json"))?;
        sessions.retain(|_, s| s.expires > now);
        Ok(Accounts { dir: d, cfg, sessions: Mutex::new(sessions), failures: Mutex::new(BTreeMap::new()) })
    }

    fn save(&self, sessions: &BTreeMap<String, Session>) -> Result<()> {
        write_private(&self.dir.join("sessions.json"), &serde_json::to_vec_pretty(sessions)?)
    }

    /// Whether the person has a password set (read fresh: the CLI may have set it).
    pub fn has_password(&self, person: &str) -> bool {
        read_json::<BTreeMap<String, String>>(&self.dir.join("passwords.json")).map(|m| m.contains_key(person)).unwrap_or(false)
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

    /// Check a password. The caller opens the session.
    pub fn sign_in(&self, person: &str, password: &str) -> Result<std::result::Result<(), SignInError>> {
        if let Some(until) = self.locked(person) {
            return Ok(Err(SignInError::Locked(until)));
        }
        let all: BTreeMap<String, String> = read_json(&self.dir.join("passwords.json"))?;
        // Verify against a throwaway hash when the person is unknown, so
        // timing does not tell which names exist.
        static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let dummy = DUMMY.get_or_init(|| {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default().hash_password(salt.as_str().as_bytes(), &salt).map(|h| h.to_string()).unwrap_or_default()
        });
        let phc = all.get(person).unwrap_or(dummy);
        let ok = PasswordHash::new(phc).map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok()).unwrap_or(false) && all.contains_key(person);
        if !ok {
            if all.contains_key(person) {
                self.failed(person);
            }
            return Ok(Err(SignInError::Rejected));
        }
        self.succeeded(person);
        Ok(Ok(()))
    }

    /// Open a session for a person whose identity is already established.
    pub fn open_session(&self, person: &str, client: &str) -> Result<(Session, String)> {
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
        };
        let mut s = self.sessions.lock().unwrap();
        s.insert(token_hash(&token), session.clone());
        self.save(&s)?;
        Ok((session, token))
    }

    /// The live session a token belongs to.
    pub fn session(&self, token: &str) -> Option<Session> {
        let s = self.sessions.lock().unwrap();
        s.get(&token_hash(token)).filter(|x| x.expires > Utc::now()).cloned()
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

    pub fn sign_out(&self, token: &str) -> Result<Option<Session>> {
        let mut s = self.sessions.lock().unwrap();
        let gone = s.remove(&token_hash(token));
        if gone.is_some() {
            self.save(&s)?;
        }
        Ok(gone)
    }
}
