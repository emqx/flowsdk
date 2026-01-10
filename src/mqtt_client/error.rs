// SPDX-License-Identifier: MPL-2.0

//! Error types for MQTT client operations
//!
//! This module provides a comprehensive error type system that distinguishes between
//! recoverable and unrecoverable errors, enabling intelligent error handling and retry logic.

use crate::mqtt_serde::parser::ParseError;
use std::fmt;
use std::io;

/// Comprehensive error type for MQTT client operations
#[derive(Debug, Clone, serde::Serialize)]
pub enum MqttClientError {
    // ==================== Connection Errors (Recoverable) ====================
    /// Connection refused by broker with reason code
    ConnectionRefused {
        reason_code: u8,
        description: String,
    },

    /// Connection lost unexpectedly
    ConnectionLost { reason: String },

    /// Network I/O error occurred
    NetworkError {
        #[serde(skip)]
        kind: io::ErrorKind,
        message: String,
    },

    // ==================== Protocol Errors (May be recoverable) ====================
    /// MQTT protocol violation detected
    ProtocolViolation { message: String },

    /// Failed to parse MQTT packet
    PacketParsing {
        parse_error: String,
        raw_data: Vec<u8>, // First 100 bytes for debugging
    },

    /// Invalid or unexpected packet identifier
    InvalidPacketId { packet_id: u16 },

    /// Received unexpected packet type
    UnexpectedPacket { expected: String, received: String },

    // ==================== Session Errors (Recoverable) ====================
    /// No active session available
    NoActiveSession,

    /// Session expired on broker
    SessionExpired,

    /// Packet ID space exhausted (all 65535 IDs in use)
    PacketIdExhausted,

    // ==================== Operation Errors (Recoverable) ====================
    /// Operation timed out waiting for response
    OperationTimeout { operation: String, timeout_ms: u64 },

    /// Operation was cancelled before completion
    OperationCancelled { operation: String },

    /// PUBLISH operation failed
    PublishFailed {
        packet_id: Option<u16>,
        reason_code: u8,
        reason_string: Option<String>,
    },

    /// SUBSCRIBE operation failed
    SubscribeFailed {
        topics: Vec<String>,
        reason_codes: Vec<u8>,
    },

    /// UNSUBSCRIBE operation failed
    UnsubscribeFailed {
        topics: Vec<String>,
        reason_codes: Vec<u8>,
    },

    // ==================== State Errors (Application logic) ====================
    /// Operation attempted in invalid connection state
    InvalidState { expected: String, actual: String },

    /// Not connected to broker
    NotConnected,

    /// Already connected to broker
    AlreadyConnected,

    // ==================== Resource Errors (May indicate system issues) ====================
    /// Buffer is full, cannot accept more data
    BufferFull {
        buffer_type: String,
        capacity: usize,
    },

    /// Internal channel closed unexpectedly
    ChannelClosed { channel: String },

    // ==================== Authentication Errors (Application logic) ====================
    /// Authentication failed
    AuthenticationFailed {
        method: Option<String>,
        reason_code: u8,
        reason_string: Option<String>,
    },

    // ==================== Configuration Errors (Unrecoverable) ====================
    /// Invalid client configuration
    InvalidConfiguration { field: String, reason: String },

    // ==================== Internal Errors (Unrecoverable) ====================
    /// Internal client error (should not happen)
    InternalError { message: String },
}

