// SPDX-License-Identifier: MPL-2.0
#![cfg(any(feature = "tls", feature = "rustls-tls", feature = "quic"))]

fn identity() -> (
    openssl::pkey::PKey<openssl::pkey::Private>,
    openssl::x509::X509,
) {
    use openssl::{
        asn1::Asn1Time,
        ec::{EcGroup, EcKey},
        hash::MessageDigest,
        nid::Nid,
        pkey::PKey,
        x509::{X509NameBuilder, X509},
    };
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
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    (key, cert.build())
}

// This integration-test executable never installs a global provider. Keep it
// separate from tests that do, so feature unification cannot hide initialization
// failures when ring and OpenSSL are both enabled.
#[cfg(feature = "rustls-tls")]
#[test]
fn rustls_config_works_without_a_global_provider() {
    use flowsdk::mqtt_client::transport::RustlsTlsConfig;
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    let config = RustlsTlsConfig::builder().build().to_client_config();
    assert!(config.is_ok(), "{config:?}");
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
}

#[cfg(any(feature = "tls", feature = "rustls-tls"))]
#[tokio::test]
async fn enabled_tls_backends_exchange_data_with_a_local_server() {
    use flowsdk::mqtt_client::transport::Transport;
    use openssl::ssl::{SslAcceptor, SslMethod, SslVersion};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let workflow = async {
        let (key, cert) = identity();
        let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls_server()).unwrap();
        // Also exercise TLS 1.2 interoperability when QUIC enables another
        // provider. The QUIC backend must not change the TCP TLS provider.
        acceptor
            .set_min_proto_version(Some(SslVersion::TLS1_2))
            .unwrap();
        acceptor
            .set_max_proto_version(Some(SslVersion::TLS1_2))
            .unwrap();
        acceptor
            .set_cipher_list("ECDHE-ECDSA-AES256-GCM-SHA384")
            .unwrap();
        acceptor.set_private_key(&key).unwrap();
        acceptor.set_certificate(&cert).unwrap();
        let acceptor = acceptor.build();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let count = usize::from(cfg!(feature = "tls")) + usize::from(cfg!(feature = "rustls-tls"));
        let server = std::thread::spawn(move || {
            for _ in 0..count {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut stream = acceptor.accept(socket).unwrap();
                let mut bytes = [0; 4];
                stream.read_exact(&mut bytes).unwrap();
                assert_eq!(&bytes, b"ping");
                stream.write_all(&bytes).unwrap();
                stream.flush().unwrap();
                let _ = stream.shutdown();
            }
        });
        async fn exchange(mut client: impl Transport) {
            client.write_all(b"ping").await.unwrap();
            client.flush().await.unwrap();
            let mut reply = [0; 4];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply, b"ping");
            client.close().await.unwrap();
        }
        #[cfg(feature = "tls")]
        {
            use flowsdk::mqtt_client::transport::{tls::TlsConfig, TlsTransport};
            let config = TlsConfig::builder()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true)
                .build();
            exchange(
                TlsTransport::connect_with_config(&address, &config)
                    .await
                    .unwrap(),
            )
            .await;
        }
        #[cfg(feature = "rustls-tls")]
        {
            use flowsdk::mqtt_client::transport::{RustlsTlsConfig, RustlsTlsTransport};
            let config = RustlsTlsConfig::builder()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true)
                .build();
            exchange(
                RustlsTlsTransport::connect_with_config(&address, config)
                    .await
                    .unwrap(),
            )
            .await;
        }
        server.join().unwrap();
    };
    tokio::time::timeout(Duration::from_secs(15), workflow)
        .await
        .unwrap();
    #[cfg(feature = "rustls-tls")]
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
}

#[cfg(feature = "quic")]
#[tokio::test]
async fn quic_handshake_works_without_a_global_provider() {
    use flowsdk::mqtt_client::transport::{
        quic::{QuicConfig, QuicTransport},
        Transport,
    };
    #[cfg(feature = "quic-openssl")]
    use quinn_openssl as quinn;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use std::{sync::Arc, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    let workflow = async {
        let (key, cert) = identity();
        #[cfg(feature = "quic-proto-openssl")]
        let provider = rustls_openssl::default_provider();
        #[cfg(not(feature = "quic-proto-openssl"))]
        let provider = rustls::crypto::ring::default_provider();
        let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.to_der().unwrap())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                    key.private_key_to_pkcs8().unwrap(),
                )),
            )
            .unwrap();
        tls.alpn_protocols = vec![b"mqtt".to_vec()];
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap();
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
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            let mut bytes = [0; 4];
            recv.read_exact(&mut bytes).await.unwrap();
            send.write_all(&bytes).await.unwrap();
            // Retain the connection until the client acknowledges the reply.
            let mut ack = [0; 1];
            recv.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, [1]);
        });
        let mut transport = QuicTransport::connect_with_config(
            &address.to_string(),
            QuicConfig::builder()
                .insecure_skip_verify(true)
                .alpn(b"mqtt")
                .build(),
        )
        .await
        .unwrap();
        transport.write_all(b"ping").await.unwrap();
        transport.flush().await.unwrap();
        let mut reply = [0; 4];
        transport.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"ping");
        transport.write_all(&[1]).await.unwrap();
        transport.flush().await.unwrap();
        server.await.unwrap();
        transport.close().await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(15), workflow)
        .await
        .unwrap();
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
}
