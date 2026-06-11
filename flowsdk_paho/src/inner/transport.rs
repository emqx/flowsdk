// SPDX-License-Identifier: MPL-2.0
//! Blocking TCP and TLS transport for the Paho compatibility layer.
//!
//! Uses `std::net::TcpStream` for TCP and `native_tls` for TLS.
//! All I/O is blocking — designed for use on the dedicated I/O thread.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::client_state::TlsConnectorHandle;
use crate::common::uri_parser::{ParsedUri, TransportType};

/// Resolve a `host:port` string to a `SocketAddr` via DNS and connect with timeout.
fn tcp_connect(addr: &str, timeout: Duration) -> io::Result<TcpStream> {
    // DNS resolution: ToSocketAddrs handles hostnames (unlike SocketAddr::parse).
    let mut last_err = io::Error::new(io::ErrorKind::Other, "no addresses resolved");
    let socket_addrs = addr.to_socket_addrs().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Failed to resolve '{}': {}", addr, e),
        )
    })?;

    for socket_addr in socket_addrs {
        match TcpStream::connect_timeout(&socket_addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = e,
        }
    }

    Err(last_err)
}

/// A transport that supports both TCP and TLS, using dynamic dispatch.
pub enum BlockingTransport {
    Tcp(TcpStream),
    #[cfg(feature = "tls")]
    Tls(native_tls::TlsStream<TcpStream>),
}

impl BlockingTransport {
    /// Connect to the given parsed URI.
    ///
    /// For TCP, creates a `TcpStream`. For TLS, wraps it in `native_tls::TlsStream`,
    /// using `tls_connector` if supplied (built from `MQTTClient_SSLOptions`) or a
    /// default connector otherwise. The `connect_timeout` applies to the TCP phase.
    pub fn connect(
        uri: &ParsedUri,
        connect_timeout: Duration,
        tls_connector: Option<TlsConnectorHandle>,
    ) -> io::Result<Self> {
        match uri.transport {
            TransportType::Tcp => {
                let stream = tcp_connect(&uri.addr, connect_timeout)?;
                stream.set_nodelay(true)?;
                Ok(BlockingTransport::Tcp(stream))
            }
            TransportType::Tls => {
                #[cfg(feature = "tls")]
                {
                    let tcp_stream = tcp_connect(&uri.addr, connect_timeout)?;
                    tcp_stream.set_nodelay(true)?;

                    // Use the caller-supplied connector (from SSL options) or a default.
                    let connector = match tls_connector {
                        Some(c) => c,
                        None => std::sync::Arc::new(native_tls::TlsConnector::new().map_err(
                            |e| {
                                io::Error::new(
                                    io::ErrorKind::Other,
                                    format!("TLS connector error: {}", e),
                                )
                            },
                        )?),
                    };

                    let tls_stream = connector.connect(&uri.hostname, tcp_stream).map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            format!("TLS handshake failed: {}", e),
                        )
                    })?;

                    Ok(BlockingTransport::Tls(tls_stream))
                }
                #[cfg(not(feature = "tls"))]
                {
                    let _ = tls_connector;
                    Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "TLS support not compiled in (enable 'tls' feature)",
                    ))
                }
            }
        }
    }

    /// Set read timeout on the underlying socket.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            BlockingTransport::Tcp(stream) => stream.set_read_timeout(timeout),
            #[cfg(feature = "tls")]
            BlockingTransport::Tls(stream) => stream.get_ref().set_read_timeout(timeout),
        }
    }

    /// Set write timeout on the underlying socket.
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            BlockingTransport::Tcp(stream) => stream.set_write_timeout(timeout),
            #[cfg(feature = "tls")]
            BlockingTransport::Tls(stream) => stream.get_ref().set_write_timeout(timeout),
        }
    }

    /// Try to clone the underlying TCP stream for use with `mio::Poll`.
    /// Returns the raw fd/socket for registering with the poll.
    #[cfg(unix)]
    pub fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd;
        match self {
            BlockingTransport::Tcp(stream) => stream.as_raw_fd(),
            #[cfg(feature = "tls")]
            BlockingTransport::Tls(stream) => stream.get_ref().as_raw_fd(),
        }
    }

    /// Shutdown the transport.
    pub fn shutdown(&self) -> io::Result<()> {
        match self {
            BlockingTransport::Tcp(stream) => stream.shutdown(std::net::Shutdown::Both),
            #[cfg(feature = "tls")]
            BlockingTransport::Tls(stream) => stream.get_ref().shutdown(std::net::Shutdown::Both),
        }
    }
}

