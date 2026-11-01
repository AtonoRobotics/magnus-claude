//! The person's apps over MCP, added and signed in to from the app (the
//! record's product scope: email, calendar, files, chat, business systems).
//!
//! Apps in the config file stay as written there; apps the owner adds live in
//! the log. Sign-in follows MCP's authorization spec (2025-06-18): the app's
//! protected-resource metadata names its authorization server; Magnus
//! registers itself there (RFC 7591) unless the owner gave a client id, and
//! signs in with the authorization code and PKCE, the code coming back to a
//! loopback address on the person's computer (RFC 8252), where the desktop
//! app catches it. Tokens are kept in the vault and refreshed when they
//! expire; the kill switch revokes them (RFC 7009).

use crate::config::Connector;
use crate::kernel::Kernel;
use crate::vault::Secret;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const OAUTH: &str = "oauth";

/// The vault name of an app's credential: the organisation's key for an
/// app that takes one, or, before connections were each person's own, the
/// one sign-in everyone's runs used (no longer used; revoked with the app).
pub fn credential_name(app: &str) -> String {
    format!("connector_{app}")
}

/// The vault name of one person's own sign-in to an app (connectors spec:
/// the organisation provides the connector, each person signs in with
/// their own account, and no one else acts through it). App names never
/// hold `__`, so the person is what follows the first one.
pub fn connection_name(app: &str, person: &str) -> String {
    format!("connector_{app}__{person}")
}

/// Whether a vault name is one of an app's sign-ins: a person's own, or
/// the old shared one.
pub fn is_sign_in_of(name: &str, app: &str) -> bool {
    name == credential_name(app) || name.starts_with(&format!("{}__", credential_name(app)))
}

/// A sign-in waiting for its code.
struct Pending {
    app: String,
    /// Who is signing in: the sign-in becomes theirs alone.
    person: String,
    verifier: String,
    redirect_uri: String,
    client: Client,
    /// The authorization server this sign-in went to, and whether it says
    /// so in its answer (RFC 9207): an answer from another one is refused.
    issuer: String,
    iss_required: bool,
    at: Instant,
}

/// Where tokens come from, and who Magnus is there. Kept with the tokens.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Client {
    token_endpoint: String,
    #[serde(default)]
    revocation_endpoint: Option<String>,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    resource: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Tokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    expires_at: Option<i64>,
    /// Seconds an access token lasts, as last said: kept for a refresh
    /// answer that does not say (expires_in is only recommended).
    #[serde(default)]
    lifetime: Option<i64>,
    client: Client,
    /// When the person signed in (Unix seconds).
    #[serde(default)]
    connected_at: Option<i64>,
    /// The account they signed in as, where the app's answer names it.
    #[serde(default)]
    account: Option<String>,
}

/// The account a sign-in's answer names, from its ID token's claims, for
/// the person to see which account they connected (not for any decision:
/// the token came straight from the app's token endpoint).
fn account_of(answer: &Value) -> Option<String> {
    use base64::Engine;
    let claims = answer["id_token"].as_str()?.split('.').nth(1)?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(claims.trim_end_matches('=')).ok()?;
    let c: Value = serde_json::from_slice(&raw).ok()?;
    ["email", "preferred_username", "name"].iter().find_map(|k| c[*k].as_str().map(String::from))
}

/// Where one person stands with one of the organisation's apps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    /// Signed in and working.
    Connected,
    /// Signed in once, and the sign-in no longer works (refused at refresh,
    /// or revoked at the app): one button signs in again.
    NeedsAttention,
    /// Not signed in.
    Available,
}

impl Standing {
    pub fn as_str(self) -> &'static str {
        match self {
            Standing::Connected => "connected",
            Standing::NeedsAttention => "needs_attention",
            Standing::Available => "available",
        }
    }
}

/// An app's name as people read it: "google-calendar" is "Google calendar".
pub fn title(name: &str) -> String {
    let words = name.replace(['-', '_'], " ");
    let mut c = words.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// What an app lets Magnus do, in plain words, from the classes of its
/// tools: one sentence for its card, and one line per kind of action.
pub fn allows(c: &Connector) -> (String, Vec<String>) {
    const KINDS: [(&str, &str, &str); 4] = [
        ("read", "read", "Reads"),
        ("reversible_write", "make changes that can be undone", "Changes that can be undone"),
        ("irreversible", "make changes that cannot be undone, when allowed", "Changes that cannot be undone"),
        ("financial", "move money, when allowed", "Moves money"),
    ];
    let mut can = vec![];
    let mut lines = vec![];
    for (risk, verb, head) in KINDS {
        let tools: Vec<String> = c.tools.iter().filter(|(_, t)| t.risk == risk).map(|(n, _)| n.replace(['_', '-'], " ")).collect();
        if !tools.is_empty() {
            can.push(verb);
            lines.push(format!("{head}: {}", tools.join(", ")));
        }
    }
    let name = title(&c.name);
    let sentence = match can.as_slice() {
        [] => format!("Magnus can work in {name} once your organisation sets it up."),
        [one] => format!("Magnus can {one} in {name}."),
        [rest @ .., last] => format!("Magnus can {} and {last} in {name}.", rest.join(", ")),
    };
    (sentence, lines)
}

/// A sign-in left unfinished this long is dropped.
const PENDING_FOR: Duration = Duration::from_secs(15 * 60);

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static P: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// One refresh at a time: apps that rotate refresh tokens accept each once.
fn refreshing() -> &'static tokio::sync::Mutex<()> {
    static R: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).build().expect("http client")
}