impl MqttClientError {
    /// Returns true if the error is recoverable (retry/reconnect possible)
    ///
    /// Recoverable errors are typically transient network issues, timeouts,
    /// or session-related problems that can be resolved by retrying or reconnecting.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Self::ConnectionLost { .. }
                | Self::NetworkError { .. }
                | Self::OperationTimeout { .. }
                | Self::SessionExpired
                | Self::NotConnected
                | Self::NoActiveSession
                | Self::BufferFull { .. }
        )
    }

    /// Returns true if the error should trigger automatic reconnection
    ///
    /// These are errors that indicate the connection to the broker is broken
    /// and can potentially be restored by reconnecting.
    pub fn should_reconnect(&self) -> bool {
        match self {
            Self::ConnectionLost { .. } => true,
            Self::NetworkError { kind, .. } => matches!(
                kind,
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionAborted
            ),
            Self::NotConnected => false, // Already disconnected
            _ => false,
        }
    }

    /// Returns true if the error is fatal (client should stop)
    ///
    /// Fatal errors indicate fundamental issues that cannot be resolved
    /// by retrying or reconnecting.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::InvalidConfiguration { .. }
                | Self::ProtocolViolation { .. }
                | Self::InternalError { .. }
        )
    }

    /// Returns true if the error is related to authentication
    pub fn is_auth_error(&self) -> bool {
        matches!(
            self,
            Self::AuthenticationFailed { .. }
                | Self::ConnectionRefused {
                    reason_code: 0x86 | 0x87 | 0x8C,
                    ..
                }
        )
    }

    /// Get a user-friendly error message
    ///
    /// This provides a clear, actionable message that can be displayed to users
    /// or logged for troubleshooting.
    pub fn user_message(&self) -> String {
        match self {
            Self::ConnectionRefused {
                reason_code,
                description,
            } => {
                format!(
                    "Connection refused by broker: {} (code: 0x{:02X})",
                    description, reason_code
                )
            }
            Self::ConnectionLost { reason } => {
                format!("Connection to broker lost: {}", reason)
            }
            Self::NetworkError { kind, message } => {
                format!("Network error ({:?}): {}", kind, message)
            }
            Self::ProtocolViolation { message } => {
                format!("MQTT protocol violation: {}", message)
            }
            Self::PacketParsing {
                parse_error,
                raw_data,
            } => {
                let data_preview = if raw_data.len() > 20 {
                    format!(
                        "{}... ({} bytes)",
                        hex::encode(&raw_data[..20]),
                        raw_data.len()
                    )
                } else {
                    hex::encode(raw_data)
                };
                format!(
                    "Failed to parse MQTT packet: {} (data: {})",
                    parse_error, data_preview
                )
            }
            Self::InvalidPacketId { packet_id } => {
                format!("Invalid packet identifier: {}", packet_id)
            }
            Self::UnexpectedPacket { expected, received } => {
                format!("Expected {} packet, received {}", expected, received)
            }
            Self::NoActiveSession => "No active session. Connect to broker first.".to_string(),
            Self::SessionExpired => "Session expired on broker. Reconnection required.".to_string(),
            Self::PacketIdExhausted => {
                "All packet identifiers are in use. Wait for pending operations to complete."
                    .to_string()
            }
            Self::OperationTimeout {
                operation,
                timeout_ms,
            } => {
                format!(
                    "Operation '{}' timed out after {} ms",
                    operation, timeout_ms
                )
            }
            Self::OperationCancelled { operation } => {
                format!("Operation '{}' was cancelled", operation)
            }
            Self::PublishFailed {
                packet_id,
                reason_code,
                reason_string,
            } => {
                let id_str = packet_id
                    .map(|id| format!(" (packet ID: {})", id))
                    .unwrap_or_default();
                let reason_str = reason_string
                    .as_ref()
                    .map(|s| format!(": {}", s))
                    .unwrap_or_default();
                format!(
                    "Publish failed{} - code: 0x{:02X}{}",
                    id_str, reason_code, reason_str
                )
            }
            Self::SubscribeFailed {
                topics,
                reason_codes,
            } => {
                format!(
                    "Subscribe failed for topics {:?} with reason codes: {:?}",
                    topics, reason_codes
                )
            }
            Self::UnsubscribeFailed {
                topics,
                reason_codes,
            } => {
                format!(
                    "Unsubscribe failed for topics {:?} with reason codes: {:?}",
                    topics, reason_codes
                )
            }
            Self::InvalidState { expected, actual } => {
                format!("Invalid state: expected {}, but was {}", expected, actual)
            }
            Self::NotConnected => "Not connected to broker. Call connect() first.".to_string(),
            Self::AlreadyConnected => "Already connected to broker.".to_string(),
            Self::BufferFull {
                buffer_type,
                capacity,
            } => {
                format!(
                    "{} buffer full (capacity: {}). Try again later.",
                    buffer_type, capacity
                )
            }
            Self::ChannelClosed { channel } => {
                format!("Internal channel '{}' closed unexpectedly", channel)
            }
            Self::AuthenticationFailed {
                method,
                reason_code,
                reason_string,
            } => {
                let method_str = method
                    .as_ref()
                    .map(|m| format!(" (method: {})", m))
                    .unwrap_or_default();
                let reason_str = reason_string
                    .as_ref()
                    .map(|s| format!(": {}", s))
                    .unwrap_or_default();
                format!(
                    "Authentication failed{} - code: 0x{:02X}{}",
                    method_str, reason_code, reason_str
                )
            }
            Self::InvalidConfiguration { field, reason } => {
                format!("Invalid configuration for '{}': {}", field, reason)
            }
            Self::InternalError { message } => {
                format!("Internal error: {}", message)
            }
        }
    }

    /// Convert from io::Error with context
    ///
    /// # Arguments
    /// * `error` - The IO error to convert
    /// * `context` - Contextual information about where the error occurred
    pub fn from_io_error(error: io::Error, context: &str) -> Self {
        Self::NetworkError {
            kind: error.kind(),
            message: format!("{}: {}", context, error),
        }
    }

    /// Convert from ParseError
    pub fn from_parse_error(error: ParseError) -> Self {
        Self::PacketParsing {
            parse_error: error.to_string(),
            raw_data: Vec::new(), // Will be populated by caller if needed
        }
    }

    /// Convert from ParseError with raw data
    pub fn from_parse_error_with_data(error: ParseError, raw_data: Vec<u8>) -> Self {
        Self::PacketParsing {
            parse_error: error.to_string(),
            raw_data,
        }
    }
}

