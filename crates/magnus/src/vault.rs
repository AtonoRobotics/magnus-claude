//! The vault: secrets encrypted at rest under the state directory.
//!
//! Only two places read a secret: the boundary's credential stage, at the
//! moment it calls an app, and the model client, at the moment it calls an
//! adapter (K28). Nothing returns a secret over the API.

use anyhow::{Context, Result};
use base64::Engine;
use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
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
        Ok(Vault { dir, cipher, lock: Mutex::new(()), sealed: Default::default() })
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
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), value.as_bytes())
            .map_err(|_| anyhow::anyhow!("encrypt"))?;
        let mut blob = nonce.to_vec();
        blob.extend(ct);
        all.insert(name.into(), base64::engine::general_purpose::STANDARD.encode(blob));
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
        let blob = base64::engine::general_purpose::STANDARD.decode(b64)?;
        anyhow::ensure!(blob.len() > 24, "vault entry {name} is corrupt");
        let pt = self
            .cipher
            .decrypt(XNonce::from_slice(&blob[..24]), &blob[24..])
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
