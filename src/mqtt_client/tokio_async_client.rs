// SPDX-License-Identifier: MPL-2.0

use std::io;
#[cfg(feature = "quic")]
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

#[cfg(feature = "quic")]
use super::transport::quic::{QuicConfig, QuicTransport};
#[cfg(feature = "tls")]
use super::transport::TlsTransport;
use super::transport::{BoxedTransport, TcpTransport, Transport};

use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::common::properties::Property;

use crate::mqtt_serde::parser::ParseError;

use super::client::{
    AuthResult, ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use super::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use super::engine::{MqttEngine, MqttEvent, MqttMessage};
use super::error::MqttClientError;
use super::opts::MqttClientOptions;

mod worker;
use worker::TokioClientWorker;

type Response<T> = oneshot::Sender<Result<T, MqttClientError>>;

async fn receive_response<T>(
    rx: oneshot::Receiver<Result<T, MqttClientError>>,
) -> Result<T, MqttClientError> {
    rx.await.map_err(|_| MqttClientError::ChannelClosed {
        channel: "async client response".into(),
    })?
}

/// Events that can occur during MQTT client operation
#[derive(Debug)]
pub enum TokioMqttEvent {
    /// Connection established with broker
    Connected(ConnectionResult),
    /// Disconnected from broker (reason code if available)
    Disconnected(Option<u8>),
    /// Message published successfully
    Published(PublishResult),
    /// Subscription completed
    Subscribed(SubscribeResult),
    /// Unsubscription completed
    Unsubscribed(UnsubscribeResult),
    /// Incoming message received from broker
    MessageReceived(MqttMessage),
    /// Ping response received
    PingResponse(PingResult),
    /// Error occurred during operation (enhanced with MqttClientError)
    Error(MqttClientError),
    /// TLS peer certificate received (for certificate validation)
    PeerCertReceived(Vec<u8>),
    /// Connection lost (will attempt to reconnect if enabled)
    ConnectionLost,
    /// Reconnection attempt started
    ReconnectAttempt(u32),
    /// All pending operations cleared (on reconnect with clean_start)
    PendingOperationsCleared,
    /// Parse error occurred with details
    ParseError {
        /// First 100 bytes of raw data for debugging
        raw_data: Vec<u8>,
        /// The parse error that occurred
        error: ParseError,
        /// Whether this error is recoverable
        recoverable: bool,
    },
    /// Authentication challenge or response received (MQTT v5 enhanced auth)
    AuthReceived(AuthResult),
}

/// Trait for handling MQTT events in async context
///
/// Callbacks run on the client's worker. Delegate acknowledgement-waiting and
/// completion-waiting calls to another task so the worker can keep processing I/O.
#[async_trait]
pub trait TokioMqttEventHandler: Send + Sync {
    /// Called when connection to broker is established
    async fn on_connected(&mut self, result: &ConnectionResult) {
        let _ = result;
    }

    /// Called when disconnected from broker
    async fn on_disconnected(&mut self, reason: Option<u8>) {
        let _ = reason;
    }

    /// Called for a broker MQTT 5 DISCONNECT before `on_disconnected`.
    async fn on_disconnect_received(&mut self, reason_code: u8, properties: &[Property]) {
        let _ = (reason_code, properties);
    }

    /// Called when an incoming QoS 2 exchange reaches PUBREL.
    /// With manual acknowledgements, arrange a `pubcomp` call from another task.
    async fn on_pubrel_received(&mut self, packet_id: u16) {
        let _ = packet_id;
    }

    /// Called when a message is successfully published
    async fn on_published(&mut self, result: &PublishResult) {
        let _ = result;
    }

    /// Called when subscription is completed
    async fn on_subscribed(&mut self, result: &SubscribeResult) {
        let _ = result;
    }

    /// Called when unsubscription is completed
    async fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
        let _ = result;
    }

    /// Called when an incoming publish message is received
    async fn on_message_received(&mut self, publish: &MqttMessage) {
        let _ = publish;
    }

    /// Called when ping response is received
    async fn on_ping_response(&mut self, result: &PingResult) {
        let _ = result;
    }

    /// Called when an error occurs (enhanced with MqttClientError)
    async fn on_error(&mut self, error: &MqttClientError) {
        let _ = error;
    }

    /// Called when a parse error occurs with raw packet data for debugging
    async fn on_parse_error(&mut self, raw_data: &[u8], error: &ParseError, recoverable: bool) {
        let _ = (raw_data, error, recoverable);
    }

    /// Called when TLS peer certificate is received (for custom validation)
    async fn on_peer_cert_received(&mut self, cert: &[u8]) {
        let _ = cert;
    }

    /// Called when connection is lost unexpectedly
    async fn on_connection_lost(&mut self) {}

    /// Called when attempting to reconnect
    async fn on_reconnect_attempt(&mut self, attempt: u32) {
        let _ = attempt;
    }

    /// Called when pending operations are cleared (usually on reconnect)
    async fn on_pending_operations_cleared(&mut self) {}

    /// Called when AUTH packet is received from broker (MQTT v5 enhanced authentication)
    /// This is used for multi-step authentication like SCRAM, OAuth, or custom auth
    async fn on_auth_received(&mut self, result: &AuthResult) {
        let _ = result;
    }
}

/// Commands that can be sent to the async worker
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum TokioClientCommand {
    /// Connect to the broker
    Connect,
    /// Connect to the broker and wait for acknowledgment
    ConnectSync {
        response_tx: tokio::sync::oneshot::Sender<Result<ConnectionResult, MqttClientError>>,
    },
    /// Subscribe to topics
    Subscribe(SubscribeCommand),
    /// Subscribe to topics and wait for acknowledgment
    SubscribeSync {
        command: SubscribeCommand,
        response_tx: Response<SubscribeResult>,
    },
    /// Publish a message (fire-and-forget)
    Publish(PublishCommand),
    /// Publish a message and wait for acknowledgment (QoS 1 = PUBACK, QoS 2 = PUBCOMP)
    PublishSync {
        command: PublishCommand,
        response_tx: Response<PublishResult>,
    },
    /// Unsubscribe from topics
    Unsubscribe(UnsubscribeCommand),
    /// Unsubscribe from topics and wait for acknowledgment
    UnsubscribeSync {
        command: UnsubscribeCommand,
        response_tx: Response<UnsubscribeResult>,
    },
    /// Send ping to broker
    Ping,
    /// Send ping to broker and wait for response
    PingSync { response_tx: Response<PingResult> },
    /// Disconnect from broker
    Disconnect {
        reason_code: u8,
        properties: Vec<Property>,
        response_tx: Option<Response<()>>,
    },
    Acknowledge {
        kind: u8,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
        response_tx: Response<()>,
    },
    /// Shutdown the client and worker
    Shutdown,
    /// Enable/disable automatic reconnection
    SetAutoReconnect { enabled: bool },
    /// Send AUTH packet for enhanced authentication (MQTT v5)
    Auth {
        reason_code: u8,
        properties: Vec<Property>,
    },
    /// Send a raw MQTT packet
    SendPacket(MqttPacket),
}

/// Configuration for the tokio async client
#[derive(Debug, Clone)]
pub struct TokioAsyncClientConfig {
    /// Enable automatic reconnection on connection loss
    pub auto_reconnect: bool,
    /// Maximum reconnect delay in milliseconds
    pub max_reconnect_delay_ms: u64,
    /// Maximum number of reconnect attempts (0 = infinite)
    pub max_reconnect_attempts: u32,
    /// Queue size for pending commands
    pub command_queue_size: usize,
    /// Enable buffering of messages during disconnection
    pub buffer_messages: bool,
    /// Maximum size of message buffer
    pub max_buffer_size: usize,

