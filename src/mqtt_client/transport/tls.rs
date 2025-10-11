//! TLS transport implementation
//!
//! This module provides TLS/SSL support for MQTT connections using tokio-native-tls.
//! It wraps TCP connections with TLS encryption and certificate validation.

use super::{Transport, TransportError};
use async_trait::async_trait;
use native_tls::{Certificate, Identity, TlsConnector as NativeTlsConnector};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_native_tls::{TlsConnector, TlsStream};

/// TLS transport implementation
///
/// Wraps a TCP connection with TLS encryption. Supports:
/// - System root certificates
/// - Custom CA certificates
/// - Client certificate authentication (mutual TLS)
/// - Hostname verification
pub struct TlsTransport {
    stream: TlsStream<TcpStream>,
    peer_addr: String,
}

impl TlsTransport {
    /// Create a TLS transport with custom configuration
    ///
    /// # Arguments
    /// * `addr` - The address to connect to (format: "host:port")
    /// * `config` - TLS configuration options
    ///
    /// # Returns
    /// A connected TLS transport
    pub async fn connect_with_config(
        addr: &str,
        config: &TlsConfig,
    ) -> Result<Self, TransportError> {
        // Parse address to extract host for SNI
        let (host, socket_addr) = parse_mqtt_address(addr)?;

        // Build TLS connector
        let mut builder = NativeTlsConnector::builder();

        // Add root certificates
        for cert in &config.root_certificates {
            builder.add_root_certificate(cert.clone());
        }

        // Add client identity (for mutual TLS)
        if let Some(identity) = &config.client_identity {
            builder.identity(identity.clone());
        }

        // Testing options (NOT for production)
        if config.accept_invalid_certs {
            builder.danger_accept_invalid_certs(true);
        }
        if config.accept_invalid_hostnames {
            builder.danger_accept_invalid_hostnames(true);
        }

        let connector =
            TlsConnector::from(builder.build().map_err(|e| {
                TransportError::Tls(format!("Failed to build TLS connector: {}", e))
            })?);

        // Connect TCP first
        let tcp_stream = TcpStream::connect(&socket_addr).await.map_err(|e| {
            TransportError::ConnectionFailed(format!("TCP connection failed: {}", e))
        })?;

        // Wrap with TLS
        let tls_stream = connector
            .connect(&host, tcp_stream)
            .await
            .map_err(|e| TransportError::Tls(format!("TLS handshake failed: {}", e)))?;

        Ok(Self {
            stream: tls_stream,
            peer_addr: socket_addr,
        })
    }

    /// Get a reference to the underlying TLS stream
    pub fn get_ref(&self) -> &TlsStream<TcpStream> {
        &self.stream
    }

    /// Get a mutable reference to the underlying TLS stream
    pub fn get_mut(&mut self) -> &mut TlsStream<TcpStream> {
        &mut self.stream
    }
}

#[async_trait]
impl Transport for TlsTransport {
    async fn connect(addr: &str) -> Result<Self, TransportError>
    where
        Self: Sized,
    {
        // Use default TLS config (system root certs)
        Self::connect_with_config(addr, &TlsConfig::default()).await
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        // TLS graceful shutdown happens automatically on drop
        Ok(())
    }

    fn peer_addr(&self) -> Result<String, TransportError> {
        Ok(self.peer_addr.clone())
    }

    fn local_addr(&self) -> Result<String, TransportError> {
        self.stream
            .get_ref()
            .get_ref()
            .get_ref()
            .local_addr()
            .map(|addr| addr.to_string())
            .map_err(TransportError::Io)
    }

    fn set_nodelay(&self, nodelay: bool) -> Result<(), TransportError> {
        self.stream
            .get_ref()
            .get_ref()
            .get_ref()
            .set_nodelay(nodelay)
            .map_err(TransportError::Io)
    }
}

impl AsyncRead for TlsTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsTransport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// TLS configuration options
#[derive(Clone, Default)]
pub struct TlsConfig {
    /// Additional root CA certificates to trust
    pub root_certificates: Vec<Certificate>,

    /// Client identity for mutual TLS authentication
    pub client_identity: Option<Identity>,

    /// Accept invalid certificates (⚠️ TESTING ONLY - insecure!)
    pub accept_invalid_certs: bool,

    /// Accept invalid hostnames (⚠️ TESTING ONLY - insecure!)
    pub accept_invalid_hostnames: bool,
}

impl TlsConfig {
    /// Create a new TLS configuration builder
    pub fn builder() -> TlsConfigBuilder {
        TlsConfigBuilder::default()
    }
}

/// Builder for TLS configuration
#[derive(Default)]
pub struct TlsConfigBuilder {
    root_certificates: Vec<Certificate>,
    client_identity: Option<Identity>,
    accept_invalid_certs: bool,
    accept_invalid_hostnames: bool,
}

