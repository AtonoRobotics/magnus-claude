//! The return leg of a browser sign-in to an app (OAuth for native apps,
//! RFC 8252): a listener on this machine's loopback address, on a port the
//! system picks, that takes the one request carrying the provider's answer
//! and hands its address to the page. The appliance holds the PKCE verifier,
//! so the code is useless to anyone who sees only this address.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Magnus</title><body style=\"font-family:sans-serif;margin:3em\"><h1>Done</h1><p>You can close this tab and return to Magnus.</p>";

#[derive(Default)]
pub struct Loopback {
    open: Mutex<HashMap<String, TcpListener>>,
}

impl Loopback {
    /// Listen on 127.0.0.1 and say where the provider should send the browser.
    pub fn listen(&self) -> Result<(String, String), String> {
        let l = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("could not listen on this machine: {e}"))?;
        let port = l.local_addr().map_err(|e| e.to_string())?.port();
        let id = {
            use sha2::Digest;
            hex::encode(&sha2::Sha256::digest(format!("{port}{:?}{}", Instant::now(), std::process::id()).as_bytes())[..12])
        };
        let mut open = self.open.lock().unwrap();
        if open.len() >= 4 {
            return Err("too many sign-ins are waiting in this app".into());
        }
        open.insert(id.clone(), l);
        Ok((id, format!("http://127.0.0.1:{port}/")))
    }

    /// Wait for the provider's redirect and return the address it opened.
    pub fn wait(&self, id: &str, within: Duration) -> Result<String, String> {
        let l = self.open.lock().unwrap().remove(id).ok_or("this sign-in is not waiting here")?;
        let port = l.local_addr().map_err(|e| e.to_string())?.port();
        l.set_nonblocking(true).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            let mut s = match l.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(e) => return Err(e.to_string()),
            };
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = vec![0u8; 8192];
            let mut n = 0;
            while n < buf.len() {
                match s.read(&mut buf[n..]) {
                    Ok(0) => break,
                    Ok(k) => {
                        n += k;
                        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head = String::from_utf8_lossy(&buf[..n]);
            let target = head.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|r| r.split(' ').next()).unwrap_or_default().to_string();
            // Only the answer counts: a browser may also ask for a favicon.
            let answer = target.starts_with('/') && (target.contains("code=") || target.contains("error="));
            let reply = if answer { format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}", PAGE.len()) } else { "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string() };
            let _ = s.write_all(reply.as_bytes());
            if answer {
                return Ok(format!("http://127.0.0.1:{port}{target}"));
            }
        }
        Err("the sign-in did not come back in time; start again".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_is_taken_from_the_first_request_with_an_answer() {
        let lb = Loopback::default();
        let (id, uri) = lb.listen().unwrap();
        assert!(uri.starts_with("http://127.0.0.1:") && uri.ends_with('/'));
        let addr = uri.trim_start_matches("http://").trim_end_matches('/').to_string();
        let t = std::thread::spawn(move || {
            for path in ["/favicon.ico", "/?code=abc&state=xyz"] {
                let mut s = std::net::TcpStream::connect(&addr).unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
                let mut out = String::new();
                let _ = s.read_to_string(&mut out);
            }
        });
        let got = lb.wait(&id, Duration::from_secs(10)).unwrap();
        t.join().unwrap();
        assert_eq!(got, format!("{uri}?code=abc&state=xyz"));
        assert!(lb.wait(&id, Duration::from_millis(10)).is_err(), "a sign-in was taken twice");
    }

    #[test]
    fn waiting_ends() {
        let lb = Loopback::default();
        let (id, _) = lb.listen().unwrap();
        assert!(lb.wait(&id, Duration::from_millis(200)).is_err());
    }
}