    /// Enable TCP_NODELAY (disable Nagle) on the underlying socket
    pub tcp_nodelay: bool,
    /// Timeout for connect operation in milliseconds (None = no timeout)
    pub connect_timeout_ms: Option<u64>,
    /// Timeout for subscribe operation in milliseconds (None = no timeout)
    pub subscribe_timeout_ms: Option<u64>,
    /// Timeout for publish acknowledgment in milliseconds (None = no timeout)
    pub publish_ack_timeout_ms: Option<u64>,
    /// Timeout for unsubscribe operation in milliseconds (None = no timeout)
    pub unsubscribe_timeout_ms: Option<u64>,
    /// Timeout for ping operation in milliseconds (None = no timeout)
    pub ping_timeout_ms: Option<u64>,
    /// Bound for transport writes/closes and local completion waits (milliseconds).
    /// Must be nonzero, including when broker response timeouts are disabled.
    pub default_operation_timeout_ms: u64,
    /// Maximum number of QoS 1 and QoS 2 publications that the client
    /// is willing to process concurrently (MQTT v5 only)
    /// None preserves core options (default 65535); Some(0) is invalid.
    pub receive_maximum: Option<u16>,
    /// Maximum number of topic aliases that the client accepts from the server (MQTT v5 only)
    /// None preserves core CONNECT properties; Some(0) disables aliases.
    /// Valid range: 0-65535
    pub topic_alias_maximum: Option<u16>,
    /// QUIC transport: Enable 0-RTT (early data) for faster reconnections
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_enable_0rtt: bool,
    /// QUIC transport: Skip TLS certificate verification (⚠️ DANGEROUS - testing only!)
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_insecure_skip_verify: bool,
    /// QUIC transport: Custom root CA certificates in PEM format
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_custom_root_ca_pem: Option<String>,
    /// QUIC transport: Client certificate chain in PEM format for mTLS
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_client_cert_pem: Option<String>,
    /// QUIC transport: Client private key in PEM format for mTLS
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_client_key_pem: Option<String>,
    /// QUIC transport: Datagram receive buffer size in bytes (0 = disable datagrams)
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_datagram_receive_buffer_size: usize,
    /// QUIC transport: Local UDP address to bind before connecting
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_local_bind_addr: Option<SocketAddr>,
    /// QUIC transport: Enable TLS key logging (writes to SSLKEYLOGFILE)
    /// Only used when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub quic_enable_key_log: bool,
    /// Rustls TLS transport: Enable TLS key logging (writes to SSLKEYLOGFILE)
    /// Only used when connecting via mqtts:// URLs with rustls-tls backend
    #[cfg(feature = "rustls-tls")]
    pub tls_enable_key_log: bool,
    /// Enable priority queue for QoS-based message prioritization
    pub priority_queue_enabled: bool,
    /// Maximum size of the priority queue
    pub priority_queue_limit: usize,
}

impl Default for TokioAsyncClientConfig {
    fn default() -> Self {
        TokioAsyncClientConfig {
            auto_reconnect: true,
            max_reconnect_delay_ms: 30000,
            max_reconnect_attempts: 0, // infinite
            command_queue_size: 1000,
            buffer_messages: true,
            max_buffer_size: 1000,
            tcp_nodelay: true,
            connect_timeout_ms: Some(30000),     // 30 seconds
            subscribe_timeout_ms: Some(10000),   // 10 seconds
            publish_ack_timeout_ms: Some(10000), // 10 seconds
            unsubscribe_timeout_ms: Some(10000), // 10 seconds
            ping_timeout_ms: Some(5000),         // 5 seconds
            default_operation_timeout_ms: 30000, // 30 seconds
            receive_maximum: None,               // Use MQTT v5 default (65535)
            topic_alias_maximum: None,           // Topic aliases not supported by default
            #[cfg(feature = "quic")]
            quic_enable_0rtt: false, // Disable 0-RTT by default for safety
            #[cfg(feature = "quic")]
            quic_insecure_skip_verify: false, // Enable certificate verification by default
            #[cfg(feature = "quic")]
            quic_custom_root_ca_pem: None, // Use system root CAs by default
            #[cfg(feature = "quic")]
            quic_client_cert_pem: None, // No client cert by default
            #[cfg(feature = "quic")]
            quic_client_key_pem: None, // No client key by default
            #[cfg(feature = "quic")]
            quic_datagram_receive_buffer_size: 0, // Disable datagram
            #[cfg(feature = "quic")]
            quic_local_bind_addr: None, // Use OS-selected source address by default
            #[cfg(feature = "quic")]
            quic_enable_key_log: false, // Disable key logging by default
            #[cfg(feature = "rustls-tls")]
            tls_enable_key_log: false, // Disable key logging by default
            priority_queue_enabled: true,
            priority_queue_limit: 1000,
        }
    }
}

impl TokioAsyncClientConfig {
    /// Create a new configuration builder
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::tokio_async_client::TokioAsyncClientConfig;
    ///
    /// let config = TokioAsyncClientConfig::builder()
    ///     .auto_reconnect(true)
    ///     .max_reconnect_delay_ms(2000)
    ///     .max_reconnect_attempts(10)
    ///     .build();
    /// ```
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }
}

/// Builder for TokioAsyncClientConfig
///
/// Provides a fluent API for constructing client configuration with sensible defaults.
///
/// # Example
/// ```no_run
/// use flowsdk::mqtt_client::tokio_async_client::TokioAsyncClientConfig;
///
/// let config = TokioAsyncClientConfig::builder()
///     .auto_reconnect(true)
///     .max_reconnect_delay_ms(2000)
///     .max_reconnect_attempts(10)
///     .connect_timeout_ms(60000)
///     .subscribe_timeout_ms(5000)
///     .tcp_nodelay(false)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    config: TokioAsyncClientConfig,
}

impl ConfigBuilder {
    /// Create a new builder with default values
    pub fn new() -> Self {
        ConfigBuilder {
            config: TokioAsyncClientConfig::default(),
        }
    }

    /// Build the final configuration
    pub fn build(self) -> TokioAsyncClientConfig {
        self.config
    }

    // ==================== Reconnection Settings ====================

    /// Enable or disable automatic reconnection on connection loss
    pub fn auto_reconnect(mut self, enabled: bool) -> Self {
        self.config.auto_reconnect = enabled;
        self
    }

    /// Set maximum reconnect delay in milliseconds
    pub fn max_reconnect_delay_ms(mut self, max_delay_ms: u64) -> Self {
        self.config.max_reconnect_delay_ms = max_delay_ms;
        self
    }

    /// Set maximum number of reconnect attempts (0 = infinite)
    pub fn max_reconnect_attempts(mut self, max_attempts: u32) -> Self {
        self.config.max_reconnect_attempts = max_attempts;
        self
    }

    /// Enable or disable priority queue
    pub fn priority_queue_enabled(mut self, enabled: bool) -> Self {
        self.config.priority_queue_enabled = enabled;
        self
    }

    /// Set priority queue limit
    pub fn priority_queue_limit(mut self, limit: usize) -> Self {
        self.config.priority_queue_limit = limit;
        self
    }

    // ==================== Timeout Settings ====================

