//! The organisation's people at the identity provider, as the console
//! manages them: Keycloak's admin API, reached as the console's service
//! account (`magnus-admin-svc`, admin console spec, Server and API), which
//! proves itself with the console's own key and holds user management in
//! the console's realm only. It is called only after the admin's own
//! session has passed the guard (and a step-up, where one is needed).

use super::auth::{Key, ROLE};
use super::session::Idp;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

pub struct Directory {
    pub client_id: String,
    /// The realm's admin API, at its public address (`Idp::inside` maps it).
    base: String,
    token: tokio::sync::Mutex<Option<(String, std::time::Instant)>>,
}

/// What the provider said when it refused.
#[derive(Debug)]
pub struct Refused {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the identity provider answered {}: {}", self.status, self.message)
    }
}

impl std::error::Error for Refused {}

/// A Keycloak user as the console lists them.
pub fn person(u: &Value, admins: &[String]) -> Value {
    let id = u["id"].as_str().unwrap_or_default();
    let name = [u["firstName"].as_str(), u["lastName"].as_str()].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let invited = u["requiredActions"].as_array().is_some_and(|a| a.iter().any(|x| x == "UPDATE_PASSWORD"));
    let status = if u["enabled"] == false {
        "disabled"
    } else if invited {
        "invited"
    } else {
        "active"
    };
    json!({
        "id": id,
        "name": if name.is_empty() { u["username"].clone() } else { json!(name) },
        "email": u["email"],
        "role": if admins.iter().any(|a| a == id) { "admin" } else { "member" },
        "status": status,
        "created": u["createdTimestamp"].as_i64().and_then(chrono::DateTime::from_timestamp_millis),
    })
}

impl Directory {
    pub fn new(issuer: &str, client_id: String) -> Directory {
        Directory { client_id, base: issuer.trim_end_matches('/').replacen("/realms/", "/admin/realms/", 1), token: Default::default() }
    }

    async fn token(&self, idp: &Idp, key: &Key) -> Result<String> {
        let mut g = self.token.lock().await;
        if let Some((t, until)) = g.as_ref() {
            if *until > std::time::Instant::now() {
                return Ok(t.clone());
            }
        }
        let b = idp.token_as(key, &self.client_id, &[("grant_type", "client_credentials")]).await.with_context(|| format!("sign in as the console's service account {}", self.client_id))?;
        let t = b["access_token"].as_str().ok_or_else(|| anyhow!("the provider sent the service account no token"))?.to_string();
        // Taken again well before it lapses.
        let life = b["expires_in"].as_u64().unwrap_or(60).saturating_sub(30).max(5);
        *g = Some((t.clone(), std::time::Instant::now() + std::time::Duration::from_secs(life)));
        Ok(t)
    }

