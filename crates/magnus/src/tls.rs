//! HTTPS for people's apps on the network.
//!
//! The owner can name a certificate and key. Otherwise the appliance makes a
//! self-signed certificate on first start and keeps it; apps pin its SHA-256
//! fingerprint on their first connection, as SSH pins a host key.

use anyhow::{Context, Result};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

/// The server configuration and the fingerprint of its certificate.
pub fn load(state_dir: &Path, cert: Option<&Path>, key: Option<&Path>) -> Result<(rustls::ServerConfig, String)> {
    let (cert_path, key_path) = match (cert, key) {
        (Some(c), Some(k)) => (c.to_path_buf(), k.to_path_buf()),
        (None, None) => {
            let dir = state_dir.join("tls");
            std::fs::create_dir_all(&dir)?;
            let (c, k) = (dir.join("cert.pem"), dir.join("key.pem"));
            if !c.exists() || !k.exists() {
                let host = hostname();
                let mut names = vec!["localhost".to_string(), "magnus".to_string()];
                if !host.is_empty() {
                    names.push(host);
                }
                let generated = rcgen::generate_simple_self_signed(names).context("make the appliance certificate")?;
                crate::vault::write_private(&k, generated.key_pair.serialize_pem().as_bytes())?;
                std::fs::write(&c, generated.cert.pem())?;
            }
            (c, k)
        }
        _ => anyhow::bail!("set both appliance.tls_cert and appliance.tls_key, or neither"),
    };
    let chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(&cert_path).with_context(|| format!("read {}", cert_path.display()))?.collect::<std::result::Result<_, _>>()?;
    anyhow::ensure!(!chain.is_empty(), "{} holds no certificate", cert_path.display());
    let key = PrivateKeyDer::from_pem_file(&key_path).with_context(|| format!("read {}", key_path.display()))?;
    let fingerprint = fingerprint(&chain[0]);
    let mut cfg = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(chain, key)?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok((cfg, fingerprint))
}

/// SHA-256 of the certificate, as colon-separated hex pairs.
pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(cert.as_ref()).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_default()
}

/// A TCP listener that completes TLS handshakes off the accept path, so one
/// slow client cannot hold up the others.
pub struct Listener {
    rx: tokio::sync::mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
}

impl Listener {
    pub async fn bind(addr: &str, cfg: rustls::ServerConfig) -> Result<Listener> {
        let tcp = TcpListener::bind(addr).await.with_context(|| format!("listen on {addr}"))?;
        let local = tcp.local_addr()?;
        let acceptor = TlsAcceptor::from(Arc::new(cfg));
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                let (sock, peer) = match tcp.accept().await {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!("tls accept: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let (acceptor, tx) = (acceptor.clone(), tx.clone());
                tokio::spawn(async move {
                    match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(sock)).await {
                        Ok(Ok(s)) => {
                            let _ = tx.send((s, peer)).await;
                        }
                        Ok(Err(e)) => tracing::debug!("tls handshake with {peer}: {e}"),
                        Err(_) => tracing::debug!("tls handshake with {peer} timed out"),
                    }
                });
            }
        });
        Ok(Listener { rx, local })
    }
}

impl axum::serve::Listener for Listener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            if let Some(x) = self.rx.recv().await {
                return x;
            }
            // The accept task never ends while the listener lives.
            std::future::pending::<()>().await;
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}
