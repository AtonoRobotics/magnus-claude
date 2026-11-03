//! What Google's brokered connectors share (connectors spec, Brokered API
//! connectors): one request to a Google REST API as the person, over the
//! kit's way out (TLS checked for the API's name, through the appliance's
//! egress proxy), and Google's refusals in words a person can act on (from
//! AtonoRobotics/magnus-claude#117's `rest::explain`, for Google's REST
//! APIs). A request that may have changed something and has no clear
//! answer has an unknown outcome.

use magnus_brokered_kit::{https, send, Failure, Response};
use serde_json::Value;

/// One of Google's REST APIs: its host, its path prefix, its name as a
/// person knows it, and the scope its tools work with.
pub struct Api {
    pub host: &'static str,
    pub base: &'static str,
    pub name: &'static str,
    pub scope: &'static str,
}

impl Api {
    /// One request as the person: its JSON answer (`{}` for none), or
    /// what the refusal means.
    pub fn call(&self, method: &str, path: &str, token: &str, body: Option<&Value>) -> Result<Value, Failure> {
        let data = body.map(|b| b.to_string());
        self.json(&self.request(method, &format!("{}{path}", self.base), token, data.as_deref().map(|d| ("application/json", d.as_bytes())), method != "GET")?, method != "GET")
    }

    /// A POST that only asks (Calendar's `freeBusy`): with no answer,
    /// nothing was done.
    pub fn ask(&self, path: &str, token: &str, body: &Value) -> Result<Value, Failure> {
        self.json(&self.request("POST", &format!("{}{path}", self.base), token, Some(("application/json", body.to_string().as_bytes())), false)?, false)
    }

    /// A GET whose answer is not JSON (a file's bytes), as it came.
    pub fn bytes(&self, path: &str, token: &str) -> Result<Vec<u8>, Failure> {
        self.request("GET", &format!("{}{path}", self.base), token, None, false)
    }

    /// A write with a body of its own type, at a path of the host's (Drive's
    /// uploads are under `/upload`, beside its API).
    pub fn upload(&self, method: &str, path: &str, token: &str, kind: &str, data: &[u8]) -> Result<Value, Failure> {
        self.json(&self.request(method, path, token, Some((kind, data)), true)?, true)
    }

    fn request(&self, method: &str, path: &str, token: &str, body: Option<(&str, &[u8])>, wrote: bool) -> Result<Vec<u8>, Failure> {
        let name = self.name;
        let s = https(&format!("{}:443", self.host)).map_err(|e| format!("{e}. Nothing was changed."))?;
        let r = send(s, method, self.host, path, token, body).map_err(|e| if wrote { Failure::Unknown(format!("{e}: whether {name} did this is unknown")) } else { Failure::NotDone(e) })?;
        if r.status >= 400 {
            return Err(self.explain(&r, wrote));
        }
        Ok(r.body)
    }

    /// An answer of success as JSON (`{}` for none). One that is not JSON
    /// says nothing of what was done, so after a write it is an unknown
    /// outcome: tried again, it could do the write twice.
    pub fn json(&self, body: &[u8], wrote: bool) -> Result<Value, Failure> {
        let text = String::from_utf8_lossy(body).to_string();
        if text.trim().is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        serde_json::from_str(&text).map_err(|_| {
            let what = format!("{}'s answer was not JSON: {}", self.name, clip(&text));
            if wrote { Failure::Unknown(format!("{what}; whether it did this is unknown")) } else { Failure::NotDone(what) }
        })
    }

    /// What a refusal means.
    pub fn explain(&self, r: &Response, wrote: bool) -> Failure {
        let name = self.name;
        let body = String::from_utf8_lossy(&r.body).to_string();
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let reason = v["error"]["errors"][0]["reason"].as_str().or_else(|| v["error"]["details"].as_array().and_then(|d| d.iter().find_map(|x| x["reason"].as_str()))).unwrap_or_default();
        let message = v["error"]["message"].as_str().unwrap_or_default().trim();
        let blob = format!("{reason} {message}").to_ascii_lowercase();
        let wait = r.header("retry-after").map(|w| format!(" (in {w} seconds)")).unwrap_or_default();
        if r.status == 401 || reason == "authError" {
            return Failure::NotDone(format!("{name} rejected the sign-in: it has expired or was revoked. Sign in again. Nothing was changed."));
        }
        if r.status == 403 && (blob.contains("accessnotconfigured") || blob.contains("service_disabled") || blob.contains("has not been used in project") || blob.contains("it is disabled")) {
            let why = if reason.is_empty() { "accessNotConfigured" } else { reason };
            return Failure::NotDone(format!("The {name} API is not turned on for this Google Cloud project ({why}). An admin enables it in the project's API library. Nothing was changed."));
        }
        let challenge = r.header("www-authenticate").filter(|h| h.contains("insufficient_scope"));
        if r.status == 403 && (challenge.is_some() || blob.contains("insufficientpermissions") || blob.contains("access_token_scope_insufficient") || blob.contains("insufficient authentication scopes")) {
            let wanted = challenge.and_then(|h| h.split("scope=\"").nth(1)).and_then(|t| t.split('"').next()).map(|t| t.split_whitespace().map(String::from).collect::<Vec<_>>()).filter(|w| !w.is_empty()).unwrap_or_else(|| vec![self.scope.to_string()]);
            return Failure::NeedsScope(format!("This sign-in does not include the access {name} needs for this. Sign in again and allow it. Nothing was changed."), wanted);
        }
        if r.status == 429 || blob.contains("ratelimitexceeded") || blob.contains("rate limit") {
            return Failure::NotDone(format!("{name} is limiting requests. Try again shortly{wait}. Nothing was changed."));
        }
        if r.status == 404 {
            return Failure::NotDone(format!("{name} could not find that. Nothing was changed."));
        }
        if r.status >= 500 && wrote {
            return Failure::Unknown(format!("{name} answered HTTP {} and whether it did this is unknown.", r.status));
        }
        let detail = if reason.is_empty() { clip(message) } else { format!("{reason}: {}", clip(message)) };
        Failure::NotDone(format!("{name} refused this (HTTP {}{}). Nothing was changed.", r.status, if detail.is_empty() { String::new() } else { format!(", {detail}") }))
    }
}

pub fn clip(s: &str) -> String {
    s.chars().take(180).collect()
}

/// A string argument, or empty.
pub fn s(a: &Value, k: &str) -> String {
    a[k].as_str().unwrap_or_default().to_string()
}

/// A list of strings argument.
pub fn list(a: &Value, k: &str) -> Vec<String> {
    a[k].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect()
}

/// A path segment or query value, percent-encoded.
pub fn enc(v: &str) -> String {
    v.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A query string of the pairs that have a value.
pub fn query(pairs: &[(&str, Option<String>)]) -> String {
    let q: Vec<String> = pairs.iter().filter_map(|(k, v)| v.as_ref().filter(|v| !v.is_empty()).map(|v| format!("{k}={}", enc(v)))).collect();
    if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) }
}

/// A whole-number argument, as it goes in a query.
pub fn opt_num(a: &Value, k: &str) -> Option<String> {
    a[k].as_u64().map(|n| n.to_string())
}

#[cfg(test)]
#[path = "../../../tests/brokered-google/google.rs"]
mod tests;
