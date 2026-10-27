//! Sign-in and token refresh.
//!
//! `oauth_pkce` (ChatGPT for Codex): settings `auth` = { kind, authorize_url,
//! token_url, client_id, redirect_uri, scopes, extra_params, account_claim
//! (JSON pointer into the id token's claims), account_header }. The person
//! opens the link, signs in, and pastes back the address the browser lands on.
//!
//! `oauth_device` (xAI for Grok): { kind, device_url, token_url, client_id,
//! scopes }. The person opens the link and enters the code; completion polls
//! the token endpoint.

use base64::Engine;
use magnus_adapter_proto::{ErrorClass, Event, Sink};
use rand::RngCore;
use serde_json::{json, Value};
use std::time::Duration;

fn b64url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn random(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b64url(&b)
}

fn claims(id_token: &str) -> Value {
    let part = id_token.split('.').nth(1).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part.trim_end_matches('='))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

fn credential_from(a: &Value, tokens: &Value, previous: Option<&Value>) -> Value {
    let now = chrono::Utc::now().timestamp();
    let expires_at = tokens["expires_in"].as_i64().map(|s| now + s).unwrap_or(now + 3600);
    let id_token = tokens["id_token"].as_str().unwrap_or_default();
    let account = a["account_claim"]
        .as_str()
        .and_then(|p| claims(id_token).pointer(p).and_then(|v| v.as_str().map(String::from)))
        .or_else(|| previous.and_then(|p| p["account_id"].as_str().map(String::from)));
    json!({
        "access_token": tokens["access_token"],
        "refresh_token": tokens["refresh_token"].as_str().map(String::from).or_else(|| previous.and_then(|p| p["refresh_token"].as_str().map(String::from))),
        "expires_at": expires_at,
        "account_id": account,
    })
}

