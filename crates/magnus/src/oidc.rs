//! Sign-in through an OpenID Connect provider, with the device-code grant
//! (RFC 8628): the appliance asks the provider for a code, the person
//! approves it on the provider's own page, and the appliance collects the
//! ID token itself. No password reaches Magnus, and no provider token is
//! kept: the ID token names the person, then a Magnus session takes over.
//!
//! The ID token comes straight from the provider's token endpoint over TLS,
//! so TLS stands in for checking its signature (OpenID Connect Core 1.0,
//! 3.1.3.7, step 6); the issuer, audience and expiry are still checked.

use crate::config::Oidc as Cfg;
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
    device_authorization_endpoint: Option<String>,
}

struct Pending {
    device_code: String,
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

pub enum Polled {
    /// Not approved yet; ask again later.
    Pending,
    /// Approved: the person claim from the ID token, and whether the
    /// sign-in began in the app.
    Done { person: String, app: bool, rights: Rights },
    /// Over: declined, expired or refused, with a message for the person.
    Failed(String),
}

/// Groups and roles the provider gave in the ID token, where the owner
/// named their claims; None where not.
#[derive(Debug, Default, Clone)]
pub struct Rights {
    pub groups: Option<Vec<String>>,
    pub roles: Option<Vec<String>>,
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
        Ok(Provider { cfg, http, discovery: tokio::sync::OnceCell::new(), pending: Mutex::new(BTreeMap::new()) })
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
                match &d.device_authorization_endpoint {
                    Some(e) => checked(e)?,
                    None => bail!("{} does not offer the device-code grant", self.cfg.issuer),
                };
                Ok(d)
            })
            .await
    }

    pub async fn start(&self, app: bool) -> Result<Started> {
        let now = Utc::now();
        {
            let mut p = self.pending.lock().unwrap();
            p.retain(|_, f| f.expires > now);
            if p.len() >= self.cfg.max_pending {
                bail!("too many sign-ins are waiting; try again in a few minutes");
            }
        }
        let d = self.discovery().await?;
        let endpoint = d.device_authorization_endpoint.as_deref().expect("checked in discovery");
        let r = self.http.post(endpoint).form(&[("client_id", self.cfg.client_id.as_str()), ("scope", self.cfg.scope.as_str())]).send().await?;
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
            Pending { device_code: field("device_code")?, interval: Duration::seconds(interval), next_poll: now, expires: now + Duration::seconds(expires_in), app },
        );
        Ok(started)
    }

    /// Ask the provider once whether the sign-in was approved. Asking more
    /// often than the provider allows answers Pending without asking it.
    pub async fn poll(&self, flow: &str) -> Result<Polled> {
        let now = Utc::now();
        let (device_code, app) = {
            let mut p = self.pending.lock().unwrap();
            let Some(f) = p.get(flow) else { return Ok(Polled::Failed("This sign-in is over. Start again.".into())) };
            if f.expires <= now {
                p.remove(flow);
                return Ok(Polled::Failed("The sign-in code expired. Start again.".into()));
            }
            if now < f.next_poll {
                return Ok(Polled::Pending);
            }
            (f.device_code.clone(), f.app)
        };
        let d = self.discovery().await?;
        let r = self
            .http
            .post(&d.token_endpoint)
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", device_code.as_str()), ("client_id", self.cfg.client_id.as_str())])
            .send()
            .await?;
        let status = r.status();
        let b: Value = r.json().await.unwrap_or_default();
        let mut p = self.pending.lock().unwrap();
        if status.is_success() {
            p.remove(flow);
            let id_token = b["id_token"].as_str().ok_or_else(|| anyhow!("{} sent no ID token (is \"openid\" in the scope?)", self.label()))?;
            return match claims(id_token).and_then(|c| person(&c, &d.issuer, &self.cfg.client_id, &self.cfg.person_claim, now).map(|p| (p, c))) {
                Ok((person, c)) => {
                    let rights = Rights { groups: names_at(&c, &self.cfg.groups_claim), roles: names_at(&c, &self.cfg.roles_claim) };
                    Ok(Polled::Done { person, app, rights })
                }
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
