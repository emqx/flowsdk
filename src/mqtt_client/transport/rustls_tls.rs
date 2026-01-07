// SPDX-License-Identifier: MPL-2.0

//! Rustls-based TLS transport implementation (feature-gated)

#[cfg(feature = "rustls-tls")]
mod imp {
    use super::super::{Transport, TransportError};
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::net::TcpStream;

    use rustls::client::danger::{ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig as RustlsClientConfig, RootCertStore};
    use rustls_native_certs;
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, UnixTime};

    use crate::mqtt_client::transport::parse_mqtt_address;
    use tokio_rustls::{client::TlsStream as RustlsTlsStream, TlsConnector as RustlsTlsConnector};

    /// TLS configuration for Rustls backend
    #[derive(Clone, Default, Debug)]
    pub struct RustlsTlsConfig {
        /// Use system root certificates (platform trust store). Enabled by default.
        pub use_system_roots: bool,
        /// Additional custom root certificates (DER bytes)
        pub custom_root_certs: Vec<Vec<u8>>,
        /// Optional client certificate chain (DER bytes per cert)
        pub client_cert_chain: Option<Vec<Vec<u8>>>,
        /// Optional client private key (DER bytes)
        pub client_private_key: Option<Vec<u8>>,
        /// ⚠️ DANGEROUS: Skip certificate chain validation (testing only!)
        pub danger_accept_invalid_certs: bool,
        /// ⚠️ DANGEROUS: Skip hostname verification (testing only!)
        pub danger_accept_invalid_hostnames: bool,
        /// Optional ALPN protocols (usually not needed for MQTT over TCP)
        pub alpn_protocols: Vec<Vec<u8>>,
    }

    impl RustlsTlsConfig {
        /// Create a new builder for `RustlsTlsConfig`.
        pub fn builder() -> RustlsTlsConfigBuilder {
            RustlsTlsConfigBuilder::default()
        }

        /// Build a rustls ClientConfig from this configuration.
        pub fn to_client_config(&self) -> Result<RustlsClientConfig, TransportError> {
            // Prepare root store
            let mut roots = RootCertStore::empty();

            if self.use_system_roots {
                let native_certs = rustls_native_certs::load_native_certs().map_err(|e| {
                    TransportError::Tls(format!("failed to load platform root certs: {}", e))
                })?;
                for cert in native_certs {
                    roots.add(cert).map_err(|e| {
                        TransportError::Tls(format!("failed to add root cert: {:?}", e))
                    })?;
                }
            }

            // Add any custom roots
            for cert in &self.custom_root_certs {
                roots.add(cert.clone().into()).map_err(|e| {
                    TransportError::Tls(format!("failed to add custom root: {:?}", e))
                })?;
            }

            // Build rustls config with or without client auth
            let mut cfg = if let (Some(chain), Some(key)) = (
                self.client_cert_chain.clone(),
                self.client_private_key.clone(),
            ) {
                let chain_der: Vec<CertificateDer<'static>> =
                    chain.into_iter().map(CertificateDer::from).collect();
                let parsed_key = PrivateKeyDer::try_from(key).map_err(|e| {
                    TransportError::Tls(format!(
                        "failed to parse client private key DER for mTLS: {:?}",
                        e
                    ))
                })?;
                let key_der: PrivateKeyDer<'static> = parsed_key.clone_key();

                RustlsClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_client_auth_cert(chain_der, key_der)
                    .map_err(|e| {
                        TransportError::Tls(format!(
                            "failed to build rustls client config with client auth: {}",
                            e
                        ))
                    })?
            } else {
                RustlsClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth()
            };

            // Apply ALPN if provided
            if !self.alpn_protocols.is_empty() {
                cfg.alpn_protocols = self.alpn_protocols.clone();
            }

            // Apply dangerous overrides if requested
            if self.danger_accept_invalid_certs || self.danger_accept_invalid_hostnames {
                cfg.dangerous()
                    .set_certificate_verifier(Arc::new(InsecureServerCertVerifier {
                        skip_name_check: self.danger_accept_invalid_hostnames,
                    }));
            }

            Ok(cfg)
        }
    }

    /// Builder for `RustlsTlsConfig` with ergonomic helpers
    #[derive(Default)]
    pub struct RustlsTlsConfigBuilder {
        use_system_roots: bool,
        custom_root_certs: Vec<Vec<u8>>,
        client_cert_chain: Option<Vec<Vec<u8>>>,
        client_private_key: Option<Vec<u8>>,
        danger_accept_invalid_certs: bool,
        danger_accept_invalid_hostnames: bool,
        alpn_protocols: Vec<Vec<u8>>,
    }

    impl RustlsTlsConfigBuilder {
        /// Enable/disable loading platform root certificates (default: true)
        pub fn use_system_roots(mut self, enable: bool) -> Self {
            self.use_system_roots = enable;
            self
        }

        /// Add custom root certificates from PEM bytes (may contain multiple CERTIFICATE sections).
        pub fn add_roots_from_pem(mut self, pem_data: &[u8]) -> Result<Self, TransportError> {
            let iter = CertificateDer::pem_slice_iter(pem_data);
            let mut any = false;
            for c in iter.flatten() {
                self.custom_root_certs
                    .push(c.into_owned().as_ref().to_vec());
                any = true;
            }
            if !any {
                return Err(TransportError::Tls(
                    "no valid certificates found in PEM data".to_string(),
                ));
            }
            Ok(self)
        }

        /// Load custom root certificates from a PEM file path.
        pub fn add_roots_from_pem_file(
            mut self,
            file: impl AsRef<std::path::Path>,
        ) -> Result<Self, TransportError> {
            match CertificateDer::pem_file_iter(file) {
                Ok(iter) => {
                    for item in iter.flatten() {
                        self.custom_root_certs
                            .push(item.into_owned().as_ref().to_vec());
                    }
                    Ok(self)
                }
                Err(e) => Err(TransportError::Tls(format!(
                    "failed to read/parse root certificates PEM file: {:?}",
                    e
                ))),
            }
        }

        /// Set client certificate chain and private key from PEM bytes.
        pub fn client_auth_from_pem(
            mut self,
            cert_chain_pem: &[u8],
            private_key_pem: &[u8],
        ) -> Result<Self, TransportError> {
            // Collect all CERTIFICATE sections
            let chain_iter = CertificateDer::pem_slice_iter(cert_chain_pem);
            let chain: Vec<Vec<u8>> = chain_iter
                .filter_map(|r| r.ok().map(|c| c.into_owned().as_ref().to_vec()))
                .collect();
            if chain.is_empty() {
                return Err(TransportError::Tls(
                    "no valid certificates found in client cert PEM".to_string(),
                ));
            }
            // Parse a single private key
            use std::io::Cursor;
            let mut cursor = Cursor::new(private_key_pem);
            let pk = PrivateKeyDer::from_pem_reader(&mut cursor).map_err(|e| {
                TransportError::Tls(format!("failed to parse client private key PEM: {:?}", e))
            })?;
            let der = pk.clone_key().secret_der().to_vec();

            self.client_cert_chain = Some(chain);
            self.client_private_key = Some(der);
            Ok(self)
        }

        /// Set client certificate chain and private key from PEM files.
        pub fn client_auth_from_pem_files(
            mut self,
            cert_chain_file: impl AsRef<std::path::Path>,
            private_key_file: impl AsRef<std::path::Path>,
        ) -> Result<Self, TransportError> {
            let chain = match CertificateDer::pem_file_iter(cert_chain_file) {
                Ok(iter) => iter
                    .filter_map(|r| r.ok().map(|c| c.into_owned().as_ref().to_vec()))
                    .collect(),
                Err(e) => {
                    return Err(TransportError::Tls(format!(
                        "failed to read/parse client certificate chain: {:?}",
                        e
                    )))
                }
            };
            let pk = PrivateKeyDer::from_pem_file(private_key_file).map_err(|e| {
                TransportError::Tls(format!("failed to parse client private key: {:?}", e))
            })?;
            let der = pk.clone_key().secret_der().to_vec();

            self.client_cert_chain = Some(chain);
            self.client_private_key = Some(der);
            Ok(self)
        }

        /// ⚠️ DANGEROUS: Skip certificate chain validation (testing only)
        pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
            self.danger_accept_invalid_certs = accept;
            self
        }

        /// ⚠️ DANGEROUS: Skip hostname verification (testing only)
        pub fn danger_accept_invalid_hostnames(mut self, accept: bool) -> Self {
            self.danger_accept_invalid_hostnames = accept;
            self
        }

        /// Configure ALPN list
        pub fn alpn_list(mut self, prots: Vec<Vec<u8>>) -> Self {
            self.alpn_protocols = prots;
            self
        }

        /// Build the final `RustlsTlsConfig`.
        pub fn build(self) -> RustlsTlsConfig {
            RustlsTlsConfig {
                use_system_roots: self.use_system_roots,
                custom_root_certs: self.custom_root_certs,
                client_cert_chain: self.client_cert_chain,
                client_private_key: self.client_private_key,
                danger_accept_invalid_certs: self.danger_accept_invalid_certs,
                danger_accept_invalid_hostnames: self.danger_accept_invalid_hostnames,
                alpn_protocols: self.alpn_protocols,
            }
        }
    }

    /// ⚠️ DANGEROUS: A certificate verifier that accepts all certificates or skips name checks.
    /// This should ONLY be used for testing and development!
    #[derive(Debug)]
    struct InsecureServerCertVerifier {
        skip_name_check: bool,
    }

    impl ServerCertVerifier for InsecureServerCertVerifier {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            // If skip_name_check is false, we should verify the hostname
            if !self.skip_name_check {
                // Perform hostname verification using rustls's built-in verification
                let cert = rustls::server::ParsedCertificate::try_from(end_entity)?;
                rustls::client::verify_server_name(&cert, server_name)?;
            }
            // Accept any certificate without validation; optionally ignore name
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![
                rustls::SignatureScheme::RSA_PKCS1_SHA256,
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                rustls::SignatureScheme::RSA_PKCS1_SHA384,
                rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
                rustls::SignatureScheme::RSA_PKCS1_SHA512,
                rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
                rustls::SignatureScheme::RSA_PSS_SHA256,
                rustls::SignatureScheme::RSA_PSS_SHA384,
                rustls::SignatureScheme::RSA_PSS_SHA512,
                rustls::SignatureScheme::ED25519,
            ]
        }
    }

    /// Rustls-backed TLS transport for MQTT over TCP
    pub struct RustlsTlsTransport {
        stream: RustlsTlsStream<TcpStream>,
        peer_addr: String,
    }

    impl RustlsTlsTransport {
        /// Connect using default configuration (system roots)
        pub async fn connect_rustls(addr: &str) -> Result<Self, TransportError> {
            let cfg = RustlsTlsConfig::default();
            Self::connect_with_config(addr, cfg).await
        }

        /// Connect using an explicit configuration
        pub async fn connect_with_config(
            addr: &str,
            cfg: RustlsTlsConfig,
        ) -> Result<Self, TransportError> {
            // Parse address
            let (host, socket_addr) = parse_mqtt_address(addr)?;

            // Build rustls client config
            let client_cfg = cfg.to_client_config()?;
            let connector = RustlsTlsConnector::from(Arc::new(client_cfg));

            // Resolve server name for SNI
            let server_name = {
                use std::convert::TryFrom;
                match ServerName::try_from(host.clone()) {
                    Ok(sn) => sn,
                    Err(_e) => {
                        return Err(TransportError::InvalidAddress(format!(
                            "invalid DNS name for TLS: {}",
                            host
                        )))
                    }
                }
            };

            // Establish TCP
            let tcp = TcpStream::connect(&socket_addr).await.map_err(|e| {
                TransportError::ConnectionFailed(format!("TCP connection failed: {}", e))
            })?;

            // TLS handshake
            let tls_stream = connector
                .connect(server_name, tcp)
                .await
                .map_err(|e| TransportError::Tls(format!("TLS handshake failed: {}", e)))?;

            Ok(Self {
                stream: tls_stream,
                peer_addr: socket_addr,
            })
        }

        /// Access the underlying TLS stream
        pub fn get_ref(&self) -> &RustlsTlsStream<TcpStream> {
            &self.stream
        }

        /// Access the underlying TLS stream mutably
        pub fn get_mut(&mut self) -> &mut RustlsTlsStream<TcpStream> {
            &mut self.stream
        }
    }

    #[async_trait]
    impl Transport for RustlsTlsTransport {
        async fn connect(addr: &str) -> Result<Self, TransportError>
        where
            Self: Sized,
        {
            Self::connect_rustls(addr).await
        }

        async fn close(&mut self) -> Result<(), TransportError> {
            // Rustls/TCP closes on drop; no explicit shutdown available here
            Ok(())
        }

        fn peer_addr(&self) -> Result<String, TransportError> {
            Ok(self.peer_addr.clone())
        }

        fn local_addr(&self) -> Result<String, TransportError> {
            self.stream
                .get_ref()
                .0
                .local_addr()
                .map(|a| a.to_string())
                .map_err(TransportError::Io)
        }

        fn set_nodelay(&self, nodelay: bool) -> Result<(), TransportError> {
            self.stream
                .get_ref()
                .0
                .set_nodelay(nodelay)
                .map_err(TransportError::Io)
        }
    }

    impl tokio::io::AsyncRead for RustlsTlsTransport {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.stream).poll_read(cx, buf)
        }
    }

    impl tokio::io::AsyncWrite for RustlsTlsTransport {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::pin::Pin::new(&mut self.stream).poll_write(cx, buf)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.stream).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.stream).poll_shutdown(cx)
        }
    }
}

#[cfg(feature = "rustls-tls")]
pub use imp::{RustlsTlsConfig, RustlsTlsTransport};
