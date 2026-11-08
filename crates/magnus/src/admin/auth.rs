//! Signing in to the admin console (admin console spec, Authentication and
//! session; owner's rulings 1 and 2: no passkeys, so an admin signs in and
//! steps up with their password, and their code when they use one, on the
//! identity provider's own page).
//!
//! The console is `magnus-admin`, a confidential client of the realm: the
//! authorization code flow with PKCE, its only redirect its own
//! `/auth/callback`, and it proves itself to the provider with a key of its
//! own (`private_key_jwt`, ES256), kept on the appliance. The ID token is
//! checked as the user API checks one, and its signature too: against the
//! realm's keys, ES256 or RS256 only, its issuer, audience, expiry and
//! nonce; identity is `sub`. Only the realm role `admin` gets in. The
//! session is the console's, server-side: the browser holds only a cookie
//! (`__Host-magnus_admin`), every change carries the session's CSRF token,
//! and the provider is asked again at least every 4 minutes, so an admin
//! turned off there loses the console within 5.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use ring::signature::{self, KeyPair};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub const COOKIE: &str = "__Host-magnus_admin";
/// A sign-in or step-up on the provider's page must come back within this.
const PENDING_MINUTES: i64 = 10;
/// The realm role that opens the console.
pub const ROLE: &str = "admin";

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// The console's own key, made on its first start and kept on the
/// appliance (readable by its owner only). Keycloak holds its public half.
pub struct Key {
    pair: signature::EcdsaKeyPair,
    kid: String,
}

impl Key {
    pub fn load_or_make(dir: &Path) -> Result<Key> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("client-key.p8");
        let rng = ring::rand::SystemRandom::new();
        if !path.exists() {
            let doc = signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng).map_err(|_| anyhow!("make the console's key"))?;
            crate::vault::write_private(&path, doc.as_ref())?;
        }
        let der = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let pair = signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &der, &rng).map_err(|_| anyhow!("{} is not the console's key", path.display()))?;
        use sha2::Digest;
        let kid = b64(&sha2::Sha256::digest(pair.public_key().as_ref())[..16]);
        Ok(Key { pair, kid })
    }

    /// Its public half as a JWK set, for the provider's client.
    pub fn jwks(&self) -> Value {
        let p = self.pair.public_key().as_ref();
        json!({ "keys": [{ "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig", "kid": self.kid, "x": b64(&p[1..33]), "y": b64(&p[33..65]) }] })
    }

    fn sign(&self, header: &Value, claims: &Value) -> Result<String> {
        let input = format!("{}.{}", b64(header.to_string().as_bytes()), b64(claims.to_string().as_bytes()));
        let sig = self.pair.sign(&ring::rand::SystemRandom::new(), input.as_bytes()).map_err(|_| anyhow!("sign"))?;
        Ok(format!("{input}.{}", b64(sig.as_ref())))
    }

    /// A client assertion for one call to `aud` (RFC 7523).
    pub fn assertion(&self, client_id: &str, aud: &str) -> Result<String> {
        let now = Utc::now().timestamp();
        self.sign(
            &json!({ "alg": "ES256", "typ": "JWT", "kid": self.kid }),
            &json!({ "iss": client_id, "sub": client_id, "aud": aud, "jti": crate::connectors::random(), "iat": now, "exp": now + 60 }),
        )
    }
}

/// Check a JWT's signature against a JWK set: ES256 or RS256 only.
/// An admin's console session. The provider's refresh token stays here,
/// never in the browser.
#[derive(Clone, Debug)]
pub struct Session {
    pub sub: String,
    pub name: String,
    pub csrf: String,
    pub started: DateTime<Utc>,
    pub seen: DateTime<Utc>,
    pub refreshed: DateTime<Utc>,
    pub stepped_up: Option<DateTime<Utc>>,
}

impl Session {
    /// Whether a password was typed in the last `window` seconds.
    pub fn stepped_up(&self, now: DateTime<Utc>, window: i64) -> bool {
        self.stepped_up.is_some_and(|t| now - t < Duration::seconds(window))
    }
}

#[derive(Default)]
pub struct Store {
    pub sessions: Mutex<HashMap<String, Session>>,
    /// Login-CSRF tokens for the local forms: token to minted. In-memory
    /// only; a restart just asks the browser to reload its form (#456).
    pub login_tokens: Mutex<HashMap<String, DateTime<Utc>>>,
}

impl Store {
    pub fn add_login_token(&self, token: String) -> Result<()> {
        let mut m = self.login_tokens.lock().unwrap();
        let now = Utc::now();
        m.retain(|_, minted| now - *minted < Duration::minutes(PENDING_MINUTES));
        if m.len() >= 200 {
            bail!("too many sign-ins are waiting; try again in a few minutes");
        }
        m.insert(token, now);
        Ok(())
    }

    /// Take a login token: true once, while fresh.
    pub fn take_login_token(&self, token: &str) -> bool {
        let minted = self.login_tokens.lock().unwrap().remove(token);
        minted.is_some_and(|t| Utc::now() - t < Duration::minutes(PENDING_MINUTES))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_consoles_own_signature_verifies_and_nothing_else_does() {
        let dir = tempfile::tempdir().unwrap();
        let key = Key::load_or_make(dir.path()).unwrap();
        let again = Key::load_or_make(dir.path()).unwrap();
        assert_eq!(key.jwks(), again.jwks(), "the key is not kept");
        let jwt = key.assertion("magnus-admin", "https://idp.test/token").unwrap();
        let c = verify(&jwt, &key.jwks()).unwrap();
        assert_eq!((c["iss"].as_str(), c["aud"].as_str()), (Some("magnus-admin"), Some("https://idp.test/token")));
        // Tampered claims, another key, or an algorithm outside the list fail.
        let mut parts: Vec<String> = jwt.split('.').map(String::from).collect();
        parts[1] = b64(json!({ "iss": "someone", "sub": "else" }).to_string().as_bytes());
        assert!(verify(&parts.join("."), &key.jwks()).is_err());
        let other = Key::load_or_make(tempfile::tempdir().unwrap().path()).unwrap();
        assert!(verify(&jwt, &other.jwks()).is_err());
        let none = format!("{}.{}.", b64(br#"{"alg":"none"}"#), b64(br#"{"sub":"x"}"#));
        assert!(verify(&none, &key.jwks()).is_err());
    }

    /// A login token takes once, while fresh: unknown and used ones refuse (#456).
    #[test]
    fn login_tokens_take_once() {
        let s = Store::default();
        s.add_login_token("t1".to_string()).unwrap();
        assert!(s.take_login_token("t1"), "a fresh token refused");
        assert!(!s.take_login_token("t1"), "a used token took again");
        assert!(!s.take_login_token("nope"), "an unknown token took");
        s.login_tokens.lock().unwrap().insert("old".to_string(), Utc::now() - Duration::minutes(PENDING_MINUTES + 1));
        assert!(!s.take_login_token("old"), "an expired token took");
    }
}