pub(crate) fn random() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

pub(crate) fn challenge(verifier: &str) -> String {
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// RFC 8252 loopback redirects only: the code goes to the person's own
/// computer, never to another host.
pub(crate) fn loopback(uri: &str) -> Result<()> {
    let u = url::Url::parse(uri).context("the redirect address is not a URL")?;
    let host = u.host_str().unwrap_or_default();
    if u.scheme() != "http" || !matches!(host, "127.0.0.1" | "[::1]" | "localhost") || u.port().is_none() {
        bail!("the redirect address must be http://127.0.0.1:<port>/... on the person's computer");
    }
    Ok(())
}

/// The well-known address for `suffix` of `base`, path-aware (RFC 8414 §3,
/// RFC 9728 §3): `{origin}/.well-known/{suffix}{path}`, then without the path.
fn well_known(base: &str, suffix: &str) -> Result<Vec<String>> {
    let u = url::Url::parse(base)?;
    let origin = u.origin().ascii_serialization();
    let path = u.path().trim_end_matches('/');
    let mut out = vec![];
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{suffix}{path}"));
    }
    out.push(format!("{origin}/.well-known/{suffix}"));
    Ok(out)
}

async fn first_json(c: &reqwest::Client, urls: &[String]) -> Option<Value> {
    for u in urls {
        if let Ok(r) = c.get(u).header("accept", "application/json").send().await {
            if r.status().is_success() {
                if let Ok(v) = r.json::<Value>().await {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// What an app calls itself in its protected-resource metadata
/// (`resource_name`, RFC 9728), from the address its sign-in answer names
/// or the standard one. None when it does not say. (The name a server gives
/// at initialize is often its software's, "StatelessServer", so it is not
/// used.)
async fn own_name(url: &str) -> Option<String> {
    let r = http()
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "magnus", "version": env!("CARGO_PKG_VERSION") } } }))
        .send()
        .await
        .ok()?;
    let named = r.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).and_then(|h| {
        let i = h.find("resource_metadata=\"")? + "resource_metadata=\"".len();
        Some(h[i..].split('"').next()?.to_string())
    });
    // RFC 9728 §3.1: the metadata's standard address is the resource's
    // path under /.well-known/oauth-protected-resource.
    let standard = url::Url::parse(url).ok().map(|u| format!("{}/.well-known/oauth-protected-resource{}", u.origin().ascii_serialization(), u.path().trim_end_matches('/')));
    let urls: Vec<String> = named.into_iter().chain(standard).collect();
    first_json(&http(), &urls).await?["resource_name"].as_str().map(String::from)
}

