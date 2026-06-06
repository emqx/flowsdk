// SPDX-License-Identifier: MPL-2.0
//! Blocking TCP and TLS transport for the Paho compatibility layer.
//!
//! Uses `std::net::TcpStream` for TCP and `native_tls` for TLS.
//! All I/O is blocking — designed for use on the dedicated I/O thread.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

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
    /// For TCP, creates a `TcpStream`. For TLS, wraps it in `native_tls::TlsStream`.
    /// The `connect_timeout` is applied to the TCP connect phase.
    pub fn connect(uri: &ParsedUri, connect_timeout: Duration) -> io::Result<Self> {
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

                    let connector = native_tls::TlsConnector::new().map_err(|e| {
                        io::Error::new(io::ErrorKind::Other, format!("TLS connector error: {}", e))
                    })?;

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
