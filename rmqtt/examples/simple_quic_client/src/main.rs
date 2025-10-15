use bytes::Bytes;
use bytestring::ByteString;
use futures::{SinkExt, StreamExt};
use quinn::crypto::rustls::QuicClientConfig;
use quinn::Endpoint;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{DigitallySignedStruct, SignatureScheme};
use simple_logger::SimpleLogger;
use std::num::NonZeroU16;
use std::sync::Arc;
use tokio_util::codec::Framed;

/// AWS-LC based TLS provider (non-Windows platforms)
#[cfg(not(target_os = "windows"))]
pub use rustls::crypto::aws_lc_rs as tls_provider;
/// Ring-based TLS provider (Windows platforms)
#[cfg(target_os = "windows")]
pub use rustls::crypto::ring as tls_provider;

use rmqtt_codec::{
    types::{Protocol, Publish, QoS},
    v3::{Codec as CodecV3, Connect},
    MqttCodec, MqttPacket,
};
use rmqtt_net::{QuinnBiStream, Result};

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let client_config = build_client_config()?;

    // Bind local UDP port
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    // Connect to the QUIC server
    let server_addr = "127.0.0.1:9443".parse()?;
    let conn = endpoint.connect(server_addr, "localhost")?.await?;

    // Open a bidirectional QUIC stream
    let (send, recv) = conn.open_bi().await?;
    let stream = QuinnBiStream::new(send, recv);

    // Wrap the stream using Framed for MQTT codec handling
    let mut framed = Framed::new(stream, MqttCodec::V3(CodecV3::new(1024 * 1024)));

    // Send CONNECT packet
    let connect = Connect {
        protocol: Protocol(4),
        clean_session: true,
        keep_alive: 60,
        last_will: None,
        client_id: "cid-001".into(),
        username: None,
        password: None,
    };
    framed.send(MqttPacket::V3(rmqtt_codec::v3::Packet::Connect(Box::new(connect)))).await?;
    framed.flush().await?;
    log::info!("Sent CONNECT");

    // Wait for CONNACK response
    if let Some(Ok((MqttPacket::V3(rmqtt_codec::v3::Packet::ConnectAck(ack)), _))) = framed.next().await {
        log::info!("Received CONNACK: {:?}", ack);
    }

    // Send PUBLISH packet
    let publish = Publish {
        dup: false,
        retain: false,
        qos: QoS::AtLeastOnce,
        topic: ByteString::from_static("test"),
        packet_id: Some(NonZeroU16::new(1).unwrap()),
        payload: Bytes::from_static(b"data ..."),
        properties: None,
    };
    framed.send(MqttPacket::V3(rmqtt_codec::v3::Packet::Publish(Box::new(publish)))).await?;
    framed.flush().await?;
    log::info!("Sent PUBLISH");

    // Wait for PUBACK response
    if let Some(Ok((MqttPacket::V3(rmqtt_codec::v3::Packet::PublishAck { packet_id }), _))) =
        framed.next().await
    {
        log::info!("Received PUBACK for packet_id {:?}", packet_id);
    }

    framed.close().await?;

    Ok(())
}

fn build_client_config() -> Result<quinn::ClientConfig> {
    // Select TLS provider
    let provider = Arc::new(tls_provider::default_provider());

    // Client-side TLS configuration (allow self-signed certificates)
    let roots = rustls::RootCertStore::empty();
    let mut client_crypto = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();

    client_crypto.alpn_protocols = vec![b"mqtt".to_vec(), b"mqttv5".to_vec()];
    client_crypto.dangerous().set_certificate_verifier(Arc::new(SkipServerVerification));

    let server_crypto = QuicClientConfig::try_from(client_crypto)?;
    let client_config = quinn::ClientConfig::new(Arc::new(server_crypto));

    Ok(client_config)
}

// ======== SkipServerVerification ========
/// Custom certificate verifier that skips all certificate validation.
/// This should **only** be used for testing purposes.
#[derive(Debug)]
struct SkipServerVerification;

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        use rustls::SignatureScheme::*;
        vec![
            ECDSA_NISTP256_SHA256,
            ECDSA_NISTP384_SHA384,
            RSA_PSS_SHA256,
            RSA_PSS_SHA384,
            RSA_PSS_SHA512,
            RSA_PKCS1_SHA256,
            RSA_PKCS1_SHA384,
            RSA_PKCS1_SHA512,
        ]
    }
}