/// Whether an app asks for a sign-in: an unauthenticated initialize answered
/// with 401. Its protected-resource metadata, when it says where.
pub async fn asks_sign_in(url: &str) -> Result<Option<Option<String>>> {
    let r = http()
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "magnus", "version": env!("CARGO_PKG_VERSION") } } }))
        .send()
        .await
        .with_context(|| format!("could not reach {url}"))?;
    if r.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(None);
    }
    let meta = r.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).and_then(|h| {
        let i = h.find("resource_metadata=\"")? + "resource_metadata=\"".len();
        Some(h[i..].split('"').next()?.to_string())
    });
    Ok(Some(meta))
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// The app's authorization server metadata: from its protected-resource
/// metadata (RFC 9728), then the server's own (RFC 8414, else OpenID). The
/// metadata must be for this app and that server (RFC 9728 §3.3, RFC 8414
/// §3.3): metadata for another resource or another issuer is refused, so
/// a sign-in never goes where the app's metadata was not about.
async fn discover(app_url: &str, resource_metadata: Option<String>) -> Result<(Value, Vec<String>, String)> {
    let c = http();
    let prm_urls = match resource_metadata {
        Some(u) => vec![u],
        None => well_known(app_url, "oauth-protected-resource")?,
    };
    let prm = first_json(&c, &prm_urls).await.ok_or_else(|| anyhow!("the app does not say where to sign in (no protected-resource metadata)"))?;
    if let Some(r) = prm["resource"].as_str() {
        if !same_url(r, app_url) {
            bail!("the app's sign-in metadata is for {r}, not {app_url}");
        }
    }
    let issuer = prm["authorization_servers"][0].as_str().ok_or_else(|| anyhow!("the app names no authorization server"))?.to_string();
    let scopes: Vec<String> = prm["scopes_supported"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect();
    let mut urls = well_known(&issuer, "oauth-authorization-server")?;
    urls.extend(well_known(&issuer, "openid-configuration")?);
    urls.push(format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/')));
    let asm = first_json(&c, &urls).await.ok_or_else(|| anyhow!("{issuer} publishes no authorization server metadata"))?;
    match asm["issuer"].as_str() {
        Some(i) if same_url(i, &issuer) => {}
        other => bail!("the authorization server metadata found for {issuer} names issuer {}", other.unwrap_or("none")),
    }
    Ok((asm, scopes, issuer))
}

impl Kernel {
    /// The organisation's apps: the config file's, those the owner added,
    /// and the whole catalog, already set up (connectors spec rev 46).
    pub fn connectors(&self) -> Vec<Connector> {
        let mut all = self.cfg.connectors.clone();
        let added: Vec<Connector> = self.read(|s| s.connectors.values().cloned().collect());
        all.extend(added.into_iter().filter(|c| self.cfg.connectors.iter().all(|f| f.name != c.name)));
        for m in crate::catalog::manifests() {
            if all.iter().all(|c| c.name != m.id) {
                all.push(crate::catalog::connector(m));
            }
        }
        all
    }

    /// Whether a catalog connector's provider client is complete on this
    /// appliance: its secret, when it has one, provisioned by Alpha Vector.
    /// Only a deployment that has not yet had it is incomplete.
    pub fn provider_ready(&self, c: &Connector) -> bool {
        match c.registration.as_deref() {
            None => true,
            Some(p) => crate::catalog::client(p).is_some_and(|cl| !cl.needs_secret() || self.read(|s| s.registrations.contains(p))),
        }
    }

    pub fn connector(&self, name: &str) -> Option<Connector> {
        self.connectors().into_iter().find(|c| c.name == name)
    }

    /// The bearer token for an app, for `person`: the organisation's key
    /// for an app that takes one, or for an app people sign in to, the
    /// person's own access token, refreshed when it has expired. None when
    /// there is none (they have not signed in, or the refresh failed: they
    /// sign in again).
    pub async fn connector_secret(&self, c: &Connector, person: &str) -> Result<Option<Secret>> {
        let Some(org) = &c.credential else { return Ok(None) };
        if c.auth.as_deref() != Some(OAUTH) {
            return self.vault.get(org);
        }
        let name = &connection_name(&c.name, person);
        let _g = refreshing().lock().await;
        let Some(raw) = self.vault.get(name)? else { return Ok(None) };
        let mut t: Tokens = serde_json::from_str(raw.expose()).context("stored sign-in")?;
        // A minute early, so a token does not expire on its way.
        let now = chrono::Utc::now().timestamp();
        if t.expires_at.is_some_and(|at| at - 60 <= now) {
            let Some(refresh) = t.refresh_token.clone() else { return Ok(None) };
            let mut form = vec![("grant_type", "refresh_token".to_string()), ("refresh_token", refresh), ("client_id", t.client.client_id.clone()), ("resource", t.client.resource.clone())];
            if let Some(s) = &t.client.client_secret {
                form.push(("client_secret", s.clone()));
            }
            let r = http().post(&t.client.token_endpoint).form(&form).send().await;
            let v: Value = match r {
                Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
                Ok(r) => {
                    tracing::warn!("refresh for {}: HTTP {}", c.name, r.status());
                    return Ok(None);
                }
                Err(e) => {
                    tracing::warn!("refresh for {}: {e}", c.name);
                    return Ok(None);
                }
            };
            let Some(access) = v["access_token"].as_str() else { return Ok(None) };
            t.access_token = access.into();
            if let Some(r) = v["refresh_token"].as_str() {
                t.refresh_token = Some(r.into());
            }
            // A token that had a lifetime keeps one: without it, it would
            // never be refreshed again and sign the person out at expiry.
            t.lifetime = v["expires_in"].as_i64().or(t.lifetime).or(Some(3600));
            t.expires_at = t.lifetime.map(|s| now + s);
            self.vault.put(name, &serde_json::to_string(&t)?)?;
        }
        Ok(Some(Secret::new(t.access_token)))
    }

    /// Start signing in to an app: register if needed, and return the
    /// address to open in the person's browser.
    pub async fn connector_sign_in(&self, c: &Connector, person: &str, redirect_uri: &str) -> Result<(String, String)> {
        loopback(redirect_uri)?;
        let meta = asks_sign_in(&c.url).await?.unwrap_or(None);
        let (asm, scopes, issuer) = discover(&c.url, meta).await?;
        let s = |k: &str| asm[k].as_str().map(String::from);
        let (Some(authorize), Some(token)) = (s("authorization_endpoint"), s("token_endpoint")) else { bail!("the app's authorization server names no authorization or token endpoint") };
        if asm["code_challenge_methods_supported"].as_array().is_some_and(|m| !m.iter().any(|x| x == "S256")) {
            bail!("the app's authorization server does not support PKCE (S256), which MCP requires");
        }
        // A catalog connector signs in through Alpha Vector's own client
        // with its provider, from the catalog; its secret, when it has one,
        // is the one Alpha Vector provisioned to this appliance.
        let shown = c.title.clone().unwrap_or_else(|| c.name.clone());
        let registered: Option<(String, Option<String>)> = match &c.registration {
            Some(provider) => {
                let client = crate::catalog::client(provider).ok_or_else(|| anyhow!("{} cannot be connected on this appliance yet", shown))?;
                let secret = match client.needs_secret() {
                    true => {
                        let raw = self.vault.get(&crate::catalog::registration_name(provider))?.ok_or_else(|| anyhow!("{} cannot be connected on this appliance yet: Alpha Vector has not finished setting it up here", shown))?;
                        let v: Value = serde_json::from_str(raw.expose()).context("stored registration")?;
                        Some(v["client_secret"].as_str().ok_or_else(|| anyhow!("{} cannot be connected on this appliance yet", shown))?.to_string())
                    }
                    false => None,
                };
                Some((client.client_id.clone(), secret))
            }
            None => None,
        };
        let (client_id, client_secret) = match (registered, &c.client_id, s("registration_endpoint")) {
            (Some(r), _, _) => r,
            (None, Some(id), _) => (id.clone(), None),
            (None, None, Some(reg)) => {
                let r = http()
                    .post(&reg)
                    .json(&json!({
                        "client_name": "Magnus", "redirect_uris": [redirect_uri], "grant_types": ["authorization_code", "refresh_token"],
                        "response_types": ["code"], "token_endpoint_auth_method": "none"
                    }))
                    .send()
                    .await?;
                if !r.status().is_success() {
                    bail!("the app's authorization server refused to register Magnus: HTTP {}", r.status());
                }
                let v: Value = r.json().await?;
                (v["client_id"].as_str().ok_or_else(|| anyhow!("registration returned no client id"))?.to_string(), v["client_secret"].as_str().map(String::from))
            }
            (None, None, None) => bail!("the app's authorization server does not register clients; register Magnus there and give its client id"),
        };
        let verifier = random();
        let state = random();
        let mut u = url::Url::parse(&authorize)?;
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("response_type", "code")
                .append_pair("client_id", &client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("code_challenge", &challenge(&verifier))
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &state)
                .append_pair("resource", &c.url);
            // Exactly what the app's manifest asks for, where it names it.
            let scopes = if c.scopes.is_empty() { scopes } else { c.scopes.clone() };
            if !scopes.is_empty() {
                q.append_pair("scope", &scopes.join(" "));
            }
            // A refresh token that lasts (Google gives one only when asked
            // for offline access).
            if authorize.starts_with("https://accounts.google.com/") {
                q.append_pair("access_type", "offline").append_pair("prompt", "consent");
            }
        }
        let client = Client { token_endpoint: token, revocation_endpoint: s("revocation_endpoint"), client_id, client_secret, resource: c.url.clone() };
        let mut p = pending().lock().unwrap();
        p.retain(|_, x| x.at.elapsed() < PENDING_FOR);
        let iss_required = asm["authorization_response_iss_parameter_supported"] == true;
        p.insert(state.clone(), Pending { app: c.name.clone(), person: person.into(), verifier, redirect_uri: redirect_uri.into(), client, issuer, iss_required, at: Instant::now() });
        Ok((u.to_string(), state))
    }

    /// Finish a sign-in with the code the browser brought back.
    /// Only the person who started it can finish it.
    pub async fn connector_sign_in_complete(&self, app: &str, person: &str, state: &str, code: &str, iss: Option<&str>) -> Result<()> {
        let p = pending().lock().unwrap().remove(state).filter(|p| p.app == app && p.person == person && p.at.elapsed() < PENDING_FOR).ok_or_else(|| anyhow!("this sign-in expired or is unknown; start again"))?;
        // The answer must come from the server the sign-in went to (RFC
        // 9207): otherwise the code and verifier would go to this one's token
        // endpoint for another server's code (a mix-up).
        match iss {
            Some(i) if !same_url(i, &p.issuer) => bail!("the sign-in was answered by {i}, not {}; start again", p.issuer),
            None if p.iss_required => bail!("{} says it names itself in its answer, and this answer names no one; start again", p.issuer),
            _ => {}
        }
        let c = &p.client;
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", p.redirect_uri.clone()),
            ("client_id", c.client_id.clone()),
            ("code_verifier", p.verifier.clone()),
            ("resource", c.resource.clone()),
        ];
        if let Some(s) = &c.client_secret {
            form.push(("client_secret", s.clone()));
        }
        let r = http().post(&c.token_endpoint).form(&form).send().await?;
        if !r.status().is_success() {
            bail!("the app's authorization server refused the sign-in: HTTP {}", r.status());
        }
        let v: Value = r.json().await?;
        let access = v["access_token"].as_str().ok_or_else(|| anyhow!("the sign-in returned no access token"))?;
        let t = Tokens {
            access_token: access.into(),
            refresh_token: v["refresh_token"].as_str().map(String::from),
            expires_at: v["expires_in"].as_i64().map(|s| chrono::Utc::now().timestamp() + s),
            lifetime: v["expires_in"].as_i64(),
            client: p.client,
            connected_at: Some(chrono::Utc::now().timestamp()),
            account: account_of(&v),
        };
        self.vault.put(&connection_name(app, person), &serde_json::to_string(&t)?)?;
        self.live_tools.lock().await.remove(&connection_name(app, person));
        Ok(())
    }

    /// Where `person` stands with an app, and when they signed in and as
    /// whom, where known.
    pub async fn standing(&self, c: &Connector, person: &str) -> (Standing, Option<i64>, Option<String>) {
        if c.credential.is_none() || c.auth.as_deref() != Some(OAUTH) {
            return (if self.connected(c, person) { Standing::Connected } else { Standing::Available }, None, None);
        }
        let Ok(Some(raw)) = self.vault.get(&connection_name(&c.name, person)) else { return (Standing::Available, None, None) };
        let t: Option<Tokens> = serde_json::from_str(raw.expose()).ok();
        let (at, account) = t.map(|t| (t.connected_at, t.account)).unwrap_or_default();
        let works = self.connector_secret(c, person).await.ok().flatten().is_some();
        (if works { Standing::Connected } else { Standing::NeedsAttention }, at, account)
    }

    /// What Magnus did through an app for a person, newest first: each
    /// call's tool in plain words, its task, when, and whether it worked.
    pub fn recent_uses(&self, app: &str, person: &str, n: usize) -> Vec<Value> {
        let prefix = format!("{app}__");
        let mut uses: Vec<(chrono::DateTime<chrono::Utc>, Value)> = self.read(|s| {
            s.runs
                .values()
                .filter(|r| r.person == person)
                .flat_map(|r| r.intents.values().filter(|i| i.tool.starts_with(&prefix)).map(move |i| (r, i)))
                .map(|(r, i)| {
                    let done = i.result.as_ref().map(|x| !x.is_error);
                    (i.at, json!({ "action": i.tool[prefix.len()..].replace(['_', '-'], " "), "task": r.task, "at": i.at, "worked": done }))
                })
                .collect()
        });
        uses.sort_by(|a, b| b.0.cmp(&a.0));
        uses.into_iter().take(n).map(|(_, v)| v).collect()
    }

    /// Whether `person` can use an app now: it takes no sign-in, or they
    /// have signed in to it themselves.
    pub fn connected(&self, c: &Connector, person: &str) -> bool {
        match (&c.credential, c.auth.as_deref()) {
            (None, _) => true,
            (Some(_), Some(OAUTH)) => self.vault.names().is_ok_and(|n| n.contains(&connection_name(&c.name, person))),
            (Some(org), _) => self.vault.names().is_ok_and(|n| n.contains(org)),
        }
    }

    /// End one person's sign-in to an app: revoked at the app, then deleted.
    pub async fn disconnect(&self, app: &str, person: &str) -> Result<bool> {
        match self.vault.remove(&connection_name(app, person))? {
            Some(s) => {
                let (done, detail) = revoke(s.expose()).await;
                tracing::info!("{person} disconnected {app}: revoked {done}: {detail}");
                self.live_tools.lock().await.remove(&connection_name(app, person));
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Revoke a signed-in app's tokens at its authorization server (RFC 7009):
/// whether it was done there, and what happened.
pub async fn revoke(stored: &str) -> (bool, String) {
    let Ok(t) = serde_json::from_str::<Tokens>(stored) else { return (false, "the stored sign-in could not be read".into()) };
    let Some(endpoint) = t.client.revocation_endpoint.clone() else {
        return (false, "the app publishes no revocation endpoint; its tokens stay valid until they expire".into());
    };
    let mut done = vec![];
    for (token, hint) in [(t.refresh_token.clone(), "refresh_token"), (Some(t.access_token.clone()), "access_token")] {
        let Some(token) = token else { continue };
        let mut form = vec![("token", token), ("token_type_hint", hint.to_string()), ("client_id", t.client.client_id.clone())];
        if let Some(s) = &t.client.client_secret {
            form.push(("client_secret", s.clone()));
        }
        match http().post(&endpoint).form(&form).send().await {
            Ok(r) if r.status().is_success() => done.push(hint),
            Ok(r) => return (false, format!("the app refused to revoke its {hint}: HTTP {}", r.status())),
            Err(e) => return (false, format!("could not reach the app's revocation endpoint: {e}")),
        }
    }
    (true, format!("revoked by the app ({})", done.join(", ")))
}

/// The owner's starting classification for a tool, from the app's own hints
/// (MCP tool annotations). Hints are the app's claims, so a tool that says
/// nothing is assumed to do the most.
pub fn suggested_risk(annotations: &Value) -> &'static str {
    match (annotations["readOnlyHint"].as_bool(), annotations["destructiveHint"].as_bool()) {
        (Some(true), _) => "read",
        (_, Some(false)) => "reversible_write",
        _ => "irreversible",
    }
}

// The API: the organisation's apps, which the owner adds, classifies and
// removes; and each person's own sign-in to them, which only they make and
// end, and which only their runs use.

use crate::api::{err, internal, owner, who};
use crate::events::Ev;
use axum::{
    extract::{Path, State as AxState},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, post, put},
    Json, Router,
};
use std::sync::Arc;

type K = AxState<Arc<Kernel>>;

pub fn router() -> Router<Arc<Kernel>> {
    Router::new()
        .route("/v1/connectors", axum::routing::get(list).post(add))
        .route("/v1/connectors/{name}", axum::routing::get(detail).delete(remove))
        .route("/v1/connectors/{name}/tools/{tool}", put(classify))
        .route("/v1/connectors/{name}/sign-in", post(sign_in))
        .route("/v1/connectors/{name}/sign-in/complete", post(sign_in_complete))
        .route("/v1/connectors/{name}/connection", delete(disconnect))
        .route("/v1/registrations/{provider}", put(set_registration))
}

/// Alpha Vector provisions its client secret for a provider to this
/// appliance (its deployment calls this; no person types it, and nothing
/// shows it again), until the relay holds it (connectors spec rev 46). The
/// log records only that it was provisioned.
async fn set_registration(AxState(k): K, h: HeaderMap, Path(provider): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match owner(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if crate::catalog::client(&provider).is_none_or(|c| !c.needs_secret()) {
        return err(StatusCode::NOT_FOUND, "no catalog provider takes a secret by that name");
    }
    let secret = b["client_secret"].as_str().map(str::trim).unwrap_or_default();
    if secret.is_empty() {
        return err(StatusCode::BAD_REQUEST, "give the provider's client secret");
    }
    if let Err(e) = k.vault.put(&crate::catalog::registration_name(&provider), &json!({ "client_secret": secret }).to_string()) {
        return internal(e);
    }
    match k.emit(&format!("person:{}", p.id), Ev::RegistrationSet { provider, set: true }).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e),
    }
}

macro_rules! owner {
    ($k:expr, $h:expr) => {
        match owner(&$k, &$h) {
            Ok(p) => p,
            Err(r) => return r,
        }
    };
}

/// The organisation's apps, each with whether the person asking is
/// signed in to it. The owner also gets each app's tools as the app lists
/// them now (through the owner's own sign-in), each with the owner's
/// classification (none: the worker is not offered it) and the
/// classification the app's own hints suggest.
async fn list(AxState(k): K, h: HeaderMap) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let added: Vec<String> = k.read(|s| s.connectors.keys().cloned().collect());
    let mut out = vec![];
    for c in k.connectors() {
        let from_app = added.contains(&c.name) && k.cfg.connectors.iter().all(|f| f.name != c.name);
        let mut card = card(&k, &c, &p.id).await;
        let secret = k.connector_secret(&c, &p.id).await.ok().flatten();
        let signed_in = c.credential.is_none() || secret.is_some();
        if !p.is_owner() {
            out.push(card);
            continue;
        }
        let (tools, problem) = if !signed_in {
            (vec![], Some(if c.auth.as_deref() == Some(OAUTH) { "sign in to this app".to_string() } else { "no credential is stored".to_string() }))
        } else {
            match k.mcp.list_tools(&c.url, secret.as_ref().map(|s| s.expose())).await {
                Ok(ts) => (
                    ts.iter()
                        .map(|t| {
                            json!({
                                "name": t.name, "description": t.description, "annotations": t.annotations,
                                "risk": c.tools.get(&t.name).map(|x| x.risk.clone()), "suggested": suggested_risk(&t.annotations),
                            })
                        })
                        .collect(),
                    None,
                ),
                Err(e) => (vec![], Some(format!("{e:#}"))),
            }
        };
        let admin = json!({
            "url": c.url, "audience": c.audience, "label": c.label, "access_groups": c.access_groups,
            "source": if c.catalog { "catalog" } else if from_app { "app" } else { "config" }, "problem": problem, "tools": tools,
        });
        for (k, v) in admin.as_object().into_iter().flatten() {
            card[k] = v.clone();
        }
        out.push(card);
    }
    Json(json!({ "connectors": out })).into_response()
}

