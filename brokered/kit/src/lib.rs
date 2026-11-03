//! What every brokered connector's server shares (connectors spec, Brokered
//! API connectors). The server is the image's `/init`, alone in its own
//! Firecracker microVM, which has no network device and no disk: it answers
//! MCP on vsock, one session per connection, with newline-delimited
//! JSON-RPC (MCP's stdio framing, carried on vsock), and reaches its
//! provider only through the appliance's egress proxy, by HTTPS CONNECT to
//! the host on vsock. The access token comes with each call
//! (`params._meta["io.magnus/access-token"]`) and is used for that call
//! alone: nothing is kept between calls and nothing is written anywhere.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;

pub mod schema;

/// The vsock port the server answers MCP on.
pub const MCP_PORT: u32 = 5252;
/// The port on the host (vsock CID 2) where the appliance's egress proxy
/// answers this microVM, and nothing else does.
pub const PROXY_PORT: u32 = 3128;
/// Where the appliance puts the access token in each call.
pub const TOKEN_META: &str = "io.magnus/access-token";
pub const PROTOCOL: &str = "2025-06-18";
/// The most a proxy's answer to CONNECT may be (its status line and headers).
const HEAD_MAX: usize = 8192;

/// One tool: its schemas and what it does with its input and the call's
/// access token.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input: Value,
    pub output: Value,
    pub run: fn(&Value, &str) -> Result<Value, String>,
}

/// The answer to one message, or none for a notification.
pub fn answer(msg: &Value, server: &str, tools: &[Tool]) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str().unwrap_or_default() {
        "initialize" => json!({ "protocolVersion": PROTOCOL, "capabilities": { "tools": {} }, "serverInfo": { "name": server, "version": env!("CARGO_PKG_VERSION") } }),
        "tools/list" => json!({ "tools": tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.input, "outputSchema": t.output })).collect::<Vec<_>>() }),
        "tools/call" => call(&msg["params"], tools),
        other => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("no method {other}") } })),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn failed(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

fn call(params: &Value, tools: &[Tool]) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let Some(t) = tools.iter().find(|t| t.name == name) else { return failed(format!("no tool {name}")) };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    if let Err(e) = schema::check(&t.input, &args) {
        return failed(format!("{name}: the input does not match its schema: {e}"));
    }
    let Some(token) = params["_meta"][TOKEN_META].as_str().filter(|t| !t.is_empty()) else {
        return failed(format!("{name}: no access token came with this call"));
    };
    match (t.run)(&args, token) {
        Ok(v) => match schema::check(&t.output, &v) {
            Ok(()) => json!({ "content": [{ "type": "text", "text": v.to_string() }], "structuredContent": v, "isError": false }),
            Err(e) => failed(format!("{name}: the answer does not match its schema: {e}")),
        },
        Err(e) => failed(e),
    }
}

/// One session: answer each line until the appliance closes it.
pub fn session(r: impl Read, mut w: impl Write, server: &str, tools: &[Tool]) {
    for line in BufReader::new(r).lines() {
        let Ok(line) = line else { return };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(a) = answer(&msg, server, tools) {
            if writeln!(w, "{a}").and_then(|_| w.flush()).is_err() {
                return;
            }
        }
    }
}

fn vsock(cid: u32, port: u32, listen: bool) -> std::io::Result<UnixStream> {
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_port = port;
        addr.svm_cid = cid;
        let (a, n) = (&addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_vm>() as u32);
        let rc = if listen { if libc::bind(fd, a, n) == 0 { libc::listen(fd, 64) } else { -1 } } else { libc::connect(fd, a, n) };
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        // Any stream socket reads and writes as one.
        Ok(UnixStream::from_raw_fd(fd))
    }
}