impl fmt::Display for MqttClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

impl std::error::Error for MqttClientError {}

// Conversion from io::Error (without context)
impl From<io::Error> for MqttClientError {
    fn from(error: io::Error) -> Self {
        Self::NetworkError {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

// Conversion to io::Error for backward compatibility
impl From<MqttClientError> for io::Error {
    fn from(error: MqttClientError) -> Self {
        match error {
            // Map OperationTimeout to TimedOut
            MqttClientError::OperationTimeout {
                operation,
                timeout_ms,
            } => io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{} operation timed out after {}ms", operation, timeout_ms),
            ),

            // Map NetworkError back to io::Error (passthrough)
            MqttClientError::NetworkError { kind, message } => io::Error::new(kind, message),

            // Map ConnectionLost to ConnectionReset
            MqttClientError::ConnectionLost { reason } => {
                io::Error::new(io::ErrorKind::ConnectionReset, reason)
            }

            // Map NotConnected to NotConnected
            MqttClientError::NotConnected => io::Error::new(
                io::ErrorKind::NotConnected,
                "Client is not connected to broker",
            ),

            // Map ChannelClosed to BrokenPipe
            MqttClientError::ChannelClosed { channel } => io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("Channel '{}' was closed unexpectedly", channel),
            ),

            // Map BufferFull to WouldBlock (resource temporarily unavailable)
            MqttClientError::BufferFull {
                buffer_type,
                capacity,
            } => io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("{} buffer is full (capacity: {})", buffer_type, capacity),
            ),

            // Map InvalidConfiguration to InvalidInput
            MqttClientError::InvalidConfiguration { field, reason } => io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid configuration for '{}': {}", field, reason),
            ),

            // Map ProtocolViolation to InvalidData
            MqttClientError::ProtocolViolation { message } => {
                io::Error::new(io::ErrorKind::InvalidData, message)
            }

            // Map PacketParsing to InvalidData
            MqttClientError::PacketParsing {
                parse_error,
                raw_data: _,
            } => io::Error::new(io::ErrorKind::InvalidData, parse_error),

            // Map OperationCancelled to Interrupted
            MqttClientError::OperationCancelled { operation } => io::Error::new(
                io::ErrorKind::Interrupted,
                format!("{} operation was cancelled", operation),
            ),

            // Map all other errors to Other with the error's display message
            other => io::Error::other(other.to_string()),
        }
    }
}

// Conversion from ParseError
impl From<ParseError> for MqttClientError {
    fn from(error: ParseError) -> Self {
        Self::from_parse_error(error)
    }
}

