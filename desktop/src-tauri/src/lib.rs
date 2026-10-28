//! The desktop app: a client of the appliance. It signs in as the person,
//! holds only their session token (in the OS keyring), and makes every
//! request itself, so the page never sees the token. It holds no provider
//! or tool credentials; approvals are made here, outside the agent's context.

mod pin;
mod speech;
mod store;

use base64::Engine;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

type Res<T> = Result<T, String>;

struct App {
    store: store::Store,
    client: Mutex<Option<(String, Option<String>, reqwest::Client)>>,
    live: Mutex<HashMap<String, tokio::task::AbortHandle>>,
    speech: speech::Speech,
}

impl App {
    /// The appliance's address and a client bound to its pinned certificate.
    fn conn(&self) -> Res<(String, reqwest::Client)> {
        let cfg = self.store.config();
        let url = cfg.appliance.ok_or("not connected to an appliance")?;
        let pin = cfg.pins.get(&url).cloned();
        if url.starts_with("https://") && pin.is_none() {
            return Err("the appliance's certificate is not trusted yet".into());
        }
        let mut c = self.client.lock().unwrap();
        if let Some((u, p, client)) = c.as_ref() {
            if *u == url && *p == pin {
                return Ok((url, client.clone()));
            }
        }
        let client = pin::client(pin.as_deref())?;
        *c = Some((url.clone(), pin, client.clone()));
        Ok((url, client))
    }

    fn session(&self) -> Res<(String, reqwest::Client, String)> {
        let (url, c) = self.conn()?;
        let token = self.store.token(&url).ok_or("signed out")?;
        Ok((url, c, token))
    }
}

/// An appliance address: scheme and host (and port), nothing else.
fn normalize(raw: &str) -> Res<String> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let u = url::Url::parse(&with_scheme).map_err(|_| format!("{raw} is not an address"))?;
    if !matches!(u.scheme(), "https" | "http") || u.host_str().is_none() {
        return Err(format!("{raw} is not an http or https address"));
    }
    Ok(u.origin().ascii_serialization())
}

async fn body_of(r: reqwest::Response) -> (u16, Value) {
    let status = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    let body = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(text)) };
    (status, body)
}

fn message(body: &Value, status: u16) -> String {
    body["error"].as_str().or(body["message"].as_str()).or(body.as_str()).map(String::from).unwrap_or_else(|| format!("the appliance answered {status}"))
}

#[tauri::command]
async fn restore(app: State<'_, App>) -> Res<Value> {
    let Ok((url, c, token)) = app.session() else { return Ok(Value::Null) };
    match c.get(format!("{url}/v1/me")).bearer_auth(&token).send().await {
        Ok(r) if r.status().is_success() => Ok(json!({ "person": body_of(r).await.1, "appliance": url })),
        Ok(r) if r.status().as_u16() == 401 => {
            app.store.clear_token(&url);
            Ok(json!({ "note": "Your session ended. Sign in again." }))
        }
        Ok(r) => {
            let (s, b) = body_of(r).await;
            Ok(json!({ "note": message(&b, s) }))
        }
        Err(e) => Ok(json!({ "note": format!("Cannot reach {url}: {}", pin::describe(&e)) })),
    }
}

#[tauri::command]
async fn connect(app: State<'_, App>, url: String) -> Res<Value> {
    let url = normalize(&url)?;
    if url.starts_with("http://") {
        let c = pin::client(None)?;
        c.get(format!("{url}/v1/health")).send().await.map_err(|e| format!("Cannot reach {url}: {}", pin::describe(&e)))?;
        app.store.set_appliance(&url)?;
        return Ok(json!({ "status": "trusted", "url": url }));
    }
    let fp = pin::look(&url).await.map_err(|e| format!("Cannot reach {url}: {e}"))?;
    let status = match app.store.config().pins.get(&url) {
        Some(p) if *p == fp => "trusted",
        Some(_) => "changed",
        None => "new",
    };
    if status == "trusted" {
        app.store.set_appliance(&url)?;
    }
    Ok(json!({ "status": status, "fingerprint": fp, "url": url }))
}

#[tauri::command]
async fn trust(app: State<'_, App>, url: String, fingerprint: String) -> Res<()> {
    let url = normalize(&url)?;
    // Trust only what the appliance presents now.
    let now = pin::look(&url).await?;
    if now != fingerprint {
        return Err("The certificate changed while you were checking it. Start again.".into());
    }
    app.store.set_pin(&url, &fingerprint)
}

