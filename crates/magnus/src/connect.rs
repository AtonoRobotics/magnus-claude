//! Connectors: the person's apps over MCP. They come from the configuration
//! file or are added by the owner in the app; their tools are classified by
//! the owner when first seen.

use crate::config::{Connector, ToolClass};
use crate::kernel::Kernel;
use serde_json::{json, Value};

/// The connectors the owner can add from the app (`presets/connectors.toml`).
pub fn presets() -> Vec<Value> {
    let t: toml::Value = toml::from_str(include_str!("../presets/connectors.toml")).expect("presets/connectors.toml");
    let v = serde_json::to_value(t).expect("presets as JSON");
    v["preset"].as_array().cloned().unwrap_or_default()
}

/// A class for a tool the owner has not classified yet, from its name and
/// inputs: what the review card proposes. Doubt proposes the riskier class.
pub fn suggest(remote: &str, schema: &Value) -> ToolClass {
    let n = remote.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| part == *w));
    let risk = if has(&["send", "reply", "forward", "post", "share", "invite", "publish", "delete", "remove", "trash", "purge"]) {
        "irreversible"
    } else if has(&["get", "list", "search", "read", "find", "query", "fetch", "lookup", "describe", "show", "view", "download", "count", "info", "stat", "tree"]) {
        "read"
    } else if has(&["create", "update", "write", "edit", "move", "draft", "modify", "add", "set", "rename", "copy", "upload", "mark", "label", "archive"]) {
        "reversible_write"
    } else {
        "irreversible"
    };
    let sends = has(&["send", "reply", "forward", "post", "invite"])
        .then(|| ["body", "message", "content", "text", "html_body", "htmlBody", "comment", "description"].into_iter().find(|f| schema["properties"].get(*f).is_some()).map(String::from))
        .flatten();
    ToolClass { risk: risk.into(), label: None, sends }
}

impl Kernel {
    /// Every connector: the configuration file's, then the owner's.
    pub fn connectors(&self) -> Vec<Connector> {
        let mut v = self.cfg.connectors.clone();
        v.extend(self.read(|s| s.connectors.values().cloned().collect::<Vec<_>>()));
        v
    }

    pub fn connector(&self, name: &str) -> Option<Connector> {
        self.cfg.connectors.iter().find(|c| c.name == name).cloned().or_else(|| self.read(|s| s.connectors.get(name).cloned()))
    }

    /// A tool's class: the owner's from review, else the configuration's.
    pub fn class_of(&self, tool: &str) -> Option<ToolClass> {
        let (app, remote) = tool.split_once("__")?;
        self.read(|s| s.classes.get(tool).cloned()).or_else(|| self.connector(app).and_then(|c| c.tools.get(remote).cloned()))
    }

