// SPDX-License-Identifier: MPL-2.0

use super::{ffi_types::*, InsecureServerCertVerifier};
use rustls::pki_types::CertificateDer;
use std::{fs::File, io::BufReader, sync::Arc};

fn configuration(error: impl std::fmt::Display) -> MqttErrorFFI {
    MqttErrorFFI::Configuration {
        detail: error.to_string(),
    }
}

fn certificates(path: &str) -> Result<Vec<CertificateDer<'static>>, MqttErrorFFI> {
    let file = File::open(path)
        .map_err(|e| configuration(format!("Cannot read certificate {path}: {e}")))?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(configuration)?;
    if certs.is_empty() {
        return Err(configuration(format!("No certificates found in {path}")));
    }
    Ok(certs)
}

pub(super) fn client_config(
    opts: &MqttTlsOptionsFFI,
) -> Result<rustls::ClientConfig, MqttErrorFFI> {
    #[cfg(feature = "quic-openssl")]
    let provider = rustls_openssl::default_provider();
    #[cfg(not(feature = "quic-openssl"))]
    let provider = rustls::crypto::ring::default_provider();
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .map_err(configuration)?;

    let builder = if opts.insecure_skip_verify {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier))
    } else {
        let mut roots = rustls::RootCertStore::empty();
        let certs = match &opts.ca_cert_file {
            Some(path) => certificates(path)?,
            None => rustls_native_certs::load_native_certs().map_err(configuration)?,
        };
        for cert in certs {
            roots.add(cert).map_err(configuration)?;
        }
        if roots.is_empty() {
            return Err(configuration("No trusted CA certificates available"));
        }
        builder.with_root_certificates(roots)
    };

    let mut config = match (&opts.client_cert_file, &opts.client_key_file) {
        (None, None) => builder.with_no_client_auth(),
        (Some(cert_path), Some(key_path)) => {
            let certs = certificates(cert_path)?;
            let file = File::open(key_path)
                .map_err(|e| configuration(format!("Cannot read private key {key_path}: {e}")))?;
            let key = rustls_pemfile::private_key(&mut BufReader::new(file))
                .map_err(configuration)?
                .ok_or_else(|| configuration(format!("No private key found in {key_path}")))?;
            // This provider does not expose SigningKey::public_key to rustls,
            // so rustls cannot perform its usual certificate/key match check.
            #[cfg(feature = "quic-openssl")]
            {
                let certificate =
                    openssl::x509::X509::from_der(certs[0].as_ref()).map_err(configuration)?;
                let public_key = certificate.public_key().map_err(configuration)?;
                let private_key = openssl::pkey::PKey::private_key_from_der(key.secret_der())
                    .map_err(configuration)?;
                if !private_key.public_eq(&public_key) {
                    return Err(configuration(
                        "Client certificate and private key do not match",
                    ));
                }
            }
            builder
                .with_client_auth_cert(certs, key)
                .map_err(configuration)?
        }
        _ => {
            return Err(configuration(
                "client_cert_file and client_key_file must be provided together",
            ))
        }
    };
    config.alpn_protocols = if opts.alpn_protocols.is_empty() {
        vec![b"mqtt".to_vec()]
    } else {
        opts.alpn_protocols
            .iter()
            .map(|p| p.as_bytes().to_vec())
            .collect()
    };
    if opts.enable_key_log {
        config.key_log = Arc::new(rustls::KeyLogFile::new());
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::{
        asn1::Asn1Time,
        ec::{EcGroup, EcKey},
        hash::MessageDigest,
        nid::Nid,
        pkey::PKey,
        x509::{X509NameBuilder, X509},
    };

    fn identity() -> (tempfile::TempDir, MqttTlsOptionsFFI) {
        let dir = tempfile::tempdir().unwrap();
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
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
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, cert.build().to_pem().unwrap()).unwrap();
        std::fs::write(&key_path, key.private_key_to_pem_pkcs8().unwrap()).unwrap();
        let opts = MqttTlsOptionsFFI {
            ca_cert_file: Some(cert_path.to_str().unwrap().into()),
            client_cert_file: Some(cert_path.to_str().unwrap().into()),
            client_key_file: Some(key_path.to_str().unwrap().into()),
            ..Default::default()
        };
        (dir, opts)
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_backpressure_preserves_large_publish_and_disconnect() {
        use flowsdk::mqtt_client::{
            commands::PublishCommand, opts::MqttClientOptions, tls_engine::TlsMqttEngine,
        };
        use std::io::{Read, Write};
        let (_dir, mut opts) = identity();
        opts.insecure_skip_verify = true;
        let config = client_config(&opts).unwrap();
        let cert = std::fs::read(opts.client_cert_file.as_ref().unwrap()).unwrap();
        let key = std::fs::read(opts.client_key_file.as_ref().unwrap()).unwrap();
        let certs = rustls_pemfile::certs(&mut cert.as_slice())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let key = rustls_pemfile::private_key(&mut key.as_slice())
            .unwrap()
            .unwrap();
        let mut client_roots = rustls::RootCertStore::empty();
        client_roots.add(certs[0].clone()).unwrap();
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(client_roots),
            config.crypto_provider().clone(),
        )
        .build()
        .unwrap();
        let server_config =
            rustls::ServerConfig::builder_with_provider(config.crypto_provider().clone())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_client_cert_verifier(verifier)
                .with_single_cert(certs, key)
                .unwrap();
        let mut server = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
        let mut client = TlsMqttEngine::new(
            MqttClientOptions::builder().auto_keepalive(false).build(),
            "localhost",
            Arc::new(config),
        )
        .unwrap();
        let mut received = Vec::new();
        fn exchange(
            client: &mut TlsMqttEngine,
            server: &mut rustls::ServerConnection,
            received: &mut Vec<u8>,
        ) {
            let events = client.handle_tick(std::time::Instant::now());
            assert!(!events
                .iter()
                .any(|event| matches!(event, flowsdk::mqtt_client::engine::MqttEvent::Error(_))));
            let bytes = client.take_socket_data();
            let mut input = bytes.as_slice();
            while !input.is_empty() {
                server.read_tls(&mut input).unwrap();
                server.process_new_packets().unwrap();
                let mut chunk = [0; 16384];
                loop {
                    match server.reader().read(&mut chunk) {
                        Ok(0) => break,
                        Ok(len) => received.extend_from_slice(&chunk[..len]),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) => panic!("server plaintext: {error}"),
                    }
                }
            }
            let mut output = vec![];
            server.write_tls(&mut output).unwrap();
            client.handle_socket_data(&output).unwrap();
        }
        client.connect().unwrap();
        for _ in 0..20 {
            exchange(&mut client, &mut server, &mut received);
        }
        assert_eq!(received[0], 0x10);
        received.clear();
        server.writer().write_all(&[0x20, 3, 0, 0, 0]).unwrap();
        for _ in 0..3 {
            exchange(&mut client, &mut server, &mut received);
        }
        assert!(client.is_connected());
        assert_eq!(server.peer_certificates().unwrap().len(), 1);
        let payload = vec![0x5a; 512 * 1024];
        use flowsdk::mqtt_serde::mqttv5::{common::properties::Property, publishv5::MqttPublish};
        let properties = vec![
            Property::CorrelationData(vec![0, 255]),
            Property::UserProperty("source".into(), "one".into()),
            Property::UserProperty("source".into(), "two".into()),
        ];
        let incoming = MqttPacket::Publish5(MqttPublish::new_with_prop(
            0,
            "large".into(),
            None,
            payload.clone(),
            true,
            false,
            properties.clone(),
        ))
        .to_bytes()
        .unwrap();
        server.set_buffer_limit(None);
        server.writer().write_all(&incoming).unwrap();
        let mut encrypted = vec![];
        server.write_tls(&mut encrypted).unwrap();
        client.handle_socket_data(&encrypted).unwrap();
        let events = client.handle_tick(std::time::Instant::now());
        assert!(!events.is_empty(), "TLS produced no MQTT events");
        for event in &events {
            if let flowsdk::mqtt_client::engine::MqttEvent::Error(error) = event {
                panic!("TLS MQTT error: {error}");
            }
        }
        assert!(events.iter().any(|event| matches!(event, flowsdk::mqtt_client::engine::MqttEvent::MessageReceived(message) if message.payload == payload && message.properties == properties && message.retain && !message.dup)));

        client
            .publish(PublishCommand::simple("large", payload.clone(), 0, true))
            .unwrap();
        client.try_disconnect().unwrap();
        client.handle_tick(std::time::Instant::now());
        assert!(!client.disconnect_complete());
        for _ in 0..100 {
            exchange(&mut client, &mut server, &mut received);
            if client.disconnect_complete() {
                break;
            }
        }
        assert!(client.disconnect_complete());
        use flowsdk::mqtt_serde::{control_packet::MqttPacket, parser::ParseOk};
        match MqttPacket::from_bytes_with_version(&received, 5).unwrap() {
            ParseOk::Packet(MqttPacket::Publish5(packet), consumed) => {
                assert_eq!(packet.payload, payload);
                assert!(packet.retain);
                assert!(matches!(
                    MqttPacket::from_bytes_with_version(&received[consumed..], 5).unwrap(),
                    ParseOk::Packet(MqttPacket::Disconnect5(_), _)
                ));
            }
            packet => panic!("Expected complete PUBLISH: {packet:?}"),
        }
    }

    #[test]
    fn client_identity_is_kept_with_or_without_server_verification() {
        let (_dir, mut opts) = identity();
        for insecure in [false, true] {
            opts.insecure_skip_verify = insecure;
            let config = client_config(&opts).unwrap();
            assert!(config.client_auth_cert_resolver.has_certs());
            assert_eq!(config.alpn_protocols, vec![b"mqtt".to_vec()]);
        }
    }

    #[test]
    fn incomplete_unreadable_or_invalid_identity_is_an_error() {
        let (_dir, opts) = identity();
        for field in ["certificate", "key"] {
            let mut incomplete = opts.clone();
            if field == "certificate" {
                incomplete.client_cert_file = None;
            } else {
                incomplete.client_key_file = None;
            }
            assert!(client_config(&incomplete).is_err());
        }
        let mut invalid = opts.clone();
        invalid.client_key_file = Some("/nonexistent/flowsdk-key.pem".into());
        assert!(client_config(&invalid).is_err());
        std::fs::write(opts.client_key_file.as_ref().unwrap(), b"not a PEM key").unwrap();
        assert!(client_config(&opts).is_err());
    }

    #[test]
    fn mismatched_private_key_is_an_error() {
        let (_first, mut opts) = identity();
        let (_second, other) = identity();
        opts.client_key_file = other.client_key_file;
        assert!(client_config(&opts).is_err());
    }

    #[test]
    fn invalid_ca_is_not_silently_ignored() {
        let (_dir, mut opts) = identity();
        opts.ca_cert_file = Some("/nonexistent/flowsdk-ca.pem".into());
        assert!(client_config(&opts).is_err());
        opts.ca_cert_file = opts.client_key_file.clone();
        assert!(client_config(&opts).is_err());
    }

    #[test]
    fn explicit_alpn_is_preserved() {
        let opts = MqttTlsOptionsFFI {
            insecure_skip_verify: true,
            alpn_protocols: vec!["mqtt-next".into()],
            ..Default::default()
        };
        let config = client_config(&opts).unwrap();
        assert!(!config.client_auth_cert_resolver.has_certs());
        assert_eq!(config.alpn_protocols, vec![b"mqtt-next".to_vec()]);
    }
}