// ─── TLS connector construction from Paho SSL options ────────────────────

/// Outcome of trying to build a TLS connector from `MQTTClient_SSLOptions`.
pub enum TlsSetupError {
    /// SSL options were supplied but the library was built without TLS support.
    NotSupported,
    /// The `MQTTClient_SSLOptions` struct failed validation.
    BadStructure,
    /// Certificate/key loading or connector construction failed.
    Failed(String),
}

/// Build an optional TLS connector handle from a (possibly null) pointer to
/// `MQTTClient_SSLOptions`. `Ok(None)` means no SSL options were supplied.
///
/// # Safety
/// `ssl` must be null or point to a valid `MQTTClient_SSLOptions`.
pub unsafe fn tls_handle_from_options(
    ssl: *const crate::common::structs::MQTTClient_SSLOptions,
) -> Result<Option<TlsConnectorHandle>, TlsSetupError> {
    if ssl.is_null() {
        return Ok(None);
    }
    let opts = &*ssl;
    if !opts.validate() {
        return Err(TlsSetupError::BadStructure);
    }
    #[cfg(feature = "tls")]
    {
        let connector = build_tls_connector(opts).map_err(TlsSetupError::Failed)?;
        Ok(Some(std::sync::Arc::new(connector)))
    }
    #[cfg(not(feature = "tls"))]
    {
        Err(TlsSetupError::NotSupported)
    }
}


/// Build a `native_tls::TlsConnector` from a Paho `MQTTClient_SSLOptions`.
///
/// Maps the supported fields:
/// - `trustStore`        → additional root CA certificate(s) (PEM, may hold many)
/// - `keyStore`/`privateKey` → client identity (PEM cert chain + PKCS#8 key)
/// - `enableServerCertAuth = 0` → accept invalid server certificates
/// - `verify = 0` (struct_version ≥ 1) → accept mismatched hostnames
/// - `sslVersion`        → minimum protocol version (0 default, 1 TLS1.0, 2 TLS1.1, 3 TLS1.2)
///
/// `enabledCipherSuites` and `CApath` are not expressible through `native-tls`
/// and are ignored (a warning is printed for `CApath`). Returns an error string
/// on file-read or certificate-parse failure.
#[cfg(feature = "tls")]
pub fn build_tls_connector(
    ssl: &crate::common::structs::MQTTClient_SSLOptions,
) -> Result<native_tls::TlsConnector, String> {
    use native_tls::{Certificate, Identity, Protocol, TlsConnector};

    let mut builder = TlsConnector::builder();

    // Root CA trust store (PEM file, possibly containing multiple certificates).
    if let Some(path) = unsafe { cstr_to_opt(ssl.trustStore) } {
        let pem = std::fs::read(&path)
            .map_err(|e| format!("trustStore '{}': {}", path, e))?;
        let mut added = 0;
        for block in split_pem_certificates(&pem) {
            match Certificate::from_pem(&block) {
                Ok(cert) => {
                    builder.add_root_certificate(cert);
                    added += 1;
                }
                Err(e) => return Err(format!("parsing trustStore '{}': {}", path, e)),
            }
        }
        if added == 0 {
            return Err(format!("trustStore '{}' contained no certificates", path));
        }
    }

    // Client identity: certificate chain (keyStore) + private key (privateKey).
    let key_store = unsafe { cstr_to_opt(ssl.keyStore) };
    let private_key = unsafe { cstr_to_opt(ssl.privateKey) };
    if let (Some(cert_path), Some(key_path)) = (key_store.as_ref(), private_key.as_ref()) {
        let cert_pem = std::fs::read(cert_path)
            .map_err(|e| format!("keyStore '{}': {}", cert_path, e))?;
        let key_pem = std::fs::read(key_path)
            .map_err(|e| format!("privateKey '{}': {}", key_path, e))?;
        let identity = Identity::from_pkcs8(&cert_pem, &key_pem).map_err(|e| {
            format!(
                "building client identity from '{}' + '{}': {} \
                 (note: encrypted private keys are not supported)",
                cert_path, key_path, e
            )
        })?;
        builder.identity(identity);
    } else if key_store.is_some() != private_key.is_some() {
        return Err(
            "both keyStore and privateKey must be set together for a client certificate"
                .to_string(),
        );
    }

    // Server certificate / hostname verification toggles.
    if ssl.enableServerCertAuth == 0 {
        builder.danger_accept_invalid_certs(true);
    }
    if ssl.struct_version >= 1 && ssl.verify == 0 {
        builder.danger_accept_invalid_hostnames(true);
    }

    // Minimum TLS protocol version.
    let min = match ssl.sslVersion {
        1 => Some(Protocol::Tlsv10),
        2 => Some(Protocol::Tlsv11),
        3 => Some(Protocol::Tlsv12),
        _ => None, // 0 = library default
    };
    if min.is_some() {
        builder.min_protocol_version(min);
    }

    if ssl.struct_version >= 2 && !ssl.CApath.is_null() {
        eprintln!("flowsdk_paho: SSLOptions.CApath is not supported by native-tls; ignoring");
    }

    builder
        .build()
        .map_err(|e| format!("building TLS connector: {}", e))
}

