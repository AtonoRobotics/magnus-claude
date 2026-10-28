//! HTTPS to the appliance with its certificate pinned on first use, as SSH
//! pins a host key. The appliance usually has a self-signed certificate and
//! no public name, so there is no authority to ask; the person compares the
//! fingerprint once with the one the owner sees on the appliance.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// SHA-256 of the certificate as colon-separated hex pairs, as the appliance prints it.
pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(cert.as_ref()).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

#[derive(Debug)]
struct Pinned {
    /// None only for the first look, which sends no credentials.
    want: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, Error> {
        let fp = fingerprint(end_entity);
        *self.seen.lock().unwrap() = Some(fp.clone());
        match &self.want {
            Some(w) if *w != fp => Err(Error::General("the appliance's certificate is not the one you trusted".into())),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    // The server must still prove it holds the certificate's key.
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn build(want: Option<String>) -> Result<(reqwest::Client, Arc<Mutex<Option<String>>>), String> {
    let provider = Arc::new(ring::default_provider());
    let seen = Arc::new(Mutex::new(None));
    let verifier = Arc::new(Pinned { want, seen: seen.clone(), provider: provider.clone() });
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    Ok((client, seen))
}

/// A client that talks only to the appliance holding the pinned certificate
/// (or plain HTTP, which the appliance accepts for sign-in only from itself).
pub fn client(pin: Option<&str>) -> Result<reqwest::Client, String> {
    build(pin.map(String::from)).map(|c| c.0)
}

/// Connect once without trusting anything, to learn the certificate's
/// fingerprint. Nothing secret is sent on this connection.
pub async fn look(url: &str) -> Result<String, String> {
    let (c, seen) = build(None)?;
    let r = c.get(format!("{url}/v1/health")).send().await;
    let fp = seen.lock().unwrap().clone();
    match (fp, r) {
        (Some(fp), _) => Ok(fp),
        (None, Err(e)) => Err(describe(&e)),
        (None, Ok(_)) => Err("the appliance did not present a certificate".into()),
    }
}

/// The innermost cause of a request error, which is the one a person can act on.
pub fn describe(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut src: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    msg
}
