//! rustls server + client configuration from the self-signed Ed25519 cert.
//!
//! The server presents our cert; the client pins the **expected** fingerprint
//! (the human-confirmed one) and refuses any peer whose cert SPKI doesn't
//! hash to it. This is the LocalSend trust model: no CA, just the
//! out-of-band fingerprint shown in the UI.

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{ClientConfig, ServerConfig};

use crate::identity::DeviceFingerprint;

/// Build a server config that presents our self-signed cert.
///
/// We disable client auth (LocalSend doesn't use mutual TLS at the transport
/// layer; the sender is authenticated by the receiver's PIN/confirm step in
/// the application layer).
pub fn server_config(cert_pem: &str, key_pem: &str) -> std::io::Result<Arc<ServerConfig>> {
    let cert: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no private key in pem"))?;
    let key = match key {
        PrivateKeyDer::Pkcs8(k) => PrivateKeyDer::Pkcs8(k),
        PrivateKeyDer::Sec1(k) => PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(k.secret_sec1_der().to_vec())),
        PrivateKeyDer::Pkcs1(k) => PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(k.secret_pkcs1_der().to_vec())),
        other => other,
    };
    let cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert, key)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Arc::new(cfg))
}

/// A client config that **pins** the expected fingerprint.
///
/// We accept any self-signed cert whose SubjectPublicKeyInfo SHA-256 matches
/// `expected`. All other peers are rejected. Implemented via a custom
/// `ServerCertVerifier` that ignores the chain and just hashes the leaf.
pub fn client_config_pinned(expected: DeviceFingerprint) -> std::io::Result<Arc<ClientConfig>> {
    let verifier = Arc::new(PinningVerifier { expected });
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

/// A client config that skips verification entirely. Used only for the
/// `/info` probe when we don't yet know the peer's fingerprint (we learn it
/// from mDNS txt records, but the human must still confirm before transfer).
pub fn client_config_unverified_for_probe() -> std::io::Result<Arc<ClientConfig>> {
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

#[derive(Debug)]
struct PinningVerifier {
    expected: DeviceFingerprint,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let der = end_entity.as_ref();
        let spki = extract_spki_from_cert_der(der).unwrap_or(der);
        let digest = ketra_core::hash::sha256(spki);
        let got = DeviceFingerprint::from_sha256(&digest);
        if got == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "peer fingerprint mismatch: expected {} got {}",
                self.expected.hex(),
                got.hex()
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
        ]
    }
}

/// A no-op verifier used for the initial `/info` probe (before we've pinned
/// a fingerprint). Never used for actual file transfer.
#[derive(Debug)]
struct NoVerifier;

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
        ]
    }
}

/// Walk a DER cert to find the SubjectPublicKeyInfo. Mirror of
/// `identity::extract_spki_der` but takes DER directly.
fn extract_spki_from_cert_der(der: &[u8]) -> Option<&[u8]> {
    let mut p = DerCursor { buf: der, pos: 0 };
    let cert_body = p.expect_tag(0x30)?;
    let mut p = DerCursor { buf: cert_body, pos: 0 };
    let tbs_body = p.expect_tag(0x30)?;
    let mut p = DerCursor { buf: tbs_body, pos: 0 };
    if p.peek_tag() == Some(0xA0) {
        let _ = p.read_tlv();
    }
    let _serial = p.read_tlv();
    let _sigalg = p.read_tlv();
    let _issuer = p.read_tlv();
    let _validity = p.read_tlv();
    let _subject = p.read_tlv();
    let spki = p.read_tlv();
    Some(spki)
}

struct DerCursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> DerCursor<'a> {
    fn peek_tag(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }
    fn expect_tag(&mut self, expected: u8) -> Option<&'a [u8]> {
        let tag = *self.buf.get(self.pos)?;
        if tag != expected {
            return None;
        }
        Some(self.read_tlv())
    }
    fn read_tlv(&mut self) -> &'a [u8] {
        let start = self.pos;
        let _tag = self.buf[start];
        let len_byte = self.buf[start + 1] as usize;
        let (len, body_off) = if len_byte & 0x80 == 0 {
            (len_byte, start + 2)
        } else {
            let n = len_byte & 0x7F;
            let mut len = 0usize;
            for i in 0..n {
                len = (len << 8) | (self.buf[start + 2 + i] as usize);
            }
            (len, start + 2 + n)
        };
        let slice = &self.buf[body_off..body_off + len];
        self.pos = body_off + len;
        slice
    }
}