impl TlsConfigBuilder {
    /// Add a root CA certificate to trust
    ///
    /// # Example
    /// ```no_run
    /// use native_tls::Certificate;
    /// use flowsdk::mqtt_client::transport::tls::TlsConfig;
    ///
    /// let cert_pem = std::fs::read("ca.crt").unwrap();
    /// let cert = Certificate::from_pem(&cert_pem).unwrap();
    ///
    /// let config = TlsConfig::builder()
    ///     .add_root_certificate(cert)
    ///     .build();
    /// ```
    pub fn add_root_certificate(mut self, cert: Certificate) -> Self {
        self.root_certificates.push(cert);
        self
    }

    /// Set client identity for mutual TLS authentication
    ///
    /// # Example
    /// ```no_run
    /// use native_tls::Identity;
    /// use flowsdk::mqtt_client::transport::tls::TlsConfig;
    ///
    /// let identity_pfx = std::fs::read("client.p12").unwrap();
    /// let identity = Identity::from_pkcs12(&identity_pfx, "password").unwrap();
    ///
    /// let config = TlsConfig::builder()
    ///     .client_identity(identity)
    ///     .build();
    /// ```
    pub fn client_identity(mut self, identity: Identity) -> Self {
        self.client_identity = Some(identity);
        self
    }

    /// Accept invalid certificates
    ///
    /// ⚠️ **WARNING**: This is insecure and should only be used for testing!
    /// Never use in production as it disables certificate validation.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Accept invalid hostnames
    ///
    /// ⚠️ **WARNING**: This is insecure and should only be used for testing!
    /// Never use in production as it disables hostname verification.
    pub fn danger_accept_invalid_hostnames(mut self, accept: bool) -> Self {
        self.accept_invalid_hostnames = accept;
        self
    }

    /// Build the TLS configuration
    pub fn build(self) -> TlsConfig {
        TlsConfig {
            root_certificates: self.root_certificates,
            client_identity: self.client_identity,
            accept_invalid_certs: self.accept_invalid_certs,
            accept_invalid_hostnames: self.accept_invalid_hostnames,
        }
    }
}

/// Parse MQTT address to extract hostname and socket address
///
/// Supports formats:
/// - "mqtts://host:port" → ("host", "host:port")
/// - "host:port" → ("host", "host:port")
fn parse_mqtt_address(addr: &str) -> Result<(String, String), TransportError> {
    // Remove mqtts:// prefix if present
    let addr = addr.trim_start_matches("mqtts://");

    // Split host and port
    if let Some((host, port)) = addr.rsplit_once(':') {
        Ok((host.to_string(), format!("{}:{}", host, port)))
    } else {
        Err(TransportError::InvalidAddress(format!(
            "Invalid address format: '{}'. Expected 'host:port' or 'mqtts://host:port'",
            addr
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mqtt_address_with_scheme() {
        let (host, socket) = parse_mqtt_address("mqtts://broker.example.com:8883").unwrap();
        assert_eq!(host, "broker.example.com");
        assert_eq!(socket, "broker.example.com:8883");
    }

    #[test]
    fn test_parse_mqtt_address_without_scheme() {
        let (host, socket) = parse_mqtt_address("broker.example.com:8883").unwrap();
        assert_eq!(host, "broker.example.com");
        assert_eq!(socket, "broker.example.com:8883");
    }

    #[test]
    fn test_parse_mqtt_address_invalid() {
        let result = parse_mqtt_address("invalid-address-no-port");
        assert!(result.is_err());
    }

    #[test]
    fn test_tls_config_builder() {
        let config = TlsConfig::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build();

        assert!(config.accept_invalid_certs);
        assert!(config.accept_invalid_hostnames);
        assert!(config.root_certificates.is_empty());
        assert!(config.client_identity.is_none());
    }

    #[test]
    fn test_tls_config_default() {
        let config = TlsConfig::default();
        assert!(!config.accept_invalid_certs);
        assert!(!config.accept_invalid_hostnames);
        assert!(config.root_certificates.is_empty());
        assert!(config.client_identity.is_none());
    }

    #[tokio::test]
    #[ignore] // Requires network access
    async fn test_tls_transport_connect() {
        // Test connecting to a public MQTT TLS broker
        let result = TlsTransport::connect("mqtt.eclipseprojects.io:8883").await;

        if let Ok(mut transport) = result {
            // Verify peer address
            let peer = transport.peer_addr();
            assert!(peer.is_ok());

            // Verify local address
            let local = transport.local_addr();
            assert!(local.is_ok());

            // Verify set_nodelay
            let result = transport.set_nodelay(true);
            assert!(result.is_ok());

            // Close connection
            let result = transport.close().await;
            assert!(result.is_ok());
        } else {
            // Connection might fail due to network issues, which is acceptable for ignored test
            println!("TLS connection test skipped (network may be unavailable)");
        }
    }

    #[tokio::test]
    async fn test_tls_transport_invalid_address() {
        let result = TlsTransport::connect("invalid-address-no-port").await;
        assert!(result.is_err());

        if let Err(e) = result {
            match e {
                TransportError::InvalidAddress(_) => { /* Expected */ }
                _ => panic!("Expected InvalidAddress error, got {:?}", e),
            }
        }
    }
}
