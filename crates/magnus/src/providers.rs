//! The other catalog connectors, on each provider's generally available
//! HTTP API. None of these addresses is a gated preview MCP server.
//! Sign-in is the provider's own OAuth, with the client the appliance was
//! given. Shopify is not here: its authorize address contains the shop,
//! which a catalog manifest cannot know.

use crate::apps::{AppTool, CallError, CallOutput};
use crate::rest::explain;
use serde_json::{json, Value};

struct Api {
    id: &'static str,
    /// Host of the API the tools call, or a path prefix on a test double.
    host: &'static str,
    prefix: &'static str,
    authorize: &'static str,
    token: &'static str,
    revoke: Option<&'static str>,
    issuer: &'static str,
    /// Slack puts a person's scopes in `user_scope`, not `scope`.
    user_scope: bool,
    /// Microsoft's authorize address contains the tenant.
    microsoft: bool,
}

const APIS: &[Api] = &[
    Api { id: "microsoft", host: "graph.microsoft.com", prefix: "/microsoft/", authorize: "", token: "", revoke: None, issuer: "https://login.microsoftonline.com", user_scope: false, microsoft: true },
    Api { id: "github", host: "api.github.com", prefix: "/github/", authorize: "https://github.com/login/oauth/authorize", token: "https://github.com/login/oauth/access_token", revoke: None, issuer: "https://github.com", user_scope: false, microsoft: false },
    Api { id: "slack", host: "slack.com", prefix: "/slack/", authorize: "https://slack.com/oauth/v2/authorize", token: "https://slack.com/api/oauth.v2.access", revoke: Some("https://slack.com/api/auth.revoke"), issuer: "https://slack.com", user_scope: true, microsoft: false },
    Api { id: "vercel", host: "api.vercel.com", prefix: "/vercel/", authorize: "https://vercel.com/oauth/authorize", token: "https://api.vercel.com/login/oauth/token", revoke: Some("https://api.vercel.com/login/oauth/token/revoke"), issuer: "https://vercel.com", user_scope: false, microsoft: false },
    Api { id: "sentry", host: "sentry.io", prefix: "/sentry/", authorize: "https://sentry.io/oauth/authorize/", token: "https://sentry.io/oauth/token/", revoke: None, issuer: "https://sentry.io", user_scope: false, microsoft: false },
    Api { id: "asana", host: "app.asana.com", prefix: "/asana/", authorize: "https://app.asana.com/-/oauth_authorize", token: "https://app.asana.com/-/oauth_token", revoke: None, issuer: "https://app.asana.com", user_scope: false, microsoft: false },
    Api { id: "gitlab", host: "gitlab.com", prefix: "/gitlab/", authorize: "https://gitlab.com/oauth/authorize", token: "https://gitlab.com/oauth/token", revoke: None, issuer: "https://gitlab.com", user_scope: false, microsoft: false },
    Api { id: "huggingface", host: "huggingface.co", prefix: "/huggingface/", authorize: "https://huggingface.co/oauth/authorize", token: "https://huggingface.co/oauth/token", revoke: None, issuer: "https://huggingface.co", user_scope: false, microsoft: false },
    Api { id: "supabase", host: "api.supabase.com", prefix: "/supabase/", authorize: "https://api.supabase.com/v1/oauth/authorize", token: "https://api.supabase.com/v1/oauth/token", revoke: None, issuer: "https://api.supabase.com", user_scope: false, microsoft: false },
    Api { id: "cloudflare", host: "api.cloudflare.com", prefix: "/cloudflare/", authorize: "https://dash.cloudflare.com/oauth2/auth", token: "https://dash.cloudflare.com/oauth2/token", revoke: Some("https://dash.cloudflare.com/oauth2/revoke"), issuer: "https://dash.cloudflare.com", user_scope: false, microsoft: false },
];

fn local(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

fn find(url: &str) -> Option<&'static Api> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str().unwrap_or("");
    if local(host) {
        let path = u.path().trim_end_matches('/');
        return APIS.iter().find(|a| {
            let prefix = a.prefix.trim_end_matches('/');
            path == prefix || path.starts_with(&format!("{prefix}/"))
        });
    }
    APIS.iter().find(|a| host == a.host)
}

pub fn handles(url: &str) -> bool {
    find(url).is_some()
}

pub fn user_scope(url: &str) -> bool {
    find(url).is_some_and(|a| a.user_scope)
}