    /// Set timeout for connect operation in milliseconds
    pub fn connect_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.connect_timeout_ms = Some(timeout_ms);
        self
    }

    /// Disable timeout for connect operation (wait indefinitely)
    pub fn no_connect_timeout(mut self) -> Self {
        self.config.connect_timeout_ms = None;
        self
    }

    /// Set timeout for subscribe operation in milliseconds
    pub fn subscribe_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.subscribe_timeout_ms = Some(timeout_ms);
        self
    }

    /// Disable timeout for subscribe operation (wait indefinitely)
    pub fn no_subscribe_timeout(mut self) -> Self {
        self.config.subscribe_timeout_ms = None;
        self
    }

    /// Set timeout for publish acknowledgment in milliseconds
    pub fn publish_ack_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.publish_ack_timeout_ms = Some(timeout_ms);
        self
    }

    /// Disable timeout for publish acknowledgment (wait indefinitely)
    pub fn no_publish_ack_timeout(mut self) -> Self {
        self.config.publish_ack_timeout_ms = None;
        self
    }

    /// Set timeout for unsubscribe operation in milliseconds
    pub fn unsubscribe_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.unsubscribe_timeout_ms = Some(timeout_ms);
        self
    }

    /// Disable timeout for unsubscribe operation (wait indefinitely)
    pub fn no_unsubscribe_timeout(mut self) -> Self {
        self.config.unsubscribe_timeout_ms = None;
        self
    }

    /// Set timeout for ping operation in milliseconds
    pub fn ping_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.ping_timeout_ms = Some(timeout_ms);
        self
    }

    /// Disable timeout for ping operation (wait indefinitely)
    pub fn no_ping_timeout(mut self) -> Self {
        self.config.ping_timeout_ms = None;
        self
    }

    /// Set the nonzero timeout for transport I/O and local completion waits.
    pub fn default_operation_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.default_operation_timeout_ms = timeout_ms;
        self
    }

    // ==================== Buffer Settings ====================

    /// Set queue size for pending commands
    pub fn command_queue_size(mut self, size: usize) -> Self {
        self.config.command_queue_size = size;
        self
    }

    /// Enable or disable buffering of messages during disconnection
    pub fn buffer_messages(mut self, enabled: bool) -> Self {
        self.config.buffer_messages = enabled;
        self
    }

    /// Set maximum size of message buffer
    pub fn max_buffer_size(mut self, size: usize) -> Self {
        self.config.max_buffer_size = size;
        self
    }

    // ==================== Other Settings ====================

    /// Enable or disable TCP_NODELAY (disable Nagle algorithm)
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.config.tcp_nodelay = enabled;
        self
    }

    // ==================== Convenience Methods ====================

    /// Disable wrapper broker-response timeouts and the transport-connect timeout.
    ///
    /// Explicit engine deadlines still apply. Transport writes, closes, and local
    /// completion waits remain bounded by `default_operation_timeout_ms`.
    pub fn no_timeouts(mut self) -> Self {
        self.config.connect_timeout_ms = None;
        self.config.subscribe_timeout_ms = None;
        self.config.publish_ack_timeout_ms = None;
        self.config.unsubscribe_timeout_ms = None;
        self.config.ping_timeout_ms = None;
        self
    }

    /// Reset all timeouts to their default values
    pub fn default_timeouts(mut self) -> Self {
        self.config.connect_timeout_ms = Some(30000);
        self.config.subscribe_timeout_ms = Some(10000);
        self.config.publish_ack_timeout_ms = Some(10000);
        self.config.unsubscribe_timeout_ms = Some(10000);
        self.config.ping_timeout_ms = Some(5000);
        self.config.default_operation_timeout_ms = 30000;
        self
    }

    /// Apply a timeout profile optimized for local networks
    ///
    /// - Connect: 5 seconds
    /// - Subscribe/Publish/Unsubscribe: 3 seconds
    /// - Ping: 2 seconds
    pub fn local_network_timeouts(mut self) -> Self {
        self.config.connect_timeout_ms = Some(5000);
        self.config.subscribe_timeout_ms = Some(3000);
        self.config.publish_ack_timeout_ms = Some(3000);
        self.config.unsubscribe_timeout_ms = Some(3000);
        self.config.ping_timeout_ms = Some(2000);
        self.config.default_operation_timeout_ms = 5000;
        self
    }

    /// Apply a timeout profile optimized for internet/cloud networks
    ///
    /// - Connect: 30 seconds
    /// - Subscribe/Publish/Unsubscribe: 10 seconds
    /// - Ping: 5 seconds
    pub fn internet_timeouts(mut self) -> Self {
        self.config.connect_timeout_ms = Some(30000);
        self.config.subscribe_timeout_ms = Some(10000);
        self.config.publish_ack_timeout_ms = Some(10000);
        self.config.unsubscribe_timeout_ms = Some(10000);
        self.config.ping_timeout_ms = Some(5000);
        self.config.default_operation_timeout_ms = 30000;
        self
    }

    /// Apply a timeout profile optimized for satellite/high-latency networks
    ///
    /// - Connect: 120 seconds
    /// - Subscribe/Publish/Unsubscribe: 60 seconds
    /// - Ping: 30 seconds
    pub fn satellite_timeouts(mut self) -> Self {
        self.config.connect_timeout_ms = Some(120000);
        self.config.subscribe_timeout_ms = Some(60000);
        self.config.publish_ack_timeout_ms = Some(60000);
        self.config.unsubscribe_timeout_ms = Some(60000);
        self.config.ping_timeout_ms = Some(30000);
        self.config.default_operation_timeout_ms = 120000;
        self
    }

    // ==================== MQTT v5 Flow Control ====================

    /// Set the Receive Maximum value for flow control (MQTT v5)
    ///
    /// This limits the number of QoS 1 and QoS 2 publications the client
    /// will process concurrently. The server must not send more PUBLISH
    /// packets than this value before receiving acknowledgments.
    ///
    /// # Arguments
    /// * `max` - Maximum concurrent QoS 1/2 messages (1-65535)
    ///
    /// # Panics
    /// Panics if `max` is 0 (protocol violation)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::tokio_async_client::TokioAsyncClientConfig;
    ///
    /// let config = TokioAsyncClientConfig::builder()
    ///     .receive_maximum(100)  // Limit to 100 concurrent messages
    ///     .build();
    /// ```
    pub fn receive_maximum(mut self, max: u16) -> Self {
        if max == 0 {
            panic!("receive_maximum must be greater than 0 (MQTT v5 protocol violation)");
        }
        self.config.receive_maximum = Some(max);
        self
    }

    /// Clear the Receive Maximum value (use MQTT v5 default of 65535)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::tokio_async_client::TokioAsyncClientConfig;
    ///
    /// let config = TokioAsyncClientConfig::builder()
    ///     .receive_maximum(100)
    ///     .no_receive_maximum()  // Reset to default
    ///     .build();
    /// ```
    pub fn no_receive_maximum(mut self) -> Self {
        self.config.receive_maximum = None;
        self
    }

    /// Set the Topic Alias Maximum value
    ///
    /// This property indicates the maximum number of topic aliases that the client
    /// accepts from the server (MQTT v5 only). Topic aliases allow the server to
    /// send a numeric alias instead of the full topic name to reduce packet size.
    ///
    /// # Arguments
    /// * `max` - Maximum topic aliases (0-65535)
    ///   - 0 = topic aliases not supported
    ///   - 1-65535 = maximum number of topic aliases
    ///
    /// # Example
    /// ```
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let config = TokioAsyncClientConfig::builder()
    ///     .topic_alias_maximum(10)  // Accept up to 10 topic aliases
    ///     .build();
    /// ```
    pub fn topic_alias_maximum(mut self, max: u16) -> Self {
        self.config.topic_alias_maximum = Some(max);
        self
    }

    /// Disable topic alias support
    ///
    /// Sets topic_alias_maximum to None (default), indicating that topic
    /// aliases are not supported by the client.
    ///
    /// # Example
    /// ```
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let config = TokioAsyncClientConfig::builder()
    ///     .topic_alias_maximum(10)
    ///     .no_topic_alias()  // Disable topic aliases
    ///     .build();
    /// ```
    pub fn no_topic_alias(mut self) -> Self {
        self.config.topic_alias_maximum = None;
        self
    }

    /// Enable 0-RTT (early data) for QUIC connections
    ///
    /// ⚠️ Warning: 0-RTT data is vulnerable to replay attacks. Only use if
    /// you understand the security implications.
    ///
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let config = TokioAsyncClientConfig::builder()
    ///     .quic_enable_0rtt(true)
    ///     .build();
    /// ```
    #[cfg(feature = "quic")]
    pub fn quic_enable_0rtt(mut self, enable: bool) -> Self {
        self.config.quic_enable_0rtt = enable;
        self
    }

    /// Skip TLS certificate verification for QUIC connections
    ///
    /// ⚠️ DANGER: This disables all certificate validation. Only use for
    /// testing/development. Never use in production!
    ///
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let config = TokioAsyncClientConfig::builder()
    ///     .quic_insecure_skip_verify(true)  // For testing only!
    ///     .build();
    /// ```
    #[cfg(feature = "quic")]
    pub fn quic_insecure_skip_verify(mut self, skip: bool) -> Self {
        self.config.quic_insecure_skip_verify = skip;
        self
    }

    /// Set custom root CA certificates for QUIC connections (PEM format)
    ///
    /// Provide custom root CA certificates to verify the QUIC server's
    /// certificate. The PEM string should contain one or more certificates
    /// in PEM format.
    ///
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let ca_pem = std::fs::read_to_string("ca.pem").unwrap();
    /// let config = TokioAsyncClientConfig::builder()
    ///     .quic_custom_root_ca_pem(ca_pem)
    ///     .build();
    /// ```
    #[cfg(feature = "quic")]
    pub fn quic_custom_root_ca_pem(mut self, pem: String) -> Self {
        self.config.quic_custom_root_ca_pem = Some(pem);
        self
    }

    /// Set client certificate for QUIC mTLS (PEM format)
    ///
    /// Provide a client certificate chain for mutual TLS authentication.
    /// The PEM string should contain the certificate chain.
    ///
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let cert_pem = std::fs::read_to_string("client.pem").unwrap();
    /// let key_pem = std::fs::read_to_string("client.key").unwrap();
    /// let config = TokioAsyncClientConfig::builder()
    ///     .quic_client_cert_pem(cert_pem)
    ///     .quic_client_key_pem(key_pem)
    ///     .build();
    /// ```
    #[cfg(feature = "quic")]
    pub fn quic_client_cert_pem(mut self, pem: String) -> Self {
        self.config.quic_client_cert_pem = Some(pem);
        self
    }

    /// Set client private key for QUIC mTLS (PEM format)
    ///
    /// Provide a client private key for mutual TLS authentication.
    /// The PEM string should contain the private key in PEM format.
    ///
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let cert_pem = std::fs::read_to_string("client.pem").unwrap();
    /// let key_pem = std::fs::read_to_string("client.key").unwrap();
    /// let config = TokioAsyncClientConfig::builder()
    ///     .quic_client_cert_pem(cert_pem)
    ///     .quic_client_key_pem(key_pem)
    ///     .build();
    /// ```
    #[cfg(feature = "quic")]
    pub fn quic_client_key_pem(mut self, pem: String) -> Self {
        self.config.quic_client_key_pem = Some(pem);
        self
    }

    /// Set client datagram receive buffer size for QUIC.
    ///
    /// Maximum number of incoming application datagram bytes to buffer, set 0 to disable incoming datagrams
    ///
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::TokioAsyncClientConfig;
    /// let config = TokioAsyncClientConfig::builder()
    ///     .quic_datagram_receive_buffer_size(1000)
    ///     .build();
    /// ```
    #[cfg(feature = "quic")]
    pub fn quic_datagram_receive_buffer_size(mut self, size: usize) -> Self {
        self.config.quic_datagram_receive_buffer_size = size;
        self
    }

    /// Bind QUIC connections to a local UDP address before connecting.
    ///
    /// Use port `0` to let the OS assign an ephemeral source port.
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub fn quic_local_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.config.quic_local_bind_addr = Some(addr);
        self
    }

    /// Enable TLS key logging for QUIC connections
    ///
    /// When enabled, TLS session keys are written to the file specified by
    /// the `SSLKEYLOGFILE` environment variable. This allows tools like
    /// Wireshark to decrypt captured QUIC traffic.
    ///
    /// Only applies when connecting via quic:// URLs (requires `quic` feature)
    #[cfg(feature = "quic")]
    pub fn quic_enable_key_log(mut self, enable: bool) -> Self {
        self.config.quic_enable_key_log = enable;
        self
    }

    /// Enable TLS key logging for rustls TLS connections
    ///
    /// When enabled, TLS session keys are written to the file specified by
    /// the `SSLKEYLOGFILE` environment variable. This allows tools like
    /// Wireshark to decrypt captured TLS traffic.
    ///
    /// Only applies when connecting via mqtts:// URLs with rustls-tls backend
    #[cfg(feature = "rustls-tls")]
    pub fn tls_enable_key_log(mut self, enable: bool) -> Self {
        self.config.tls_enable_key_log = enable;
        self
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokio-based async MQTT client
pub struct TokioAsyncMqttClient {
    /// Command sender to worker task
    command_tx: mpsc::Sender<TokioClientCommand>,
    /// Client configuration
    _config: TokioAsyncClientConfig,
    worker: tokio::task::JoinHandle<Result<(), MqttClientError>>,
}

impl TokioAsyncMqttClient {
    /// Create a new tokio async MQTT client
    pub async fn new(
        mut mqtt_options: MqttClientOptions,
        event_handler: Box<dyn TokioMqttEventHandler>,
        mut config: TokioAsyncClientConfig,
    ) -> io::Result<Self> {
        worker::resolve_options(&mut mqtt_options, &mut config)?;
        let (command_tx, command_rx) = mpsc::channel(config.command_queue_size);

        // Spawn the worker task
        let worker =
            TokioClientWorker::new(mqtt_options, event_handler, command_rx, config.clone());
        let worker = tokio::spawn(worker.run());

        Ok(TokioAsyncMqttClient {
            command_tx,
            _config: config,
            worker,
        })
    }

    /// Create a new async MQTT client with default configuration
    pub async fn with_default_config(
        mqtt_options: MqttClientOptions,
        event_handler: Box<dyn TokioMqttEventHandler>,
    ) -> io::Result<Self> {
        Self::new(
            mqtt_options,
            event_handler,
            TokioAsyncClientConfig::default(),
        )
        .await
    }

    /// Internal helper to apply timeout to sync operations
    async fn with_timeout<F, T>(
        &self,
        future: F,
        timeout_ms: Option<u64>,
        operation_name: &str,
    ) -> Result<T, MqttClientError>
    where
        F: std::future::Future<Output = Result<T, MqttClientError>>,
    {
        if let Some(timeout) = timeout_ms {
            match tokio::time::timeout(Duration::from_millis(timeout), future).await {
                Ok(result) => result,
                Err(_) => Err(MqttClientError::OperationTimeout {
                    operation: operation_name.to_string(),
                    timeout_ms: timeout,
                }),
            }
        } else {
            future.await
        }
    }

    /// Connect to the MQTT broker (non-blocking)
    pub async fn connect(&self) -> io::Result<()> {
        self.send_command(TokioClientCommand::Connect).await
    }

    /// Subscribe to a topic (non-blocking)
    pub async fn subscribe(&self, topic: &str, qos: u8) -> io::Result<()> {
        self.subscribe_with_command(SubscribeCommand::single(topic, qos))
            .await
    }

    /// Subscribe with fully customized command
    pub async fn subscribe_with_command(&self, command: SubscribeCommand) -> io::Result<()> {
        self.send_command(TokioClientCommand::Subscribe(command))
            .await
    }

    /// Publish a message (non-blocking)
    pub async fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
    ) -> io::Result<()> {
        let command = PublishCommand::simple(topic, payload.to_vec(), qos, retain);
        self.publish_with_command(command).await
    }

    /// Publish a message with priority (non-blocking)
    ///
    /// # Arguments
    /// * `topic` - Topic to publish to
    /// * `payload` - Message payload
    /// * `qos` - Quality of Service (0, 1, or 2)
    /// * `retain` - Whether broker should retain this message
    /// * `priority` - Message priority (0=lowest, 255=highest, default=128)
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // High priority alert message
    /// client.publish_with_priority("alerts/critical", b"System down", 1, false, 255).await?;
    ///
    /// // Normal priority data
    /// client.publish_with_priority("sensors/temp", b"23.5", 0, false, 128).await?;
    ///
    /// // Low priority log
    /// client.publish_with_priority("logs/debug", b"Debug info", 0, false, 50).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish_with_priority(
        &self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
        priority: u8,
    ) -> io::Result<()> {
        let command = PublishCommand::with_priority(topic, payload.to_vec(), qos, retain, priority);
        self.publish_with_command(command).await
    }

    /// Publish with fully customized command
    pub async fn publish_with_command(&self, command: PublishCommand) -> io::Result<()> {
        self.send_command(TokioClientCommand::Publish(command))
            .await
    }

    /// Publish a message and wait for acknowledgment (QoS 1 = PUBACK, QoS 2 = PUBCOMP)
    ///
    /// This method blocks until the broker acknowledges the message:
    /// - QoS 0: Returns after engine validation and enqueueing (no acknowledgment)
    /// - QoS 1: Waits for PUBACK from broker
    /// - QoS 2: Waits for PUBCOMP from broker (full QoS 2 flow)
    ///
    /// Uses the timeout configured in `TokioAsyncClientConfig::publish_ack_timeout_ms`.
    ///
    /// # Arguments
    /// * `topic` - Topic to publish to
    /// * `payload` - Message payload
    /// * `qos` - Quality of Service level (0, 1, or 2)
    /// * `retain` - Whether broker should retain this message
    ///
    /// # Returns
    /// `PublishResult` containing packet_id, reason_code, and properties
    ///
    /// # Errors
    /// Returns `MqttClientError::OperationTimeout` if the operation times out.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Simple sync publish
    /// let result = client.publish_sync("sensors/temp", b"23.5", 2, false).await?;
    /// println!("Published with packet ID: {:?}", result.packet_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish_sync(
        &self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
    ) -> Result<PublishResult, MqttClientError> {
        let command = PublishCommand::simple(topic, payload.to_vec(), qos, retain);
        self.publish_with_command_sync(command).await
    }

    /// Publish with priority and wait for acknowledgment
    ///
    /// This method blocks until the broker acknowledges the message with priority support:
    /// - QoS 0: Returns after engine validation and enqueueing
    /// - QoS 1: Waits for PUBACK from broker
    /// - QoS 2: Waits for PUBCOMP from broker
    ///
    /// Higher priority messages are sent before lower priority messages when queued.
    ///
    /// # Arguments
    /// * `topic` - Topic to publish to
    /// * `payload` - Message payload
    /// * `qos` - Quality of Service (0, 1, or 2)
    /// * `retain` - Whether broker should retain this message
    /// * `priority` - Message priority (0=lowest, 255=highest, default=128)
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Send critical alert with highest priority and wait for ack
    /// let result = client.publish_sync_with_priority(
    ///     "alerts/critical", b"Emergency", 2, false, 255
    /// ).await?;
    /// println!("Critical message sent with packet ID: {:?}", result.packet_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish_sync_with_priority(
        &self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
        priority: u8,
    ) -> Result<PublishResult, MqttClientError> {
        let command = PublishCommand::with_priority(topic, payload.to_vec(), qos, retain, priority);
        self.publish_with_command_sync(command).await
    }

    /// Publish with fully customized command and wait for acknowledgment
    pub async fn publish_with_command_sync(
        &self,
        command: PublishCommand,
    ) -> Result<PublishResult, MqttClientError> {
        let timeout = self._config.publish_ack_timeout_ms;
        self.with_timeout(self.publish_sync_internal(command), timeout, "publish")
            .await
    }

    /// Internal publish implementation without timeout
    async fn publish_sync_internal(
        &self,
        command: PublishCommand,
    ) -> Result<PublishResult, MqttClientError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(TokioClientCommand::PublishSync {
            command,
            response_tx: tx,
        })
        .await?;
        receive_response(rx).await
    }

    /// Unsubscribe from topics (non-blocking)
    pub async fn unsubscribe(&self, topics: Vec<&str>) -> io::Result<()> {
        let topics: Vec<String> = topics.into_iter().map(|s| s.to_string()).collect();
        self.unsubscribe_with_command(UnsubscribeCommand::from_topics(topics))
            .await
    }

    /// Unsubscribe with fully customized command
    pub async fn unsubscribe_with_command(&self, command: UnsubscribeCommand) -> io::Result<()> {
        self.send_command(TokioClientCommand::Unsubscribe(command))
            .await
    }

    /// Send ping to broker (non-blocking)
    pub async fn ping(&self) -> io::Result<()> {
        self.send_command(TokioClientCommand::Ping).await
    }

    /// Disconnect from broker (non-blocking)
    pub async fn disconnect(&self) -> io::Result<()> {
        self.disconnect_with(0, Vec::new()).await
    }

    /// Queue a disconnect with MQTT 5 reason code and properties.
    pub async fn disconnect_with(
        &self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> io::Result<()> {
        self.send_command(TokioClientCommand::Disconnect {
            reason_code,
            properties,
            response_tx: None,
        })
        .await
    }

    /// Flush DISCONNECT, close the transport and wait for local completion.
    pub async fn disconnect_sync(&self) -> Result<(), MqttClientError> {
        self.disconnect_with_sync(0, Vec::new()).await
    }

    /// Wait for disconnect completion, preserving MQTT 5 reason code and properties.
    pub async fn disconnect_with_sync(
        &self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        let (tx, rx) = oneshot::channel();
        self.with_timeout(
            async {
                self.send_command(TokioClientCommand::Disconnect {
                    reason_code,
                    properties,
                    response_tx: Some(tx),
                })
                .await?;
                receive_response(rx).await
            },
            Some(self._config.default_operation_timeout_ms),
            "disconnect",
        )
        .await
    }

    /// Acknowledge an incoming QoS 1 publication. Call from outside event callbacks.
    pub async fn puback(
        &self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.acknowledge(4, packet_id, reason_code, properties)
            .await
    }

    /// Accept an incoming QoS 2 publication. Call from outside event callbacks.
    pub async fn pubrec(
        &self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.acknowledge(5, packet_id, reason_code, properties)
            .await
    }

    /// Complete an incoming QoS 2 exchange after `on_pubrel_received`.
    pub async fn pubcomp(
        &self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.acknowledge(7, packet_id, reason_code, properties)
            .await
    }

    async fn acknowledge(
        &self,
        kind: u8,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        let (tx, rx) = oneshot::channel();
        self.with_timeout(
            async {
                self.send_command(TokioClientCommand::Acknowledge {
                    kind,
                    packet_id,
                    reason_code,
                    properties,
                    response_tx: tx,
                })
                .await?;
                receive_response(rx).await
            },
            Some(self._config.default_operation_timeout_ms),
            "acknowledge",
        )
        .await
    }

    /// Enable or disable automatic reconnection
    pub async fn set_auto_reconnect(&self, enabled: bool) -> io::Result<()> {
        self.send_command(TokioClientCommand::SetAutoReconnect { enabled })
            .await
    }

    /// Attempt graceful disconnect and wait for the worker to terminate.
    /// Transport I/O remains bounded by `default_operation_timeout_ms`.
    pub async fn shutdown(self) -> io::Result<()> {
        let _ = self.send_command(TokioClientCommand::Shutdown).await;
        self.worker
            .await
            .map_err(io::Error::other)?
            .map_err(Into::into)
    }

    /// Send a raw MQTT packet to the broker (non-blocking)
    pub async fn send_packet(&self, packet: MqttPacket) -> io::Result<()> {
        self.send_command(TokioClientCommand::SendPacket(packet))
            .await
    }

    /// Send AUTH packet for enhanced authentication (MQTT v5)
    ///
    /// Used for multi-step authentication protocols like SCRAM, OAuth, Kerberos, etc.
    ///
    /// # Arguments
    /// * `reason_code` - Authentication reason code (0x00 = Success, 0x18 = Continue, 0x19 = Re-authenticate)
    /// * `properties` - Authentication properties (typically AuthenticationMethod and AuthenticationData)
    pub async fn auth(&self, reason_code: u8, properties: Vec<Property>) -> io::Result<()> {
        self.send_command(TokioClientCommand::Auth {
            reason_code,
            properties,
        })
        .await
    }

    /// Send AUTH packet to continue authentication (reason code 0x18)
    pub async fn auth_continue(&self, properties: Vec<Property>) -> io::Result<()> {
        self.auth(0x18, properties).await
    }

    /// Send AUTH packet to re-authenticate (reason code 0x19)
    pub async fn auth_re_authenticate(&self, properties: Vec<Property>) -> io::Result<()> {
        self.auth(0x19, properties).await
    }

    /// Connect to broker and wait for CONNACK acknowledgment
    ///
    /// This method blocks until the broker acknowledges the connection with CONNACK.
    /// Uses the timeout configured in `TokioAsyncClientConfig::connect_timeout_ms`.
    ///
    /// # Returns
    /// `ConnectionResult` containing reason_code, session_present, and properties
    ///
    /// # Errors
    /// Returns `MqttClientError::OperationTimeout` if the operation times out.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// let result = client.connect_sync().await?;
    /// if result.is_success() {
    ///     println!("Connected! Session present: {}", result.session_present);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect_sync(&self) -> Result<ConnectionResult, MqttClientError> {
        let timeout = self._config.connect_timeout_ms;
        self.with_timeout(self.connect_sync_internal(), timeout, "connect")
            .await
    }

    /// Internal connect implementation without timeout
    async fn connect_sync_internal(&self) -> Result<ConnectionResult, MqttClientError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(TokioClientCommand::ConnectSync { response_tx: tx })
            .await?;
        receive_response(rx).await
    }

    /// Subscribe to topics and wait for SUBACK acknowledgment
    ///
    /// This method blocks until the broker acknowledges the subscription with SUBACK.
    /// Uses the timeout configured in `TokioAsyncClientConfig::subscribe_timeout_ms`.
    ///
    /// # Arguments
    /// * `topic` - Topic to subscribe to
    /// * `qos` - Requested Quality of Service level
    ///
    /// # Returns
    /// `SubscribeResult` containing packet_id, reason_codes, and properties
    ///
    /// # Errors
    /// Returns `MqttClientError::OperationTimeout` if the operation times out.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// let result = client.subscribe_sync("sensors/#", 1).await?;
    /// println!("Subscribed with QoS: {:?}", result.reason_codes);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_sync(
        &self,
        topic: &str,
        qos: u8,
    ) -> Result<SubscribeResult, MqttClientError> {
        self.subscribe_with_command_sync(SubscribeCommand::single(topic, qos))
            .await
    }

    /// Subscribe with fully customized command and wait for acknowledgment
    pub async fn subscribe_with_command_sync(
        &self,
        command: SubscribeCommand,
    ) -> Result<SubscribeResult, MqttClientError> {
        let timeout = self._config.subscribe_timeout_ms;
        self.with_timeout(self.subscribe_sync_internal(command), timeout, "subscribe")
            .await
    }

    /// Internal subscribe implementation without timeout
    async fn subscribe_sync_internal(
        &self,
        command: SubscribeCommand,
    ) -> Result<SubscribeResult, MqttClientError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(TokioClientCommand::SubscribeSync {
            command,
            response_tx: tx,
        })
        .await?;
        receive_response(rx).await
    }

    /// Unsubscribe from topics and wait for UNSUBACK acknowledgment
    ///
    /// This method blocks until the broker acknowledges the unsubscription with UNSUBACK.
    /// Uses the timeout configured in `TokioAsyncClientConfig::unsubscribe_timeout_ms`.
    ///
    /// # Arguments
    /// * `topics` - List of topics to unsubscribe from
    ///
    /// # Returns
    /// `UnsubscribeResult` containing packet_id, reason_codes, and properties
    ///
    /// # Errors
    /// Returns `MqttClientError::OperationTimeout` if the operation times out.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// let result = client.unsubscribe_sync(vec!["sensors/#"]).await?;
    /// println!("Unsubscribed: packet_id={:?}", result.packet_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unsubscribe_sync(
        &self,
        topics: Vec<&str>,
    ) -> Result<UnsubscribeResult, MqttClientError> {
        let topics: Vec<String> = topics.into_iter().map(|s| s.to_string()).collect();
        self.unsubscribe_with_command_sync(UnsubscribeCommand::from_topics(topics))
            .await
    }

    /// Unsubscribe with fully customized command and wait for acknowledgment
    pub async fn unsubscribe_with_command_sync(
        &self,
        command: UnsubscribeCommand,
    ) -> Result<UnsubscribeResult, MqttClientError> {
        let timeout = self._config.unsubscribe_timeout_ms;
        self.with_timeout(
            self.unsubscribe_sync_internal(command),
            timeout,
            "unsubscribe",
        )
        .await
    }

    /// Internal unsubscribe implementation without timeout
    async fn unsubscribe_sync_internal(
        &self,
        command: UnsubscribeCommand,
    ) -> Result<UnsubscribeResult, MqttClientError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(TokioClientCommand::UnsubscribeSync {
            command,
            response_tx: tx,
        })
        .await?;
        receive_response(rx).await
    }

    /// Send ping and wait for PINGRESP acknowledgment
    ///
    /// This method blocks until the broker responds with PINGRESP.
    /// Uses the timeout configured in `TokioAsyncClientConfig::ping_timeout_ms`.
    ///
    /// # Returns
    /// `PingResult` indicating successful ping response
    ///
    /// # Errors
    /// Returns `MqttClientError::OperationTimeout` if the operation times out.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Check connection
    /// match client.ping_sync().await {
    ///     Ok(_) => println!("Connection alive!"),
    ///     Err(e) => println!("Connection error: {}", e),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ping_sync(&self) -> Result<PingResult, MqttClientError> {
        let timeout = self._config.ping_timeout_ms;
        self.with_timeout(self.ping_sync_internal(), timeout, "ping")
            .await
    }

    /// Internal ping implementation without timeout
    async fn ping_sync_internal(&self) -> Result<PingResult, MqttClientError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(TokioClientCommand::PingSync { response_tx: tx })
            .await?;
        receive_response(rx).await
    }

    // ==================== Custom Timeout Override Methods ====================

    /// Connect with a custom timeout duration
    ///
    /// Allows overriding the default connect timeout for this specific operation.
    /// Useful for scenarios requiring longer or shorter timeouts than the configured default.
    ///
    /// # Arguments
    /// * `timeout_ms` - Custom timeout in milliseconds
    ///
    /// # Returns
    /// `Result<ConnectionResult, MqttClientError>` - Connection result or timeout error
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Connect with 60 second timeout for satellite network
    /// let result = client.connect_sync_with_timeout(60000).await?;
    /// println!("Connected: {:?}", result.session_present);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect_sync_with_timeout(
        &self,
        timeout_ms: u64,
    ) -> Result<ConnectionResult, MqttClientError> {
        self.with_timeout(self.connect_sync_internal(), Some(timeout_ms), "connect")
            .await
    }

    /// Subscribe with a custom timeout duration
    ///
    /// Allows overriding the default subscribe timeout for this specific operation.
    ///
    /// # Arguments
    /// * `topic` - Topic filter to subscribe to
    /// * `qos` - Requested Quality of Service level
    /// * `timeout_ms` - Custom timeout in milliseconds
    ///
    /// # Returns
    /// `Result<SubscribeResult, MqttClientError>` - Subscribe result or timeout error
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Subscribe with 5 second timeout for time-critical subscription
    /// let result = client.subscribe_sync_with_timeout("sensors/+", 1, 5000).await?;
    /// println!("Subscribed with reason codes: {:?}", result.reason_codes);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_sync_with_timeout(
        &self,
        topic: &str,
        qos: u8,
        timeout_ms: u64,
    ) -> Result<SubscribeResult, MqttClientError> {
        let subscribe_command = SubscribeCommand::single(topic, qos);
        self.with_timeout(
            self.subscribe_sync_internal(subscribe_command),
            Some(timeout_ms),
            "subscribe",
        )
        .await
    }

    /// Publish with a custom timeout duration
    ///
    /// Allows overriding the default publish acknowledgment timeout for this specific operation.
    ///
    /// # Arguments
    /// * `topic` - Topic to publish to
    /// * `payload` - Message payload
    /// * `qos` - Quality of Service level (0, 1, or 2)
    /// * `retain` - Whether broker should retain this message
    /// * `timeout_ms` - Custom timeout in milliseconds
    ///
    /// # Returns
    /// `Result<PublishResult, MqttClientError>` - Publish result or timeout error
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Publish critical alert with 3 second timeout
    /// let result = client.publish_sync_with_timeout(
    ///     "alerts/critical",
    ///     b"SYSTEM FAILURE",
    ///     2,
    ///     false,
    ///     3000
    /// ).await?;
    /// println!("Published with packet ID: {:?}", result.packet_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish_sync_with_timeout(
        &self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
        timeout_ms: u64,
    ) -> Result<PublishResult, MqttClientError> {
        let publish_command = PublishCommand::simple(topic, payload.to_vec(), qos, retain);
        self.with_timeout(
            self.publish_sync_internal(publish_command),
            Some(timeout_ms),
            "publish",
        )
        .await
    }

    /// Unsubscribe with a custom timeout duration
    ///
    /// Allows overriding the default unsubscribe timeout for this specific operation.
    ///
    /// # Arguments
    /// * `topics` - Topic filters to unsubscribe from
    /// * `timeout_ms` - Custom timeout in milliseconds
    ///
    /// # Returns
    /// `Result<UnsubscribeResult, MqttClientError>` - Unsubscribe result or timeout error
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Unsubscribe with 15 second timeout for slow network
    /// let result = client.unsubscribe_sync_with_timeout(
    ///     vec!["sensors/+", "alerts/#"],
    ///     15000
    /// ).await?;
    /// println!("Unsubscribed with reason codes: {:?}", result.reason_codes);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unsubscribe_sync_with_timeout(
        &self,
        topics: Vec<&str>,
        timeout_ms: u64,
    ) -> Result<UnsubscribeResult, MqttClientError> {
        let topics: Vec<String> = topics.into_iter().map(|s| s.to_string()).collect();
        let unsubscribe_command = UnsubscribeCommand::from_topics(topics);
        self.with_timeout(
            self.unsubscribe_sync_internal(unsubscribe_command),
            Some(timeout_ms),
            "unsubscribe",
        )
        .await
    }

    /// Send PINGREQ and wait for PINGRESP with a custom timeout duration
    ///
    /// Allows overriding the default ping timeout for this specific operation.
    ///
    /// # Arguments
    /// * `timeout_ms` - Custom timeout in milliseconds
    ///
    /// # Returns
    /// `Result<PingResult, MqttClientError>` - Ping result or timeout error
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Quick ping with 2 second timeout for health check
    /// let result = client.ping_sync_with_timeout(2000).await?;
    /// println!("Ping successful!");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ping_sync_with_timeout(
        &self,
        timeout_ms: u64,
    ) -> Result<PingResult, MqttClientError> {
        self.with_timeout(self.ping_sync_internal(), Some(timeout_ms), "ping")
            .await
    }

    // ==================== End Custom Timeout Override Methods ====================

    /// Publish a message and wait for acknowledgment (QoS 1 = PUBACK, QoS 2 = PUBCOMP)
    ///
    /// This method blocks until the broker acknowledges the message:
    /// - QoS 0: Returns after engine validation and enqueueing (no acknowledgment)
    /// - QoS 1: Waits for PUBACK from broker
    /// - QoS 2: Waits for PUBCOMP from broker (full QoS 2 flow)
    ///
    /// # Arguments
    /// * `topic` - Topic to publish to
    /// * `payload` - Message payload
    /// * `qos` - Quality of Service level (0, 1, or 2)
    /// * `retain` - Whether broker should retain this message
    ///
    /// # Returns
    /// `PublishResult` containing packet_id, reason_code, and properties
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
    /// # use flowsdk::mqtt_client::tokio_async_client::TokioMqttEventHandler;
    /// # use tokio;
    /// # use std::time::Duration;
    /// # #[derive(Clone)]
    /// # struct Handler;
    /// # #[async_trait::async_trait]
    /// # impl TokioMqttEventHandler for Handler {}
    /// # async fn example() -> std::io::Result<()> {
    /// # let options = MqttClientOptions::builder().peer("localhost:1883").client_id("example").build();
    /// # let client = TokioAsyncMqttClient::new(options, Box::new(Handler), TokioAsyncClientConfig::default()).await?;
    /// // Simple sync publish
    /// let result = client.publish_sync("sensors/temp", b"23.5", 2, false).await?;
    /// println!("Published with packet ID: {:?}", result.packet_id);
    ///
    /// // With timeout
    /// let result = tokio::time::timeout(
    ///     Duration::from_secs(5),
    ///     client.publish_sync("sensors/temp", b"23.5", 2, false)
    /// ).await??;
    /// # Ok(())
    /// # }
    /// ```
    /// Send a command to the worker task
    async fn send_command(&self, command: TokioClientCommand) -> io::Result<()> {
        self.command_tx.send(command).await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Worker task is no longer running",
            )
        })
    }
}

