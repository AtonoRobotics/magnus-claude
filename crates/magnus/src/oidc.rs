//! Sign-in through an OpenID Connect provider (Keycloak, for one).
//!
//! The app signs in with the authorization code and PKCE (RFC 7636) in the
//! person's own browser, which comes back to a loopback address on their
//! computer (RFC 8252): the appliance makes the verifier, state and nonce
//! and keeps them, the app passes back the code, and the appliance
//! exchanges it. The code is useless without the verifier, which never
//! leaves the appliance. Passkeys and two-factor are the provider's.
//!
//! A browser on the appliance's own page signs in with the device-code
//! grant (RFC 8628) instead: the person approves a code on the provider's
//! page, and the appliance collects the ID token itself.
//!
//! No password reaches Magnus. The ID token comes straight from the
//! provider's token endpoint over TLS, so TLS stands in for checking its
//! signature (OpenID Connect Core 1.0, 3.1.3.7, step 6); the issuer,
//! audience, expiry and nonce are still checked.

use crate::config::Oidc as Cfg;
pub use crate::identity::{Begun, Polled, Rights};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Deserialize, Clone, Debug)]
struct Discovery {
    issuer: String,
    token_endpoint: String,
    authorization_endpoint: Option<String>,
    device_authorization_endpoint: Option<String>,
}

/// A sign-in in the person's browser, by its `state`.
struct CodeFlow {
    verifier: String,
    nonce: String,
    redirect_uri: String,
    expires: DateTime<Utc>,
    app: bool,
}


struct Pending {
    device_code: String,
    /// PKCE for the device grant too: a provider that requires it for the
    /// client (Keycloak does, once S256 is set) requires it here.
    verifier: String,
    interval: Duration,
    next_poll: DateTime<Utc>,
    expires: DateTime<Utc>,
    app: bool,
}

/// What the person needs to approve a sign-in.
pub struct Started {
    pub flow: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    pub interval: i64,
}


/// A list of names at a JSON pointer. Keycloak writes groups as paths
/// (`/finance`); the leading slash is not part of the name.
fn names_at(c: &Value, pointer: &Option<String>) -> Option<Vec<String>> {
    let p = pointer.as_deref()?;
    let v = c.pointer(p).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Some(v.iter().filter_map(|x| x.as_str()).map(|s| s.trim_start_matches('/').to_string()).filter(|s| !s.is_empty()).collect())
}

pub struct Provider {
    cfg: Cfg,
    http: reqwest::Client,
    discovery: tokio::sync::OnceCell<Discovery>,
    pending: Mutex<BTreeMap<String, Pending>>,
    code_flows: Mutex<BTreeMap<String, CodeFlow>>,
}

/// Only https, except on this machine (for tests and a local provider).
fn checked(url: &str) -> Result<url::Url> {
    let u = url::Url::parse(url).with_context(|| format!("{url} is not an address"))?;
    let local = matches!(u.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback()) || matches!(u.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback()) || u.host_str() == Some("localhost");
    if u.scheme() != "https" && !(u.scheme() == "http" && local) {
        bail!("{url} is not https");
    }
    Ok(u)
}

/// The claims of an ID token, without checking its signature (see above).
fn claims(id_token: &str) -> Result<Value> {
    let payload = id_token.split('.').nth(1).ok_or_else(|| anyhow!("the ID token is not a JWT"))?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).context("the ID token does not decode")?;
    serde_json::from_slice(&raw).context("the ID token's claims do not parse")
}

/// The person an ID token names, after checking it is for us and current.
fn person(c: &Value, issuer: &str, client_id: &str, claim: &str, now: DateTime<Utc>) -> Result<String> {
    if c["iss"].as_str() != Some(issuer) {
        bail!("the ID token is from {}, not {issuer}", c["iss"]);
    }
    let aud_ok = match &c["aud"] {
        Value::String(a) => a == client_id,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(client_id)),
        _ => false,
    };
    if !aud_ok || c.get("azp").and_then(|v| v.as_str()).is_some_and(|azp| azp != client_id) {
        bail!("the ID token is not for client {client_id}");
    }
    let exp = c["exp"].as_i64().ok_or_else(|| anyhow!("the ID token has no expiry"))?;
    if exp <= now.timestamp() {
        bail!("the ID token has expired");
    }
    let who = c[claim].as_str().ok_or_else(|| anyhow!("the ID token has no {claim} claim"))?;
    Ok(who.trim().to_lowercase())
}