    /// A connector as the app shows it to `person`: what it is, whether they
    /// are signed in, and its tools with their classes and review state.
    pub fn connector_view(&self, c: &Connector, person: &str, owner: bool) -> Value {
        let prefix = format!("{}__", c.name);
        let tools: Vec<Value> = self.read(|s| {
            let mut names: Vec<&String> = s.pins.keys().filter(|t| t.starts_with(&prefix)).collect();
            names.sort();
            names
                .into_iter()
                .map(|t| {
                    let p = &s.pins[t];
                    json!({ "tool": t, "approved": p.approved, "changed": s.changed.contains_key(t), "class": s.classes.get(t).or_else(|| c.tools.get(&t[prefix.len()..])) })
                })
                .collect()
        });
        let from_file = self.cfg.connectors.iter().any(|x| x.name == c.name);
        let mut v = json!({
            "name": c.name,
            "title": c.title(),
            "transport": c.transport,
            "sign_in": c.auth.as_ref().map(|a| a.kind.clone()),
            "signed_in": if c.per_person() { self.read(|s| s.signed_in.contains(&(c.name.clone(), person.to_string()))) } else { c.credential.is_none() || self.vault.get(c.credential.as_deref().unwrap_or_default()).ok().flatten().is_some() },
            "label": c.label,
            "audience": c.audience,
            "tools": tools,
            "from_file": from_file,
        });
        if owner {
            v["connector"] = serde_json::to_value(c).unwrap_or_default();
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_a_valid_connector_once_filled_in() {
        let ps = presets();
        assert!(ps.len() >= 5);
        for p in ps {
            let filled = p["connector"].to_string().replace("{client_id}", "client-1").replace("{url}", "http://127.0.0.1:3365/mcp").replace("{folder}", "/srv/files");
            let c: Connector = serde_json::from_str(&filled).unwrap_or_else(|e| panic!("{}: {e}", p["id"]));
            c.validate().unwrap_or_else(|e| panic!("{}: {e}", p["id"]));
            for ask in p["asks"].as_array().unwrap() {
                assert!(p["connector"].to_string().contains(&format!("{{{}}}", ask.as_str().unwrap())), "{} asks {ask} but never uses it", p["id"]);
            }
        }
    }

    #[test]
    fn suggestions_lean_to_the_riskier_class() {
        let schema = json!({ "properties": { "to": {}, "body": {} } });
        let s = suggest("send_message", &schema);
        assert_eq!((s.risk.as_str(), s.sends.as_deref()), ("irreversible", Some("body")));
        assert_eq!(suggest("list_inbox", &json!({})).risk, "read");
        assert_eq!(suggest("search_files", &json!({})).risk, "read");
        assert_eq!(suggest("create_draft", &json!({})).risk, "reversible_write");
        assert_eq!(suggest("delete_event", &json!({})).risk, "irreversible");
        assert_eq!(suggest("frobnicate", &json!({})).risk, "irreversible");
        assert_eq!(suggest("read_and_delete", &json!({})).risk, "irreversible");
        assert!(suggest("create_event", &json!({ "properties": { "description": {} } })).sends.is_none());
    }
}

// ---------- sign-in ----------

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// A person's tokens for an app, as the vault holds them.
#[derive(Serialize, Deserialize, Clone)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub issued_at: Option<DateTime<Utc>>,
}

impl Tokens {
    /// Whether the access token is good for a while yet: renewed a minute
    /// before it expires, or halfway through a shorter life.
    fn fresh(&self, now: DateTime<Utc>) -> bool {
        let Some(exp) = self.expires_at else { return true };
        let margin = self.issued_at.map(|i| (exp - i) / 2).filter(|m| *m < Duration::seconds(60)).unwrap_or(Duration::seconds(60));
        exp > now + margin
    }
}

/// The vault entry of a person's tokens for a connector.
pub fn vault_name(connector: &str, person: &str) -> String {
    format!("connector:{connector}:{person}")
}

/// What the boundary has for calling an app as a person.
pub enum Cred {
    /// The app takes no credential.
    None,
    Bearer(crate::vault::Secret),
    /// The person must sign in (again) first: why, for them.
    Missing(String),
}

struct Pending {
    connector: String,
    person: String,
    expires: DateTime<Utc>,
    how: How,
}

enum How {
    Device { device_code: String, interval: Duration, next_poll: DateTime<Utc> },
    Loopback { verifier: String, state: String, redirect_uri: String },
    ApiKey,
}

pub enum Polled {
    Pending,
    Done,
    Failed(String),
}

/// Sign-ins in progress and the locks that keep one refresh per token.
pub struct Flows {
    http: reqwest::Client,
    pending: Mutex<BTreeMap<String, Pending>>,
    refresh: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Default for Flows {
    fn default() -> Self {
        Flows {
            http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().expect("http client"),
            pending: Mutex::new(BTreeMap::new()),
            refresh: tokio::sync::Mutex::new(HashMap::new()),
        }
    }
}

fn random(bytes: usize) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// The PKCE S256 challenge for a verifier (RFC 7636 4.2).
pub fn challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// A redirect may only return to the person's own machine (RFC 8252 7.3).
pub fn loopback_ok(uri: &str) -> bool {
    let Ok(u) = url::Url::parse(uri) else { return false };
    let host_ok = matches!(u.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback()) || matches!(u.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback()) || u.host_str() == Some("localhost");
    u.scheme() == "http" && host_ok && u.port().is_some() && u.username().is_empty() && u.password().is_none() && u.fragment().is_none()
}

/// A token endpoint's answer: tokens, or the OAuth error code and text.
/// Some providers (GitHub) report errors with HTTP 200.
fn tokens_from(status: reqwest::StatusCode, b: &Value, now: DateTime<Utc>) -> std::result::Result<Tokens, (String, String)> {
    if let Some(e) = b["error"].as_str() {
        return Err((e.to_string(), b["error_description"].as_str().unwrap_or(e).to_string()));
    }
    let Some(access) = b["access_token"].as_str().filter(|t| !t.is_empty()) else {
        return Err(("no_token".into(), format!("the provider answered HTTP {status} without a token")));
    };
    Ok(Tokens {
        access_token: access.into(),
        refresh_token: b["refresh_token"].as_str().map(String::from),
        expires_at: b["expires_in"].as_i64().or_else(|| b["expires_in"].as_str().and_then(|s| s.parse().ok())).map(|s| now + Duration::seconds(s)),
        issued_at: Some(now),
    })
}

impl Kernel {
    fn client_secret(&self, a: &crate::config::ConnectorAuth) -> Option<String> {
        a.client_secret.as_ref().and_then(|n| self.vault.get(n).ok().flatten()).map(|s| s.expose().to_string())
    }

