//! The brokered runtime's test server. `fetch` asks its provider (a test
//! host named in the call) for a path with the call's token, as a real
//! connector does; `probe` tries every way out a wrong connector might
//! (a host not in its manifest through the proxy, a direct connection,
//! another port on the host); `leak` answers with the token it was given,
//! as a broken one might. Each answer names the microVM it ran in, which
//! the tests use to see one per person, reused, and stopped.

use magnus_brokered_kit::{exchange, serve, tunnel, Failure, Tool, PROXY_PORT};
use serde_json::{json, Value};
use std::sync::OnceLock;

/// This microVM's name, drawn when it boots.
fn instance() -> &'static str {
    static I: OnceLock<String> = OnceLock::new();
    I.get_or_init(|| {
        let mut b = [0u8; 8];
        unsafe { libc::getrandom(b.as_mut_ptr() as *mut libc::c_void, b.len(), 0) };
        b.iter().map(|x| format!("{x:02x}")).collect()
    })
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

fn fetch(a: &Value, token: &str) -> Result<Value, Failure> {
    let host = s(a, "host");
    let r = exchange(tunnel(&host)?, "GET", &host, &s(a, "path"), token, None)?;
    Ok(json!({ "status": r.status, "body": String::from_utf8_lossy(&r.body), "instance": instance() }))
}

fn probe(a: &Value, _: &str) -> Result<Value, Failure> {
    let host = s(a, "host");
    let through_proxy = match tunnel(&host) {
        Ok(_) => "connected".to_string(),
        Err(e) => e,
    };
    let direct = match host.parse::<std::net::SocketAddr>().map_err(|e| e.to_string()).and_then(|at| std::net::TcpStream::connect_timeout(&at, std::time::Duration::from_secs(3)).map_err(|e| e.to_string())) {
        Ok(_) => "connected".to_string(),
        Err(e) => e,
    };
    // Another port on the host: only the proxy's answers.
    let other = unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_port = PROXY_PORT + 1;
        addr.svm_cid = libc::VMADDR_CID_HOST;
        let rc = libc::connect(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_vm>() as u32);
        let e = std::io::Error::last_os_error();
        libc::close(fd);
        if rc == 0 { "connected".to_string() } else { e.to_string() }
    };
    Ok(json!({ "through_proxy": through_proxy, "direct": direct, "other_port": other }))
}

fn leak(_: &Value, token: &str) -> Result<Value, Failure> {
    Ok(json!({ "token": token }))
}

fn main() {
    let host = json!({ "type": "object", "required": ["host"], "additionalProperties": false, "properties": { "host": { "type": "string" }, "path": { "type": "string" } } });
    serve(
        "magnus-brokered-test",
        vec![
            Tool { name: "fetch", description: "Get a path from the provider with the call's access token.", input: host.clone(), output: json!({ "type": "object", "required": ["status", "body", "instance"], "properties": { "status": { "type": "integer" }, "body": { "type": "string" }, "instance": { "type": "string" } } }), run: fetch },
            Tool { name: "probe", description: "Try every way out of the microVM.", input: host, output: json!({ "type": "object", "required": ["through_proxy", "direct", "other_port"] }), run: probe },
            Tool { name: "leak", description: "Answer with the access token this call came with.", input: json!({ "type": "object", "properties": {} }), output: json!({ "type": "object", "required": ["token"] }), run: leak },
        ],
    )
}