async fn token_request(http: &reqwest::Client, a: &Value, form: &[(&str, &str)]) -> Result<Value, (ErrorClass, String)> {
    let url = a["token_url"].as_str().ok_or((ErrorClass::InvalidRequest, "no token_url in settings".to_string()))?;
    let resp = http.post(url).form(form).send().await.map_err(|e| (ErrorClass::Unavailable, format!("token endpoint unreachable: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if (200..300).contains(&status) && v["access_token"].is_string() {
        return Ok(v);
    }
    let err = v["error"].as_str().unwrap_or_default().to_string();
    let class = match (status, err.as_str()) {
        (_, "authorization_pending") | (_, "slow_down") => ErrorClass::Unavailable,
        (500..=599, _) => ErrorClass::Unavailable,
        _ => ErrorClass::AuthExpired,
    };
    Err((class, format!("token endpoint returned {status}: {}", if err.is_empty() { body.chars().take(200).collect() } else { err })))
}

/// A usable access token for an OAuth credential; refreshes when close to
/// expiry or when forced (after a 401).
pub async fn oauth_bearer(http: &reqwest::Client, settings: &Value, credential: &str, force: bool) -> Result<(String, Option<String>, Option<String>), (ErrorClass, String)> {
    let c: Value = serde_json::from_str(credential).map_err(|_| (ErrorClass::AuthExpired, "the stored sign-in is not readable; sign in again".to_string()))?;
    let a = &settings["auth"];
    let account = c["account_id"].as_str().map(String::from);
    let expires = c["expires_at"].as_i64().unwrap_or(0);
    if !force && expires - chrono::Utc::now().timestamp() > 300 {
        let t = c["access_token"].as_str().unwrap_or_default().to_string();
        return Ok((t, account, None));
    }
    let Some(refresh) = c["refresh_token"].as_str() else {
        return Err((ErrorClass::AuthExpired, "the sign-in expired and cannot be refreshed; sign in again".into()));
    };
    let client = a["client_id"].as_str().unwrap_or_default();
    let mut form = vec![("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", client)];
    if let Some(s) = a["scopes"].as_str() {
        form.push(("scope", s));
    }
    let tokens = token_request(http, a, &form).await.map_err(|(cl, m)| {
        if cl == ErrorClass::Unavailable { (cl, m) } else { (ErrorClass::AuthExpired, format!("refresh failed, sign in again ({m})")) }
    })?;
    let next = credential_from(a, &tokens, Some(&c));
    let token = next["access_token"].as_str().unwrap_or_default().to_string();
    Ok((token, next["account_id"].as_str().map(String::from), Some(next.to_string())))
}

pub async fn begin(http: &reqwest::Client, settings: &Value, sink: &mut Sink) -> anyhow::Result<()> {
    let a = &settings["auth"];
    match a["kind"].as_str().unwrap_or("api_key") {
        "api_key" => {
            sink.send(&Event::AuthStep {
                flow: json!({ "kind": "api_key" }),
                url: settings["key_url"].as_str().map(String::from),
                user_code: None,
                instructions: "Create an API key with the provider and paste it here.".into(),
            })
            .await
        }
        "oauth_pkce" => {
            let verifier = random(48);
            let challenge = {
                use sha2::Digest;
                b64url(&sha2::Sha256::digest(verifier.as_bytes()))
            };
            let state = random(24);
            let mut url = url::Url::parse(a["authorize_url"].as_str().unwrap_or_default())?;
            {
                let mut q = url.query_pairs_mut();
                q.append_pair("response_type", "code")
                    .append_pair("client_id", a["client_id"].as_str().unwrap_or_default())
                    .append_pair("redirect_uri", a["redirect_uri"].as_str().unwrap_or_default())
                    .append_pair("scope", a["scopes"].as_str().unwrap_or_default())
                    .append_pair("code_challenge", &challenge)
                    .append_pair("code_challenge_method", "S256")
                    .append_pair("state", &state);
                for (k, v) in a["extra_params"].as_object().into_iter().flatten() {
                    q.append_pair(k, v.as_str().unwrap_or_default());
                }
            }
            sink.send(&Event::AuthStep {
                flow: json!({ "kind": "oauth_pkce", "verifier": verifier, "state": state }),
                url: Some(url.to_string()),
                user_code: None,
                instructions: "Open the link and sign in. Your browser then lands on a page that may not load; copy that page's full address and paste it here.".into(),
            })
            .await
        }
        "oauth_device" => {
            let url = a["device_url"].as_str().unwrap_or_default();
            let resp = http
                .post(url)
                .form(&[("client_id", a["client_id"].as_str().unwrap_or_default()), ("scope", a["scopes"].as_str().unwrap_or_default())])
                .send()
                .await;
            let v: Value = match resp {
                Ok(r) => r.json().await.unwrap_or(Value::Null),
                Err(e) => return sink.error(ErrorClass::Unavailable, format!("device endpoint unreachable: {e}")).await,
            };
            let Some(device) = v["device_code"].as_str() else {
                return sink.error(ErrorClass::AuthExpired, format!("the provider refused the sign-in request: {v}")).await;
            };
            sink.send(&Event::AuthStep {
                flow: json!({ "kind": "oauth_device", "device_code": device, "interval": v["interval"].as_u64().unwrap_or(5) }),
                url: v["verification_uri_complete"].as_str().or_else(|| v["verification_uri"].as_str()).map(String::from),
                user_code: v["user_code"].as_str().map(String::from),
                instructions: "Open the link, confirm the code shown, and sign in. Then press Done.".into(),
            })
            .await
        }
        k => sink.error(ErrorClass::InvalidRequest, format!("unknown auth kind {k}")).await,
    }
}

pub async fn complete(http: &reqwest::Client, settings: &Value, flow: &Value, input: &Value, sink: &mut Sink) -> anyhow::Result<()> {
    let a = &settings["auth"];
    match flow["kind"].as_str().unwrap_or_default() {
        "api_key" => {
            let key = input["api_key"].as_str().unwrap_or_default().trim().to_string();
            if key.is_empty() {
                return sink.error(ErrorClass::AuthExpired, "no API key given").await;
            }
            // Prove the key works before the kernel stores it.
            let r = http.get(format!("{}{}", crate::base(settings), settings["models_path"].as_str().unwrap_or("/models"))).bearer_auth(&key).send().await;
            match r {
                Ok(r) if r.status().is_success() => sink.send(&Event::AuthDone { credential: key }).await,
                Ok(r) => {
                    let st = r.status().as_u16();
                    let (c, m) = crate::classify(st, &r.text().await.unwrap_or_default());
                    sink.error(c, m).await
                }
                Err(e) => sink.error(ErrorClass::Unavailable, format!("could not reach the provider: {e}")).await,
            }
        }
        "oauth_pkce" => {
            let pasted = input["callback_url"].as_str().or_else(|| input["code"].as_str()).unwrap_or_default().trim().to_string();
            let (code, state) = match url::Url::parse(&pasted) {
                Ok(u) => {
                    let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
                    (q.get("code").cloned().unwrap_or_default(), q.get("state").cloned())
                }
                Err(_) => (pasted.clone(), None),
            };
            if code.is_empty() {
                return sink.error(ErrorClass::AuthExpired, "the pasted address has no sign-in code").await;
            }
            if let Some(s) = state {
                if Some(s.as_str()) != flow["state"].as_str() {
                    return sink.error(ErrorClass::AuthExpired, "the pasted address belongs to a different sign-in; start again").await;
                }
            }
            let form = [
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", a["redirect_uri"].as_str().unwrap_or_default()),
                ("client_id", a["client_id"].as_str().unwrap_or_default()),
                ("code_verifier", flow["verifier"].as_str().unwrap_or_default()),
            ];
            match token_request(http, a, &form).await {
                Ok(t) => sink.send(&Event::AuthDone { credential: credential_from(a, &t, None).to_string() }).await,
                Err((c, m)) => sink.error(c, m).await,
            }
        }
        "oauth_device" => {
            let device = flow["device_code"].as_str().unwrap_or_default();
            let interval = flow["interval"].as_u64().unwrap_or(5).max(1);
            let form = [
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device),
                ("client_id", a["client_id"].as_str().unwrap_or_default()),
            ];
            let start = std::time::Instant::now();
            loop {
                match token_request(http, a, &form).await {
                    Ok(t) => return sink.send(&Event::AuthDone { credential: credential_from(a, &t, None).to_string() }).await,
                    Err((ErrorClass::Unavailable, m)) if start.elapsed() < Duration::from_secs(120) => {
                        let _ = m;
                        tokio::time::sleep(Duration::from_secs(interval)).await;
                    }
                    Err((c, m)) => return sink.error(c, m).await,
                }
            }
        }
        k => sink.error(ErrorClass::InvalidRequest, format!("unknown sign-in flow {k}")).await,
    }
}