/// Read an optional C string pointer into an owned `String`.
#[cfg(feature = "tls")]
unsafe fn cstr_to_opt(ptr: *const libc::c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

/// Split a PEM blob into individual `-----BEGIN CERTIFICATE-----` … blocks so
/// each can be parsed separately (`native_tls::Certificate::from_pem` reads one).
#[cfg(feature = "tls")]
fn split_pem_certificates(pem: &[u8]) -> Vec<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let text = String::from_utf8_lossy(pem);
    let mut out = Vec::new();
    let mut rest = text.as_ref();
    while let Some(start) = rest.find(BEGIN) {
        if let Some(end_rel) = rest[start..].find(END) {
            let end = start + end_rel + END.len();
            out.push(rest[start..end].as_bytes().to_vec());
            rest = &rest[end..];
        } else {
            break;
        }
    }
    out
}

#[cfg(all(test, feature = "tls"))]
mod tls_tests {
    use super::*;
    use crate::common::structs::{MQTTClient_SSLOptions, MQTTCLIENT_SSL_STRUCT_ID};

    fn default_ssl() -> MQTTClient_SSLOptions {
        MQTTClient_SSLOptions {
            struct_id: MQTTCLIENT_SSL_STRUCT_ID,
            struct_version: 2,
            trustStore: std::ptr::null(),
            keyStore: std::ptr::null(),
            privateKey: std::ptr::null(),
            privateKeyPassword: std::ptr::null(),
            enabledCipherSuites: std::ptr::null(),
            enableServerCertAuth: 1,
            sslVersion: 0,
            verify: 1,
            CApath: std::ptr::null(),
        }
    }

    #[test]
    fn default_options_build_a_connector() {
        let ssl = default_ssl();
        assert!(build_tls_connector(&ssl).is_ok());
    }

    #[test]
    fn null_ssl_yields_no_handle() {
        let handle = unsafe { tls_handle_from_options(std::ptr::null()) };
        assert!(matches!(handle, Ok(None)));
    }

    #[test]
    fn missing_trust_store_file_errors() {
        let mut ssl = default_ssl();
        let path = std::ffi::CString::new("/nonexistent/ca-does-not-exist.pem").unwrap();
        ssl.trustStore = path.as_ptr();
        assert!(matches!(build_tls_connector(&ssl), Err(_)));
    }

    #[test]
    fn split_pem_counts_blocks() {
        let pem = b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n\
                    -----BEGIN CERTIFICATE-----\nBBBB\n-----END CERTIFICATE-----\n";
        assert_eq!(split_pem_certificates(pem).len(), 2);
        assert_eq!(split_pem_certificates(b"no certs here").len(), 0);
    }
}

impl Read for BlockingTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            BlockingTransport::Tcp(stream) => stream.read(buf),
            #[cfg(feature = "tls")]
            BlockingTransport::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for BlockingTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            BlockingTransport::Tcp(stream) => stream.write(buf),
            #[cfg(feature = "tls")]
            BlockingTransport::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            BlockingTransport::Tcp(stream) => stream.flush(),
            #[cfg(feature = "tls")]
            BlockingTransport::Tls(stream) => stream.flush(),
        }
    }
}