/// As the microVM's PID 1: answer MCP on vsock, a thread per session.
pub fn serve(server: &'static str, tools: Vec<Tool>) -> ! {
    let tools: &'static [Tool] = Box::leak(tools.into_boxed_slice());
    let l = vsock(libc::VMADDR_CID_ANY, MCP_PORT, true).unwrap_or_else(|e| panic!("vsock listen: {e}"));
    loop {
        let fd = unsafe { libc::accept4(std::os::fd::AsRawFd::as_raw_fd(&l), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
        if fd < 0 {
            continue;
        }
        let s = unsafe { UnixStream::from_raw_fd(fd) };
        std::thread::spawn(move || {
            let Ok(w) = s.try_clone() else { return };
            session(s, w, server, tools)
        });
    }
}

/// A connection to `target` (`host:port`) through the appliance's egress
/// proxy, which admits only the hosts in the connector's manifest: the
/// proxy's refusal when it does not.
pub fn tunnel(target: &str) -> Result<UnixStream, String> {
    let mut s = vsock(libc::VMADDR_CID_HOST, PROXY_PORT, false).map_err(|e| format!("could not reach the appliance's proxy: {e}"))?;
    write!(s, "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").map_err(|e| e.to_string())?;
    let mut head = vec![];
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut b) {
            Ok(1) if head.len() < HEAD_MAX => head.push(b[0]),
            _ => return Err(format!("the proxy closed the connection to {target}")),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let status = head.lines().next().unwrap_or_default().to_string();
    match status.split_whitespace().nth(1) {
        Some("200") => Ok(s),
        _ => Err(format!("the proxy refused {target}: {status}")),
    }
}

/// An HTTP response: its status, its headers as sent, and its body.
pub struct Response {
    pub status: u16,
    pub head: String,
    pub body: Vec<u8>,
}

impl Response {
    /// A header's value, by its name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
    }
}

/// One HTTP/1.1 request over `s`, which is closed after it.
pub fn exchange(mut s: impl Read + Write, method: &str, host: &str, path: &str, token: &str, body: Option<&Value>) -> Result<Response, String> {
    let data = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n", data.len());
    if body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
    }
    req.push_str("\r\n");
    req.push_str(&data);
    s.write_all(req.as_bytes()).and_then(|_| s.flush()).map_err(|e| format!("sending to {host}: {e}"))?;
    let mut raw = vec![];
    // A TLS peer may end without its closing alert; what came is the answer.
    let _ = s.read_to_end(&mut raw);
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| format!("{host} gave no answer"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let rest = &raw[split + 4..];
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| format!("{host} gave no status"))?;
    let chunked = head.lines().any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
    Ok(Response { status, body: if chunked { dechunk(rest) } else { rest.to_vec() }, head })
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(eol) = b.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..eol]).split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
        if size == 0 || b.len() < eol + 2 + size {
            break;
        }
        out.extend_from_slice(&b[eol + 2..eol + 2 + size]);
        b = &b[(eol + 4 + size).min(b.len())..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(args: &Value, token: &str) -> Result<Value, String> {
        match args["say"].as_str() {
            Some("wrong") => Ok(json!({ "said": 1 })),
            Some(s) => Ok(json!({ "said": format!("{s} with {}", token.len()) })),
            None => Err("nothing to say".into()),
        }
    }

    fn tools() -> Vec<Tool> {
        vec![Tool {
            name: "echo",
            description: "Say it back",
            input: json!({ "type": "object", "properties": { "say": { "type": "string" } }, "additionalProperties": false }),
            output: json!({ "type": "object", "properties": { "said": { "type": "string" } }, "required": ["said"], "additionalProperties": false }),
            run: echo,
        }]
    }

    fn talk(lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let mut out = vec![];
        session(input.as_bytes(), &mut out, "test", &tools());
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn call(id: u32, args: Value, token: Option<&str>) -> Value {
        let mut p = json!({ "name": "echo", "arguments": args });
        if let Some(t) = token {
            p["_meta"] = json!({ TOKEN_META: t });
        }
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": p })
    }

    /// A session answers MCP line by line: it lists each tool with both its
    /// schemas, runs a call only with the call's token and an input its
    /// schema admits, gives back only an answer its output schema admits,
    /// and answers no notification.
    #[test]
    fn a_session_runs_a_call_only_with_its_token_and_schemas() {
        let a = talk(&[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            call(3, json!({ "say": "hi" }), Some("tok")),
            call(4, json!({ "say": "hi" }), None),
            call(5, json!({ "say": 7 }), Some("tok")),
            call(6, json!({ "say": "wrong" }), Some("tok")),
            call(7, json!({}), Some("tok")),
            json!({ "jsonrpc": "2.0", "id": 8, "method": "resources/list" }),
        ]);
        assert_eq!(a.iter().map(|m| m["id"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(a[0]["result"]["protocolVersion"], PROTOCOL);
        let t = &a[1]["result"]["tools"][0];
        assert_eq!((t["name"].as_str(), t["inputSchema"]["type"].as_str(), t["outputSchema"]["required"][0].as_str()), (Some("echo"), Some("object"), Some("said")));
        assert_eq!(a[2]["result"]["structuredContent"], json!({ "said": "hi with 3" }));
        assert_eq!(a[2]["result"]["isError"], false);
        for (m, says) in [(&a[3], "no access token"), (&a[4], "input does not match"), (&a[5], "answer does not match"), (&a[6], "nothing to say")] {
            assert_eq!(m["result"]["isError"], true, "{m}");
            assert!(m["result"]["content"][0]["text"].as_str().unwrap().contains(says), "{m}");
            assert!(m["result"].get("structuredContent").is_none(), "{m}");
        }
        assert_eq!(a[7]["error"]["code"], -32601);
    }

    /// One request carries the token and the body, and the answer's status,
    /// headers and body come back, a chunked body joined.
    #[test]
    fn an_exchange_sends_the_token_and_reads_a_chunked_answer() {
        struct Peer(Vec<u8>, std::io::Cursor<Vec<u8>>);
        impl Read for Peer {
            fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
                self.1.read(b)
            }
        }
        impl Write for Peer {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.write(b)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let answer = b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n".to_vec();
        let mut p = Peer(vec![], std::io::Cursor::new(answer));
        let r = exchange(&mut p, "POST", "api.example", "/v1/x", "tok", Some(&json!({ "b": 2 }))).unwrap();
        assert_eq!((r.status, r.header("retry-after"), r.body.as_slice()), (429, Some("7"), &b"{\"a\":1}"[..]));
        let sent = String::from_utf8(p.0).unwrap();
        assert!(sent.starts_with("POST /v1/x HTTP/1.1\r\nHost: api.example\r\nAuthorization: Bearer tok\r\n"), "{sent}");
        assert!(sent.ends_with("\r\n\r\n{\"b\":2}"), "{sent}");
    }
}
