#![cfg(all(feature = "tls", feature = "rustls-tls", feature = "quic"))]

use flowsdk::mqtt_client::transport::{
    quic::{QuicConfig, QuicTransport},
    tls::TlsConfig,
    RustlsTlsConfig, RustlsTlsTransport, TcpTransport, TlsTransport, Transport,
};
use openssl::{
    asn1::Asn1Time,
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkey::PKey,
    x509::{extension::SubjectAlternativeName, X509NameBuilder, X509},
};
#[cfg(feature = "quic-openssl")]
use quinn_openssl as quinn;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Identity {
    cert: Vec<u8>,
    key: Vec<u8>,
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
}

#[tokio::test]
async fn tokio_quic_client_completes_mqtt_commands_after_rebinding() {
    use flowsdk::mqtt_client::{
        commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand},
        engine::MqttEvent,
        tokio_quic_client::TokioQuicMqttClient,
        MqttClientOptions,
    };
    use flowsdk::mqtt_serde::{control_packet::MqttPacket, parser::ParseOk};
    let workflow = async {
        let identity = Identity::new();
        let mut server_config = identity.server();
        server_config.alpn_protocols = vec![b"mqtt".to_vec()];
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_config).unwrap();
        let endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(quinn::ServerConfig::with_crypto(Arc::new(crypto))),
            std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
            quinn::default_runtime().unwrap(),
        )
        .unwrap();
        let address = endpoint.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let connection = endpoint.accept().await.unwrap().await.unwrap();
            let mut streams = tokio::task::JoinSet::new();
            // The engine uses distinct control, subscription, and publish streams.
            for _ in 0..3 {
                let (mut send, mut recv) = connection.accept_bi().await.unwrap();
                streams.spawn(async move {
                    let mut packets = Vec::new();
                    loop {
                        let mut first = [0];
                        recv.read_exact(&mut first).await.unwrap();
                        let mut bytes = first.to_vec();
                        let mut length = 0usize;
                        let mut shift = 0;
                        loop {
                            recv.read_exact(&mut first).await.unwrap();
                            bytes.push(first[0]);
                            length |= usize::from(first[0] & 0x7f) << shift;
                            if first[0] & 0x80 == 0 {
                                break;
                            }
                            shift += 7;
                            assert!(shift <= 21);
                        }
                        let offset = bytes.len();
                        bytes.resize(offset + length, 0);
                        recv.read_exact(&mut bytes[offset..]).await.unwrap();
                        let ParseOk::Packet(packet, _) =
                            MqttPacket::from_bytes_with_version(&bytes, 5).unwrap()
                        else {
                            panic!("Expected a complete MQTT packet");
                        };
                        let response = match &packet {
                            MqttPacket::Connect5(p) => {
                                assert_eq!(p.client_id, "tokio-quic-coverage");
                                vec![0x20, 3, 0, 0, 0]
                            }
                            MqttPacket::Subscribe5(p) => {
                                vec![0x90, 4, (p.packet_id >> 8) as u8, p.packet_id as u8, 0, 1]
                            }
                            MqttPacket::Publish5(p) => {
                                assert_eq!(p.payload, b"payload");
                                let id = p.packet_id.unwrap();
                                vec![0x40, 2, (id >> 8) as u8, id as u8]
                            }
                            MqttPacket::Unsubscribe5(p) => {
                                vec![0xb0, 4, (p.packet_id >> 8) as u8, p.packet_id as u8, 0, 0]
                            }
                            MqttPacket::Disconnect5(_) => {
                                packets.push(packet);
                                break;
                            }
                            other => panic!("Unexpected packet: {other:?}"),
                        };
                        let complete = matches!(
                            packet,
                            MqttPacket::Publish5(_) | MqttPacket::Unsubscribe5(_)
                        );
                        packets.push(packet);
                        send.write_all(&response).await.unwrap();
                        if complete {
                            break;
                        }
                    }
                    (send, recv, packets)
                });
            }
            // Keep data streams open until the control stream receives DISCONNECT.
            let mut completed = Vec::new();
            while let Some(result) = streams.join_next().await {
                completed.push(result.unwrap());
            }
            completed
                .into_iter()
                .flat_map(|(_, _, packets)| packets)
                .collect::<Vec<_>>()
        });
        let mut client = TokioQuicMqttClient::new(
            MqttClientOptions::builder()
                .client_id("tokio-quic-coverage")
                .reconnect(false)
                .build(),
        )
        .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(identity.cert.into()).unwrap();
        let mut crypto = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        crypto.alpn_protocols = vec![b"mqtt".to_vec()];
        assert!(client
            .connect_with_bind(
                address,
                "localhost".into(),
                crypto.clone(),
                Some("[::1]:0".parse().unwrap())
            )
            .await
            .is_err());
        client
            .connect(address, "localhost".into(), crypto)
            .await
            .unwrap();
        assert!(
            matches!(client.next_event().await, Some(MqttEvent::Connected(result)) if result.is_success())
        );
        let bound = client.rebind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        assert_ne!(bound.port(), 0);
        let id = client
            .subscribe(SubscribeCommand::single("topic", 1))
            .await
            .unwrap();
        assert!(
            matches!(client.next_event().await, Some(MqttEvent::Subscribed(result)) if result.packet_id == id && result.is_success())
        );
        let id = client
            .publish(PublishCommand::simple(
                "topic",
                b"payload".to_vec(),
                1,
                false,
            ))
            .await
            .unwrap();
        let event = client.next_event().await;
        assert!(
            matches!(&event, Some(MqttEvent::Published(result)) if result.packet_id == id && result.is_success()),
            "{event:?}"
        );
        let id = client
            .unsubscribe(UnsubscribeCommand::from_topics(vec!["topic".into()]))
            .await
            .unwrap();
        assert!(
            matches!(client.next_event().await, Some(MqttEvent::Unsubscribed(result)) if result.packet_id == id && result.is_success())
        );
        client.disconnect().await.unwrap();
        assert_eq!(server.await.unwrap().len(), 5);
    };
    tokio::time::timeout(Duration::from_secs(15), workflow)
        .await
        .unwrap();
}