#[tauri::command]
async fn sign_in(app: State<'_, App>, person: String, password: String) -> Res<Value> {
    let (url, c) = app.conn()?;
    let r = c
        .post(format!("{url}/v1/sign-in"))
        .json(&json!({ "person": person, "password": password, "client": "app" }))
        .send()
        .await
        .map_err(|e| format!("Cannot reach {url}: {}", pin::describe(&e)))?;
    let (status, body) = body_of(r).await;
    if status >= 400 {
        return Err(message(&body, status));
    }
    let token = body["token"].as_str().ok_or("the appliance sent no session")?;
    app.store.set_token(&url, token)?;
    Ok(body["person"].clone())
}

/// How this appliance signs people in: "oidc" (with the provider's name) or "password".
#[tauri::command]
async fn sign_in_method(app: State<'_, App>) -> Res<Value> {
    let (url, c) = app.conn()?;
    let r = c.get(format!("{url}/v1/sign-in")).send().await.map_err(|e| format!("Cannot reach {url}: {}", pin::describe(&e)))?;
    let (status, body) = body_of(r).await;
    if status >= 400 {
        return Err(message(&body, status));
    }
    Ok(body)
}

/// Start a sign-in through the appliance's identity provider. The page gets
/// the code and the provider's page to show; the appliance holds the rest.
#[tauri::command]
async fn device_start(app: State<'_, App>) -> Res<Value> {
    let (url, c) = app.conn()?;
    let r = c.post(format!("{url}/v1/sign-in/device")).json(&json!({ "client": "app" })).send().await.map_err(|e| format!("Cannot reach {url}: {}", pin::describe(&e)))?;
    let (status, body) = body_of(r).await;
    if status >= 400 {
        return Err(message(&body, status));
    }
    Ok(body)
}

/// Ask whether the person approved. On approval the session goes to the
/// keyring and the page gets only who signed in.
#[tauri::command]
async fn device_poll(app: State<'_, App>, flow: String) -> Res<Value> {
    if flow.is_empty() || !flow.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("not a sign-in".into());
    }
    let (url, c) = app.conn()?;
    let r = c.post(format!("{url}/v1/sign-in/device/{flow}")).send().await.map_err(|e| format!("Cannot reach {url}: {}", pin::describe(&e)))?;
    let (status, body) = body_of(r).await;
    if status == 202 {
        return Ok(json!({ "pending": true }));
    }
    if status >= 400 {
        return Err(message(&body, status));
    }
    let token = body["token"].as_str().ok_or("the appliance sent no session")?;
    app.store.set_token(&url, token)?;
    Ok(json!({ "person": body["person"] }))
}

#[tauri::command]
async fn sign_out(app: State<'_, App>) -> Res<()> {
    if let Ok((url, c, token)) = app.session() {
        let _ = c.post(format!("{url}/v1/sign-out")).bearer_auth(&token).send().await;
        app.store.clear_token(&url);
    }
    Ok(())
}

/// Only the appliance's API, and never the calls that hand out a session.
fn allowed(path: &str) -> Res<()> {
    if !path.starts_with("/v1/") || path.contains("..") || path.contains('#') || path.starts_with("/v1/sign-in") || path.starts_with("/v1/sign-out") {
        return Err(format!("{path} is not an appliance call this app makes"));
    }
    Ok(())
}

#[tauri::command]
async fn api(app: State<'_, App>, method: String, path: String, body: Option<Value>) -> Res<Value> {
    allowed(&path)?;
    let (url, c, token) = app.session()?;
    let m = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        other => return Err(format!("{other} is not a method this app uses")),
    };
    let mut req = c.request(m, format!("{url}{path}")).bearer_auth(&token);
    if let Some(b) = body.filter(|b| !b.is_null()) {
        req = req.json(&b);
    }
    let r = req.send().await.map_err(|e| format!("Cannot reach {url}: {}", pin::describe(&e)))?;
    let (status, body) = body_of(r).await;
    Ok(json!({ "status": status, "body": body }))
}

fn output_path(path: &str) -> Res<()> {
    allowed(path)?;
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if !(parts.len() == 5 && parts[1] == "tasks" && parts[3] == "outputs") {
        return Err("not a delivered file".into());
    }
    Ok(())
}

