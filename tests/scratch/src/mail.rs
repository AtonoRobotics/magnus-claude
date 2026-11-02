//! An SMTP server for tests: the identity provider sends its real email to
//! it (plain SMTP, no TLS, no sign-in), and the test reads what arrived.
//! It listens where the test Keycloak, in its container, reaches this
//! machine: the gateway of the container's network (`docker inspect`), or
//! this machine's address on its network when Keycloak runs elsewhere.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Clone, Debug)]
pub struct Mail {
    pub to: Vec<String>,
    pub data: String,
}

pub struct Inbox {
    pub addr: SocketAddr,
    mail: Arc<Mutex<Vec<Mail>>>,
    connections: Arc<std::sync::atomic::AtomicUsize>,
}

/// This machine as the test Keycloak's container reaches it: its network's
/// gateway.
fn seen_from_keycloak() -> Option<std::net::IpAddr> {
    let out = std::process::Command::new("docker").args(["inspect", "magnus-claude-kc", "--format", "{{range .NetworkSettings.Networks}}{{.Gateway}} {{end}}"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().find_map(|g| g.parse().ok())
}

impl Inbox {
    pub async fn start() -> Inbox {
        let ip = seen_from_keycloak().unwrap_or_else(crate::admin::lan_ip);
        let l = tokio::net::TcpListener::bind((ip, 0)).await.unwrap();
        let addr = l.local_addr().unwrap();
        let mail: Arc<Mutex<Vec<Mail>>> = Default::default();
        let connections: Arc<std::sync::atomic::AtomicUsize> = Default::default();
        let (m, n) = (mail.clone(), connections.clone());
        tokio::spawn(async move {
            while let Ok((s, _)) = l.accept().await {
                n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let m = m.clone();
                tokio::spawn(async move {
                    let _ = session(s, m).await;
                });
            }
        });
        Inbox { addr, mail, connections }
    }

    /// How many connections the server has taken: none means the sender
    /// never reached it.
    pub fn connections(&self) -> usize {
        self.connections.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The first mail to `to`, waiting up to a minute for it.
    pub async fn wait_for(&self, to: &str) -> Mail {
        for _ in 0..600 {
            if let Some(m) = self.mail.lock().unwrap().iter().find(|m| m.to.iter().any(|t| t.eq_ignore_ascii_case(to))) {
                return m.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("no mail came for {to}; mail that came: {:?}", self.mail.lock().unwrap().iter().map(|m| &m.to).collect::<Vec<_>>());
    }

    pub fn count_for(&self, to: &str) -> usize {
        self.mail.lock().unwrap().iter().filter(|m| m.to.iter().any(|t| t.eq_ignore_ascii_case(to))).count()
    }
}

async fn session(s: tokio::net::TcpStream, mail: Arc<Mutex<Vec<Mail>>>) -> std::io::Result<()> {
    let (r, mut w) = s.into_split();
    let mut r = BufReader::new(r);
    w.write_all(b"220 inbox.test ESMTP\r\n").await?;
    let mut to = vec![];
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let cmd = line.trim_end().to_ascii_uppercase();
        if cmd.starts_with("EHLO") {
            w.write_all(b"250-inbox.test\r\n250 8BITMIME\r\n").await?;
        } else if cmd.starts_with("RCPT TO:") {
            to.push(line.trim_end()[8..].trim().trim_matches(['<', '>']).to_string());
            w.write_all(b"250 OK\r\n").await?;
        } else if cmd == "DATA" {
            w.write_all(b"354 go on\r\n").await?;
            let mut data = String::new();
            loop {
                line.clear();
                if r.read_line(&mut line).await? == 0 {
                    return Ok(());
                }
                if line == ".\r\n" || line == ".\n" {
                    break;
                }
                data.push_str(line.strip_prefix('.').filter(|_| line.starts_with("..")).unwrap_or(&line));
            }
            mail.lock().unwrap().push(Mail { to: std::mem::take(&mut to), data });
            w.write_all(b"250 OK\r\n").await?;
        } else if cmd == "QUIT" {
            w.write_all(b"221 bye\r\n").await?;
            return Ok(());
        } else {
            // HELO, MAIL FROM, RSET, NOOP
            w.write_all(b"250 OK\r\n").await?;
        }
    }
}

/// The text of a mail with its transfer encodings undone: quoted-printable
/// soft breaks and escapes, and base64 parts.
pub fn text_of(data: &str) -> String {
    use base64::Engine;
    let mut out = String::new();
    // Quoted-printable: `=\r\n` joins lines, `=XX` is a byte.
    let joined = data.replace("=\r\n", "").replace("=\n", "");
    let b = joined.as_bytes();
    let mut bytes = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' && i + 2 < b.len() {
            if let Ok(x) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                bytes.push(x);
                i += 3;
                continue;
            }
        }
        bytes.push(b[i]);
        i += 1;
    }
    out.push_str(&String::from_utf8_lossy(&bytes));
    // Base64 parts: each run of base64 lines after a blank line.
    for block in data.split("\r\n\r\n").chain(data.split("\n\n")) {
        let compact: String = block.lines().map(str::trim).collect();
        if compact.len() >= 40 && compact.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=') {
            if let Ok(d) = base64::engine::general_purpose::STANDARD.decode(compact) {
                out.push('\n');
                out.push_str(&String::from_utf8_lossy(&d));
            }
        }
    }
    out
}

/// The first link in a mail whose address contains `part`.
pub fn link_in(data: &str, part: &str) -> Option<String> {
    let t = text_of(data);
    t.split(|c: char| c.is_whitespace() || c == '"' || c == '<' || c == '>' || c == '\'')
        .filter(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| w.replace("&amp;", "&"))
        .find(|w| w.contains(part))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_link_is_found_through_quoted_printable_and_base64() {
        let qp = "Content-Transfer-Encoding: quoted-printable\r\n\r\nClick <a href=3D\"http://id.test/realms/r/login-actions/action-token?key=3Dab=\r\ncd&amp;client_id=3Dx\">here</a>\r\n";
        assert_eq!(super::link_in(qp, "action-token").as_deref(), Some("http://id.test/realms/r/login-actions/action-token?key=abcd&client_id=x"));
        use base64::Engine;
        let body = base64::engine::general_purpose::STANDARD.encode("Set it here: http://id.test/realms/r/login-actions/action-token?key=zz9 thanks");
        let b64 = format!("Content-Transfer-Encoding: base64\r\n\r\n{}\r\n{}\r\n", &body[..40], &body[40..]);
        assert_eq!(super::link_in(&b64, "action-token").as_deref(), Some("http://id.test/realms/r/login-actions/action-token?key=zz9"));
    }
}