    /// One admin API call: `path` is under the realm's admin address.
    pub async fn call(&self, idp: &Idp, key: &Key, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = idp.inside(&format!("{}{path}", self.base));
        let mut rq = idp.http.request(method.clone(), &url).bearer_auth(self.token(idp, key).await?);
        if let Some(b) = body {
            rq = rq.json(b);
        }
        let r = rq.send().await.with_context(|| format!("reach the identity provider's admin API at {url}"))?;
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if !status.is_success() {
            let v: Value = serde_json::from_str(&text).unwrap_or_default();
            let message = v["errorMessage"].as_str().or(v["error_description"].as_str()).or(v["error"].as_str()).map(String::from).unwrap_or_else(|| text.chars().take(200).collect());
            return Err(Refused { status: status.as_u16(), message }.into());
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// The ids of everyone holding the realm role `admin` directly and
    /// turned on. Admins by group are not counted, so the last-admin
    /// check can only err towards refusing.
    pub async fn admins(&self, idp: &Idp, key: &Key, with_disabled: bool) -> Result<Vec<String>> {
        let r = self.call(idp, key, reqwest::Method::GET, &format!("/roles/{ROLE}/users?first=0&max=1000&briefRepresentation=false"), None).await;
        let list = match r {
            Ok(v) => v,
            Err(e) if e.downcast_ref::<Refused>().is_some_and(|r| r.status == 404) => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        Ok(list.as_array().into_iter().flatten().filter(|u| with_disabled || u["enabled"] != false).filter_map(|u| u["id"].as_str().map(String::from)).collect())
    }

    pub async fn users(&self, idp: &Idp, key: &Key) -> Result<Vec<Value>> {
        Ok(self.call(idp, key, reqwest::Method::GET, "/users?first=0&max=1000&briefRepresentation=false", None).await?.as_array().cloned().unwrap_or_default())
    }

    pub async fn user(&self, idp: &Idp, key: &Key, id: &str) -> Result<Value> {
        self.call(idp, key, reqwest::Method::GET, &format!("/users/{}", seg(id)), None).await
    }

    /// Give or take the realm role `admin`.
    pub async fn set_admin(&self, idp: &Idp, key: &Key, id: &str, admin: bool) -> Result<()> {
        let role = self.call(idp, key, reqwest::Method::GET, &format!("/roles/{ROLE}"), None).await?;
        let method = if admin { reqwest::Method::POST } else { reqwest::Method::DELETE };
        self.call(idp, key, method, &format!("/users/{}/role-mappings/realm", seg(id)), Some(&json!([role]))).await?;
        Ok(())
    }

    pub async fn set_enabled(&self, idp: &Idp, key: &Key, id: &str, on: bool) -> Result<()> {
        self.call(idp, key, reqwest::Method::PUT, &format!("/users/{}", seg(id)), Some(&json!({ "enabled": on }))).await?;
        Ok(())
    }

    /// End every session the person holds at the provider, so no app signs
    /// them in again without their password.
    pub async fn sign_out(&self, idp: &Idp, key: &Key, id: &str) -> Result<()> {
        self.call(idp, key, reqwest::Method::POST, &format!("/users/{}/logout", seg(id)), None).await?;
        Ok(())
    }

    /// Make the account, turned on and waiting for its password; its id.
    pub async fn create(&self, idp: &Idp, key: &Key, email: &str, name: &str) -> Result<String> {
        let (first, last) = name.split_once(' ').map_or((name, ""), |(f, l)| (f, l));
        let body = json!({ "username": email, "email": email, "firstName": first, "lastName": last, "enabled": true, "requiredActions": ["UPDATE_PASSWORD"] });
        self.call(idp, key, reqwest::Method::POST, "/users", Some(&body)).await?;
        let found = self.call(idp, key, reqwest::Method::GET, &format!("/users?exact=true&email={}", seg(email)), None).await?;
        found.as_array().and_then(|a| a.first()).and_then(|u| u["id"].as_str()).map(String::from).ok_or_else(|| anyhow!("the provider made the account but does not find it"))
    }

    /// Email the person the provider's link to choose their password
    /// (owner's ruling 1: a password, and a code if they use one; no
    /// passkeys). It lasts three days.
    pub async fn send_invite(&self, idp: &Idp, key: &Key, id: &str) -> Result<()> {
        self.call(idp, key, reqwest::Method::PUT, &format!("/users/{}/execute-actions-email?lifespan=259200", seg(id)), Some(&json!(["UPDATE_PASSWORD"]))).await?;
        Ok(())
    }

    pub async fn delete(&self, idp: &Idp, key: &Key, id: &str) -> Result<()> {
        self.call(idp, key, reqwest::Method::DELETE, &format!("/users/{}", seg(id)), None).await?;
        Ok(())
    }
}

/// One path segment, escaped.
fn seg(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_person_is_invited_until_they_choose_a_password_and_disabled_above_all() {
        let admins = vec!["a".to_string()];
        let p = super::person(&json!({ "id": "a", "username": "al@x", "firstName": "Al", "lastName": "Ng", "enabled": true, "requiredActions": [] }), &admins);
        assert_eq!((p["name"].as_str(), p["role"].as_str(), p["status"].as_str()), (Some("Al Ng"), Some("admin"), Some("active")));
        let p = super::person(&json!({ "id": "b", "username": "bo@x", "enabled": true, "requiredActions": ["UPDATE_PASSWORD"] }), &admins);
        assert_eq!((p["name"].as_str(), p["role"].as_str(), p["status"].as_str()), (Some("bo@x"), Some("member"), Some("invited")));
        let p = super::person(&json!({ "id": "b", "username": "bo@x", "enabled": false, "requiredActions": ["UPDATE_PASSWORD"] }), &admins);
        assert_eq!(p["status"], "disabled");
    }

    #[test]
    fn the_admin_api_is_the_realms_own() {
        let d = super::Directory::new("https://id.acme.test/realms/magnus/", "svc".into());
        assert_eq!(d.base, "https://id.acme.test/admin/realms/magnus");
    }
}
