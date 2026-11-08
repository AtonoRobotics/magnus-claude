//! The console manages people through the identity provider's admin API
//! where the server keeps it: on strix-alpha, Keycloak answers its admin
//! API only on an origin of its own (`hostname-admin`,
//! `https://127.0.0.1:19446`), and its public address refuses `/admin/`.
//! The appliance already reaches it there (`sign_in.oidc.status_admin_url`,
//! DECISIONS.md); the console listed no one, because it asked the public
//! address. Here the test Keycloak stands behind two doors, as there: a
//! public one that refuses `/admin/`, and an admin one that serves only it.
//! The console, told nothing of its own, takes the appliance's admin
//! origin, lists the realm's people, and an admin turns one off and on.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use magnus_invariants::*;
use magnus_scratch::admin::*;
use magnus_scratch::*;
use serde_json::Value;

/// A door in front of the provider at `to`: it passes what `lets` allows
/// and refuses the rest with 403, as a server that keeps its admin API
/// elsewhere does.
async fn door(to: String, lets: fn(&str) -> bool) -> String {
    #[derive(Clone)]
    struct D {
        to: String,
        lets: fn(&str) -> bool,
        http: reqwest::Client,
    }
    async fn pass(State(d): State<D>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
        let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        if !(d.lets)(uri.path()) {
            return (StatusCode::FORBIDDEN, "not on this address").into_response();
        }
        let mut rq = d.http.request(method, format!("{}{path}", d.to)).body(body);
        for (k, v) in headers.iter() {
            if k != "host" && k != "content-length" {
                rq = rq.header(k, v);
            }
        }
        match rq.send().await {
            Ok(r) => {
                let status = r.status();
                let mut out = HeaderMap::new();
                for (k, v) in r.headers() {
                    if k != "transfer-encoding" && k != "content-length" {
                        out.insert(k.clone(), v.clone());
                    }
                }
                (status, out, r.bytes().await.unwrap_or_default()).into_response()
            }
            Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
        }
    }
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let d = D { to, lets, http: reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap() };
    tokio::spawn(async move { axum::serve(l, axum::Router::new().fallback(pass).with_state(d)).await });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_console_manages_people_through_the_providers_own_admin_origin() {
    let kc = Keycloak::start().await;
    kc.add_person("Alice").await;
    kc.set_roles("Alice", &["admin"]).await;
    kc.add_person("Bob").await;
    let m = FakeModel::start(steps(vec![])).await;
    let mut app = Appliance::start(Spec::default(), &m).await;
    // The console's sign-in is its own: Alice is a local console user, as
    // well as a person on the realm the directory still reads (slice B).
    local_user(&app, "Alice", PASSWORD, true);
    kc.configure(&mut app).await;
    let server = kc.issuer.split_once("/realms/").unwrap().0.to_string();
    let public = door(server.clone(), |p| !p.starts_with("/admin")).await;
    let admin_origin = door(server, |p| p.starts_with("/admin")).await;

    // The appliance's own setting names the admin origin and its CA (a CA
    // file is read even where, as here, the origin is plain HTTP).
    let ca = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let ca_path = app.dir.path().join("idp-admin-ca.pem");
    std::fs::write(&ca_path, ca.cert.pem()).unwrap();
    let cfg_path = app.dir.path().join("magnus.toml");
    let mut cfg: toml::Table = std::fs::read_to_string(&cfg_path).unwrap().parse().unwrap();
    let oidc = cfg["sign_in"].as_table_mut().unwrap()["oidc"].as_table_mut().unwrap();
    oidc.insert("status_admin_url".into(), admin_origin.clone().into());
    oidc.insert("status_admin_ca".into(), ca_path.display().to_string().into());
    std::fs::write(&cfg_path, toml::to_string(&cfg).unwrap()).unwrap();

    // The console reaches the provider only through the public door.
    let mine = format!("{}/32", lan_ip());
    let p = prepare(&app, &[&mine], &[("issuer", kc.issuer.clone().into()), ("idp_internal", public.clone().into())]);
    let jwks = p.jwks().await;
    kc.add_admin_client("magnus-admin", &format!("{}/auth/callback", p.url()), &jwks).await;
    kc.add_admin_service("magnus-admin-svc", &jwks).await;
    let url = p.url();
    let c = launch(p).await;
    let b = browser(&c, false);
    let sid = sign_in(&url, c.addr, "Alice", PASSWORD, "admin", false).await.unwrap_or_else(|e| panic!("{e}"));
    let me: Value = with_cookie(b.get(format!("{url}/api/v1/session")), &sid).send().await.unwrap().json().await.unwrap();
    let csrf = me["csrf"].as_str().unwrap().to_string();

    // The public door really refuses the admin API, as strix-alpha's does.
    let refused = reqwest::get(format!("{public}/admin/realms/master")).await.unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    // Everyone in the realm is listed.
    let r = with_cookie(b.get(format!("{url}/api/v1/users")), &sid).send().await.unwrap();
    let status = r.status();
    let users: Value = r.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "the console could not list the realm's people: {users}");
    let names: Vec<String> = users["users"].as_array().unwrap().iter().filter_map(|u| u["email"].as_str().map(String::from)).collect();
    for who in ["alice", "bob"] {
        assert!(names.iter().any(|e| e.starts_with(who)), "{who} is not listed: {users}");
    }

    // And managed there: Bob turned off, then on again.
    let bob = users["users"].as_array().unwrap().iter().find(|u| u["email"].as_str().is_some_and(|e| e.starts_with("bob"))).unwrap()["id"].as_str().unwrap().to_string();
    for (change, status) in [("disable", "disabled"), ("enable", "active")] {
        let r = with_cookie(b.post(format!("{url}/api/v1/users/{bob}/{change}")), &sid).header("x-csrf-token", &csrf).send().await.unwrap();
        assert!(r.status().is_success(), "{change}: {}", r.text().await.unwrap());
        let u: Value = with_cookie(b.get(format!("{url}/api/v1/users/{bob}")), &sid).send().await.unwrap().json().await.unwrap();
        assert_eq!(u["status"].as_str().or(u["user"]["status"].as_str()), Some(status), "{u}");
    }

    // The screen, in every look and at phone width.
    sign_in_and_visit(&url, c.addr, "Alice", PASSWORD, "admin", false, Some("#/users")).await.unwrap_or_else(|e| panic!("{e}"));
}