    async fn token_request(&self, a: &crate::config::ConnectorAuth, form: Vec<(&str, String)>) -> Result<std::result::Result<Tokens, (String, String)>> {
        let mut form = form;
        form.push(("client_id", a.client_id.clone()));
        if let Some(s) = self.client_secret(a) {
            form.push(("client_secret", s));
        }
        let endpoint = a.token_endpoint.as_deref().ok_or_else(|| anyhow!("no token endpoint"))?;
        let r = self.connect.http.post(endpoint).header("accept", "application/json").form(&form).send().await?;
        let status = r.status();
        let b: Value = r.json().await.unwrap_or_default();
        Ok(tokens_from(status, &b, Utc::now()))
    }

    /// Start a person's sign-in to an app: what to show them.
    pub async fn connector_sign_in(&self, c: &Connector, person: &str, redirect_uri: Option<&str>) -> std::result::Result<Value, String> {
        let a = c.auth.as_ref().ok_or_else(|| format!("{} takes no sign-in", c.title()))?;
        let now = Utc::now();
        let flow = crate::kernel::new_id("connect");
        let ten = now + Duration::minutes(10);
        let (pending, shown) = match a.kind.as_str() {
            "api_key" => (
                Pending { connector: c.name.clone(), person: person.into(), expires: ten, how: How::ApiKey },
                json!({ "kind": "api_key", "instructions": format!("Paste your API key for {}. It goes to the appliance's vault.", c.title()) }),
            ),
            "device" => {
                let endpoint = a.device_authorization_endpoint.as_deref().unwrap_or_default();
                let r = self
                    .connect
                    .http
                    .post(endpoint)
                    .header("accept", "application/json")
                    .form(&[("client_id", a.client_id.as_str()), ("scope", a.scopes.join(" ").as_str())])
                    .send()
                    .await
                    .map_err(|e| format!("{} could not be reached: {e}", c.title()))?;
                let status = r.status();
                let b: Value = r.json().await.unwrap_or_default();
                let field = |k: &str| b[k].as_str().map(String::from).ok_or_else(|| format!("{} did not start a sign-in: {}", c.title(), b["error_description"].as_str().or(b["error"].as_str()).unwrap_or(status.as_str())));
                let device_code = field("device_code")?;
                let user_code = field("user_code")?;
                let uri = b["verification_uri"].as_str().or(b["verification_url"].as_str()).map(String::from).ok_or_else(|| format!("{} sent no verification address", c.title()))?;
                let expires_in = b["expires_in"].as_i64().unwrap_or(900);
                let interval = b["interval"].as_i64().unwrap_or(5);
                (
                    Pending { connector: c.name.clone(), person: person.into(), expires: now + Duration::seconds(expires_in), how: How::Device { device_code, interval: Duration::seconds(interval), next_poll: now } },
                    json!({ "kind": "device", "user_code": user_code, "url": b["verification_uri_complete"].as_str().unwrap_or(&uri), "interval": interval, "expires_in": expires_in,
                        "instructions": format!("Open {uri}, enter the code, and approve Magnus for {}.", c.title()) }),
                )
            }
            "loopback" => {
                let redirect_uri = redirect_uri.filter(|u| loopback_ok(u)).ok_or("the sign-in must return to an address on your own machine (http://127.0.0.1:<port>/)")?;
                let (verifier, state) = (random(48), random(24));
                let mut u = url::Url::parse(a.authorization_endpoint.as_deref().unwrap_or_default()).map_err(|e| e.to_string())?;
                {
                    let mut q = u.query_pairs_mut();
                    q.append_pair("response_type", "code").append_pair("client_id", &a.client_id).append_pair("redirect_uri", redirect_uri);
                    q.append_pair("scope", &a.scopes.join(" ")).append_pair("state", &state).append_pair("code_challenge", &challenge(&verifier)).append_pair("code_challenge_method", "S256");
                    for (k, v) in &a.params {
                        q.append_pair(k, v);
                    }
                }
                (
                    Pending { connector: c.name.clone(), person: person.into(), expires: ten, how: How::Loopback { verifier, state, redirect_uri: redirect_uri.into() } },
                    json!({ "kind": "loopback", "url": u.to_string(), "instructions": format!("Sign in to {} in your browser and approve Magnus. Your browser then returns to this machine.", c.title()) }),
                )
            }
            other => return Err(format!("unknown sign-in kind {other}")),
        };
        let mut p = self.connect.pending.lock().unwrap();
        p.retain(|_, f| f.expires > now);
        if p.values().filter(|f| f.person == person).count() >= 8 {
            return Err("too many sign-ins are waiting; finish or wait for one".into());
        }
        p.insert(flow.clone(), pending);
        let mut shown = shown;
        shown["flow"] = json!(flow);
        Ok(shown)
    }

