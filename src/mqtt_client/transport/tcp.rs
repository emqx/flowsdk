// SPDX-License-Identifier: MPL-2.0

//! TCP transport implementation

use super::{Transport, TransportError};
use async_trait::async_trait;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// TCP transport implementation
///
/// This is a simple wrapper around `TcpStream` that implements the `Transport` trait.
pub struct TcpTransport {
    stream: TcpStream,
}

impl TcpTransport {
    /// Create a new TCP transport from an existing TcpStream
    pub fn from_stream(stream: TcpStream) -> Self {
        Self { stream }
    }

    /// Get a reference to the underlying TcpStream
    pub fn get_ref(&self) -> &TcpStream {
        &self.stream
    }

    /// Get a mutable reference to the underlying TcpStream
    pub fn get_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }

    /// Consume self and return the underlying TcpStream
    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

#[async_trait]
impl Transport for TcpTransport {
    async fn connect(addr: &str) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(addr).await.map_err(|e| {
            TransportError::ConnectionFailed(format!("TCP connection failed: {}", e))
        })?;
        Ok(Self { stream })
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        // Graceful shutdown - the stream will be dropped automatically
        Ok(())
    }

    fn peer_addr(&self) -> Result<String, TransportError> {
        self.stream
            .peer_addr()
            .map(|addr| addr.to_string())
            .map_err(TransportError::Io)
    }

    fn local_addr(&self) -> Result<String, TransportError> {
        self.stream
            .local_addr()
            .map(|addr| addr.to_string())
            .map_err(TransportError::Io)
    }

    fn set_nodelay(&self, nodelay: bool) -> Result<(), TransportError> {
        self.stream.set_nodelay(nodelay).map_err(TransportError::Io)
    }
}

impl AsyncRead for TcpTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(ctx, buf)
    }
}

impl AsyncWrite for TcpTransport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(ctx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(ctx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires external service
    async fn test_tcp_transport_connect() {
        // Test connecting to a public MQTT broker (non-TLS)
        let result = TcpTransport::connect("broker.emqx.io:1883").await;
        assert!(result.is_ok(), "Should connect to public broker");

        if let Ok(mut transport) = result {
            // Test peer address
            let peer = transport.peer_addr();
            assert!(peer.is_ok(), "Should get peer address");

            // Test local address
            let local = transport.local_addr();
            assert!(local.is_ok(), "Should get local address");

            // Test set_nodelay
            let result = transport.set_nodelay(true);
            assert!(result.is_ok(), "Should set TCP_NODELAY");

            // Test close
            let result = transport.close().await;
            assert!(result.is_ok(), "Should close connection");
        }
    }

    #[tokio::test]
    async fn test_tcp_transport_invalid_address() {
        let result = TcpTransport::connect("invalid-address-that-does-not-exist:1883").await;
        assert!(result.is_err(), "Should fail with invalid address");
    }

    #[tokio::test]
    async fn test_tcp_transport_connection_refused() {
        // Try to connect to localhost on a port that's likely not listening
        let result = TcpTransport::connect("127.0.0.1:65535").await;
        assert!(result.is_err(), "Should fail when connection is refused");
    }
}