#[cfg(test)]
mod config_builder_tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[async_trait]
    impl Transport for tokio::io::DuplexStream {
        async fn connect(_: &str) -> Result<Self, super::super::transport::TransportError> {
            unreachable!("test transport is constructed in memory")
        }
        async fn close(&mut self) -> Result<(), super::super::transport::TransportError> {
            self.shutdown().await?;
            Ok(())
        }
        fn peer_addr(&self) -> Result<String, super::super::transport::TransportError> {
            Ok("memory".into())
        }
        fn local_addr(&self) -> Result<String, super::super::transport::TransportError> {
            Ok("memory".into())
        }
    }

    struct MessageRecorder(mpsc::UnboundedSender<Vec<u8>>);

    #[async_trait]
    impl TokioMqttEventHandler for MessageRecorder {
        async fn on_message_received(&mut self, message: &MqttMessage) {
            self.0.send(message.payload.to_vec()).unwrap();
        }
    }

    #[tokio::test]
    async fn outgoing_drain_dispatches_buffered_message_events() {
        use crate::mqtt_serde::mqttv5::publishv5::MqttPublish;
        for write_fails in [false, true] {
            let (messages, mut received) = mpsc::unbounded_channel();
            let (_commands, rx) = mpsc::channel(1);
            let mut worker = TokioClientWorker::new(
                MqttClientOptions::builder()
                    .keep_alive(0)
                    .max_outgoing_packet_count(1)
                    .build(),
                Box::new(MessageRecorder(messages)),
                rx,
                TokioAsyncClientConfig::default(),
            );
            worker.engine.connect().unwrap();
            worker.engine.take_outgoing();
            let events = worker.engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
            worker.dispatch_events(events).await;
            let (transport, peer) = tokio::io::duplex(1024);
            worker.stream = Some(Box::new(transport));
            let mut peer = Some(peer);
            if write_fails {
                peer.take();
            }
            let bytes: Vec<_> = (1..=3)
                .flat_map(|id| {
                    MqttPacket::Publish5(MqttPublish::new(
                        1,
                        "t".into(),
                        Some(id),
                        vec![id as u8],
                        false,
                        false,
                    ))
                    .to_bytes()
                    .unwrap()
                })
                .collect();
            let events = worker.engine.handle_incoming(&bytes);
            worker.dispatch_events(events).await;
            assert_eq!(worker.handle_outgoing().await.is_err(), write_fails);
            for id in 1..=3 {
                assert_eq!(received.try_recv().unwrap(), vec![id]);
            }
            assert!(received.try_recv().is_err());
            assert!(worker.engine.take_events().is_empty());
            if let Some(mut peer) = peer {
                let mut acknowledgements = [0; 12];
                peer.read_exact(&mut acknowledgements).await.unwrap();
                assert_eq!(
                    acknowledgements,
                    [0x40, 2, 0, 1, 0x40, 2, 0, 2, 0x40, 2, 0, 3]
                );
                worker.handle_outgoing().await.unwrap();
                assert!(received.try_recv().is_err());
            }
        }
    }

    #[test]
    fn test_receive_maximum_default() {
        let config = TokioAsyncClientConfig::default();
        assert_eq!(config.receive_maximum, None);
    }

    #[test]
    fn test_receive_maximum_builder() {
        let config = TokioAsyncClientConfig::builder()
            .receive_maximum(100)
            .build();

        assert_eq!(config.receive_maximum, Some(100));
    }

    #[test]
    fn test_receive_maximum_min_value() {
        let config = TokioAsyncClientConfig::builder().receive_maximum(1).build();

        assert_eq!(config.receive_maximum, Some(1));
    }

    #[test]
    fn test_receive_maximum_max_value() {
        let config = TokioAsyncClientConfig::builder()
            .receive_maximum(65535)
            .build();

        assert_eq!(config.receive_maximum, Some(65535));
    }

    #[test]
    #[should_panic(expected = "receive_maximum must be greater than 0")]
    fn test_receive_maximum_zero_panics() {
        TokioAsyncClientConfig::builder().receive_maximum(0).build();
    }

    #[test]
    fn test_no_receive_maximum() {
        let config = TokioAsyncClientConfig::builder()
            .receive_maximum(100)
            .no_receive_maximum()
            .build();

        assert_eq!(config.receive_maximum, None);
    }

    #[test]
    fn test_receive_maximum_chain() {
        // Test that we can chain with other config options
        let config = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .receive_maximum(50)
            .tcp_nodelay(true)
            .connect_timeout_ms(5000)
            .build();

        assert_eq!(config.receive_maximum, Some(50));
        assert!(!config.auto_reconnect);
        assert!(config.tcp_nodelay);
        assert_eq!(config.connect_timeout_ms, Some(5000));
    }

    #[test]
    fn test_config_builder_clone() {
        let builder = TokioAsyncClientConfig::builder()
            .receive_maximum(200)
            .auto_reconnect(false);

        let builder_clone = builder.clone();

        let config1 = builder.build();
        let config2 = builder_clone.build();

        assert_eq!(config1.receive_maximum, config2.receive_maximum);
        assert_eq!(config1.auto_reconnect, config2.auto_reconnect);
    }

    #[test]
    fn test_receive_maximum_mqtt_v5_compliance() {
        // Test various valid values according to MQTT v5 spec
        let test_values = vec![1, 10, 100, 1000, 10000, 32767, 65535];

        for value in test_values {
            let config = TokioAsyncClientConfig::builder()
                .receive_maximum(value)
                .build();

            assert_eq!(config.receive_maximum, Some(value));
        }
    }

    #[test]
    fn test_config_default_values() {
        // Verify default config has correct receive_maximum
        let config = TokioAsyncClientConfig::default();

        assert_eq!(config.receive_maximum, None); // Should use MQTT v5 default (65535)
        assert!(config.auto_reconnect);
        assert!(config.tcp_nodelay);
    }

    // ==================== Topic Alias Maximum Tests ====================

    #[test]
    fn test_topic_alias_maximum_default() {
        let config = TokioAsyncClientConfig::default();
        assert_eq!(config.topic_alias_maximum, None);
    }

    #[test]
    fn test_topic_alias_maximum_zero() {
        // 0 means topic aliases not supported
        let config = TokioAsyncClientConfig::builder()
            .topic_alias_maximum(0)
            .build();

        assert_eq!(config.topic_alias_maximum, Some(0));
    }

    #[test]
    fn test_topic_alias_maximum_builder() {
        let config = TokioAsyncClientConfig::builder()
            .topic_alias_maximum(10)
            .build();

        assert_eq!(config.topic_alias_maximum, Some(10));
    }

    #[test]
    fn test_topic_alias_maximum_max_value() {
        let config = TokioAsyncClientConfig::builder()
            .topic_alias_maximum(65535)
            .build();

        assert_eq!(config.topic_alias_maximum, Some(65535));
    }

    #[test]
    fn test_no_topic_alias() {
        let config = TokioAsyncClientConfig::builder()
            .topic_alias_maximum(100)
            .no_topic_alias()
            .build();

        assert_eq!(config.topic_alias_maximum, None);
    }

    #[test]
    fn test_topic_alias_chain() {
        let config = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .topic_alias_maximum(50)
            .receive_maximum(100)
            .tcp_nodelay(true)
            .build();

        assert_eq!(config.topic_alias_maximum, Some(50));
        assert_eq!(config.receive_maximum, Some(100));
        assert!(!config.auto_reconnect);
        assert!(config.tcp_nodelay);
    }

    #[test]
    fn test_topic_alias_mqtt_v5_compliance() {
        // Test various valid values according to MQTT v5 spec
        let test_values = vec![0, 1, 10, 100, 1000, 32767, 65535];

        for value in test_values {
            let config = TokioAsyncClientConfig::builder()
                .topic_alias_maximum(value)
                .build();

            assert_eq!(config.topic_alias_maximum, Some(value));
        }
    }

    // Simple test event handler for testing
    struct TestEventHandler;

    #[async_trait::async_trait]
    impl TokioMqttEventHandler for TestEventHandler {
        async fn on_connected(&mut self, _result: &ConnectionResult) {}
        async fn on_disconnected(&mut self, _reason: Option<u8>) {}
        async fn on_published(&mut self, _result: &PublishResult) {}
        async fn on_subscribed(&mut self, _result: &SubscribeResult) {}
        async fn on_unsubscribed(&mut self, _result: &UnsubscribeResult) {}
        async fn on_message_received(&mut self, _publish: &MqttMessage) {}
        async fn on_ping_response(&mut self, _result: &PingResult) {}
        async fn on_error(&mut self, _error: &MqttClientError) {}
        async fn on_connection_lost(&mut self) {}
    }

    #[tokio::test]
    async fn test_priority_publishing_variations() {
        // Test various priority levels and QoS combinations
        let mqtt_options = MqttClientOptions::builder()
            .peer("broker.emqx.io:1883")
            .client_id("test_priority_variations")
            .build();

        let async_config = TokioAsyncClientConfig::builder()
            .priority_queue_enabled(true)
            .priority_queue_limit(100)
            .buffer_messages(true)
            .build();

        let event_handler = Box::new(TestEventHandler);

        let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config)
            .await
            .unwrap();

        // Test all priority levels (0-255) with different QoS
        let _ = client
            .publish_with_priority("test/min", b"Minimum priority", 0, false, 0)
            .await;
        let _ = client
            .publish_with_priority("test/low", b"Low priority", 1, false, 50)
            .await;
        let _ = client
            .publish_with_priority("test/mid", b"Medium priority", 2, false, 128)
            .await;
        let _ = client
            .publish_with_priority("test/high", b"High priority", 1, false, 200)
            .await;
        let _ = client
            .publish_with_priority("test/max", b"Maximum priority", 0, false, 255)
            .await;

        // Test retained messages with priorities
        let _ = client
            .publish_with_priority("test/retained_high", b"Retained high", 1, true, 255)
            .await;
        let _ = client
            .publish_with_priority("test/retained_low", b"Retained low", 1, true, 0)
            .await;

        sleep(Duration::from_millis(200)).await;
        client.disconnect().await.ok();
        client.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_priority_queue_limit_enforcement() {
        // Test that priority queue respects capacity limits
        let mqtt_options = MqttClientOptions::builder()
            .peer("broker.emqx.io:1883")
            .client_id("test_queue_limit")
            .build();

        let async_config = TokioAsyncClientConfig::builder()
            .priority_queue_enabled(true)
            .priority_queue_limit(10) // Small limit to test eviction
            .buffer_messages(true)
            .build();

        let event_handler = Box::new(TestEventHandler);

        let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config)
            .await
            .unwrap();

        // Publish more messages than the limit before connecting
        // This tests capacity eviction (low priority messages should be dropped)
        for i in 0..15 {
            let priority = if i < 5 { 255 } else { 50 }; // Mix of high and low priorities
            let _ = client
                .publish_with_priority(
                    &format!("test/msg{}", i),
                    format!("Message {}", i).as_bytes(),
                    0,
                    false,
                    priority,
                )
                .await;
        }

        sleep(Duration::from_millis(100)).await;
        client.disconnect().await.ok();
        client.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_priority_queue_disabled() {
        // Test that messages work when priority queue is disabled
        let mqtt_options = MqttClientOptions::builder()
            .peer("broker.emqx.io:1883")
            .client_id("test_no_priority")
            .build();

        let async_config = TokioAsyncClientConfig::builder()
            .priority_queue_enabled(false) // Disable priority queue
            .buffer_messages(true)
            .build();

        let event_handler = Box::new(TestEventHandler);

        let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config)
            .await
            .unwrap();

        // Publish with priorities (should still work, just not prioritized)
        let _ = client
            .publish_with_priority("test/msg1", b"Message 1", 0, false, 255)
            .await;
        let _ = client
            .publish_with_priority("test/msg2", b"Message 2", 1, false, 0)
            .await;

        sleep(Duration::from_millis(100)).await;
        client.disconnect().await.ok();
        client.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_priority_flush_after_reconnect() {
        // Test that queued messages are flushed after reconnection
        let mqtt_options = MqttClientOptions::builder()
            .peer("broker.emqx.io:1883")
            .client_id("test_reconnect_flush")
            .build();

        let async_config = TokioAsyncClientConfig::builder()
            .priority_queue_enabled(true)
            .priority_queue_limit(50)
            .buffer_messages(true)
            .auto_reconnect(false) // Manual reconnect control
            .build();

        let event_handler = Box::new(TestEventHandler);

        let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config)
            .await
            .unwrap();

        // Queue messages before connecting
        for i in 0..5 {
            let priority = 255 - (i * 50); // Descending priorities
            let _ = client
                .publish_with_priority(
                    &format!("test/queued{}", i),
                    format!("Queued message {}", i).as_bytes(),
                    1,
                    false,
                    priority as u8,
                )
                .await;
        }

        // Connect and wait for flush
        client.connect().await.ok();
        sleep(Duration::from_millis(1000)).await;

        // Publish additional messages after connection
        let _ = client
            .publish_with_priority("test/after_connect", b"After connect", 1, false, 200)
            .await;

        sleep(Duration::from_millis(500)).await;
        client.disconnect().await.ok();
        client.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_mixed_publish_and_priority_publish() {
        // Test mixing regular publish with priority publish
        let mqtt_options = MqttClientOptions::builder()
            .peer("broker.emqx.io:1883")
            .client_id("test_mixed_publish")
            .build();

        let async_config = TokioAsyncClientConfig::builder()
            .priority_queue_enabled(true)
            .priority_queue_limit(100)
            .build();

        let event_handler = Box::new(TestEventHandler);

        let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config)
            .await
            .unwrap();

        // Mix regular publish (default priority 128) with explicit priorities
        let _ = client
            .publish("test/regular1", b"Regular publish", 0, false)
            .await;
        let _ = client
            .publish_with_priority("test/high", b"High priority", 0, false, 255)
            .await;
        let _ = client
            .publish("test/regular2", b"Another regular", 1, false)
            .await;
        let _ = client
            .publish_with_priority("test/low", b"Low priority", 1, false, 50)
            .await;

        sleep(Duration::from_millis(200)).await;
        client.disconnect().await.ok();
        client.shutdown().await.ok();
    }

    #[tokio::test]
    async fn test_sync_publish_flushes_priority_queue() {
        // Test that synchronous publish flushes priority queue first
        let mqtt_options = MqttClientOptions::builder()
            .peer("broker.emqx.io:1883")
            .client_id("test_sync_flush")
            .build();

        let async_config = TokioAsyncClientConfig::builder()
            .priority_queue_enabled(true)
            .priority_queue_limit(100)
            .publish_ack_timeout_ms(5000)
            .build();

        let event_handler = Box::new(TestEventHandler);

        let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config)
            .await
            .unwrap();

        // Queue some async messages with different priorities (before connecting)
        let _ = client
            .publish_with_priority("test/async1", b"Low priority async", 0, false, 50)
            .await;
        let _ = client
            .publish_with_priority("test/async2", b"High priority async", 0, false, 255)
            .await;
        let _ = client
            .publish_with_priority("test/async3", b"Medium priority async", 0, false, 128)
            .await;

        // Connect to trigger flush
        client.connect().await.ok();
        sleep(Duration::from_millis(1000)).await;

        // Now send a synchronous publish - it should flush queue first
        let _ = client
            .publish_sync_with_priority("test/sync", b"Sync publish", 1, false, 200)
            .await;

        sleep(Duration::from_millis(500)).await;
        client.disconnect().await.ok();
        client.shutdown().await.ok();
    }
}