impl Provider {
    pub fn new(cfg: Cfg) -> Result<Provider> {
        checked(&cfg.issuer)?;
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        Ok(Provider { cfg, http, discovery: tokio::sync::OnceCell::new(), pending: Mutex::new(BTreeMap::new()), code_flows: Mutex::new(BTreeMap::new()) })
    }

    /// What the sign-in button names.
    pub fn label(&self) -> String {
        self.cfg.label.clone().unwrap_or_else(|| url::Url::parse(&self.cfg.issuer).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_else(|| self.cfg.issuer.clone()))
    }

    async fn discovery(&self) -> Result<&Discovery> {
        self.discovery
            .get_or_try_init(|| async {
                let url = format!("{}/.well-known/openid-configuration", self.cfg.issuer.trim_end_matches('/'));
                let d: Discovery = self.http.get(&url).send().await?.error_for_status()?.json().await.with_context(|| format!("read {url}"))?;
                if d.issuer != self.cfg.issuer {
                    bail!("{url} names issuer {}, not {}", d.issuer, self.cfg.issuer);
                }
                checked(&d.token_endpoint)?;
                for e in [&d.authorization_endpoint, &d.device_authorization_endpoint].into_iter().flatten() {
                    checked(e)?;
                }
                Ok(d)
            })
            .await
    }

    /// Room for one more waiting sign-in, after forgetting the expired.
    fn room(&self, now: DateTime<Utc>) -> Result<()> {
        let mut p = self.pending.lock().unwrap();
        let mut c = self.code_flows.lock().unwrap();
        p.retain(|_, f| f.expires > now);
        c.retain(|_, f| f.expires > now);
        if p.len() + c.len() >= self.cfg.max_pending {
            bail!("too many sign-ins are waiting; try again in a few minutes");
        }
        Ok(())
    }

    /// The person and their rights from a token response, after checking
    /// the ID token is for us, current, and (in a browser sign-in) answers
    /// the nonce we sent.
    fn signed_in(&self, d: &Discovery, b: &Value, nonce: Option<&str>, now: DateTime<Utc>) -> Result<(String, Rights)> {
        let id_token = b["id_token"].as_str().ok_or_else(|| anyhow!("{} sent no ID token (is \"openid\" in the scope?)", self.label()))?;
        let c = claims(id_token)?;
        let person = person(&c, &d.issuer, &self.cfg.client_id, &self.cfg.person_claim, now)?;
        if let Some(n) = nonce {
            if c["nonce"].as_str() != Some(n) {
                bail!("the ID token does not answer this sign-in's nonce");
            }
        }
        Ok((person, Rights { groups: names_at(&c, &self.cfg.groups_claim), roles: names_at(&c, &self.cfg.roles_claim) }))
    }