/// Authorization-server metadata for a connector on one of these APIs.
/// `tenant` is Microsoft's directory (`organizations`, or a tenant id).
pub fn oauth(url: &str, tenant: &str) -> Option<Value> {
    let a = find(url)?;
    let (authorize, token, revoke, issuer) = if a.microsoft {
        let t = if tenant.is_empty() { "organizations" } else { tenant };
        (
            format!("https://login.microsoftonline.com/{t}/oauth2/v2.0/authorize"),
            format!("https://login.microsoftonline.com/{t}/oauth2/v2.0/token"),
            None,
            format!("https://login.microsoftonline.com/{t}"),
        )
    } else {
        (a.authorize.into(), a.token.into(), a.revoke.map(str::to_string), a.issuer.into())
    };
    let mut v = json!({
        "authorization_endpoint": authorize,
        "token_endpoint": token,
        "issuer": issuer,
        "code_challenge_methods_supported": ["S256"],
    });
    if let Some(r) = revoke {
        v["revocation_endpoint"] = json!(r);
    }
    if a.user_scope {
        v["scope_parameter"] = json!("user_scope");
    }
    Some(v)
}

fn tool(name: &str, description: &str) -> AppTool {
    AppTool {
        name: name.into(),
        description: description.into(),
        input_schema: json!({ "type": "object", "properties": {} }),
        annotations: json!({ "readOnlyHint": true }),
    }
}

pub fn list_tools(url: &str) -> Option<Vec<AppTool>> {
    Some(match find(url)?.id {
        "microsoft" => vec![
            tool("list_mail", "List recent messages in the person's mailbox."),
            tool("list_events", "List events on the person's calendar."),
            tool("list_files", "List files at the top of the person's drive."),
        ],
        "github" => vec![tool("list_repos", "List repositories the person can see."), tool("whoami", "Who the GitHub sign-in is.")],
        "slack" => vec![tool("list_conversations", "List conversations the person is in."), tool("whoami", "Who the Slack sign-in is.")],
        "vercel" => vec![tool("list_projects", "List the person's Vercel projects."), tool("whoami", "Who the Vercel sign-in is.")],
        "sentry" => vec![tool("list_projects", "List Sentry projects the person can see.")],
        "asana" => vec![tool("whoami", "Who the Asana sign-in is.")],
        "gitlab" => vec![tool("list_projects", "List GitLab projects the person is a member of."), tool("whoami", "Who the GitLab sign-in is.")],
        "huggingface" => vec![tool("whoami", "Who the Hugging Face sign-in is.")],
        "supabase" => vec![tool("list_projects", "List the person's Supabase projects.")],
        "cloudflare" => vec![tool("whoami", "Who the Cloudflare sign-in is."), tool("list_zones", "List zones the sign-in can see.")],
        _ => return None,
    })
}

