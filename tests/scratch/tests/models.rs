//! The model list people choose from. An adapter signed in with OAuth may
//! refresh its token while listing models: the new credential goes to the
//! vault, the list still comes back, and no caller sees the token.

use axum::{extract::State, http::HeaderMap, routing::{get, post}, Json, Router};
use magnus_invariants::appliance::bin;
use magnus_invariants::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const FRESH: &str = "fresh-access-token";

/// A provider that lists two models to the refreshed token only, and a
/// token endpoint that counts refreshes.
async fn oauth_provider() -> (String, Arc<AtomicUsize>) {
    let refreshes = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/token",
            post(|State(n): State<Arc<AtomicUsize>>| async move {
                n.fetch_add(1, Ordering::SeqCst);
                Json(json!({ "access_token": FRESH, "refresh_token": "refresh-2", "expires_in": 3600 }))
            }),
        )
        .route(
            "/models",
            get(|h: HeaderMap| async move {
                if h.get("authorization").and_then(|v| v.to_str().ok()) != Some(&format!("Bearer {FRESH}")) {
                    return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({ "error": "stale token" })));
                }
                (axum::http::StatusCode::OK, Json(json!({ "data": [{ "id": "oa-1" }, { "id": "oa-2" }] })))
            }),
        )
        .with_state(refreshes.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, refreshes)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_refreshed_while_listing_models_goes_to_the_vault_not_the_caller() {
    let (provider, refreshes) = oauth_provider().await;
    let m = FakeModel::start(script(|_| async { Reply::Text("ok".into()) })).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    let alice = app.api("alice");
    let stale = json!({ "access_token": "stale-access-token", "refresh_token": "refresh-1", "expires_at": 0 }).to_string();
    assert!(alice.put_secret("oa_credential", &stale).await.is_success());
    let path = app.dir.path().join("magnus.toml");
    let mut cfg: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    let oa = json!({
        "id": "oa",
        "command": [bin("magnus-adapter-openai")],
        "settings": { "api": "chat", "base_url": provider, "credential": "oa_credential",
            "auth": { "kind": "oauth_device", "token_url": format!("{provider}/token"), "client_id": "magnus-test" } }
    });
    cfg["adapters"].as_array_mut().unwrap().push(toml::Value::try_from(oa).unwrap());
    std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
    app.restart().await;

    let (st, v) = app.api("carol").get("/v1/models").await;
    assert!(st.is_success(), "{v}");
    let listed: Vec<Value> = v["adapters"].as_array().unwrap().iter().filter(|a| a["adapter"] == "oa").map(|a| a["models"].clone()).collect();
    assert_eq!(listed, [json!(["oa-1", "oa-2"])], "the refreshing adapter's models are missing: {v}");
    let (st, v) = alice.get("/v1/adapters/oa/catalog").await;
    assert!(st.is_success(), "{v}");
    assert_eq!(v["event"], "catalog", "{v}");
    assert!(!v.to_string().contains(FRESH) && !v.to_string().contains("refresh-2"), "the catalog handed out the credential: {v}");
    // The refreshed token was stored: the second listing did not refresh again.
    assert_eq!(refreshes.load(Ordering::SeqCst), 1, "the refreshed credential was not kept");
}