/// A name for an app: what it calls itself, else the first part of its
/// host that says something ("mcp.notion.com" is "notion"), made unique.
fn name_from(url: &str, own: Option<&str>, taken: impl Fn(&str) -> bool) -> String {
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    let from_host = host.split('.').find(|p| !["mcp", "www", "api", "app", "server"].contains(p) && !p.is_empty() && !p.chars().all(|c| c.is_ascii_digit())).unwrap_or("app").to_string();
    let base: String = own
        .map(|n| n.trim().replace([' ', '_'], "-"))
        .filter(|n| !n.is_empty())
        .unwrap_or(from_host)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_lowercase();
    let base = if base.is_empty() { "app".to_string() } else { base };
    let mut name = base.clone();
    let mut n = 2;
    while taken(&name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// One app as a person's card shows it (connectors spec, The connector
/// card and Library): its name, one sentence on what Magnus can do with
/// it, where they stand, and, signed in, as whom, since when and when last
/// used. No protocol words.
async fn card(k: &Kernel, c: &Connector, person: &str) -> Value {
    let (standing, at, account) = k.standing(c, person).await;
    let (sentence, allows) = allows(c);
    let summary = c.summary.clone().unwrap_or(sentence);
    let last = k.recent_uses(&c.name, person, 1).first().map(|u| u["at"].clone());
    json!({
        "name": c.name, "title": c.title.clone().unwrap_or_else(|| title(&c.name)), "summary": summary, "allows": allows, "catalog": c.catalog,
        "state": standing.as_str(), "signed_in": standing == Standing::Connected, "signs_in": c.auth.as_deref() == Some(OAUTH),
        "connected_at": at.and_then(|s| chrono::DateTime::from_timestamp(s, 0)), "account": account, "last_used": last,
        // False only on an appliance Alpha Vector has not finished setting up.
        "ready": k.provider_ready(c),
    })
}

/// One app's card for the person asking, with the last things Magnus did
/// through it for them (from the event log).
async fn detail(AxState(k): K, h: HeaderMap, Path(name): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(c) = k.connector(&name) else { return err(StatusCode::NOT_FOUND, "no such app") };
    let mut v = card(&k, &c, &p.id).await;
    v["recent"] = json!(k.recent_uses(&c.name, &p.id, 10));
    Json(v).into_response()
}

/// Add an app by its MCP address. Whether it needs a sign-in is asked of
/// the app itself.
async fn add(AxState(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = owner!(k, h);
    let s = |f: &str| b[f].as_str().map(str::trim).unwrap_or_default().to_string();
    let (mut name, url, audience, label) = (s("name"), s("url"), s("audience"), s("label"));
    // The address is the one field the owner fills in (connectors spec):
    // the name comes from the app. (The app's form sends what it returns
    // and what reaches it at their most guarded; they stay named here.)
    if name.is_empty() {
        let own = own_name(&url).await;
        name = name_from(&url, own.as_deref(), |n| k.connector(n).is_some() || crate::tools::builtin_apps().contains(&n));
    }
    if name.is_empty() || name.contains("__") || !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
        return err(StatusCode::BAD_REQUEST, "the name must be letters, digits, '-' or '_', without '__'");
    }
    if k.connector(&name).is_some() || crate::tools::builtin_apps().contains(&name.as_str()) {
        return err(StatusCode::CONFLICT, "an app with this name exists");
    }
    if crate::tools::Audience::parse(&audience).is_none() {
        return err(StatusCode::BAD_REQUEST, "audience is private, internal or public: who can see what the worker sends there");
    }
    // Named, never assumed: an empty label would read as "principal", and
    // the app's results would taint nothing.
    if label.is_empty() || crate::tools::Label::parse(&label).is_none() {
        return err(StatusCode::BAD_REQUEST, "label is what the app returns: principal, private, untrusted, or private+untrusted");
    }
    if !url.starts_with("https://") && !(url.starts_with("http://") && k.cfg.web.allow_private_networks) {
        return err(StatusCode::BAD_REQUEST, "the app's address must be https://");
    }
    let oauth = match asks_sign_in(&url).await {
        Ok(a) => a.is_some(),
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("{e:#}")),
    };
    // A credential only where there is one: the app's sign-in, or a key the
    // owner gives. An app that asks for neither is called without one.
    let token = s("token");
    if !oauth && !token.is_empty() {
        if let Err(e) = k.vault.put(&credential_name(&name), &token) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}"));
        }
    }
    let credential = (oauth || !token.is_empty()).then(|| credential_name(&name));
    let c = Connector {
        name: name.clone(),
        transport: "mcp_http".into(),
        url,
        credential,
        audience,
        label,
        access_groups: b["access_groups"].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(String::from)).collect(),
        system_of_record: b["system_of_record"].as_bool().unwrap_or(false),
        idempotency: "none".into(),
        tools: Default::default(),
        auth: oauth.then(|| OAUTH.to_string()),
        client_id: b["client_id"].as_str().map(String::from),
        scopes: vec![],
        registration: None,
        title: None,
        summary: None,
        catalog: false,
    };
    if let Err(e) = k.emit(&format!("person:{}", p.id), Ev::ConnectorAdded { connector: c }).await {
        return internal(e);
    }
    (StatusCode::CREATED, Json(json!({ "name": name, "sign_in": oauth }))).into_response()
}