pub async fn call(http: &reqwest::Client, url: &str, bearer: Option<&str>, name: &str, _args: &Value) -> Result<CallOutput, CallError> {
    let Some(api) = find(url) else { return Err(CallError::NotSent("not a known provider API".into())) };
    let Some(bearer) = bearer.filter(|s| !s.is_empty()) else {
        return Err(CallError::Reauth(format!("{} has no working sign-in. Sign in again. Nothing was changed.", api.id)));
    };
    let path = match (api.id, name) {
        ("microsoft", "list_mail") => "/me/messages?$top=15",
        ("microsoft", "list_events") => "/me/calendar/events?$top=15",
        ("microsoft", "list_files") => "/me/drive/root/children?$top=15",
        ("github", "list_repos") => "/user/repos?per_page=20&sort=updated",
        ("github", "whoami") => "/user",
        ("slack", "list_conversations") => "/conversations.list?limit=20",
        ("slack", "whoami") => "/auth.test",
        ("vercel", "list_projects") => "/v9/projects?limit=20",
        ("vercel", "whoami") => "/v2/user",
        ("sentry", "list_projects") => "/projects/",
        ("asana", "whoami") => "/users/me",
        ("gitlab", "list_projects") => "/projects?membership=true&per_page=20",
        ("gitlab", "whoami") => "/user",
        ("huggingface", "whoami") => "/whoami-v2",
        ("supabase", "list_projects") => "/projects",
        ("cloudflare", "whoami") => "/user",
        ("cloudflare", "list_zones") => "/zones?per_page=20",
        _ => return Err(CallError::NotSent(format!("{} has no tool {name}", api.id))),
    };
    let full = format!("{}{path}", url.trim_end_matches('/'));
    let resp = http.get(&full).bearer_auth(bearer).header("accept", "application/json").header("user-agent", "magnus").send().await.map_err(|e| {
        if e.is_connect() { CallError::NotSent(format!("could not reach {}: {e}", api.id)) } else { CallError::NotSent(format!("could not reach {}: {e}", api.id)) }
    })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| CallError::Unknown(format!("response lost: {e}")))?;
    if !status.is_success() {
        return Err(explain(status, &text, api.id, false));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(json!({ "text": text }));
    // Slack answers HTTP 200 with ok:false when the method was refused.
    if api.id == "slack" && v["ok"] == false {
        let err = v["error"].as_str().unwrap_or("refused");
        if err == "invalid_auth" || err == "token_revoked" || err == "not_authed" {
            return Err(CallError::Reauth("Slack rejected the sign-in: it has expired or was revoked. Sign in again. Nothing was changed.".into()));
        }
        return Err(CallError::NotSent(format!("Slack refused this ({err}). Nothing was changed.")));
    }
    let pretty = serde_json::to_string_pretty(&v).unwrap_or(text);
    Ok(CallOutput { text: pretty, structured: Some(v), is_error: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::StatusCode, response::IntoResponse, Router};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Rec(Arc<Mutex<Vec<String>>>);

    async fn up(body: Value) -> (String, Rec) {
        let rec = Rec(Arc::new(Mutex::new(vec![])));
        let app = Router::new()
            .fallback(move |State(rec): State<Rec>, req: axum::extract::Request| {
                let body = body.clone();
                async move {
                    rec.0.lock().unwrap().push(req.uri().path().to_string());
                    let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
                    if auth == "Bearer fail-401" {
                        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "message": "Bad credentials" }))).into_response();
                    }
                    (StatusCode::OK, axum::Json(body)).into_response()
                }
            })
            .with_state(rec.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { let _ = axum::serve(listener, app).await; });
        (format!("http://{addr}"), rec)
    }

    #[tokio::test]
    async fn github_lists_repos_on_its_rest_api_and_a_401_says_to_sign_in_again() {
        let (base, rec) = up(json!({ "login": "ada", "repos": [{ "name": "magnus" }] })).await;
        let url = format!("{base}/github");
        let http = reqwest::Client::new();
        let ok = call(&http, &url, Some("tok"), "list_repos", &json!({})).await.unwrap();
        assert!(ok.text.contains("magnus"), "{}", ok.text);
        assert!(rec.0.lock().unwrap().iter().any(|p| p.contains("/user/repos")), "{:?}", rec.0.lock().unwrap());
        let text = match call(&http, &url, Some("fail-401"), "whoami", &json!({})).await {
            Err(e) => format!("{e:?}"),
            Ok(_) => panic!("expected an error"),
        };
        assert!(text.contains("Sign in again"), "{text}");
        assert!(!text.contains("HTTP 401"), "{text}");
    }

    #[test]
    fn every_provider_api_is_generally_available_and_its_tools_match() {
        for a in APIS {
            assert!(!a.host.contains("mcp") && !a.authorize.contains("mcp") && !a.token.contains("mcp"), "{}", a.id);
            let url = format!("https://{}{}", a.host, if a.id == "microsoft" { "/v1.0" } else { "" });
            assert!(handles(&url), "{url}");
            let tools = list_tools(&url).unwrap();
            assert!(!tools.is_empty(), "{}", a.id);
            let meta = oauth(&url, "organizations").unwrap();
            let auth = meta["authorization_endpoint"].as_str().unwrap();
            assert!(auth.starts_with("https://") && !auth.contains("mcp."), "{auth}");
        }
        assert!(!handles("https://gmailmcp.googleapis.com/mcp/v1"));
        for m in crate::catalog::manifests().iter().filter(|m| m.kind == "rest") {
            assert!(handles(&m.endpoint), "{} -> {}", m.id, m.endpoint);
            let mut named: Vec<_> = m.tools.keys().cloned().collect();
            let mut live: Vec<_> = list_tools(&m.endpoint).unwrap().into_iter().map(|t| t.name).collect();
            named.sort();
            live.sort();
            assert_eq!(named, live, "{}", m.id);
        }
    }
}
