//! Errors from a provider's ordinary HTTP API, said in plain words.
//! A raw 401 or 403 is not an answer a person can act on.

use crate::apps::CallError;
use serde_json::Value;

/// What a failed HTTP API call means. `wrote` is true when the call may
/// have changed something, so a 5xx is an unknown outcome rather than a
/// clean refusal.
pub fn explain(status: reqwest::StatusCode, body: &str, what: &str, wrote: bool) -> CallError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let reason = reason_of(&v);
    let message = v["error"]["message"].as_str().or(v["error_description"].as_str()).or(v["error"].as_str()).unwrap_or("").trim();
    let blob = format!("{reason} {message} {body}").to_ascii_lowercase();
    if status == reqwest::StatusCode::UNAUTHORIZED || reason == "authError" || blob.contains("invalid authentication credentials") || blob.contains("invalid_grant") {
        return CallError::Reauth(format!("{what} rejected the sign-in: it has expired or was revoked. Sign in again. Nothing was changed."));
    }
    if status == reqwest::StatusCode::FORBIDDEN || status.as_u16() == 403 {
        if blob.contains("developer preview") || blob.contains("workspace developer preview") {
            return CallError::NotSent(format!("{what} is still on a preview service Magnus does not use. Nothing was changed."));
        }
        if reason.eq_ignore_ascii_case("accessNotConfigured") || reason.eq_ignore_ascii_case("SERVICE_DISABLED") || blob.contains("accessnotconfigured") || blob.contains("has not been used in project") || blob.contains("it is disabled") {
            let why = if reason.is_empty() { "accessNotConfigured" } else { reason };
            return CallError::NotSent(format!("{what} is not turned on for this Cloud project ({why}). An admin enables it in the project's library. Nothing was changed."));
        }
        if reason.eq_ignore_ascii_case("insufficientPermissions")
            || reason.eq_ignore_ascii_case("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
            || reason.eq_ignore_ascii_case("insufficientPermissions")
            || blob.contains("insufficient")
            || blob.contains("access_token_scope_insufficient")
        {
            return CallError::Reauth(format!("This sign-in does not include the access {what} needs. Sign in again and approve it. Nothing was changed."));
        }
        if reason.eq_ignore_ascii_case("rateLimitExceeded") || reason.eq_ignore_ascii_case("userRateLimitExceeded") || blob.contains("rate limit") {
            return CallError::NotSent(format!("{what} is limiting requests. Try again shortly. Nothing was changed."));
        }
        let detail = if reason.is_empty() { clip(message) } else { format!("{reason}: {}", clip(message)) };
        return CallError::NotSent(format!("{what} refused this ({detail}). Nothing was changed."));
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return CallError::NotSent(format!("{what} could not find that. Nothing was changed."));
    }
    if status.is_server_error() && wrote {
        return CallError::Unknown(format!("{what} returned HTTP {status} and the outcome is unknown."));
    }
    CallError::NotSent(format!("{what} returned HTTP {status}: {}. Nothing was changed.", clip(if message.is_empty() { body } else { message })))
}

fn reason_of(v: &Value) -> &str {
    v["error"]["errors"][0]["reason"]
        .as_str()
        .or_else(|| v["error"]["status"].as_str().filter(|s| s.chars().any(|c| c.is_ascii_lowercase()) && *s != "PERMISSION_DENIED"))
        .or_else(|| v["error"]["details"].as_array().and_then(|d| d.iter().find_map(|x| x["reason"].as_str())))
        .unwrap_or("")
}

fn clip(s: &str) -> String {
    s.chars().take(180).collect()
}

/// A refresh the provider refused because the sign-in itself is dead
/// (testing-mode Google refresh tokens last seven days; a revoked grant
/// looks the same). Other client errors are a refusal too. A failure to
/// reach the provider is not.
pub fn refresh_died(status: reqwest::StatusCode) -> bool {
    status.is_client_error()
}
