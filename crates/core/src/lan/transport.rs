//! Pinned TLS verifier. Unpinned TLS is permitted only inside SPAKE2 enrollment.
use super::*;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::sync::Arc;
#[derive(Debug)]
struct CertificateVerifier {
    expected: Option<Vec<u8>>,
    provider: rustls::crypto::CryptoProvider,
}
impl ServerCertVerifier for CertificateVerifier {
    fn verify_server_cert(
        &self,
        certificate: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self
            .expected
            .as_ref()
            .is_some_and(|expected| expected.as_slice() != certificate.as_ref())
        {
            return Err(rustls::Error::General(
                "SaveSync peer certificate changed".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
pub(super) fn server_config(identity: &identity::Identity) -> io::Result<quinn::ServerConfig> {
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.key_der.clone());
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(error)?
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(identity.certificate.clone())],
        key.into(),
    )
    .map_err(error)?;
    tls.alpn_protocols = vec![b"savesync/2".to_vec()];
    tls.max_early_data_size = 0;
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).map_err(error)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    let mut limits = quinn::TransportConfig::default();
    limits
        .max_concurrent_bidi_streams(1u32.into())
        .max_concurrent_uni_streams(0u32.into())
        .max_idle_timeout(Some(Duration::from_secs(20).try_into().map_err(error)?));
    config.transport_config(Arc::new(limits));
    Ok(config)
}
pub(super) async fn connect(
    address: &str,
    expected: Option<Vec<u8>>,
) -> io::Result<(quinn::Endpoint, quinn::Connection)> {
    let addr = tokio::net::lookup_host(address)
        .await
        .map_err(error)?
        .find(|addr| addr.is_ipv4())
        .ok_or_else(|| error("LAN requires an IPv4 address or resolvable host"))?;
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(error)?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(CertificateVerifier {
        expected,
        provider: rustls::crypto::ring::default_provider(),
    }))
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"savesync/2".to_vec()];
    tls.enable_early_data = false;
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse().unwrap())?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(error)?,
    )));
    let connecting = endpoint.connect(addr, "savesync.local").map_err(error)?;
    let connection = tokio::time::timeout(Duration::from_secs(5), connecting)
        .await
        .map_err(error)?
        .map_err(error)?;
    Ok((endpoint, connection))
}
pub(super) fn binding(connection: &quinn::Connection) -> io::Result<[u8; 32]> {
    let mut bytes = [0; 32];
    connection
        .export_keying_material(&mut bytes, b"EXPORTER-SaveSync-v2", b"authenticated LAN")
        .map_err(|_| error("TLS exporter unavailable"))?;
    Ok(bytes)
}
pub(super) fn certificate(connection: &quinn::Connection) -> io::Result<Vec<u8>> {
    connection
        .peer_identity()
        .and_then(|value| value.downcast::<Vec<CertificateDer<'static>>>().ok())
        .and_then(|certs| certs.first().map(|cert| cert.to_vec()))
        .ok_or_else(denied)
}
pub(super) async fn send<T: Serialize>(
    stream: &mut quinn::SendStream,
    value: &T,
) -> io::Result<()> {
    send_bytes(
        stream,
        &serde_json::to_vec(value).map_err(error)?,
        MAX_HEADER,
    )
    .await
}
pub(super) async fn receive<T: serde::de::DeserializeOwned>(
    stream: &mut quinn::RecvStream,
) -> io::Result<T> {
    serde_json::from_slice(&receive_bytes(stream, MAX_HEADER).await?).map_err(error)
}
pub(super) async fn send_bytes(
    stream: &mut quinn::SendStream,
    bytes: &[u8],
    limit: usize,
) -> io::Result<()> {
    if bytes.len() > limit {
        return Err(error("LAN frame too large"));
    }
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await
        .map_err(error)?;
    stream.write_all(bytes).await.map_err(error)
}
pub(super) async fn receive_bytes(
    stream: &mut quinn::RecvStream,
    limit: usize,
) -> io::Result<Vec<u8>> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).await.map_err(error)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > limit {
        return Err(error("LAN frame too large"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await.map_err(error)?;
    Ok(bytes)
}