fn added(k: &Kernel, name: &str) -> Result<Connector, Response> {
    if k.cfg.connectors.iter().any(|c| c.name == name) {
        return Err(err(StatusCode::CONFLICT, "this app is set in the appliance's config file; change it there"));
    }
    k.read(|s| s.connectors.get(name).cloned()).ok_or_else(|| err(StatusCode::NOT_FOUND, "no such app"))
}

/// Remove an app: its sign-in is revoked at the app and its credential deleted.
async fn remove(AxState(k): K, h: HeaderMap, Path(name): Path<String>) -> Response {
    let p = owner!(k, h);
    let c = match added(&k, &name) {
        Ok(c) => c,
        Err(r) => return r,
    };
    // The organisation's key, and every person's sign-in, each revoked.
    let names: Vec<String> = match k.vault.names() {
        Ok(n) => n.into_iter().filter(|n| is_sign_in_of(n, &name) || c.credential.as_ref() == Some(n)).collect(),
        Err(e) => return internal(e),
    };
    for cred in names {
        match k.vault.remove(&cred) {
            Ok(Some(s)) if c.auth.as_deref() == Some(OAUTH) => {
                let (done, detail) = revoke(s.expose()).await;
                tracing::info!("removed {name}: revoked {cred} {done}: {detail}");
            }
            Ok(_) => {}
            Err(e) => return internal(e),
        }
    }
    if let Err(e) = k.emit(&format!("person:{}", p.id), Ev::ConnectorRemoved { name }).await {
        return internal(e);
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Classify one tool: its risk class, or null to stop offering it.
async fn classify(AxState(k): K, h: HeaderMap, Path((name, tool)): Path<(String, String)>, Json(b): Json<Value>) -> Response {
    let p = owner!(k, h);
    if let Err(r) = added(&k, &name) {
        return r;
    }
    let risk = b["risk"].as_str().map(String::from);
    if risk.as_deref().is_some_and(|r| crate::tools::Risk::parse(r).is_none()) {
        return err(StatusCode::BAD_REQUEST, "risk is read, sandbox, reversible_write, irreversible, financial or prohibited");
    }
    if let Err(e) = k.emit(&format!("person:{}", p.id), Ev::ToolClassified { connector: name, tool, risk }).await {
        return internal(e);
    }
    StatusCode::NO_CONTENT.into_response()
}

/// A person signs in to one of the organisation's apps with their own
/// account; the sign-in is theirs alone.
async fn sign_in(AxState(k): K, h: HeaderMap, Path(name): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(c) = k.connector(&name).filter(|c| c.auth.as_deref() == Some(OAUTH)) else { return err(StatusCode::NOT_FOUND, "no app by that name signs in") };
    if k.read(|s| s.kill_switch) {
        return err(StatusCode::CONFLICT, "the kill switch is engaged");
    }
    match k.connector_sign_in(&c, &p.id, b["redirect_uri"].as_str().unwrap_or_default()).await {
        Ok((url, state)) => Json(json!({ "authorize_url": url, "state": state })).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("{e:#}")),
    }
}

async fn sign_in_complete(AxState(k): K, h: HeaderMap, Path(name): Path<String>, Json(b): Json<Value>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let (Some(state), Some(code)) = (b["state"].as_str(), b["code"].as_str()) else { return err(StatusCode::BAD_REQUEST, "state and code are required") };
    match k.connector_sign_in_complete(&name, &p.id, state, code, b["iss"].as_str()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, &format!("{e:#}")),
    }
}