    /// Begin a sign-in in the person's browser that comes back to
    /// `redirect_uri`, a loopback address on their computer.
    pub async fn begin(&self, redirect_uri: &str, app: bool) -> Result<Begun> {
        crate::connectors::loopback(redirect_uri)?;
        let now = Utc::now();
        self.room(now)?;
        let d = self.discovery().await?;
        let endpoint = d.authorization_endpoint.as_deref().ok_or_else(|| anyhow!("{} does not offer the authorization code grant", self.label()))?;
        let (verifier, state, nonce) = (crate::connectors::random(), crate::connectors::random(), crate::connectors::random());
        let mut u = url::Url::parse(endpoint)?;
        u.query_pairs_mut()
            .append_pair("client_id", &self.cfg.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &self.cfg.scope)
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &crate::connectors::challenge(&verifier))
            .append_pair("code_challenge_method", "S256");
        let expires_in = i64::from(self.cfg.sign_in_minutes) * 60;
        self.code_flows
            .lock()
            .unwrap()
            .insert(state.clone(), CodeFlow { verifier, nonce, redirect_uri: redirect_uri.into(), expires: now + Duration::seconds(expires_in), app });
        Ok(Begun { state, authorize_url: u.into(), expires_in })
    }

    /// Finish a browser sign-in with what came back to the loopback: the
    /// code, the state it answers, and the issuer if the provider names it
    /// (RFC 9207). Each state finishes once.
    pub async fn finish(&self, state: &str, code: &str, iss: Option<&str>) -> Result<Polled> {
        let now = Utc::now();
        let Some(f) = self.code_flows.lock().unwrap().remove(state) else { return Ok(Polled::Failed("This sign-in is over. Start again.".into())) };
        if f.expires <= now {
            return Ok(Polled::Failed("The sign-in took too long. Start again.".into()));
        }
        let d = self.discovery().await?;
        if iss.is_some_and(|i| i != d.issuer) {
            tracing::warn!("oidc: a sign-in came back from {}, not {}", iss.unwrap_or_default(), d.issuer);
            return Ok(Polled::Failed(format!("The sign-in came back from somewhere other than {}. Start again.", self.label())));
        }
        let r = self
            .http
            .post(&d.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", f.redirect_uri.as_str()),
                ("client_id", self.cfg.client_id.as_str()),
                ("code_verifier", f.verifier.as_str()),
            ])
            .send()
            .await?;
        let status = r.status();
        let b: Value = r.json().await.unwrap_or_default();
        if !status.is_success() {
            let why = b["error_description"].as_str().or(b["error"].as_str()).unwrap_or(status.as_str()).to_string();
            return Ok(Polled::Failed(format!("{} refused the sign-in: {why}", self.label())));
        }
        match self.signed_in(d, &b, Some(&f.nonce), now) {
            Ok((person, rights)) => Ok(Polled::Done { person, app: f.app, rights }),
            Err(e) => {
                tracing::warn!("oidc: refused an ID token: {e:#}");
                Ok(Polled::Failed(format!("{} answered, but not with a sign-in for this appliance.", self.label())))
            }
        }
    }

    pub async fn start(&self, app: bool) -> Result<Started> {
        let now = Utc::now();
        self.room(now)?;
        let d = self.discovery().await?;
        let endpoint = d.device_authorization_endpoint.as_deref().ok_or_else(|| anyhow!("{} does not offer the device-code grant", self.label()))?;
        let verifier = crate::connectors::random();
        let challenge = crate::connectors::challenge(&verifier);
        let r = self
            .http
            .post(endpoint)
            .form(&[("client_id", self.cfg.client_id.as_str()), ("scope", self.cfg.scope.as_str()), ("code_challenge", challenge.as_str()), ("code_challenge_method", "S256")])
            .send()
            .await?;
        let status = r.status();
        let b: Value = r.json().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{} refused to start a sign-in: {}", self.label(), b["error_description"].as_str().or(b["error"].as_str()).unwrap_or(status.as_str()));
        }
        let field = |k: &str| b[k].as_str().map(String::from).ok_or_else(|| anyhow!("{} sent no {k}", self.label()));
        let expires_in = b["expires_in"].as_i64().ok_or_else(|| anyhow!("{} sent no expires_in", self.label()))?;
        // RFC 8628 3.2: without an interval, wait 5 seconds between polls.
        let interval = b["interval"].as_i64().unwrap_or(5);
        let flow = crate::kernel::new_id("signin");
        let started = Started {
            flow: flow.clone(),
            user_code: field("user_code")?,
            verification_uri: field("verification_uri")?,
            verification_uri_complete: b["verification_uri_complete"].as_str().map(String::from),
            expires_in,
            interval,
        };
        self.pending.lock().unwrap().insert(
            flow,
            Pending { device_code: field("device_code")?, verifier, interval: Duration::seconds(interval), next_poll: now, expires: now + Duration::seconds(expires_in), app },
        );
        Ok(started)
    }

    /// Ask the provider once whether the sign-in was approved. Asking more
    /// often than the provider allows answers Pending without asking it.
    pub async fn poll(&self, flow: &str) -> Result<Polled> {
        let now = Utc::now();
        let (device_code, verifier, app) = {
            let mut p = self.pending.lock().unwrap();
            let Some(f) = p.get(flow) else { return Ok(Polled::Failed("This sign-in is over. Start again.".into())) };
            if f.expires <= now {
                p.remove(flow);
                return Ok(Polled::Failed("The sign-in code expired. Start again.".into()));
            }
            if now < f.next_poll {
                return Ok(Polled::Pending);
            }
            (f.device_code.clone(), f.verifier.clone(), f.app)
        };
        let d = self.discovery().await?;
        let r = self
            .http
            .post(&d.token_endpoint)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code.as_str()),
                ("client_id", self.cfg.client_id.as_str()),
                ("code_verifier", verifier.as_str()),
            ])
            .send()
            .await?;
        let status = r.status();
        let b: Value = r.json().await.unwrap_or_default();
        let mut p = self.pending.lock().unwrap();
        if status.is_success() {
            p.remove(flow);
            return match self.signed_in(d, &b, None, now) {
                Ok((person, rights)) => Ok(Polled::Done { person, app, rights }),
                Err(e) => {
                    tracing::warn!("oidc: refused an ID token: {e:#}");
                    Ok(Polled::Failed(format!("{} answered, but not with a sign-in for this appliance.", self.label())))
                }
            };
        }
        let Some(f) = p.get_mut(flow) else { return Ok(Polled::Failed("This sign-in is over. Start again.".into())) };
        match b["error"].as_str().unwrap_or_default() {
            "authorization_pending" => {
                f.next_poll = now + f.interval;
                Ok(Polled::Pending)
            }
            // RFC 8628 3.5: slow down by 5 seconds from now on.
            "slow_down" => {
                f.interval = f.interval + Duration::seconds(5);
                f.next_poll = now + f.interval;
                Ok(Polled::Pending)
            }
            "access_denied" => {
                p.remove(flow);
                Ok(Polled::Failed("The sign-in was declined.".into()))
            }
            "expired_token" => {
                p.remove(flow);
                Ok(Polled::Failed("The sign-in code expired. Start again.".into()))
            }
            other => {
                p.remove(flow);
                let why = b["error_description"].as_str().unwrap_or(if other.is_empty() { status.as_str() } else { other });
                Ok(Polled::Failed(format!("{} refused the sign-in: {why}", self.label())))
            }
        }
    }
}

