//! TLS 1.3 between paired devices.
//!
//! There are no certificates or CAs. Each side presents its Ed25519 identity
//! key as a raw public key (RFC 7250). The client accepts only the exact key
//! it stored for the device it's dialing; the server accepts only clients
//! in the [`TrustStore`]. TLS 1.3's ephemeral ECDHE gives every session fresh
//! keys with forward secrecy.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::AlwaysResolvesClientRawPublicKeys;
use rustls::crypto::{
    verify_tls13_signature_with_raw_key, CryptoProvider, WebPkiSupportedAlgorithms,
};
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, SubjectPublicKeyInfoDer,
    UnixTime,
};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::AlwaysResolvesServerRawPublicKeys;
use rustls::sign::CertifiedKey;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig,
    SignatureScheme,
};

use crate::{DeviceId, Error, Identity, Result, TrustStore, PUBLIC_KEY_LEN};

/// ALPN protocol id; peers on a different protocol version fail the handshake.
pub const ALPN: &[u8] = b"freelife/1";

/// Placeholder SNI. Peers are identified by key, not name, so it isn't checked.
pub const SERVER_NAME: &str = "peer.freelife.invalid";

/// DER prefix of an Ed25519 SubjectPublicKeyInfo:
/// `SEQUENCE { SEQUENCE { OID 1.3.101.112 }, BIT STRING (32 bytes) }`.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

pub fn ed25519_to_spki(public_key: &[u8]) -> Vec<u8> {
    [&ED25519_SPKI_PREFIX[..], public_key].concat()
}

pub fn spki_to_ed25519(spki: &[u8]) -> Option<&[u8]> {
    spki.strip_prefix(&ED25519_SPKI_PREFIX[..])
        .filter(|k| k.len() == PUBLIC_KEY_LEN)
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn certified_key(identity: &Identity, provider: &CryptoProvider) -> Result<Arc<CertifiedKey>> {
    let der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.pkcs8_der()));
    let key = provider.key_provider.load_private_key(der)?;
    let spki = key
        .public_key()
        .ok_or(Error::Crypto("signing key has no public key"))?;
    Ok(Arc::new(CertifiedKey::new(
        vec![CertificateDer::from(spki.as_ref().to_vec())],
        key,
    )))
}

/// Client config for dialing the device whose identity key is `peer_public_key`.
pub fn client_config(identity: &Identity, peer_public_key: &[u8]) -> Result<Arc<ClientConfig>> {
    let provider = provider();
    let key = certified_key(identity, &provider)?;
    let verifier = PinnedServerVerifier {
        expected_spki: ed25519_to_spki(peer_public_key),
        algorithms: provider.signature_verification_algorithms,
    };
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_cert_resolver(Arc::new(AlwaysResolvesClientRawPublicKeys::new(key)));
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(Arc::new(config))
}

/// Server config that accepts any client in `trust`.
pub fn server_config(identity: &Identity, trust: Arc<TrustStore>) -> Result<Arc<ServerConfig>> {
    let provider = provider();
    let key = certified_key(identity, &provider)?;
    let verifier = TrustedClientVerifier {
        trust,
        algorithms: provider.signature_verification_algorithms,
    };
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(verifier))
        .with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(key)));
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(Arc::new(config))
}

pub fn server_name() -> ServerName<'static> {
    ServerName::try_from(SERVER_NAME).expect("valid DNS name")
}

/// The device id behind a peer's presented raw public key.
pub fn peer_device_id(certs: Option<&[CertificateDer<'_>]>) -> Option<DeviceId> {
    let spki = certs?.first()?;
    spki_to_ed25519(spki.as_ref()).map(DeviceId::from_public_key)
}

fn verify_signature(
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
    algorithms: &WebPkiSupportedAlgorithms,
) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
    verify_tls13_signature_with_raw_key(
        message,
        &SubjectPublicKeyInfoDer::from(cert.as_ref()),
        dss,
        algorithms,
    )
}

fn tls12_refused() -> rustls::Error {
    rustls::Error::General("TLS 1.2 is not supported".into())
}

#[derive(Debug)]
struct PinnedServerVerifier {
    expected_spki: Vec<u8>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if intermediates.is_empty() && end_entity.as_ref() == self.expected_spki.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(tls12_refused())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct TrustedClientVerifier {
    trust: Arc<TrustStore>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for TrustedClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        let trusted = intermediates.is_empty()
            && spki_to_ed25519(end_entity.as_ref())
                .and_then(|key| self.trust.find_by_public_key(key))
                .is_some();
        if trusted {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(tls12_refused())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    use super::*;
    use crate::OsType;

    #[test]
    fn spki_round_trip() {
        let id = Identity::generate().unwrap();
        let spki = ed25519_to_spki(id.public_key());
        assert_eq!(spki_to_ed25519(&spki), Some(id.public_key()));
        assert_eq!(spki_to_ed25519(&spki[..spki.len() - 1]), None);
        assert_eq!(spki_to_ed25519(b"not a key"), None);
    }

    #[test]
    fn rustls_presents_the_identity_key() {
        let id = Identity::generate().unwrap();
        let key = certified_key(&id, &provider()).unwrap();
        assert_eq!(key.cert[0].as_ref(), ed25519_to_spki(id.public_key()));
    }

    async fn handshake(
        client: &Identity,
        server: &Identity,
        client_pins: &[u8],
        server_trust: Arc<TrustStore>,
    ) -> std::io::Result<Option<DeviceId>> {
        let (c, s) = tokio::io::duplex(16 * 1024);
        let connector = TlsConnector::from(client_config(client, client_pins).unwrap());
        let acceptor = TlsAcceptor::from(server_config(server, server_trust).unwrap());

        let client_side = async {
            let mut tls = connector.connect(server_name(), c).await?;
            tls.write_all(b"ping").await?;
            let mut buf = [0u8; 4];
            tls.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"pong");
            Ok::<_, std::io::Error>(())
        };
        let server_side = async {
            let mut tls = acceptor.accept(s).await?;
            let (_, conn) = tls.get_ref();
            assert_eq!(conn.alpn_protocol(), Some(ALPN));
            assert_eq!(
                conn.protocol_version(),
                Some(rustls::ProtocolVersion::TLSv1_3)
            );
            let peer = peer_device_id(conn.peer_certificates());
            let mut buf = [0u8; 4];
            tls.read_exact(&mut buf).await?;
            tls.write_all(b"pong").await?;
            tls.flush().await?;
            Ok::<_, std::io::Error>(peer)
        };
        let (c, s) = tokio::join!(client_side, server_side);
        c?;
        s
    }

    #[tokio::test]
    async fn mutually_trusted_devices_connect() {
        let (client, server) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let trust = Arc::new(TrustStore::in_memory());
        trust
            .trust(&client.device_info("client", OsType::Linux))
            .await
            .unwrap();
        let peer = handshake(&client, &server, server.public_key(), trust)
            .await
            .unwrap();
        assert_eq!(peer, Some(client.device_id()));
    }

    #[tokio::test]
    async fn server_rejects_untrusted_client() {
        let (client, server) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let empty = Arc::new(TrustStore::in_memory());
        assert!(handshake(&client, &server, server.public_key(), empty)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn client_rejects_unexpected_server_key() {
        let (client, server) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let impostor_key = Identity::generate().unwrap();
        let trust = Arc::new(TrustStore::in_memory());
        trust
            .trust(&client.device_info("client", OsType::Linux))
            .await
            .unwrap();
        assert!(
            handshake(&client, &server, impostor_key.public_key(), trust)
                .await
                .is_err()
        );
    }
}