    /// Finish a sign-in, or ask the provider once whether it is approved.
    /// `input`: `{api_key}`, or `{code, state}` / `{redirect_url}` from the
    /// redirect; nothing for a device code.
    pub async fn connector_sign_in_complete(&self, flow: &str, person: &str, input: &Value) -> Result<Polled> {
        let now = Utc::now();
        let (connector, how) = {
            let mut p = self.connect.pending.lock().unwrap();
            let Some(f) = p.get(flow).filter(|f| f.person == person) else { return Ok(Polled::Failed("This sign-in is over. Start again.".into())) };
            if f.expires <= now {
                p.remove(flow);
                return Ok(Polled::Failed("The sign-in expired. Start again.".into()));
            }
            if let How::Device { next_poll, .. } = &f.how {
                if now < *next_poll {
                    return Ok(Polled::Pending);
                }
            }
            let how = match &f.how {
                How::Device { device_code, interval, .. } => How::Device { device_code: device_code.clone(), interval: *interval, next_poll: now },
                How::Loopback { verifier, state, redirect_uri } => How::Loopback { verifier: verifier.clone(), state: state.clone(), redirect_uri: redirect_uri.clone() },
                How::ApiKey => How::ApiKey,
            };
            (f.connector.clone(), how)
        };
        let Some(c) = self.connector(&connector) else { return Ok(Polled::Failed("This app was removed.".into())) };
        let Some(a) = c.auth.clone() else { return Ok(Polled::Failed(format!("{} no longer takes a sign-in.", c.title()))) };
        let tokens = match how {
            How::ApiKey => {
                let k = input["api_key"].as_str().map(str::trim).unwrap_or_default();
                if k.is_empty() {
                    return Ok(Polled::Failed("Paste the key first.".into()));
                }
                Tokens { access_token: k.into(), refresh_token: None, expires_at: None, issued_at: Some(now) }
            }
            How::Device { device_code, interval, .. } => {
                match self.token_request(&a, vec![("grant_type", "urn:ietf:params:oauth:grant-type:device_code".into()), ("device_code", device_code)]).await? {
                    Ok(t) => t,
                    Err((code, text)) => {
                        let mut p = self.connect.pending.lock().unwrap();
                        let again = |p: &mut BTreeMap<String, Pending>, wait: Duration| {
                            if let Some(Pending { how: How::Device { interval, next_poll, .. }, .. }) = p.get_mut(flow) {
                                *interval = wait;
                                *next_poll = now + wait;
                            }
                        };
                        return Ok(match code.as_str() {
                            "authorization_pending" => {
                                again(&mut p, interval);
                                Polled::Pending
                            }
                            // RFC 8628 3.5: slow down by 5 seconds from now on.
                            "slow_down" => {
                                again(&mut p, interval + Duration::seconds(5));
                                Polled::Pending
                            }
                            _ => {
                                p.remove(flow);
                                Polled::Failed(if code == "access_denied" { "The sign-in was declined.".into() } else { format!("{} refused the sign-in: {text}", c.title()) })
                            }
                        });
                    }
                }
            }
            How::Loopback { verifier, state, redirect_uri } => {
                let (code, got_state, error) = match input["redirect_url"].as_str() {
                    Some(u) => match url::Url::parse(u.trim()) {
                        Ok(u) => {
                            let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
                            (q.get("code").cloned(), q.get("state").cloned(), q.get("error").cloned())
                        }
                        Err(_) => return Ok(Polled::Failed("That is not the address your browser was sent to.".into())),
                    },
                    None => (input["code"].as_str().map(String::from), input["state"].as_str().map(String::from), input["error"].as_str().map(String::from)),
                };
                self.connect.pending.lock().unwrap().remove(flow);
                if let Some(e) = error {
                    return Ok(Polled::Failed(if e == "access_denied" { "The sign-in was declined.".into() } else { format!("{} refused the sign-in: {e}", c.title()) }));
                }
                if got_state.as_deref() != Some(state.as_str()) {
                    return Ok(Polled::Failed("The sign-in came back for a different request. Start again.".into()));
                }
                let Some(code) = code else { return Ok(Polled::Failed("The sign-in came back without a code.".into())) };
                match self.token_request(&a, vec![("grant_type", "authorization_code".into()), ("code", code), ("redirect_uri", redirect_uri), ("code_verifier", verifier)]).await? {
                    Ok(t) => t,
                    Err((_, text)) => return Ok(Polled::Failed(format!("{} refused the sign-in: {text}", c.title()))),
                }
            }
        };
        self.connect.pending.lock().unwrap().remove(flow);
        self.vault.put(&vault_name(&c.name, person), &serde_json::to_string(&tokens)?)?;
        self.live_tools.lock().await.remove(&format!("{}:{person}", c.name));
        self.emit(&format!("person:{person}"), crate::events::Ev::ConnectorSignedIn { connector: c.name.clone(), person: person.into() }).await?;
        Ok(Polled::Done)
    }