impl Identity {
    fn new() -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let key = PKey::from_ec_key(
            EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap(),
        )
        .unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "localhost").unwrap();
        let name = name.build();
        let mut cert = X509::builder().unwrap();
        cert.set_version(2).unwrap();
        cert.set_subject_name(&name).unwrap();
        cert.set_issuer_name(&name).unwrap();
        cert.set_pubkey(&key).unwrap();
        cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        let san = SubjectAlternativeName::new()
            .dns("localhost")
            .ip("127.0.0.1")
            .build(&cert.x509v3_context(None, None))
            .unwrap();
        cert.append_extension(san).unwrap();
        cert.sign(&key, MessageDigest::sha256()).unwrap();
        let cert = cert.build();
        Self {
            cert: cert.to_der().unwrap(),
            key: key.private_key_to_pkcs8().unwrap(),
            cert_pem: cert.to_pem().unwrap(),
            key_pem: key.private_key_to_pem_pkcs8().unwrap(),
        }
    }

    fn server(&self) -> rustls::ServerConfig {
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(self.cert.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone())),
            )
            .unwrap()
    }
}

async fn exercise(transport: &mut impl Transport, peer: &str) {
    assert_eq!(transport.peer_addr().unwrap(), peer);
    assert_ne!(transport.local_addr().unwrap(), peer);
    transport.set_nodelay(true).unwrap();
    transport.write_all(b"\x00hello\xff").await.unwrap();
    transport.flush().await.unwrap();
    let mut reply = [0; 7];
    transport.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"\x00hello\xff");
}

