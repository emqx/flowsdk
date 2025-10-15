//! Transport abstraction layer for MQTT connections
//!
//! This module provides a clean abstraction for different connection types (TCP, TLS, WebSocket, etc.)
//! allowing the MQTT client to work with any underlying transport without modification.

use async_trait::async_trait;
use std::io;
use tokio::io::{AsyncRead, AsyncWrite};

pub mod tcp;

#[cfg(feature = "tls")]
pub mod tls;

#[cfg(feature = "quic")]
pub mod quic;

/// Error type for transport operations
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[cfg(feature = "tls")]
    #[error("TLS error: {0}")]
    Tls(String),

    #[cfg(feature = "quic")]
    #[error("QUIC error: {0}")]
    Quic(String),

    #[error("Transport not supported: {0}")]
    NotSupported(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),
}

/// Transport trait for different connection types
///
/// This trait provides a unified interface for TCP, TLS, WebSocket, and other
/// transport protocols. All transports must implement AsyncRead and AsyncWrite
/// for compatibility with existing I/O code.
#[async_trait]
pub trait Transport: AsyncRead + AsyncWrite + Send + Sync + Unpin {
    /// Connect to the specified address
    ///
    /// # Arguments
    /// * `addr` - The address to connect to (format depends on transport type)
    ///
    /// # Returns
    /// A new connected transport instance
    async fn connect(addr: &str) -> Result<Self, TransportError>
    where
        Self: Sized;

    /// Gracefully close the connection
    async fn close(&mut self) -> Result<(), TransportError>;

    /// Get the peer address as a string
    fn peer_addr(&self) -> Result<String, TransportError>;

    /// Get the local address as a string
    fn local_addr(&self) -> Result<String, TransportError>;

    /// Set TCP_NODELAY option (no-op for non-TCP transports)
    fn set_nodelay(&self, _nodelay: bool) -> Result<(), TransportError> {
        Ok(()) // Default implementation does nothing
    }
}

/// Boxed transport for dynamic dispatch
pub type BoxedTransport = Box<dyn Transport>;

// Re-export transport types
pub use tcp::TcpTransport;

#[cfg(feature = "tls")]
pub use tls::TlsTransport;