    pub async fn connector_sign_out(&self, c: &Connector, person: &str) -> Result<()> {
        self.vault.delete(&vault_name(&c.name, person))?;
        self.live_tools.lock().await.remove(&format!("{}:{person}", c.name));
        self.emit(&format!("person:{person}"), crate::events::Ev::ConnectorSignedOut { connector: c.name.clone(), person: person.into() }).await?;
        Ok(())
    }

    /// The credential for calling an app as `person`: the shared one, or
    /// theirs, refreshed when it is about to expire. Only the boundary and
    /// the tool listing call this; the token never reaches the model.
    pub async fn connector_cred(&self, c: &Connector, person: &str) -> Result<Cred> {
        let Some(a) = c.auth.clone() else {
            return Ok(match &c.credential {
                None => Cred::None,
                Some(n) => match self.vault.get(n)? {
                    Some(s) => Cred::Bearer(s),
                    None => Cred::Missing(format!("no credential is stored for {}", c.title())),
                },
            });
        };
        let name = vault_name(&c.name, person);
        let read = |k: &Kernel| -> Result<Option<Tokens>> { Ok(k.vault.get(&name)?.and_then(|s| serde_json::from_str(s.expose()).ok())) };
        let fresh = |t: &Tokens| t.fresh(Utc::now());
        let missing = || Cred::Missing(format!("{person} is not signed in to {}: sign in under Connectors in the app, then try again", c.title()));
        let Some(t) = read(self)? else { return Ok(missing()) };
        if fresh(&t) {
            return Ok(Cred::Bearer(crate::vault::Secret::new(t.access_token)));
        }
        // One refresh per token: a provider that rotates refresh tokens
        // would sign the person out if two ran at once.
        let lock = self.connect.refresh.lock().await.entry(name.clone()).or_default().clone();
        let _g = lock.lock().await;
        let Some(t) = read(self)? else { return Ok(missing()) };
        if fresh(&t) {
            return Ok(Cred::Bearer(crate::vault::Secret::new(t.access_token)));
        }
        let Some(refresh) = t.refresh_token.clone() else { return Ok(Cred::Missing(format!("{person}'s sign-in to {} expired: sign in again under Connectors", c.title()))) };
        match self.token_request(&a, vec![("grant_type", "refresh_token".into()), ("refresh_token", refresh.clone())]).await {
            Ok(Ok(mut n)) => {
                // A provider that does not rotate keeps the old refresh token valid.
                n.refresh_token = n.refresh_token.or(Some(refresh));
                self.vault.put(&name, &serde_json::to_string(&n)?)?;
                Ok(Cred::Bearer(crate::vault::Secret::new(n.access_token)))
            }
            Ok(Err((code, text))) => {
                tracing::warn!("refresh for {} as {person} refused: {code}", c.name);
                if code == "invalid_grant" {
                    self.connector_sign_out(c, person).await?;
                }
                Ok(Cred::Missing(format!("{} refused to renew {person}'s sign-in ({text}): sign in again under Connectors", c.title())))
            }
            // The provider could not be reached: the token is kept for the next try.
            Err(e) => Ok(Cred::Missing(format!("{}'s sign-in could not be renewed right now ({e}); try again later", c.title()))),
        }
    }
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;