/// A person ends their own sign-in to an app.
async fn disconnect(AxState(k): K, h: HeaderMap, Path(name): Path<String>) -> Response {
    let p = match who(&k, &h) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if k.connector(&name).is_none() {
        return err(StatusCode::NOT_FOUND, "no such app");
    }
    match k.disconnect(&name, &p.id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "you are not signed in to this app"),
        Err(e) => internal(e),
    }
}

#[cfg(test)]
mod naming {
    /// An app added by its address is named by what it calls itself, else
    /// by the part of its host that says something, made unique.
    #[test]
    fn an_app_is_named_from_its_address() {
        let none = |_: &str| false;
        assert_eq!(super::name_from("https://mcp.notion.com/mcp", None, none), "notion");
        assert_eq!(super::name_from("https://mcp.linear.app/mcp", None, none), "linear");
        assert_eq!(super::name_from("https://drivemcp.googleapis.com/mcp/v1", None, none), "drivemcp");
        assert_eq!(super::name_from("https://mcp.example.com/mcp", Some("Team Calendar"), none), "team-calendar");
        assert_eq!(super::name_from("http://127.0.0.1:8080/mcp", None, none), "app");
        assert_eq!(super::name_from("https://mcp.notion.com/mcp", None, |n| n == "notion"), "notion-2");
    }
}