#[async_trait::async_trait]
impl crate::identity::Identity for Provider {
    fn label(&self) -> String {
        Provider::label(self)
    }
    async fn begin(&self, redirect_uri: &str, app: bool) -> Result<Begun> {
        Provider::begin(self, redirect_uri, app).await
    }
    async fn finish(&self, state: &str, code: &str, iss: Option<&str>) -> Result<Polled> {
        Provider::finish(self, state, code, iss).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_id_token_must_be_ours_current_and_name_someone() {
        let now = Utc::now();
        let iss = "https://id.example/realms/m";
        let good = json!({ "iss": iss, "aud": "magnus", "azp": "magnus", "exp": now.timestamp() + 60, "preferred_username": "Samuel" });
        assert_eq!(person(&good, iss, "magnus", "preferred_username", now).unwrap(), "samuel");
        let mut c = good.clone();
        c["aud"] = json!(["account", "magnus"]);
        assert!(person(&c, iss, "magnus", "preferred_username", now).is_ok());
        for (k, v) in [("iss", json!("https://other/realms/m")), ("aud", json!("someone-else")), ("azp", json!("someone-else")), ("exp", json!(now.timestamp() - 1))] {
            let mut c = good.clone();
            c[k] = v;
            assert!(person(&c, iss, "magnus", "preferred_username", now).is_err(), "{k} was not checked");
        }
        assert!(person(&good, iss, "magnus", "email", now).is_err());
    }

    /// Keycloak always answers with the nonce it was sent, so only here can a
    /// token for another sign-in be shown to be refused.
    #[test]
    fn a_browser_sign_in_needs_its_own_nonce() {
        let iss = "https://id.example/realms/m";
        let cfg: Cfg = toml::from_str(&format!("issuer = \"{iss}\"\nclient_id = \"magnus\"")).unwrap();
        let p = Provider::new(cfg).unwrap();
        let d = Discovery { issuer: iss.into(), token_endpoint: format!("{iss}/token"), authorization_endpoint: None, device_authorization_endpoint: None };
        let b64 = |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
        let now = Utc::now();
        let token = |nonce: &str| json!({ "id_token": format!("{}.{}.sig", b64(&json!({})), b64(&json!({ "iss": iss, "aud": "magnus", "exp": now.timestamp() + 60, "preferred_username": "carol", "nonce": nonce }))) });
        assert_eq!(p.signed_in(&d, &token("n-1"), Some("n-1"), now).unwrap().0, "carol");
        assert!(p.signed_in(&d, &token("n-2"), Some("n-1"), now).is_err(), "a token for another sign-in was taken");
        assert!(p.signed_in(&d, &json!({ "id_token": format!("{}.{}.sig", b64(&json!({})), b64(&json!({ "iss": iss, "aud": "magnus", "exp": now.timestamp() + 60, "preferred_username": "carol" }))) }), Some("n-1"), now).is_err(), "a token with no nonce was taken");
    }

    #[test]
    fn a_jwt_payload_decodes() {
        let b64 = |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
        let t = format!("{}.{}.sig", b64(&json!({ "alg": "RS256" })), b64(&json!({ "sub": "x" })));
        assert_eq!(claims(&t).unwrap()["sub"], "x");
        assert!(claims("nope").is_err());
    }

    #[test]
    fn only_https_except_on_this_machine() {
        assert!(checked("https://id.example/realms/m").is_ok());
        assert!(checked("http://127.0.0.1:8080/realms/m").is_ok());
        assert!(checked("http://localhost:8080/realms/m").is_ok());
        assert!(checked("http://id.example/realms/m").is_err());
    }
}
