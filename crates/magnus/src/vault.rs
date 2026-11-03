//! The vault: secrets encrypted at rest under the state directory.
//!
//! Only two places read a secret: the boundary's credential stage, at the
//! moment it calls an app, and the model client, at the moment it calls an
//! adapter (K28). Nothing returns a secret over the API.

use anyhow::{Context, Result};
use base64::Engine;
use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use std::{
    collections::BTreeMap,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct Vault {
    dir: PathBuf,
    cipher: XChaCha20Poly1305,
    lock: Mutex<()>,
    /// Set while the kill switch is engaged: nothing is read or stored, so
    /// no secret leaves and no refreshed token comes back in (B11).
    sealed: std::sync::atomic::AtomicBool,
}

/// A secret value; formatting it never prints the value.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub(crate) fn new(s: String) -> Secret {
        Secret(s)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The mark of an entry encrypted bound to its name.
const BOUND: &str = "2:";

impl Vault {
    pub fn open(state_dir: &Path) -> Result<Vault> {
        let dir = state_dir.join("vault");
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let key_path = dir.join("key");
        if !key_path.exists() {
            let mut k = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut k);
            write_private(&key_path, &k)?;
        }
        let key = std::fs::read(&key_path).context("read vault key")?;
        anyhow::ensure!(key.len() == 32, "vault key is corrupt");
        let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| anyhow::anyhow!("vault key"))?;
        let v = Vault { dir, cipher, lock: Mutex::new(()), sealed: Default::default() };
        v.bind_unbound()?;
        Ok(v)
    }

    /// Entries written before each was bound to its name (or restored from
    /// a backup of then), sealed again bound to it.
    fn bind_unbound(&self) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        let mut all = self.read_all()?;
        let unbound: Vec<String> = all.iter().filter(|(_, v)| !v.starts_with(BOUND)).map(|(n, _)| n.clone()).collect();
        if unbound.is_empty() {
            return Ok(());
        }
        for n in unbound {
            let blob = base64::engine::general_purpose::STANDARD.decode(&all[&n])?;
            anyhow::ensure!(blob.len() > 24, "vault entry {n} is corrupt");
            let pt = self.cipher.decrypt(XNonce::from_slice(&blob[..24]), &blob[24..]).map_err(|_| anyhow::anyhow!("vault entry {n} does not decrypt"))?;
            all.insert(n.clone(), self.seal_as(&n, &pt)?);
        }
        write_private(&self.path(), &serde_json::to_vec(&all)?)
    }

    /// A value encrypted for the entry `name` alone: its name is the
    /// associated data, so under any other name it does not decrypt
    /// (connectors spec, Custody: bound to one user and one connector).
    fn seal_as(&self, name: &str, value: &[u8]) -> Result<String> {
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self.cipher.encrypt(XNonce::from_slice(&nonce), Payload { msg: value, aad: name.as_bytes() }).map_err(|_| anyhow::anyhow!("encrypt"))?;
        let mut blob = nonce.to_vec();
        blob.extend(ct);
        Ok(format!("{BOUND}{}", base64::engine::general_purpose::STANDARD.encode(blob)))
    }

    fn path(&self) -> PathBuf {
        self.dir.join("secrets.json")
    }

    fn read_all(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read(self.path()) {
            Ok(b) => Ok(serde_json::from_slice(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn put(&self, name: &str, value: &str) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        anyhow::ensure!(!self.is_sealed(), "the kill switch is engaged; nothing is stored in the vault");
        let mut all = self.read_all()?;
        all.insert(name.into(), self.seal_as(name, value.as_bytes())?);
        write_private(&self.path(), &serde_json::to_vec(&all)?)
    }

    pub fn get(&self, name: &str) -> Result<Option<Secret>> {
        let _g = self.lock.lock().unwrap();
        if self.is_sealed() {
            return Ok(None);
        }
        self.get_locked(name)
    }

    fn get_locked(&self, name: &str) -> Result<Option<Secret>> {
        let all = self.read_all()?;
        let Some(b64) = all.get(name) else { return Ok(None) };
        let b64 = b64.strip_prefix(BOUND).ok_or_else(|| anyhow::anyhow!("vault entry {name} is not bound to its name"))?;
        let blob = base64::engine::general_purpose::STANDARD.decode(b64)?;
        anyhow::ensure!(blob.len() > 24, "vault entry {name} is corrupt");
        let pt = self
            .cipher
            .decrypt(XNonce::from_slice(&blob[..24]), Payload { msg: &blob[24..], aad: name.as_bytes() })
            .map_err(|_| anyhow::anyhow!("vault entry {name} does not decrypt"))?;
        Ok(Some(Secret(String::from_utf8(pt)?)))
    }

    pub fn names(&self) -> Result<Vec<String>> {
        let _g = self.lock.lock().unwrap();
        Ok(self.read_all()?.into_keys().collect())
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn unseal(&self) {
        self.sealed.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Seal the vault and take the secrets named in `take` out of it,
    /// returning them once (for revocation at the provider). The rest stay,
    /// unreadable until the vault is unsealed.
    pub fn seal_and_take(&self, take: &[String]) -> Result<Vec<(String, Secret)>> {
        let _g = self.lock.lock().unwrap();
        self.sealed.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut all = self.read_all()?;
        let mut out = vec![];
        for n in take {
            if let Some(v) = self.get_locked(n)? {
                out.push((n.clone(), v));
            }
            all.remove(n);
        }
        write_private(&self.path(), &serde_json::to_vec(&all)?)?;
        Ok(out)
    }

    /// The vault's key and its encrypted contents as stored, for a backup.
    pub fn raw(&self) -> Result<(Vec<u8>, String)> {
        let _g = self.lock.lock().unwrap();
        let key = std::fs::read(self.dir.join("key"))?;
        let secrets = std::fs::read_to_string(self.path()).unwrap_or_else(|_| "{}".into());
        Ok((key, secrets))
    }

    /// Remove a secret (a connector the owner removed).
    pub fn remove(&self, name: &str) -> Result<Option<Secret>> {
        let _g = self.lock.lock().unwrap();
        let mut all = self.read_all()?;
        let old = self.get_locked(name)?;
        all.remove(name);
        write_private(&self.path(), &serde_json::to_vec(&all)?)?;
        Ok(old)
    }

    /// Seal without emptying: a restart while the kill switch is engaged.
    pub fn seal(&self) {
        self.sealed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// That every entry of a vault's stored contents opens under `key`, as
/// the vault would open it (a backup's restore test).
pub fn check(key: &[u8], secrets: &str) -> Result<()> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow::anyhow!("the backed-up vault key is corrupt"))?;
    let all: BTreeMap<String, String> = serde_json::from_str(secrets)?;
    for (name, v) in &all {
        // Bound to its name, or from before binding (bound when it opens).
        let (b64, aad) = match v.strip_prefix(BOUND) {
            Some(b) => (b, name.as_bytes()),
            None => (v.as_str(), &[][..]),
        };
        let blob = base64::engine::general_purpose::STANDARD.decode(b64)?;
        anyhow::ensure!(blob.len() > 24, "secret {name} is corrupt");
        cipher.decrypt(XNonce::from_slice(&blob[..24]), Payload { msg: &blob[24..], aad }).map_err(|_| anyhow::anyhow!("secret {name} does not decrypt"))?;
    }
    Ok(())
}

/// Replace every encoding of `secret` in `text` (raw, base64, URL-safe base64,
/// hex) so that nothing a careless app echoes back carries it further.
pub fn redact(text: &str, secret: &str) -> String {
    if secret.len() < 4 {
        return text.to_string();
    }
    let b = secret.as_bytes();
    let forms = [
        secret.to_string(),
        base64::engine::general_purpose::STANDARD.encode(b),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(b),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b),
        hex::encode(b),
        hex::encode_upper(b),
    ];
    let mut out = text.to_string();
    for f in forms.iter() {
        if !f.is_empty() {
            out = out.replace(f.as_str(), "[credential removed]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(dir: &Path) -> BTreeMap<String, String> {
        serde_json::from_slice(&std::fs::read(dir.join("vault/secrets.json")).unwrap()).unwrap()
    }

    #[test]
    fn a_sign_in_moved_under_another_name_does_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(dir.path()).unwrap();
        v.put("connector_calendar__alice", "alice-token").unwrap();
        v.put("connector_calendar__bob", "bob-token").unwrap();
        // Bob's entry copied over Alice's, as someone with the file might.
        let mut all = stored(dir.path());
        all.insert("connector_calendar__alice".into(), all["connector_calendar__bob"].clone());
        write_private(&dir.path().join("vault/secrets.json"), &serde_json::to_vec(&all).unwrap()).unwrap();
        let v = Vault::open(dir.path()).unwrap();
        assert!(v.get("connector_calendar__alice").is_err(), "Bob's sign-in opened as Alice's");
        assert_eq!(v.get("connector_calendar__bob").unwrap().unwrap().expose(), "bob-token");
    }

    #[test]
    fn an_entry_from_before_binding_is_bound_to_its_name_on_opening() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(dir.path()).unwrap();
        // As written before entries were bound: no associated data, no mark.
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mut blob = nonce.to_vec();
        blob.extend(v.cipher.encrypt(XNonce::from_slice(&nonce), b"old-token".as_slice()).unwrap());
        let all = BTreeMap::from([("connector_calendar__alice".to_string(), base64::engine::general_purpose::STANDARD.encode(blob))]);
        write_private(&dir.path().join("vault/secrets.json"), &serde_json::to_vec(&all).unwrap()).unwrap();
        let v = Vault::open(dir.path()).unwrap();
        assert_eq!(v.get("connector_calendar__alice").unwrap().unwrap().expose(), "old-token");
        assert!(stored(dir.path())["connector_calendar__alice"].starts_with(BOUND), "the old entry was not bound on opening");
    }
}