async fn bytes(app: &App, path: &str) -> Res<Vec<u8>> {
    output_path(path)?;
    let (url, c, token) = app.session()?;
    let r = c.get(format!("{url}{path}")).bearer_auth(&token).send().await.map_err(|e| pin::describe(&e))?;
    if !r.status().is_success() {
        let (s, b) = body_of(r).await;
        return Err(message(&b, s));
    }
    r.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn fetch_bytes(app: State<'_, App>, path: String) -> Res<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes(&app, &path).await?))
}

/// Save a delivered file in the person's Downloads folder.
#[tauri::command]
async fn download(app: State<'_, App>, path: String, name: String) -> Res<String> {
    let data = bytes(&app, &path).await?;
    let safe: String = std::path::Path::new(&name)
        .file_name()
        .map(|n| n.to_string_lossy().chars().map(|c| if c.is_alphanumeric() || " ._-()".contains(c) { c } else { '_' }).collect())
        .filter(|n: &String| !n.trim_matches('.').is_empty())
        .unwrap_or_else(|| "file".into());
    let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("no Downloads folder")?;
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (safe.clone(), String::new()),
    };
    let mut target = dir.join(&safe);
    let mut n = 1;
    while target.exists() {
        target = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&target, data).map_err(|e| format!("save {}: {e}", target.display()))?;
    Ok(target.display().to_string())
}

/// Relay a task's live text to the page as it streams.
#[tauri::command]
async fn live_start(handle: AppHandle, app: State<'_, App>, task: String) -> Res<()> {
    if task.is_empty() || !task.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("not a task id".into());
    }
    let (url, c, token) = app.session()?;
    let t = task.clone();
    let job = tokio::spawn(async move {
        let Ok(r) = c.get(format!("{url}/v1/tasks/{t}/live")).bearer_auth(&token).send().await else { return };
        let mut stream = r.bytes_stream();
        let mut buf = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buf.find("\n\n") {
                let event: String = buf.drain(..end + 2).collect();
                for line in event.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if let Ok(v) = serde_json::from_str::<Value>(data) {
                            let _ = handle.emit("live", json!({ "task": t, "text": v["text"] }));
                        }
                    }
                }
            }
        }
    });
    if let Some(old) = app.live.lock().unwrap().insert(task, job.abort_handle()) {
        old.abort();
    }
    Ok(())
}

#[tauri::command]
fn live_stop(app: State<'_, App>, task: String) {
    if let Some(j) = app.live.lock().unwrap().remove(&task) {
        j.abort();
    }
}

#[tauri::command]
fn notify(handle: AppHandle, title: String, body: String) -> Res<()> {
    handle.notification().builder().title(title).body(body).show().map_err(|e| e.to_string())
}

#[tauri::command]
async fn speak(app: State<'_, App>, text: String) -> Res<()> {
    app.speech.speak(&text).await
}

#[tauri::command]
async fn stop_speaking(app: State<'_, App>) -> Res<()> {
    app.speech.stop().await;
    Ok(())
}

/// Open a provider's sign-in page in the person's browser.
#[tauri::command]
fn open_url(handle: AppHandle, url: String) -> Res<()> {
    let u = url::Url::parse(&url).map_err(|_| "not a link")?;
    if u.scheme() != "https" && u.scheme() != "http" {
        return Err("only web links open from here".into());
    }
    handle.opener().open_url(u.as_str(), None::<&str>).map_err(|e| e.to_string())
}

pub fn run() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(App { store: store::Store::open(), client: Mutex::new(None), live: Mutex::new(HashMap::new()), speech: speech::Speech::default() })
        .invoke_handler(tauri::generate_handler![restore, connect, trust, sign_in_method, sign_in, device_start, device_poll, sign_out, api, fetch_bytes, download, live_start, live_stop, notify, speak, stop_speaking, open_url])
        .run(tauri::generate_context!())
        .expect("start the Magnus app");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_cannot_reach_beyond_the_api_or_take_a_session() {
        for bad in ["/v1/sign-in", "/v1/sign-in/device", "/v1/sign-in/device/signin_1", "/v1/sign-out", "/v1/../etc", "https://evil.example/v1/x", "/other", "/v1/x#y"] {
            assert!(allowed(bad).is_err(), "{bad} was allowed");
        }
        assert!(allowed("/v1/inbox").is_ok());
        assert!(output_path("/v1/tasks/t1/outputs/o1").is_ok());
        assert!(output_path("/v1/tasks/t1").is_err());
    }

    #[test]
    fn an_address_is_reduced_to_its_origin() {
        assert_eq!(normalize("magnus.local:7443").unwrap(), "https://magnus.local:7443");
        assert_eq!(normalize("https://10.0.0.5:7443/some/path").unwrap(), "https://10.0.0.5:7443");
        assert_eq!(normalize("http://127.0.0.1:7700/").unwrap(), "http://127.0.0.1:7700");
        assert!(normalize("file:///etc/passwd").is_err());
    }
}