/// Type alias for Result with MqttClientError
pub type MqttClientResult<T> = Result<T, MqttClientError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_recoverable() {
        let recoverable_errors = vec![
            MqttClientError::ConnectionLost {
                reason: "test".to_string(),
            },
            MqttClientError::NetworkError {
                kind: io::ErrorKind::ConnectionReset,
                message: "test".to_string(),
            },
            MqttClientError::OperationTimeout {
                operation: "test".to_string(),
                timeout_ms: 1000,
            },
            MqttClientError::SessionExpired,
            MqttClientError::NotConnected,
            MqttClientError::NoActiveSession,
        ];

        for error in recoverable_errors {
            assert!(
                error.is_recoverable(),
                "Expected {:?} to be recoverable",
                error
            );
        }
    }

    #[test]
    fn test_is_not_recoverable() {
        let unrecoverable_errors = vec![
            MqttClientError::InvalidConfiguration {
                field: "test".to_string(),
                reason: "test".to_string(),
            },
            MqttClientError::ProtocolViolation {
                message: "test".to_string(),
            },
            MqttClientError::InternalError {
                message: "test".to_string(),
            },
        ];

        for error in unrecoverable_errors {
            assert!(
                !error.is_recoverable(),
                "Expected {:?} to not be recoverable",
                error
            );
        }
    }

    #[test]
    fn test_should_reconnect() {
        let reconnect_errors = vec![
            MqttClientError::ConnectionLost {
                reason: "test".to_string(),
            },
            MqttClientError::NetworkError {
                kind: io::ErrorKind::ConnectionReset,
                message: "test".to_string(),
            },
            MqttClientError::NetworkError {
                kind: io::ErrorKind::BrokenPipe,
                message: "test".to_string(),
            },
        ];

        for error in reconnect_errors {
            assert!(
                error.should_reconnect(),
                "Expected {:?} to trigger reconnect",
                error
            );
        }
    }

    #[test]
    fn test_should_not_reconnect() {
        let no_reconnect_errors = vec![
            MqttClientError::OperationTimeout {
                operation: "test".to_string(),
                timeout_ms: 1000,
            },
            MqttClientError::NotConnected,
            MqttClientError::InvalidConfiguration {
                field: "test".to_string(),
                reason: "test".to_string(),
            },
        ];

        for error in no_reconnect_errors {
            assert!(
                !error.should_reconnect(),
                "Expected {:?} to not trigger reconnect",
                error
            );
        }
    }

    #[test]
    fn test_is_fatal() {
        let fatal_errors = vec![
            MqttClientError::InvalidConfiguration {
                field: "test".to_string(),
                reason: "test".to_string(),
            },
            MqttClientError::ProtocolViolation {
                message: "test".to_string(),
            },
            MqttClientError::InternalError {
                message: "test".to_string(),
            },
        ];

        for error in fatal_errors {
            assert!(error.is_fatal(), "Expected {:?} to be fatal", error);
        }
    }

    #[test]
    fn test_is_auth_error() {
        let auth_errors = vec![
            MqttClientError::AuthenticationFailed {
                method: Some("SCRAM".to_string()),
                reason_code: 0x86,
                reason_string: None,
            },
            MqttClientError::ConnectionRefused {
                reason_code: 0x86, // Bad authentication method
                description: "test".to_string(),
            },
            MqttClientError::ConnectionRefused {
                reason_code: 0x87, // Not authorized
                description: "test".to_string(),
            },
        ];

        for error in auth_errors {
            assert!(
                error.is_auth_error(),
                "Expected {:?} to be auth error",
                error
            );
        }
    }

    #[test]
    fn test_user_message() {
        let error = MqttClientError::OperationTimeout {
            operation: "connect".to_string(),
            timeout_ms: 5000,
        };
        assert_eq!(
            error.user_message(),
            "Operation 'connect' timed out after 5000 ms"
        );

        let error = MqttClientError::PublishFailed {
            packet_id: Some(42),
            reason_code: 0x80,
            reason_string: Some("Quota exceeded".to_string()),
        };
        assert_eq!(
            error.user_message(),
            "Publish failed (packet ID: 42) - code: 0x80: Quota exceeded"
        );
    }

    #[test]
    fn test_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionReset, "connection reset");
        let mqtt_err = MqttClientError::from_io_error(io_err, "write_to_transport");

        match mqtt_err {
            MqttClientError::NetworkError { kind, message } => {
                assert_eq!(kind, io::ErrorKind::ConnectionReset);
                assert!(message.contains("write_to_transport"));
            }
            _ => panic!("Expected NetworkError"),
        }
    }

    #[test]
    fn test_display() {
        let error = MqttClientError::NotConnected;
        assert_eq!(
            format!("{}", error),
            "Not connected to broker. Call connect() first."
        );
    }
}
