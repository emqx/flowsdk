//! QUIC transport implementation (feature-gated)

#[cfg(feature = "quic")]
mod imp {
    use super::super::{Transport, TransportError};
    use async_trait::async_trait;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::net::lookup_host;

    use quinn::{ClientConfig, Endpoint};
    use rustls::client::danger::{ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig as RustlsClientConfig, RootCertStore};
    use rustls_native_certs;
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, UnixTime};
    use std::convert::TryFrom;
    use std::sync::Arc;

    /// ⚠️ DANGEROUS: A certificate verifier that accepts all certificates without validation.
    /// This should ONLY be used for testing and development!
    #[derive(Debug)]
    struct InsecureServerCertVerifier;

    impl ServerCertVerifier for InsecureServerCertVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            // Accept any certificate without validation
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            // Accept any signature
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            // Accept any signature
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            // Support all signature schemes
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

    /// QUIC configuration exposed to the caller. Keep it minimal for now.
    #[derive(Debug, Clone, Default)]
    pub struct QuicConfig {
        /// ALPN protocols to advertise (raw bytes), e.g. b"mqtt"
        pub alpn_protocols: Vec<Vec<u8>>,
        /// Enable 0-RTT (early data) if supported
        pub enable_0rtt: bool,
        /// Optional custom root certificates to trust instead of platform roots (DER bytes)
        pub custom_root_certs: Option<Vec<Vec<u8>>>,
        /// Optional client certificate chain for mutual TLS (DER bytes)
        pub client_cert_chain: Option<Vec<Vec<u8>>>,
        /// Optional client private key for mutual TLS (DER bytes)
        pub client_private_key: Option<Vec<u8>>,
        /// ⚠️ DANGEROUS: Skip TLS certificate verification (for testing only!)
        pub insecure_skip_verify: bool,
        pub datagram_receive_buffer_size: usize,
    }

    /// Builder for `QuicConfig` to simplify ergonomic construction.
    pub struct QuicConfigBuilder {
        alpn_protocols: Vec<Vec<u8>>,
        enable_0rtt: bool,
        custom_root_certs: Option<Vec<Vec<u8>>>,
        client_cert_chain: Option<Vec<Vec<u8>>>,
        client_private_key: Option<Vec<u8>>,
        insecure_skip_verify: bool,
        datagram_receive_buffer_size: usize,
    }

    impl QuicConfigBuilder {
        /// Add a single ALPN protocol (as bytes) to advertise.
        pub fn alpn(mut self, proto: impl AsRef<[u8]>) -> Self {
            self.alpn_protocols.push(proto.as_ref().to_vec());
            self
        }

        /// Set the full ALPN vector (replacing any previously set values).
        pub fn alpn_list(mut self, prots: Vec<Vec<u8>>) -> Self {
            self.alpn_protocols = prots;
            self
        }

        /// Enable or disable 0-RTT (early data).
        pub fn enable_0rtt(mut self, enable: bool) -> Self {
            self.enable_0rtt = enable;
            self
        }

        /// Provide custom root certificates as DER-encoded bytes
        pub fn custom_roots(mut self, roots: Vec<Vec<u8>>) -> Self {
            self.custom_root_certs = Some(roots);
            self
        }

        /// Load custom root certificates from a PEM file (may contain multiple CERTIFICATE sections).
        /// Returns Err(TransportError::Quic) if the file cannot be read or PEM decoding fails.
        pub fn custom_roots_from_pem_file(
            mut self,
            file: impl AsRef<std::path::Path>,
        ) -> Result<Self, TransportError> {
            // Use rustls-pki-types to iterate PEM CERTIFICATE sections and collect DER bytes
            match rustls_pki_types::CertificateDer::pem_file_iter(file) {
                Ok(iter) => {
                    let roots: Vec<Vec<u8>> = iter
                        .filter_map(|r| r.ok().map(|c| c.into_owned().as_ref().to_vec()))
                        .collect();
                    self.custom_root_certs = Some(roots);
                    Ok(self)
                }
                Err(e) => Err(TransportError::Quic(format!(
                    "failed to read/parse root certificates PEM file: {:?}",
                    e
                ))),
            }
        }

        /// Load custom root certificates from PEM data (may contain multiple CERTIFICATE sections).
        /// Returns Err(TransportError::Quic) if PEM decoding fails.
        pub fn custom_roots_from_pem(mut self, pem_data: &[u8]) -> Result<Self, TransportError> {
            let iter = rustls_pki_types::CertificateDer::pem_slice_iter(pem_data);
            let roots: Vec<Vec<u8>> = iter
                .filter_map(|r| r.ok().map(|c| c.into_owned().as_ref().to_vec()))
                .collect();
            if roots.is_empty() {
                return Err(TransportError::Quic(
                    "no valid certificates found in PEM data".to_string(),
                ));
            }
            self.custom_root_certs = Some(roots);
            Ok(self)
        }

        /// Provide a client certificate chain for mutual TLS as DER-encoded bytes
        pub fn client_cert_chain(mut self, chain: Vec<Vec<u8>>) -> Self {
            self.client_cert_chain = Some(chain);
            self
        }

        /// Load a client certificate chain from a PEM file (may contain multiple CERTIFICATE sections).
        /// Returns Err(TransportError::Quic) if the file cannot be read or PEM decoding fails.
        pub fn client_cert_chain_from_pem_file(
            mut self,
            file: impl AsRef<std::path::Path>,
        ) -> Result<Self, TransportError> {
            match rustls_pki_types::CertificateDer::pem_file_iter(file) {
                Ok(iter) => {
                    let chain: Vec<Vec<u8>> = iter
                        .filter_map(|r| r.ok().map(|c| c.into_owned().as_ref().to_vec()))
                        .collect();
                    self.client_cert_chain = Some(chain);
                    Ok(self)
                }
                Err(e) => Err(TransportError::Quic(format!(
                    "failed to read/parse client certificate chain PEM file: {:?}",
                    e
                ))),
            }
        }

        /// Load a client certificate chain from PEM data (may contain multiple CERTIFICATE sections).
        /// Returns Err(TransportError::Quic) if PEM decoding fails.
        pub fn client_cert_chain_from_pem(
            mut self,
            pem_data: &[u8],
        ) -> Result<Self, TransportError> {
            let iter = rustls_pki_types::CertificateDer::pem_slice_iter(pem_data);
            let chain: Vec<Vec<u8>> = iter
                .filter_map(|r| r.ok().map(|c| c.into_owned().as_ref().to_vec()))
                .collect();
            if chain.is_empty() {
                return Err(TransportError::Quic(
                    "no valid certificates found in PEM data".to_string(),
                ));
            }
            self.client_cert_chain = Some(chain);
            Ok(self)
        }

        pub fn datagram_receive_buffer_size(mut self, size: usize) -> Self {
            self.datagram_receive_buffer_size = size;
            self
        }

        /// Provide a client private key for mutual TLS (DER-encoded bytes)
        pub fn client_private_key(mut self, key: Vec<u8>) -> Self {
            self.client_private_key = Some(key);
            self
        }

        /// Load a client private key from a PEM file. This will attempt to decode the first private-key
        /// PEM section and store its DER bytes. Returns Err(TransportError::Quic) on failure.
        pub fn client_private_key_from_pem_file(
            mut self,
            file: impl AsRef<std::path::Path>,
        ) -> Result<Self, TransportError> {
            match rustls_pki_types::PrivateKeyDer::from_pem_file(file) {
                Ok(pk) => {
                    let der = pk.clone_key().secret_der().to_vec();
                    self.client_private_key = Some(der);
                    Ok(self)
                }
                Err(e) => Err(TransportError::Quic(format!(
                    "failed to read/parse client private key PEM file: {:?}",
                    e
                ))),
            }
        }

        /// Load a client private key from PEM data. This will attempt to decode the first private-key
        /// PEM section and store its DER bytes. Returns Err(TransportError::Quic) on failure.
        pub fn client_private_key_from_pem(
            mut self,
            pem_data: &[u8],
        ) -> Result<Self, TransportError> {
            use std::io::Cursor;
            let mut cursor = Cursor::new(pem_data);
            match rustls_pki_types::PrivateKeyDer::from_pem_reader(&mut cursor) {
                Ok(pk) => {
                    let der = pk.clone_key().secret_der().to_vec();
                    self.client_private_key = Some(der);
                    Ok(self)
                }
                Err(e) => Err(TransportError::Quic(format!(
                    "failed to parse client private key from PEM data: {:?}",
                    e
                ))),
            }
        }

        /// ⚠️ DANGEROUS: Skip TLS certificate verification.
        /// This disables all certificate validation and should ONLY be used for testing!
        /// Never use this in production as it makes connections vulnerable to MITM attacks.
        pub fn insecure_skip_verify(mut self, skip: bool) -> Self {
            self.insecure_skip_verify = skip;
            self
        }

        /// Finalize the builder into a `QuicConfig`.
        pub fn build(self) -> QuicConfig {
            QuicConfig {
                alpn_protocols: self.alpn_protocols,
                enable_0rtt: self.enable_0rtt,
                custom_root_certs: self.custom_root_certs,
                client_cert_chain: self.client_cert_chain,
                client_private_key: self.client_private_key,
                insecure_skip_verify: self.insecure_skip_verify,
                datagram_receive_buffer_size: self.datagram_receive_buffer_size,
            }
        }
    }

    impl QuicConfig {
        /// Create a new builder for `QuicConfig`.
        pub fn builder() -> QuicConfigBuilder {
            QuicConfigBuilder {
                alpn_protocols: Vec::new(),
                enable_0rtt: false,
                custom_root_certs: None,
                client_cert_chain: None,
                client_private_key: None,
                insecure_skip_verify: false,
                datagram_receive_buffer_size: 0,
            }
        }
    }

    /// Minimal QUIC transport backed by quinn
    pub struct QuicTransport {
        // Keep the endpoint alive for the lifetime of the transport
        endpoint: Endpoint,
        connection: quinn::Connection,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
    }

    impl QuicTransport {
        /// Connect to the given address (host:port). This is a minimal implementation
        /// that uses default client config and system certificates via rustls-native-certs.
        pub async fn connect_quic(addr: &str) -> Result<Self, TransportError> {
            // Use default QuicConfig
            let cfg = QuicConfig::default();
            Self::connect_with_config(addr, cfg).await
        }

        /// Connect with an explicit QUIC configuration
        pub async fn connect_with_config(
            addr: &str,
            cfg: QuicConfig,
        ) -> Result<Self, TransportError> {
            // Derive server name (SNI) from addr by splitting host:port
            let server_name = addr
                .split(':')
                .next()
                .ok_or_else(|| {
                    TransportError::InvalidAddress(format!("Invalid QUIC address: {}", addr))
                })?
                .to_string();

            // Resolve address
            let mut addrs = lookup_host(addr).await.map_err(|e| {
                TransportError::InvalidAddress(format!("Failed to resolve {}: {}", addr, e))
            })?;

            let peer = addrs.next().ok_or_else(|| {
                TransportError::InvalidAddress(format!("No addresses found for {}", addr))
            })?;

            // Build a rustls ClientConfig from platform roots, apply ALPN and
            // early-data (0-RTT) settings from QuicConfig, then convert into
            // a quinn ClientConfig.

            // Build rustls config based on insecure_skip_verify flag
            let mut rustls_cfg = if cfg.insecure_skip_verify {
                // ⚠️ DANGEROUS: Skip certificate verification entirely
                RustlsClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier))
                    .with_no_client_auth()
            } else {
                // Normal certificate verification with root certificates
                // Prepare root certificates: prefer custom roots if provided, else load platform roots
                let mut roots = RootCertStore::empty();
                if let Some(custom_roots) = cfg.custom_root_certs.clone() {
                    for cert in custom_roots {
                        // cert is DER bytes (Vec<u8>) — convert into CertificateDer
                        roots.add(cert.into()).map_err(|e| {
                            TransportError::Quic(format!("failed to add custom root cert: {:?}", e))
                        })?;
                    }
                } else {
                    let native_certs = rustls_native_certs::load_native_certs().map_err(|e| {
                        TransportError::Quic(format!("failed to load platform root certs: {}", e))
                    })?;
                    for cert in native_certs {
                        roots.add(cert).map_err(|e| {
                            TransportError::Quic(format!("failed to add root cert: {:?}", e))
                        })?;
                    }
                }

                // Build rustls config; configure client auth if client cert/key provided
                if let (Some(chain), Some(key)) = (
                    cfg.client_cert_chain.clone(),
                    cfg.client_private_key.clone(),
                ) {
                    let cert_chain_der: Vec<CertificateDer<'static>> =
                        chain.into_iter().map(CertificateDer::from).collect();
                    let parsed_key = PrivateKeyDer::try_from(key).map_err(|e| {
                        TransportError::Quic(format!(
                            "failed to parse client private key DER for mTLS: {:?}",
                            e
                        ))
                    })?;
                    let key_der: PrivateKeyDer<'static> = parsed_key.clone_key();

                    RustlsClientConfig::builder()
                        .with_root_certificates(roots)
                        .with_client_auth_cert(cert_chain_der, key_der)
                        .map_err(|e| {
                            TransportError::Quic(format!(
                                "failed to build client TLS config with client cert: {}",
                                e
                            ))
                        })?
                } else {
                    // No client auth
                    RustlsClientConfig::builder()
                        .with_root_certificates(roots)
                        .with_no_client_auth()
                }
            };

            // Apply ALPN protocols if provided
            if !cfg.alpn_protocols.is_empty() {
                rustls_cfg.alpn_protocols = cfg.alpn_protocols.clone();
            }

            // Enable early data if requested
            rustls_cfg.enable_early_data = cfg.enable_0rtt;

            // Convert rustls config into a quinn QuicClientConfig, then wrap
            // it in quinn::ClientConfig
            let quic_client_cfg =
                match quinn::crypto::rustls::QuicClientConfig::try_from(rustls_cfg) {
                    Ok(c) => c,
                    Err(e) => {
                        return Err(TransportError::Quic(format!(
                            "Failed to convert rustls config to quinn QuicClientConfig: {}",
                            e
                        )));
                    }
                };

            let mut quinn_crypto = ClientConfig::new(Arc::new(quic_client_cfg));
            let mut trpt_cfg = quinn::TransportConfig::default();
            trpt_cfg.datagram_receive_buffer_size(Some(cfg.datagram_receive_buffer_size));
            quinn_crypto.transport_config(Arc::new(trpt_cfg));

            // Create an endpoint bound to an ephemeral UDP port
            let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap()).map_err(|e| {
                TransportError::ConnectionFailed(format!("QUIC endpoint create failed: {}", e))
            })?;

            endpoint.set_default_client_config(quinn_crypto);

            // Connect
            let connecting = endpoint.connect(peer, &server_name).map_err(|e| {
                TransportError::ConnectionFailed(format!("QUIC connect failed: {}", e))
            })?;

            let connection = connecting.await.map_err(|e| {
                TransportError::ConnectionFailed(format!("QUIC connect failed: {}", e))
            })?;

            // Open a bidirectional stream
            let (send, recv) = connection.open_bi().await.map_err(|e| {
                TransportError::ConnectionFailed(format!("QUIC open_bi failed: {}", e))
            })?;

            Ok(Self {
                endpoint,
                connection,
                send,
                recv,
            })
        }
    }

    #[async_trait]
    impl Transport for QuicTransport {
        async fn connect(addr: &str) -> Result<Self, TransportError>
        where
            Self: Sized,
        {
            Self::connect_quic(addr).await
        }

        async fn close(&mut self) -> Result<(), TransportError> {
            // Finish the send stream
            self.send.finish().map_err(|e| {
                TransportError::ConnectionFailed(format!("QUIC send finish failed: {}", e))
            })?;
            self.connection.close(0u32.into(), b"client close");
            Ok(())
        }

        fn peer_addr(&self) -> Result<String, TransportError> {
            Ok(self.connection.remote_address().to_string())
        }

        fn local_addr(&self) -> Result<String, TransportError> {
            // Use endpoint local_addr as connection doesn't expose local_address
            endpoint_local_addr(&self.endpoint).ok_or_else(|| {
                TransportError::Io(std::io::Error::other("local address unavailable"))
            })
        }
    }

    fn endpoint_local_addr(endpoint: &Endpoint) -> Option<String> {
        endpoint.local_addr().ok().map(|s| s.to_string())
    }

    impl AsyncRead for QuicTransport {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            // Delegate to quinn's RecvStream AsyncRead impl and map errors
            match Pin::new(&mut self.recv).poll_read(cx, buf) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::other(format!(
                    "QUIC read error: {}",
                    e
                )))),
            }
        }
    }

    impl AsyncWrite for QuicTransport {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            match Pin::new(&mut self.send).poll_write(cx, buf) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(n)) => Poll::Ready(Ok(n)),
                Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::other(format!(
                    "QUIC write error: {}",
                    e
                )))),
            }
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            match Pin::new(&mut self.send).poll_flush(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::other(format!(
                    "QUIC flush error: {}",
                    e
                )))),
            }
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            match Pin::new(&mut self.send).poll_shutdown(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::other(format!(
                    "QUIC shutdown error: {}",
                    e
                )))),
            }
        }
    }
}

#[cfg(feature = "quic")]
pub use imp::QuicConfig;
#[cfg(feature = "quic")]
pub use imp::QuicTransport;