#[tokio::test]
async fn tcp_transport_preserves_binary_data_and_stream_ownership() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = [0; 7];
            stream.read_exact(&mut data).await.unwrap();
            stream.write_all(&data).await.unwrap();
        });
        let mut client = TcpTransport::connect(&address).await.unwrap();
        exercise(&mut client, &address).await;
        assert!(client.get_ref().nodelay().unwrap());
        client.get_mut().set_nodelay(false).unwrap();
        let mut client = TcpTransport::from_stream(client.into_inner());
        assert!(!client.get_ref().nodelay().unwrap());
        client.shutdown().await.unwrap();
        client.close().await.unwrap();
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tls_backends_verify_custom_roots_and_exchange_binary_data() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let identity = Identity::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(identity.server()));
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (socket, _) = listener.accept().await.unwrap();
                let mut stream = acceptor.accept(socket).await.unwrap();
                let mut data = [0; 7];
                stream.read_exact(&mut data).await.unwrap();
                stream.write_all(&data).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });
        let config = TlsConfig::builder()
            .add_root_certificate(native_tls::Certificate::from_der(&identity.cert).unwrap())
            .build();
        let mut native = TlsTransport::connect_with_config(&address, &config)
            .await
            .unwrap();
        exercise(&mut native, &address).await;
        assert!(native
            .get_ref()
            .get_ref()
            .peer_certificate()
            .unwrap()
            .is_some());
        native.get_mut().flush().await.unwrap();
        native.shutdown().await.unwrap();
        native.close().await.unwrap();
        let config = RustlsTlsConfig::builder()
            .use_system_roots(false)
            .add_roots_from_pem(&identity.cert_pem)
            .unwrap()
            .build();
        let mut rustls = RustlsTlsTransport::connect_with_config(&address, config)
            .await
            .unwrap();
        exercise(&mut rustls, &address).await;
        assert!(rustls.get_ref().get_ref().1.peer_certificates().is_some());
        rustls.get_mut().flush().await.unwrap();
        rustls.shutdown().await.unwrap();
        rustls.close().await.unwrap();
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[test]
fn tls_pem_builders_load_identity_and_reject_malformed_inputs() {
    let identity = Identity::new();
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    std::fs::write(&cert, &identity.cert_pem).unwrap();
    std::fs::write(&key, &identity.key_pem).unwrap();
    for from_file in [false, true] {
        let builder = RustlsTlsConfig::builder().use_system_roots(false);
        let builder = if from_file {
            builder
                .add_roots_from_pem_file(&cert)
                .unwrap()
                .client_auth_from_pem_files(&cert, &key)
                .unwrap()
        } else {
            builder
                .add_roots_from_pem(&identity.cert_pem)
                .unwrap()
                .client_auth_from_pem(&identity.cert_pem, &identity.key_pem)
                .unwrap()
        };
        let config = builder
            .alpn_list(vec![b"mqtt".to_vec()])
            .enable_key_log(true)
            .build()
            .to_client_config()
            .unwrap();
        assert!(config.client_auth_cert_resolver.has_certs());
        assert_eq!(config.alpn_protocols, vec![b"mqtt".to_vec()]);
    }
    assert!(RustlsTlsConfig::builder()
        .add_roots_from_pem(b"invalid")
        .is_err());
    assert!(RustlsTlsConfig::builder()
        .add_roots_from_pem_file(dir.path().join("missing"))
        .is_err());
    assert!(RustlsTlsConfig::builder()
        .client_auth_from_pem(b"invalid", &identity.key_pem)
        .is_err());
    assert!(RustlsTlsConfig::builder()
        .client_auth_from_pem(&identity.cert_pem, b"invalid")
        .is_err());
    assert!(RustlsTlsConfig::builder()
        .client_auth_from_pem_files(&cert, &cert)
        .is_err());
}

#[tokio::test]
async fn quic_transport_verifies_roots_and_survives_local_rebind() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let identity = Identity::new();
        let mut server_config = identity.server();
        server_config.alpn_protocols = vec![b"mqtt".to_vec()];
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_config).unwrap();
        let endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(quinn::ServerConfig::with_crypto(Arc::new(crypto))),
            std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
            quinn::default_runtime().unwrap(),
        )
        .unwrap();
        let address = endpoint.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let connection = endpoint.accept().await.unwrap().await.unwrap();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            for _ in 0..2 {
                let mut data = [0; 7];
                recv.read_exact(&mut data).await.unwrap();
                send.write_all(&data).await.unwrap();
            }
            connection.closed().await;
        });
        let config = QuicConfig::builder()
            .custom_roots(vec![identity.cert])
            .alpn(b"mqtt")
            .local_bind_ip("127.0.0.1".parse().unwrap())
            .enable_0rtt(true)
            .datagram_receive_buffer_size(4096)
            .build();
        let mut client = QuicTransport::connect_with_config(&address, config)
            .await
            .unwrap();
        exercise(&mut client, &address).await;
        let old = client.local_addr().unwrap();
        let new = client.rebind("127.0.0.1:0".parse().unwrap()).unwrap();
        assert_ne!(new.to_string(), old);
        assert_eq!(client.local_addr().unwrap(), new.to_string());
        exercise(&mut client, &address).await;
        client.close().await.unwrap();
        server.await.unwrap();
    })
    .await
    .unwrap();
}