    #[test]
    fn a_redirect_returns_only_to_this_machine() {
        assert!(loopback_ok("http://127.0.0.1:53682/"));
        assert!(loopback_ok("http://localhost:8123/cb"));
        assert!(loopback_ok("http://[::1]:9000/"));
        for bad in ["https://127.0.0.1:53682/", "http://127.0.0.1/", "http://evil.example:80/", "http://127.0.0.1.evil.example:80/", "http://u:p@127.0.0.1:1/", "http://127.0.0.1:1/#x"] {
            assert!(!loopback_ok(bad), "{bad}");
        }
    }

    #[test]
    fn the_pkce_challenge_is_rfc_7636_s256() {
        // RFC 7636 appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn token_answers_are_read_whatever_their_status() {
        let now = Utc::now();
        let t = tokens_from(reqwest::StatusCode::OK, &json!({ "access_token": "a", "refresh_token": "r", "expires_in": 3600 }), now).ok().unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref()), ("a", Some("r")));
        assert_eq!(t.expires_at, Some(now + Duration::seconds(3600)));
        let e = tokens_from(reqwest::StatusCode::OK, &json!({ "error": "authorization_pending" }), now).err().unwrap();
        assert_eq!(e.0, "authorization_pending");
        assert!(tokens_from(reqwest::StatusCode::BAD_REQUEST, &json!({}), now).is_err());
    }
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn a_token_is_renewed_a_minute_early_or_halfway_through_a_short_life() {
        let now = Utc::now();
        let t = |life: i64, age: i64| Tokens { access_token: "a".into(), refresh_token: None, expires_at: Some(now - Duration::seconds(age) + Duration::seconds(life)), issued_at: Some(now - Duration::seconds(age)) };
        assert!(t(3600, 0).fresh(now));
        assert!(!t(3600, 3541).fresh(now), "less than a minute left");
        assert!(t(10, 4).fresh(now));
        assert!(!t(10, 6).fresh(now), "past half of a short life");
        assert!(Tokens { access_token: "k".into(), refresh_token: None, expires_at: None, issued_at: None }.fresh(now));
    }
}
