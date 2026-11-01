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

fn unb64(s: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).context("not base64url")
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
pub fn verify(jwt: &str, jwks: &Value) -> Result<Value> {
    let mut parts = jwt.split('.');
    let (Some(h), Some(c), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { bail!("not a signed JWT") };
    let header: Value = serde_json::from_slice(&unb64(h)?).context("its header")?;
    let alg = header["alg"].as_str().unwrap_or_default();
    let kid = header["kid"].as_str();
    let key = jwks["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| kid.is_none_or(|kid| k["kid"].as_str() == Some(kid)) && k["use"].as_str().is_none_or(|u| u == "sig") && k["alg"].as_str().is_none_or(|a| a == alg))
        .ok_or_else(|| anyhow!("signed with a key the provider does not publish"))?;
    let (msg, sig) = (format!("{h}.{c}"), unb64(s)?);
    let ok = match (alg, key["kty"].as_str()) {
        ("ES256", Some("EC")) if key["crv"] == "P-256" => {
            let mut point = vec![4u8];
            point.extend(unb64(key["x"].as_str().unwrap_or_default())?);
            point.extend(unb64(key["y"].as_str().unwrap_or_default())?);
            signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point).verify(msg.as_bytes(), &sig).is_ok()
        }
        ("RS256", Some("RSA")) => {
            let n = unb64(key["n"].as_str().unwrap_or_default())?;
            let e = unb64(key["e"].as_str().unwrap_or_default())?;
            signature::RsaPublicKeyComponents { n, e }.verify(&signature::RSA_PKCS1_2048_8192_SHA256, msg.as_bytes(), &sig).is_ok()
        }
        _ => bail!("signed with {alg}, which the console does not accept"),
    };
    if !ok {
        bail!("its signature does not verify");
    }
    serde_json::from_slice(&unb64(c)?).context("its claims")
}

/// An admin's console session. The provider's refresh token stays here,
/// never in the browser.
#[derive(Clone, Debug)]
pub struct Session {
    pub sub: String,
    pub name: String,
    pub csrf: String,
    pub refresh_token: String,
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

/// A sign-in or step-up waiting on the provider's page.
#[derive(Clone, Debug)]
pub struct Pending {
    pub verifier: String,
    pub nonce: String,
    /// The session a step-up is for; None for a sign-in.
    pub step_up: Option<String>,
    /// Where the console goes back to.
    pub then: String,
    pub redirect_uri: String,
    pub started: DateTime<Utc>,
}

#[derive(Default)]
pub struct Store {
    pub sessions: Mutex<HashMap<String, Session>>,
    pub pending: Mutex<HashMap<String, Pending>>,
}

impl Store {
    pub fn add_pending(&self, state: String, p: Pending) -> Result<()> {
        let mut m = self.pending.lock().unwrap();
        let now = Utc::now();
        m.retain(|_, x| now - x.started < Duration::minutes(PENDING_MINUTES));
        if m.len() >= 200 {
            bail!("too many sign-ins are waiting; try again in a few minutes");
        }
        m.insert(state, p);
        Ok(())
    }

    pub fn take_pending(&self, state: &str) -> Option<Pending> {
        let p = self.pending.lock().unwrap().remove(state)?;
        (Utc::now() - p.started < Duration::minutes(PENDING_MINUTES)).then_some(p)
    }
}

/// The checks every console ID token passes: our client, the realm's
/// issuer, current, answering this sign-in's nonce, and with a subject. A
/// refresh's ID token answers no sign-in (`nonce` None): Keycloak leaves
/// the nonce out of it, and every other check still holds.
pub fn check_claims(c: &Value, issuer: &str, client_id: &str, nonce: Option<&str>, now: DateTime<Utc>) -> Result<String> {
    if c["iss"].as_str() != Some(issuer) {
        bail!("from {}, not {issuer}", c["iss"]);
    }
    let aud_ok = match &c["aud"] {
        Value::String(a) => a == client_id,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(client_id)),
        _ => false,
    };
    if !aud_ok || c.get("azp").and_then(|v| v.as_str()).is_some_and(|azp| azp != client_id) {
        bail!("not for client {client_id}");
    }
    if c["exp"].as_i64().is_none_or(|e| e <= now.timestamp()) {
        bail!("expired");
    }
    if nonce.is_some_and(|n| c["nonce"].as_str() != Some(n)) {
        bail!("not an answer to this sign-in");
    }
    c["sub"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from).ok_or_else(|| anyhow!("no subject"))
}

/// The realm roles an ID token carries, at the claim the appliance reads
/// them from (`sign_in.oidc.roles_claim`, by default Keycloak's).
pub fn roles(c: &Value, pointer: &str) -> Vec<String> {
    c.pointer(pointer).and_then(|v| v.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect()
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

    #[test]
    fn an_id_token_is_for_this_client_this_realm_this_sign_in_and_current() {
        let now = Utc::now();
        let ok = json!({ "iss": "https://idp/realms/m", "aud": "magnus-admin", "exp": now.timestamp() + 60, "nonce": "n1", "sub": "abc" });
        assert_eq!(check_claims(&ok, "https://idp/realms/m", "magnus-admin", Some("n1"), now).unwrap(), "abc");
        for (k, v) in [("iss", json!("https://evil")), ("aud", json!("magnus-test")), ("exp", json!(now.timestamp() - 1)), ("nonce", json!("n2")), ("sub", json!(""))] {
            let mut c = ok.clone();
            c[k] = v;
            assert!(check_claims(&c, "https://idp/realms/m", "magnus-admin", Some("n1"), now).is_err(), "{k}");
        }
        // A refresh's ID token, without a nonce, is checked for the rest.
        let mut c = ok.clone();
        c.as_object_mut().unwrap().remove("nonce");
        assert_eq!(check_claims(&c, "https://idp/realms/m", "magnus-admin", None, now).unwrap(), "abc");
        assert!(check_claims(&c, "https://idp/realms/m", "magnus-admin", Some("n1"), now).is_err(), "a sign-in's token without its nonce passed");
        c["aud"] = json!("magnus-test");
        assert!(check_claims(&c, "https://idp/realms/m", "magnus-admin", None, now).is_err());
        let mut c = ok.clone();
        c["azp"] = json!("magnus-test");
        assert!(check_claims(&c, "https://idp/realms/m", "magnus-admin", Some("n1"), now).is_err());
    }
}
