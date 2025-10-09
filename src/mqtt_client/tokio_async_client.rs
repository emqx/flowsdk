use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, error::TryRecvError, error::TrySendError, Receiver};
use tokio::sync::oneshot;
use tokio::time::Sleep;

use crate::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::{
    authv5,
    common::properties::Property,
    connectv5, disconnectv5, pingreqv5, pingrespv5,
    pubackv5::MqttPubAck,
    pubcompv5::MqttPubComp,
    publishv5::MqttPublish,
    pubrecv5::MqttPubRec,
    pubrelv5::MqttPubRel,
    subscribev5::{self, TopicSubscription},
    unsubscribev5,
};
use crate::mqtt_serde::parser::{ParseError, ParseOk};
use crate::mqtt_session::ClientSession;

use super::client::{
    AuthResult, ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use super::error::MqttClientError;
use super::opts::MqttClientOptions;

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
    MessageReceived(MqttPublish),
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
    async fn on_message_received(&mut self, publish: &MqttPublish) {
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

/// Fully customizable publish command for MQTT v5
#[derive(Debug, Clone)]
pub struct PublishCommand {
    pub topic_name: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    pub dup: bool,
    pub packet_id: Option<u16>,
    pub properties: Vec<Property>,
}

impl PublishCommand {
    pub fn new(
        topic_name: String,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
        dup: bool,
        packet_id: Option<u16>,
        properties: Vec<Property>,
    ) -> Self {
        Self {
            topic_name,
            payload,
            qos,
            retain,
            dup,
            packet_id,
            properties,
        }
    }

    pub fn simple(topic: impl Into<String>, payload: Vec<u8>, qos: u8, retain: bool) -> Self {
        Self::new(topic.into(), payload, qos, retain, false, None, Vec::new())
    }

    fn to_mqtt_publish(&self) -> MqttPublish {
        MqttPublish::new_with_prop(
            self.qos,
            self.topic_name.clone(),
            self.packet_id,
            self.payload.clone(),
            self.retain,
            self.dup,
            self.properties.clone(),
        )
    }
}

/// Fully customizable subscribe command for MQTT v5
#[derive(Debug, Clone)]
pub struct SubscribeCommand {
    pub packet_id: Option<u16>,
    pub subscriptions: Vec<TopicSubscription>,
    pub properties: Vec<Property>,
}

impl SubscribeCommand {
    pub fn new(
        packet_id: Option<u16>,
        subscriptions: Vec<TopicSubscription>,
        properties: Vec<Property>,
    ) -> Self {
        Self {
            packet_id,
            subscriptions,
            properties,
        }
    }

    pub fn single(topic: impl Into<String>, qos: u8) -> Self {
        let subscription = TopicSubscription::new(topic.into(), qos, false, false, 0);
        Self::new(None, vec![subscription], Vec::new())
    }

    /// Create a new builder for constructing a SubscribeCommand
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    ///
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/#", 1)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn builder() -> SubscribeCommandBuilder {
        SubscribeCommandBuilder::new()
    }
}

/// Builder for creating complex MQTT v5 subscription commands
///
/// This builder provides a fluent API for constructing `SubscribeCommand` instances
/// with MQTT v5 subscription options like No Local, Retain As Published, and
/// Retain Handling.
///
/// # Examples
///
/// ## Simple Subscription
/// ```no_run
/// use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
///
/// let cmd = SubscribeCommand::builder()
///     .add_topic("sensors/temp", 1)
///     .build()
///     .unwrap();
/// ```
///
/// ## Subscription with Options
/// ```no_run
/// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
/// let cmd = SubscribeCommand::builder()
///     .add_topic("sensors/+/temp", 1)
///     .with_no_local(true)           // Don't receive own messages
///     .with_retain_handling(2)       // Don't send retained messages
///     .with_subscription_id(42)      // Track which subscription matched
///     .build()
///     .unwrap();
/// ```
///
/// ## Multiple Topics
/// ```no_run
/// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
/// let cmd = SubscribeCommand::builder()
///     .add_topic("sensors/temp", 1)
///     .with_no_local(true)
///     .add_topic("sensors/humidity", 2)
///     .with_retain_as_published(true)
///     .build()
///     .unwrap();
/// ```
///
/// # MQTT v5 Subscription Options
///
/// - **No Local**: If true, server won't forward messages published by this client
/// - **Retain As Published**: If true, retain flag is kept as-is from publisher
/// - **Retain Handling**:
///   - 0: Send retained messages at subscribe time (default)
///   - 1: Send retained only if subscription doesn't exist
///   - 2: Don't send retained messages
///
#[derive(Debug, Clone)]
pub struct SubscribeCommandBuilder {
    topics: Vec<TopicSubscription>,
    properties: Vec<Property>,
    packet_id: Option<u16>,
}

/// Error type for builder validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeBuilderError {
    /// No topics were added to the subscription
    NoTopics,
}

impl std::fmt::Display for SubscribeBuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTopics => write!(
                f,
                "No topics added to subscription. Call add_topic() at least once."
            ),
        }
    }
}

impl std::error::Error for SubscribeBuilderError {}

impl SubscribeCommandBuilder {
    /// Create a new builder with default values
    pub fn new() -> Self {
        Self {
            topics: Vec::new(),
            properties: Vec::new(),
            packet_id: None,
        }
    }

    /// Add a topic with QoS (uses default options)
    ///
    /// Default options:
    /// - no_local: false
    /// - retain_as_published: false
    /// - retain_handling: 0 (send retained messages)
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/temp", 1)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn add_topic(mut self, topic: impl Into<String>, qos: u8) -> Self {
        let subscription = TopicSubscription::new_simple(topic.into(), qos);
        self.topics.push(subscription);
        self
    }

    /// Add a topic with full subscription options
    ///
    /// # Arguments
    /// * `topic` - Topic filter (can include wildcards)
    /// * `qos` - Quality of Service level (0, 1, or 2)
    /// * `no_local` - If true, server won't forward messages published by this client
    /// * `retain_as_published` - If true, retain flag is kept as-is from publisher
    /// * `retain_handling` - 0: send retained messages, 1: send only on new sub, 2: don't send
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic_with_options("sensors/temp", 1, true, false, 2)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn add_topic_with_options(
        mut self,
        topic: impl Into<String>,
        qos: u8,
        no_local: bool,
        retain_as_published: bool,
        retain_handling: u8,
    ) -> Self {
        let subscription = TopicSubscription::new(
            topic.into(),
            qos,
            no_local,
            retain_as_published,
            retain_handling,
        );
        self.topics.push(subscription);
        self
    }

    /// Set No Local flag for the last added topic
    ///
    /// If true, the server will not forward messages published by this client
    /// to its own subscriptions.
    ///
    /// # Panics
    /// Panics if no topics have been added yet.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/temp", 1)
    ///     .with_no_local(true)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_no_local(mut self, no_local: bool) -> Self {
        if let Some(last) = self.topics.last_mut() {
            last.no_local = no_local;
        } else {
            panic!("Cannot set no_local: no topics added yet. Call add_topic() first.");
        }
        self
    }

    /// Set Retain As Published flag for the last added topic
    ///
    /// If true, the retain flag is forwarded as-is from the publisher.
    /// If false, the retain flag is always set to 0 for forwarded messages.
    ///
    /// # Panics
    /// Panics if no topics have been added yet.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/temp", 1)
    ///     .with_retain_as_published(true)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_retain_as_published(mut self, rap: bool) -> Self {
        if let Some(last) = self.topics.last_mut() {
            last.retain_as_published = rap;
        } else {
            panic!("Cannot set retain_as_published: no topics added yet. Call add_topic() first.");
        }
        self
    }

    /// Set Retain Handling option for the last added topic
    ///
    /// Values:
    /// - 0: Send retained messages at subscribe time
    /// - 1: Send retained messages only if subscription doesn't exist
    /// - 2: Don't send retained messages at subscribe time
    ///
    /// # Panics
    /// Panics if no topics have been added yet, or if value > 2.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/temp", 1)
    ///     .with_retain_handling(2)  // Don't send retained
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_retain_handling(mut self, rh: u8) -> Self {
        if rh > 2 {
            panic!("Invalid retain_handling value: {}. Must be 0, 1, or 2.", rh);
        }
        if let Some(last) = self.topics.last_mut() {
            last.retain_handling = rh;
        } else {
            panic!("Cannot set retain_handling: no topics added yet. Call add_topic() first.");
        }
        self
    }

    /// Set the Subscription Identifier property
    ///
    /// The Subscription Identifier is included in PUBLISH packets to indicate
    /// which subscription(s) matched the message.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/#", 1)
    ///     .with_subscription_id(42)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_subscription_id(mut self, id: u32) -> Self {
        self.properties.push(Property::SubscriptionIdentifier(id));
        self
    }

    /// Add a custom MQTT v5 property
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// # use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/#", 1)
    ///     .add_property(Property::UserProperty("key".into(), "value".into()))
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn add_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }

    /// Set the packet identifier (usually managed automatically)
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/#", 1)
    ///     .with_packet_id(123)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_packet_id(mut self, id: u16) -> Self {
        self.packet_id = Some(id);
        self
    }

    /// Build the final SubscribeCommand
    ///
    /// # Errors
    /// Returns `SubscribeBuilderError::NoTopics` if no topics were added.
    ///
    /// # Example
    /// ```no_run
    /// # use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/#", 1)
    ///     .with_subscription_id(42)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn build(self) -> Result<SubscribeCommand, SubscribeBuilderError> {
        if self.topics.is_empty() {
            return Err(SubscribeBuilderError::NoTopics);
        }

        Ok(SubscribeCommand {
            packet_id: self.packet_id,
            subscriptions: self.topics,
            properties: self.properties,
        })
    }
}

impl Default for SubscribeCommandBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Fully customizable unsubscribe command for MQTT v5
#[derive(Debug, Clone)]
pub struct UnsubscribeCommand {
    pub packet_id: Option<u16>,
    pub topics: Vec<String>,
    pub properties: Vec<Property>,
}

impl UnsubscribeCommand {
    pub fn new(packet_id: Option<u16>, topics: Vec<String>, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            topics,
            properties,
        }
    }

    pub fn from_topics(topics: Vec<String>) -> Self {
        Self::new(None, topics, Vec::new())
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
        response_tx: tokio::sync::oneshot::Sender<ConnectionResult>,
    },
    /// Subscribe to topics
    Subscribe(SubscribeCommand),
    /// Subscribe to topics and wait for acknowledgment
    SubscribeSync {
        command: SubscribeCommand,
        response_tx: tokio::sync::oneshot::Sender<SubscribeResult>,
    },
    /// Publish a message (fire-and-forget)
    Publish(PublishCommand),
    /// Publish a message and wait for acknowledgment (QoS 1 = PUBACK, QoS 2 = PUBCOMP)
    PublishSync {
        command: PublishCommand,
        response_tx: tokio::sync::oneshot::Sender<PublishResult>,
    },
    /// Unsubscribe from topics
    Unsubscribe(UnsubscribeCommand),
    /// Unsubscribe from topics and wait for acknowledgment
    UnsubscribeSync {
        command: UnsubscribeCommand,
        response_tx: tokio::sync::oneshot::Sender<UnsubscribeResult>,
    },
    /// Send ping to broker
    Ping,
    /// Send ping to broker and wait for response
    PingSync {
        response_tx: tokio::sync::oneshot::Sender<PingResult>,
    },
    /// Disconnect from broker
    Disconnect,
    /// Shutdown the client and worker
    Shutdown,
    /// Enable/disable automatic reconnection
    SetAutoReconnect { enabled: bool },
    /// Send a raw MQTT packet directly
    SendPacket(MqttPacket),
    /// Send AUTH packet for enhanced authentication (MQTT v5)
    Auth {
        reason_code: u8,
        properties: Vec<Property>,
    },
}

/// Configuration for the tokio async client
#[derive(Debug, Clone)]
pub struct TokioAsyncClientConfig {
    /// Enable automatic reconnection on connection loss
    pub auto_reconnect: bool,
    /// Initial reconnect delay in milliseconds
    pub reconnect_delay_ms: u64,
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
    /// Send buffer size limit
    pub send_buffer_size: usize,
    /// Recv buffer size limit
    pub recv_buffer_size: usize,
    /// Keep alive interval in seconds
    pub keep_alive_interval: u64,
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
    /// Default timeout for operations without specific timeout (milliseconds)
    pub default_operation_timeout_ms: u64,
    /// Maximum number of QoS 1 and QoS 2 publications that the client
    /// is willing to process concurrently (MQTT v5 only)
    /// None = use default (65535), Some(0) is invalid
    pub receive_maximum: Option<u16>,
    /// Maximum number of topic aliases that the client accepts from the server (MQTT v5 only)
    /// None or Some(0) = topic aliases not supported
    /// Valid range: 0-65535
    pub topic_alias_maximum: Option<u16>,
}

impl Default for TokioAsyncClientConfig {
    fn default() -> Self {
        TokioAsyncClientConfig {
            auto_reconnect: true,
            reconnect_delay_ms: 1000,
            max_reconnect_delay_ms: 30000,
            max_reconnect_attempts: 0, // infinite
            command_queue_size: 1000,
            buffer_messages: true,
            max_buffer_size: 1000,
            send_buffer_size: 1000,
            recv_buffer_size: 1000,
            keep_alive_interval: 60,
            tcp_nodelay: true,
            connect_timeout_ms: Some(30000),     // 30 seconds
            subscribe_timeout_ms: Some(10000),   // 10 seconds
            publish_ack_timeout_ms: Some(10000), // 10 seconds
            unsubscribe_timeout_ms: Some(10000), // 10 seconds
            ping_timeout_ms: Some(5000),         // 5 seconds
            default_operation_timeout_ms: 30000, // 30 seconds
            receive_maximum: None,               // Use MQTT v5 default (65535)
            topic_alias_maximum: None,           // Topic aliases not supported by default
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
    ///     .reconnect_delay_ms(2000)
    ///     .connect_timeout_ms(60000)
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
///     .reconnect_delay_ms(2000)
///     .max_reconnect_attempts(10)
///     .connect_timeout_ms(60000)
///     .subscribe_timeout_ms(5000)
///     .send_buffer_size(2000)
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

    /// Set initial reconnect delay in milliseconds
    pub fn reconnect_delay_ms(mut self, delay_ms: u64) -> Self {
        self.config.reconnect_delay_ms = delay_ms;
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

    /// Set default timeout for operations without specific timeout (milliseconds)
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

    /// Set send buffer size limit
    pub fn send_buffer_size(mut self, size: usize) -> Self {
        self.config.send_buffer_size = size;
        self
    }

    /// Set receive buffer size limit
    pub fn recv_buffer_size(mut self, size: usize) -> Self {
        self.config.recv_buffer_size = size;
        self
    }

    // ==================== Other Settings ====================

    /// Set keep alive interval in seconds
    pub fn keep_alive_interval(mut self, seconds: u64) -> Self {
        self.config.keep_alive_interval = seconds;
        self
    }

    /// Enable or disable TCP_NODELAY (disable Nagle algorithm)
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.config.tcp_nodelay = enabled;
        self
    }

    // ==================== Convenience Methods ====================

    /// Disable all operation timeouts (operations wait indefinitely)
    ///
    /// Useful for development/debugging or networks with unpredictable latency.
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
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Async stream for outbound MQTT frame bytes.
struct AsyncEgressStream {
    sender: mpsc::Sender<Vec<u8>>,
    receiver: Option<Receiver<Vec<u8>>>,
    capacity: usize,
}

impl AsyncEgressStream {
    fn new(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::channel(capacity);
        Self {
            sender,
            receiver: Some(receiver),
            capacity,
        }
    }

    /// Non-blocking enqueue for outbound bytes. Returns WouldBlock when channel is full.
    #[allow(dead_code)]
    fn try_send_bytes(&self, data: Vec<u8>) -> io::Result<()> {
        match self.sender.try_send(data) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Egress buffer full",
            )),
            Err(TrySendError::Closed(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Egress stream closed",
            )),
        }
    }

    /// Best-effort enqueue that awaits for capacity when needed.
    #[allow(dead_code)]
    async fn send_bytes(&self, data: Vec<u8>) -> io::Result<()> {
        match self.sender.try_send(data) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(data)) => self
                .sender
                .send(data)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Egress stream closed")),
            Err(TrySendError::Closed(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Egress stream closed",
            )),
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn take_receiver(&mut self) -> Receiver<Vec<u8>> {
        self.receiver.take().expect("egress receiver already taken")
    }

    fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Async stream for inbound MQTT packets, fed by transport reads.
struct AsyncIngressStream {
    sender: mpsc::Sender<MqttPacket>,
    receiver: Option<Receiver<MqttPacket>>,
    raw_buffer: BytesMut,
    pending_packets: VecDeque<MqttPacket>,
    max_pending: usize,
}

impl AsyncIngressStream {
    fn new(max_pending: usize) -> Self {
        let (sender, receiver) = mpsc::channel(max_pending);
        Self {
            sender,
            receiver: Some(receiver),
            raw_buffer: BytesMut::with_capacity(16 * 1024),
            pending_packets: VecDeque::new(),
            max_pending,
        }
    }

    fn take_receiver(&mut self) -> Receiver<MqttPacket> {
        self.receiver
            .take()
            .expect("ingress receiver already taken")
    }

    /// Push raw transport bytes, parse MQTT packets, and forward to consumers.
    fn push_raw_data(&mut self, data: &[u8], mqtt_version: u8) -> Result<(), MqttClientError> {
        self.raw_buffer.extend_from_slice(data);
        self.flush_pending()?;

        while let Some((packet, consumed)) = self.try_parse_next_packet(mqtt_version)? {
            match self.sender.try_send(packet) {
                Ok(()) => {
                    self.raw_buffer.advance(consumed);
                }
                Err(TrySendError::Full(packet)) => {
                    if self.pending_packets.len() >= self.max_pending {
                        return Err(MqttClientError::BufferFull {
                            buffer_type: "ingress buffer".to_string(),
                            capacity: self.max_pending,
                        });
                    }
                    self.pending_packets.push_back(packet);
                    self.raw_buffer.advance(consumed);
                    break;
                }
                Err(TrySendError::Closed(_)) => {
                    return Err(MqttClientError::ChannelClosed {
                        channel: "ingress stream".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Allow consumers to wake pending packets after they drain channel capacity.
    fn flush_pending(&mut self) -> Result<(), MqttClientError> {
        while let Some(packet) = self.pending_packets.pop_front() {
            match self.sender.try_send(packet) {
                Ok(()) => continue,
                Err(TrySendError::Full(packet)) => {
                    self.pending_packets.push_front(packet);
                    break;
                }
                Err(TrySendError::Closed(_)) => {
                    return Err(MqttClientError::ChannelClosed {
                        channel: "ingress stream".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    fn try_parse_next_packet(
        &self,
        mqtt_version: u8,
    ) -> Result<Option<(MqttPacket, usize)>, MqttClientError> {
        if self.raw_buffer.is_empty() {
            return Ok(None);
        }

        match MqttPacket::from_bytes_with_version(&self.raw_buffer[..], mqtt_version) {
            Ok(ParseOk::Packet(packet, consumed)) => Ok(Some((packet, consumed))),
            Ok(ParseOk::Continue(_, _)) => Ok(None),
            Ok(ParseOk::TopicName(_, _)) => Err(MqttClientError::ProtocolViolation {
                message: "Unexpected ParseOk variant: TopicName".to_string(),
            }),
            Err(ParseError::More(_, _))
            | Err(ParseError::BufferTooShort)
            | Err(ParseError::BufferEmpty) => Ok(None), // Need more data
            Err(e) => {
                // Capture raw data for debugging (first 100 bytes)
                let raw_data = self.raw_buffer.iter().take(100).copied().collect();
                Err(MqttClientError::PacketParsing {
                    parse_error: format!("{:?}", e),
                    raw_data,
                })
            }
        }
    }
}

/// Holder for ingress/egress async streams.
struct AsyncMQTTStreams {
    egress: AsyncEgressStream,
    ingress: AsyncIngressStream,
}

impl AsyncMQTTStreams {
    fn new(max_send_size: usize, max_recv_size: usize) -> Self {
        Self {
            egress: AsyncEgressStream::new(max_send_size.max(1)),
            ingress: AsyncIngressStream::new(max_recv_size.max(1)),
        }
    }

    fn take_receivers(&mut self) -> (Receiver<Vec<u8>>, Receiver<MqttPacket>) {
        (self.egress.take_receiver(), self.ingress.take_receiver())
    }

    #[allow(dead_code)]
    fn try_send_egress(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.egress.try_send_bytes(bytes)
    }

    #[allow(dead_code)]
    async fn send_egress(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.egress.send_bytes(bytes).await
    }

    fn push_ingress_data(&mut self, data: &[u8], mqtt_version: u8) -> Result<(), MqttClientError> {
        self.ingress.push_raw_data(data, mqtt_version)
    }

    fn flush_ingress_pending(&mut self) -> Result<(), MqttClientError> {
        self.ingress.flush_pending()
    }

    #[allow(dead_code)]
    fn egress_capacity(&self) -> usize {
        self.egress.capacity()
    }
}

/// Tokio-based async MQTT client
pub struct TokioAsyncMqttClient {
    /// Command sender to worker task
    command_tx: mpsc::Sender<TokioClientCommand>,
    /// Client configuration
    _config: TokioAsyncClientConfig,
}

impl TokioAsyncMqttClient {
    /// Create a new tokio async MQTT client
    pub async fn new(
        mqtt_options: MqttClientOptions,
        event_handler: Box<dyn TokioMqttEventHandler>,
        config: TokioAsyncClientConfig,
    ) -> io::Result<Self> {
        let (command_tx, command_rx) = mpsc::channel(config.command_queue_size);

        // Spawn the worker task
        let worker =
            TokioClientWorker::new(mqtt_options, event_handler, command_rx, config.clone());
        tokio::spawn(async move {
            worker.run().await;
        });

        Ok(TokioAsyncMqttClient {
            command_tx,
            _config: config,
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
        F: std::future::Future<Output = io::Result<T>>,
    {
        if let Some(timeout) = timeout_ms {
            match tokio::time::timeout(Duration::from_millis(timeout), future).await {
                Ok(result) => result.map_err(|e| MqttClientError::from_io_error(e, operation_name)),
                Err(_) => Err(MqttClientError::OperationTimeout {
                    operation: operation_name.to_string(),
                    timeout_ms: timeout,
                }),
            }
        } else {
            future
                .await
                .map_err(|e| MqttClientError::from_io_error(e, operation_name))
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

    /// Publish with fully customized command
    pub async fn publish_with_command(&self, command: PublishCommand) -> io::Result<()> {
        self.send_command(TokioClientCommand::Publish(command))
            .await
    }

    /// Publish a message and wait for acknowledgment (QoS 1 = PUBACK, QoS 2 = PUBCOMP)
    ///
    /// This method blocks until the broker acknowledges the message:
    /// - QoS 0: Returns immediately after sending (no acknowledgment)
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
    async fn publish_sync_internal(&self, command: PublishCommand) -> io::Result<PublishResult> {
        // QoS 0 messages have no acknowledgment, send fire-and-forget
        if command.qos == 0 {
            self.send_command(TokioClientCommand::Publish(command))
                .await?;
            return Ok(PublishResult {
                packet_id: None,
                reason_code: Some(0),
                properties: None,
                qos: 0,
            });
        }

        // For QoS 1/2, use oneshot channel to wait for acknowledgment
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.send_command(TokioClientCommand::PublishSync {
            command,
            response_tx: tx,
        })
        .await?;

        // Block until response received or channel closed
        rx.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Response channel closed before acknowledgment received (connection may be lost)",
            )
        })
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
        self.send_command(TokioClientCommand::Disconnect).await
    }

    /// Enable or disable automatic reconnection
    pub async fn set_auto_reconnect(&self, enabled: bool) -> io::Result<()> {
        self.send_command(TokioClientCommand::SetAutoReconnect { enabled })
            .await
    }

    /// Shutdown the client
    pub async fn shutdown(self) -> io::Result<()> {
        self.send_command(TokioClientCommand::Shutdown).await
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
    async fn connect_sync_internal(&self) -> io::Result<ConnectionResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.send_command(TokioClientCommand::ConnectSync { response_tx: tx })
            .await?;

        rx.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Response channel closed before CONNACK received (connection may have failed)",
            )
        })
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
    ) -> io::Result<SubscribeResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.send_command(TokioClientCommand::SubscribeSync {
            command,
            response_tx: tx,
        })
        .await?;

        rx.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Response channel closed before SUBACK received (connection may be lost)",
            )
        })
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
    ) -> io::Result<UnsubscribeResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.send_command(TokioClientCommand::UnsubscribeSync {
            command,
            response_tx: tx,
        })
        .await?;

        rx.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Response channel closed before UNSUBACK received (connection may be lost)",
            )
        })
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
    async fn ping_sync_internal(&self) -> io::Result<PingResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.send_command(TokioClientCommand::PingSync { response_tx: tx })
            .await?;

        rx.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Response channel closed before PINGRESP received (connection may be lost)",
            )
        })
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
    /// - QoS 0: Returns immediately after sending (no acknowledgment)
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

/// Worker task that handles MQTT operations using tokio
struct TokioClientWorker {
    /// MQTT client options
    options: MqttClientOptions,
    /// Event handler for callbacks
    event_handler: Box<dyn TokioMqttEventHandler>,
    /// Command receiver from main client
    command_rx: mpsc::Receiver<TokioClientCommand>,
    /// Worker configuration
    config: TokioAsyncClientConfig,
    /// TCP stream to MQTT broker
    stream: Option<TcpStream>,
    /// Async ingress/egress streams
    streams: AsyncMQTTStreams,
    /// Current connection state
    is_connected: bool,
    /// Reconnection state
    reconnect_attempts: u32,
    /// Message buffer for when disconnected
    message_buffer: VecDeque<PublishCommand>,
    /// Pending subscribe operations keyed by packet identifier
    pending_subscribes: HashMap<u16, Vec<String>>,
    /// Pending unsubscribe operations keyed by packet identifier
    pending_unsubscribes: HashMap<u16, Vec<String>>,
    /// Pending synchronous publish operations keyed by packet identifier
    pending_publishes: HashMap<u16, tokio::sync::oneshot::Sender<PublishResult>>,
    /// Pending synchronous connect operations (only one at a time)
    pending_connect: Option<tokio::sync::oneshot::Sender<ConnectionResult>>,
    /// Pending synchronous subscribe operations keyed by packet identifier
    pending_subscribes_sync: HashMap<u16, tokio::sync::oneshot::Sender<SubscribeResult>>,
    /// Pending synchronous unsubscribe operations keyed by packet identifier
    pending_unsubscribes_sync: HashMap<u16, tokio::sync::oneshot::Sender<UnsubscribeResult>>,
    /// Pending synchronous ping operations (only one at a time)
    pending_ping: Option<tokio::sync::oneshot::Sender<PingResult>>,
    /// Client session for packet IDs
    session: Option<ClientSession>,
    /// Keep alive timer (dynamic sleep based on last packet sent)
    keep_alive_timer: Option<Pin<Box<Sleep>>>,
    /// MQTT protocol version (3, 4, or 5)
    mqtt_version: u8,
    /// Last time any control packet was SENT (for keep-alive compliance)
    last_packet_sent: Instant,
}

impl TokioClientWorker {
    fn new(
        options: MqttClientOptions,
        event_handler: Box<dyn TokioMqttEventHandler>,
        command_rx: mpsc::Receiver<TokioClientCommand>,
        config: TokioAsyncClientConfig,
    ) -> Self {
        let send_capacity = config.send_buffer_size;
        let recv_capacity = config.recv_buffer_size;
        TokioClientWorker {
            options,
            event_handler,
            command_rx,
            config,
            stream: None,
            streams: AsyncMQTTStreams::new(send_capacity, recv_capacity),
            is_connected: false,
            reconnect_attempts: 0,
            message_buffer: VecDeque::new(),
            pending_subscribes: HashMap::new(),
            pending_unsubscribes: HashMap::new(),
            pending_publishes: HashMap::new(),
            pending_connect: None,
            pending_subscribes_sync: HashMap::new(),
            pending_unsubscribes_sync: HashMap::new(),
            pending_ping: None,
            session: None,
            keep_alive_timer: None,
            mqtt_version: 5, // Default to MQTT v5.0, will be auto-detected from first packet
            last_packet_sent: Instant::now(),
        }
    }

    /// Main async event loop coordinating command handling and socket I/O.
    async fn run(mut self) {
        let (mut egress_rx, mut ingress_rx) = self.streams.take_receivers();

        loop {
            tokio::select! {
                // Handle COMMANDS from the client API - highest priority
                cmd = self.command_rx.recv() => {
                    match cmd {
                        Some(command) => {
                            if !self.handle_command(command).await {
                                break; // Shutdown requested
                            }
                        }
                        None => {
                            // Channel closed, shutdown
                            break;
                        }
                    }
                }

                // EGRESS: Drain channel data and write to the transport
                frame = egress_rx.recv() => {
                    if let Some(frame) = frame {
                        if let Err(mqtt_err) = self.handle_egress_frame(frame, &mut egress_rx).await {
                            self.event_handler.on_error(&mqtt_err).await;
                            self.handle_connection_lost().await;
                            continue;
                        }
                    } else {
                        break;
                    }
                }

                // INGRESS: Read raw bytes from transport and push into ingress stream
                read_result = async {
                    if let Some(stream) = &mut self.stream {
                        let mut buffer = vec![0u8; 4096];
                        match stream.read(&mut buffer).await {
                            Ok(0) => Ok(None),
                            Ok(n) => {
                                buffer.truncate(n);
                                Ok(Some(buffer))
                            }
                            Err(e) => Err(e),
                        }
                    } else {
                        std::future::pending::<io::Result<Option<Vec<u8>>>>().await
                    }
                } => {
                    match read_result {
                        Ok(Some(bytes)) => {
                            if let Err(mqtt_err) = self.streams.push_ingress_data(&bytes, self.mqtt_version) {
                                // Emit ParseError event if it's a packet parsing error
                                if let MqttClientError::PacketParsing { parse_error, raw_data } = &mqtt_err {
                                    let parse_err = ParseError::ParseError(parse_error.clone());
                                    self.event_handler.on_parse_error(
                                        raw_data,
                                        &parse_err,
                                        mqtt_err.is_recoverable()
                                    ).await;
                                }

                                self.event_handler.on_error(&mqtt_err).await;
                                self.handle_connection_lost().await;
                                continue;
                            }
                        }
                        Ok(None) => {
                            let mqtt_err = MqttClientError::ConnectionLost {
                                reason: "Connection closed by server".to_string(),
                            };
                            self.event_handler.on_error(&mqtt_err).await;
                            self.handle_connection_lost().await;
                            continue;
                        }
                        Err(e) => {
                            let mqtt_err = MqttClientError::from_io_error(e, "transport read");
                            self.event_handler.on_error(&mqtt_err).await;
                            self.handle_connection_lost().await;
                            continue;
                        }
                    }
                }

                // Consume parsed MQTT packets from ingress stream
                packet = ingress_rx.recv() => {
                    if let Some(packet) = packet {
                        self.handle_mqtt_packet(packet).await;
                        if let Err(mqtt_err) = self.streams.flush_ingress_pending() {
                            self.event_handler.on_error(&mqtt_err).await;
                        }
                    } else {
                        break;
                    }
                }

                // Keep alive timer - only when connected and timer exists
                _ = async {
                    if let Some(ref mut timer) = self.keep_alive_timer {
                        timer.as_mut().await;
                        true
                    } else {
                        std::future::pending::<bool>().await
                    }
                } => {
                    self.check_keep_alive().await;
                }
            }
        }
    }

    /// Handle commands from the client API
    async fn handle_command(&mut self, command: TokioClientCommand) -> bool {
        match command {
            TokioClientCommand::Connect => {
                self.handle_connect().await;
            }
            TokioClientCommand::ConnectSync { response_tx } => {
                self.handle_connect_sync(response_tx).await;
            }
            TokioClientCommand::Subscribe(command) => {
                self.handle_subscribe(command).await;
            }
            TokioClientCommand::SubscribeSync {
                command,
                response_tx,
            } => {
                self.handle_subscribe_sync(command, response_tx).await;
            }
            TokioClientCommand::Publish(command) => {
                self.handle_publish(command).await;
            }
            TokioClientCommand::PublishSync {
                command,
                response_tx,
            } => {
                self.handle_publish_sync(command, response_tx).await;
            }
            TokioClientCommand::Unsubscribe(command) => {
                self.handle_unsubscribe(command).await;
            }
            TokioClientCommand::UnsubscribeSync {
                command,
                response_tx,
            } => {
                self.handle_unsubscribe_sync(command, response_tx).await;
            }
            TokioClientCommand::Ping => {
                self.handle_ping().await;
            }
            TokioClientCommand::PingSync { response_tx } => {
                self.handle_ping_sync(response_tx).await;
            }
            TokioClientCommand::Disconnect => {
                self.handle_disconnect().await;
            }
            TokioClientCommand::Shutdown => {
                return false; // Exit event loop
            }
            TokioClientCommand::SetAutoReconnect { enabled } => {
                self.config.auto_reconnect = enabled;
            }
            TokioClientCommand::SendPacket(packet) => {
                if let Err(e) = self.send_packet_to_broker(&packet).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send packet");
                    self.event_handler.on_error(&mqtt_err).await;
                }
            }
            TokioClientCommand::Auth {
                reason_code,
                properties,
            } => {
                self.handle_auth(reason_code, properties).await;
            }
        }
        true
    }

    /// Drain the egress channel and push MQTT frames onto the transport.
    async fn handle_egress_frame(
        &mut self,
        first: Vec<u8>,
        egress_rx: &mut Receiver<Vec<u8>>,
    ) -> Result<(), MqttClientError> {
        self.write_to_transport(&first).await?;

        // Opportunistically drain any additional frames that are ready.
        loop {
            match egress_rx.try_recv() {
                Ok(frame) => {
                    self.write_to_transport(&frame).await?;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        Ok(())
    }

    async fn write_to_transport(&mut self, frame: &[u8]) -> Result<(), MqttClientError> {
        if let Some(stream) = &mut self.stream {
            match stream.write_all(frame).await {
                Ok(_) => {
                    // Update last send time for keep-alive tracking
                    self.last_packet_sent = Instant::now();
                    Ok(())
                }
                Err(e) => {
                    // Categorize network errors for better error handling
                    let error = match e.kind() {
                        io::ErrorKind::ConnectionReset => MqttClientError::NetworkError {
                            kind: e.kind(),
                            message: "Connection reset by peer".to_string(),
                        },
                        io::ErrorKind::BrokenPipe => MqttClientError::NetworkError {
                            kind: e.kind(),
                            message: "Broken pipe (connection closed)".to_string(),
                        },
                        io::ErrorKind::UnexpectedEof => MqttClientError::NetworkError {
                            kind: e.kind(),
                            message: "Unexpected end of file".to_string(),
                        },
                        io::ErrorKind::ConnectionAborted => MqttClientError::NetworkError {
                            kind: e.kind(),
                            message: "Connection aborted".to_string(),
                        },
                        io::ErrorKind::TimedOut => MqttClientError::NetworkError {
                            kind: e.kind(),
                            message: "Write operation timed out".to_string(),
                        },
                        _ => MqttClientError::from_io_error(e, "write_to_transport"),
                    };
                    Err(error)
                }
            }
        } else {
            Err(MqttClientError::NotConnected)
        }
    }

    /// Check keep alive and send PINGREQ if needed
    ///
    /// MQTT Keep-Alive Specification:
    /// - The client MUST send a control packet within the Keep Alive period
    /// - ANY control packet counts (PUBLISH, SUBSCRIBE, PUBACK, etc.)
    /// - PINGREQ is only sent if NO other packet was sent during the period
    /// - The broker will disconnect if it doesn't receive ANY packet within 1.5x Keep Alive
    async fn check_keep_alive(&mut self) {
        if !self.is_connected || self.options.keep_alive == 0 {
            return;
        }

        let keep_alive_duration = Duration::from_secs(self.options.keep_alive as u64);
        let time_since_last_send = self.last_packet_sent.elapsed();

        // Only send PINGREQ if we haven't sent ANY packet within the keep-alive period
        if time_since_last_send >= keep_alive_duration {
            // Send PINGREQ to satisfy keep-alive requirement
            let pingreq = pingreqv5::MqttPingReq::new();
            match pingreq.to_bytes() {
                Ok(bytes) => {
                    if let Err(e) = self.streams.send_egress(bytes).await {
                        let mqtt_err = MqttClientError::from_io_error(e, "send keep-alive PINGREQ");
                        self.event_handler.on_error(&mqtt_err).await;
                        self.handle_connection_lost().await;
                        return; // Don't reset timer if connection lost
                    }
                    // last_packet_sent will be updated in write_to_transport
                }
                Err(err) => {
                    let mqtt_err = MqttClientError::ProtocolViolation {
                        message: format!("Failed to serialize PINGREQ: {:?}", err),
                    };
                    self.event_handler.on_error(&mqtt_err).await;
                    return; // Don't reset timer on error
                }
            }
        }

        // Reset timer based on time remaining until next keep-alive check
        self.reset_keep_alive_timer();
    }

    /// Reset the keep-alive timer based on when we last sent a packet
    fn reset_keep_alive_timer(&mut self) {
        if self.options.keep_alive == 0 {
            return;
        }

        let keep_alive_duration = Duration::from_secs(self.options.keep_alive as u64);
        let time_since_last_send = self.last_packet_sent.elapsed();

        // Calculate when we need to check again
        // If we just sent a packet (including PINGREQ), schedule for full keep-alive duration
        let time_until_next_check = if time_since_last_send < keep_alive_duration {
            // Schedule for when keep-alive period expires
            keep_alive_duration - time_since_last_send
        } else {
            // Edge case: already past keep-alive (shouldn't happen normally)
            // Check immediately by using a very small duration
            Duration::from_millis(100)
        };

        self.keep_alive_timer = Some(Box::pin(tokio::time::sleep(time_until_next_check)));
    }

    /// Handle connecting to broker
    async fn handle_connect(&mut self) {
        // Avoid concurrent connect attempts if a socket already exists
        if self.stream.is_some() {
            return;
        }

        let peer = self.options.peer.clone();

        match TcpStream::connect(&peer).await {
            Ok(stream) => {
                if self.config.tcp_nodelay {
                    if let Err(e) = stream.set_nodelay(true) {
                        let mqtt_err = MqttClientError::from_io_error(e, "TCP set_nodelay");
                        self.event_handler.on_error(&mqtt_err).await;
                    }
                }

                // Install the socket before enqueuing bytes so the writer branch can flush
                self.stream = Some(stream);

                // Reset reconnect attempts on successful TCP connect
                self.reconnect_attempts = 0;

                // Session management mirrors the synchronous client behaviour
                if self.options.sessionless {
                    self.session = None;
                    self.options.clean_start = true;
                } else if self.session.is_none() {
                    self.session = Some(ClientSession::new());
                }

                // Prepare CONNECT packet (MQTT v5 by default for now)
                let mut properties = vec![];

                // Add session expiry interval if specified
                if let Some(expiry_interval) = self.options.session_expiry_interval {
                    properties.push(Property::SessionExpiryInterval(expiry_interval));
                }

                let connect_packet = connectv5::MqttConnect::new(
                    self.options.client_id.clone(),
                    self.options.username.clone(),
                    self.options.password.clone(),
                    self.options.will.clone(),
                    self.options.keep_alive,
                    self.options.clean_start,
                    properties,
                );

                let connect_bytes = match connect_packet.to_bytes() {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        let mqtt_err = MqttClientError::ProtocolViolation {
                            message: format!("Failed to serialize CONNECT packet: {:?}", err),
                        };
                        self.event_handler.on_error(&mqtt_err).await;
                        return;
                    }
                };

                if let Err(e) = self.streams.send_egress(connect_bytes).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send CONNECT packet");
                    self.event_handler.on_error(&mqtt_err).await;
                    self.stream = None;
                    self.is_connected = false;
                    return;
                }

                self.is_connected = true;
                self.mqtt_version = 5; // currently only MQTT v5 is supported in async client

                // Reset send tracking for keep-alive
                self.last_packet_sent = Instant::now();

                // Set up dynamic keep-alive timer
                self.reset_keep_alive_timer();

                if self.config.buffer_messages {
                    self.flush_message_buffer().await;
                }
            }
            Err(e) => {
                let mqtt_err = MqttClientError::from_io_error(e, "TCP connect");
                self.event_handler.on_error(&mqtt_err).await;
            }
        }
    }

    /// Handle subscribe command
    async fn handle_subscribe(&mut self, mut command: SubscribeCommand) {
        if command.subscriptions.is_empty() {
            let mqtt_err = MqttClientError::InvalidState {
                expected: "at least one topic subscription".to_string(),
                actual: "empty subscription list".to_string(),
            };
            self.event_handler.on_error(&mqtt_err).await;
            return;
        }

        let packet_id = if let Some(id) = command.packet_id {
            id
        } else if let Some(session) = self.session.as_mut() {
            session.next_packet_id()
        } else {
            let mqtt_err = MqttClientError::NoActiveSession;
            self.event_handler.on_error(&mqtt_err).await;
            return;
        };

        command.packet_id = Some(packet_id);

        let subscribe_packet = subscribev5::MqttSubscribe::new(
            packet_id,
            command.subscriptions.clone(),
            command.properties.clone(),
        );

        match subscribe_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = self.streams.send_egress(bytes).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send SUBSCRIBE packet");
                    self.event_handler.on_error(&mqtt_err).await;
                    return;
                }

                let topics: Vec<String> = command
                    .subscriptions
                    .iter()
                    .map(|sub| sub.topic_filter.clone())
                    .collect();
                self.pending_subscribes.insert(packet_id, topics);
            }
            Err(err) => {
                let mqtt_err = MqttClientError::ProtocolViolation {
                    message: format!("Failed to serialize SUBSCRIBE packet: {:?}", err),
                };
                self.event_handler.on_error(&mqtt_err).await;
            }
        }
    }

    /// Handle publish command
    async fn handle_publish(&mut self, command: PublishCommand) {
        if self.stream.is_none() && self.config.buffer_messages {
            if self.message_buffer.len() >= self.config.max_buffer_size {
                let mqtt_err = MqttClientError::BufferFull {
                    buffer_type: "publish buffer".to_string(),
                    capacity: self.config.max_buffer_size,
                };
                self.event_handler.on_error(&mqtt_err).await;
                return;
            }

            self.message_buffer.push_back(command);
            return;
        }

        if self.stream.is_none() {
            let mqtt_err = MqttClientError::NotConnected;
            self.event_handler.on_error(&mqtt_err).await;
            return;
        }

        if let Err(e) = self.send_publish_command(command).await {
            let mqtt_err = MqttClientError::from_io_error(e, "send publish");
            self.event_handler.on_error(&mqtt_err).await;
        }
    }

    /// Handle synchronous publish command
    async fn handle_publish_sync(
        &mut self,
        command: PublishCommand,
        response_tx: oneshot::Sender<PublishResult>,
    ) {
        // Check connection status
        if self.stream.is_none() {
            // For disconnected state, send an error result
            let result = PublishResult {
                packet_id: command.packet_id,
                reason_code: Some(128), // Unspecified error
                properties: Some(vec![]),
                qos: command.qos,
            };
            let _ = response_tx.send(result);
            return;
        }

        // Get packet ID from command or generate new one
        let packet_id = if let Some(id) = command.packet_id {
            id
        } else if let Some(session) = self.session.as_mut() {
            session.next_packet_id()
        } else {
            // No session - send error result
            let result = PublishResult {
                packet_id: None,
                reason_code: Some(128), // Unspecified error
                properties: Some(vec![]),
                qos: command.qos,
            };
            let _ = response_tx.send(result);
            return;
        };

        // For QoS 0, complete immediately (fire-and-forget)
        if command.qos == 0 {
            if let Err(_e) = self.send_publish_command(command).await {
                let result = PublishResult {
                    packet_id: Some(packet_id),
                    reason_code: Some(128), // Unspecified error
                    properties: Some(vec![]),
                    qos: 0,
                };
                let _ = response_tx.send(result);
            } else {
                let result = PublishResult {
                    packet_id: Some(packet_id),
                    reason_code: Some(0), // Success
                    properties: Some(vec![]),
                    qos: 0,
                };
                let _ = response_tx.send(result);
            }
            return;
        }

        // For QoS 1 and QoS 2, store the response channel
        self.pending_publishes.insert(packet_id, response_tx);

        // Send the publish command
        let mut cmd = command;
        cmd.packet_id = Some(packet_id);
        let cmd_qos = cmd.qos; // Save QoS before moving

        if let Err(_e) = self.send_publish_command(cmd).await {
            // Remove from pending and send error
            if let Some(tx) = self.pending_publishes.remove(&packet_id) {
                let result = PublishResult {
                    packet_id: Some(packet_id),
                    reason_code: Some(128), // Unspecified error
                    properties: Some(vec![]),
                    qos: cmd_qos,
                };
                let _ = tx.send(result);
            }
        }
    }

    /// Handle unsubscribe command
    async fn handle_unsubscribe(&mut self, mut command: UnsubscribeCommand) {
        if command.topics.is_empty() {
            let mqtt_err = MqttClientError::InvalidState {
                expected: "at least one topic".to_string(),
                actual: "empty topic list".to_string(),
            };
            self.event_handler.on_error(&mqtt_err).await;
            return;
        }

        let packet_id = if let Some(id) = command.packet_id {
            id
        } else if let Some(session) = self.session.as_mut() {
            session.next_packet_id()
        } else {
            let mqtt_err = MqttClientError::NoActiveSession;
            self.event_handler.on_error(&mqtt_err).await;
            return;
        };

        command.packet_id = Some(packet_id);

        let unsubscribe_packet = unsubscribev5::MqttUnsubscribe::new(
            packet_id,
            command.topics.clone(),
            command.properties.clone(),
        );

        match unsubscribe_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = self.streams.send_egress(bytes).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send UNSUBSCRIBE packet");
                    self.event_handler.on_error(&mqtt_err).await;
                    return;
                }

                self.pending_unsubscribes
                    .insert(packet_id, command.topics.clone());
            }
            Err(err) => {
                let mqtt_err = MqttClientError::ProtocolViolation {
                    message: format!("Failed to serialize UNSUBSCRIBE packet: {:?}", err),
                };
                self.event_handler.on_error(&mqtt_err).await;
            }
        }
    }

    /// Handle ping command
    async fn handle_ping(&mut self) {
        if self.stream.is_none() {
            let mqtt_err = MqttClientError::NotConnected;
            self.event_handler.on_error(&mqtt_err).await;
            return;
        }

        let pingreq_packet = pingreqv5::MqttPingReq::new();

        match pingreq_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = self.streams.send_egress(bytes).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send PINGREQ packet");
                    self.event_handler.on_error(&mqtt_err).await;
                }
            }
            Err(err) => {
                let mqtt_err = MqttClientError::ProtocolViolation {
                    message: format!("Failed to serialize PINGREQ packet: {:?}", err),
                };
                self.event_handler.on_error(&mqtt_err).await;
            }
        }
    }

    /// Handle AUTH command for enhanced authentication (MQTT v5)
    async fn handle_auth(&mut self, reason_code: u8, properties: Vec<Property>) {
        if self.stream.is_none() {
            let mqtt_err = MqttClientError::NotConnected;
            self.event_handler.on_error(&mqtt_err).await;
            return;
        }

        let auth_packet = authv5::MqttAuth::new(reason_code, properties);

        match auth_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = self.streams.send_egress(bytes).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send AUTH packet");
                    self.event_handler.on_error(&mqtt_err).await;
                }
            }
            Err(err) => {
                let mqtt_err = MqttClientError::ProtocolViolation {
                    message: format!("Failed to serialize AUTH packet: {:?}", err),
                };
                self.event_handler.on_error(&mqtt_err).await;
            }
        }
    }

    /// Handle synchronous connect command
    async fn handle_connect_sync(
        &mut self,
        response_tx: tokio::sync::oneshot::Sender<ConnectionResult>,
    ) {
        // Check if already connected or connecting
        if self.stream.is_some() {
            let result = ConnectionResult {
                reason_code: 0,
                session_present: false,
                properties: Some(vec![]),
            };
            let _ = response_tx.send(result);
            return;
        }

        // Store the response channel for when CONNACK arrives
        self.pending_connect = Some(response_tx);

        // Initiate connection
        self.handle_connect().await;
    }

    /// Handle synchronous subscribe command
    async fn handle_subscribe_sync(
        &mut self,
        mut command: SubscribeCommand,
        response_tx: tokio::sync::oneshot::Sender<SubscribeResult>,
    ) {
        if self.stream.is_none() {
            let result = SubscribeResult {
                packet_id: 0,
                reason_codes: vec![128], // Unspecified error
                properties: vec![],
            };
            let _ = response_tx.send(result);
            return;
        }

        if command.subscriptions.is_empty() {
            let result = SubscribeResult {
                packet_id: 0,
                reason_codes: vec![128],
                properties: vec![],
            };
            let _ = response_tx.send(result);
            return;
        }

        // Generate packet ID
        let packet_id = if let Some(id) = command.packet_id {
            id
        } else if let Some(session) = self.session.as_mut() {
            session.next_packet_id()
        } else {
            let result = SubscribeResult {
                packet_id: 0,
                reason_codes: vec![128],
                properties: vec![],
            };
            let _ = response_tx.send(result);
            return;
        };

        command.packet_id = Some(packet_id);

        let subscribe_packet = subscribev5::MqttSubscribe::new(
            packet_id,
            command.subscriptions.clone(),
            command.properties.clone(),
        );

        match subscribe_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(_e) = self.streams.send_egress(bytes).await {
                    let result = SubscribeResult {
                        packet_id,
                        reason_codes: vec![128],
                        properties: vec![],
                    };
                    let _ = response_tx.send(result);
                    return;
                }

                // Store response channel and topics
                let topics: Vec<String> = command
                    .subscriptions
                    .iter()
                    .map(|sub| sub.topic_filter.clone())
                    .collect();
                self.pending_subscribes.insert(packet_id, topics);
                self.pending_subscribes_sync.insert(packet_id, response_tx);
            }
            Err(_err) => {
                let result = SubscribeResult {
                    packet_id,
                    reason_codes: vec![128],
                    properties: vec![],
                };
                let _ = response_tx.send(result);
            }
        }
    }

    /// Handle synchronous unsubscribe command
    async fn handle_unsubscribe_sync(
        &mut self,
        mut command: UnsubscribeCommand,
        response_tx: tokio::sync::oneshot::Sender<UnsubscribeResult>,
    ) {
        if self.stream.is_none() {
            let result = UnsubscribeResult {
                packet_id: 0,
                reason_codes: vec![128],
                properties: vec![],
            };
            let _ = response_tx.send(result);
            return;
        }

        if command.topics.is_empty() {
            let result = UnsubscribeResult {
                packet_id: 0,
                reason_codes: vec![128],
                properties: vec![],
            };
            let _ = response_tx.send(result);
            return;
        }

        // Generate packet ID
        let packet_id = if let Some(id) = command.packet_id {
            id
        } else if let Some(session) = self.session.as_mut() {
            session.next_packet_id()
        } else {
            let result = UnsubscribeResult {
                packet_id: 0,
                reason_codes: vec![128],
                properties: vec![],
            };
            let _ = response_tx.send(result);
            return;
        };

        command.packet_id = Some(packet_id);

        let unsubscribe_packet = unsubscribev5::MqttUnsubscribe::new(
            packet_id,
            command.topics.clone(),
            command.properties.clone(),
        );

        match unsubscribe_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(_e) = self.streams.send_egress(bytes).await {
                    let result = UnsubscribeResult {
                        packet_id,
                        reason_codes: vec![128],
                        properties: vec![],
                    };
                    let _ = response_tx.send(result);
                    return;
                }

                // Store response channel and topics
                self.pending_unsubscribes
                    .insert(packet_id, command.topics.clone());
                self.pending_unsubscribes_sync
                    .insert(packet_id, response_tx);
            }
            Err(_err) => {
                let result = UnsubscribeResult {
                    packet_id,
                    reason_codes: vec![128],
                    properties: vec![],
                };
                let _ = response_tx.send(result);
            }
        }
    }

    /// Handle synchronous ping command
    async fn handle_ping_sync(&mut self, response_tx: tokio::sync::oneshot::Sender<PingResult>) {
        if self.stream.is_none() {
            let result = PingResult { success: false };
            let _ = response_tx.send(result);
            return;
        }

        // Store response channel (only one ping at a time)
        self.pending_ping = Some(response_tx);

        // Send PINGREQ
        self.handle_ping().await;
    }

    /// Handle disconnect command
    async fn handle_disconnect(&mut self) {
        if self.stream.is_none() {
            self.is_connected = false;
            self.keep_alive_timer = None;
            self.message_buffer.clear();
            return;
        }

        let disconnect_packet = disconnectv5::MqttDisconnect::new_normal();

        match disconnect_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = self.streams.send_egress(bytes).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send DISCONNECT packet");
                    self.event_handler.on_error(&mqtt_err).await;
                    return;
                }
            }
            Err(err) => {
                let mqtt_err = MqttClientError::ProtocolViolation {
                    message: format!("Failed to serialize DISCONNECT packet: {:?}", err),
                };
                self.event_handler.on_error(&mqtt_err).await;
                return;
            }
        }

        self.is_connected = false;
        self.keep_alive_timer = None;
        self.message_buffer.clear();
    }

    async fn send_publish_command(&mut self, mut command: PublishCommand) -> io::Result<()> {
        if self.stream.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Transport not available for publish",
            ));
        }

        if command.qos > 0 {
            let session = self
                .session
                .as_mut()
                .ok_or_else(|| io::Error::other("No active session available for QoS publish"))?;

            if command.packet_id.is_none() {
                command.packet_id = Some(session.next_packet_id());
            }

            let publish_packet = command.to_mqtt_publish();
            session.handle_outgoing_publish(publish_packet.clone());

            let publish_bytes = publish_packet.to_bytes().map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to serialize PUBLISH packet: {:?}", err),
                )
            })?;

            return self.streams.send_egress(publish_bytes).await;
        }

        let publish_packet = command.to_mqtt_publish();
        let publish_bytes = publish_packet.to_bytes().map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to serialize PUBLISH packet: {:?}", err),
            )
        })?;

        self.streams.send_egress(publish_bytes).await
    }

    async fn send_packet_to_broker(&mut self, packet: &MqttPacket) -> io::Result<()> {
        let bytes = packet.to_bytes().map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to serialize MQTT packet: {:?}", err),
            )
        })?;

        self.streams.send_egress(bytes).await
    }

    async fn flush_message_buffer(&mut self) {
        if self.message_buffer.is_empty() {
            return;
        }

        while self.is_connected && self.stream.is_some() {
            if let Some(command) = self.message_buffer.pop_front() {
                match self.send_publish_command(command.clone()).await {
                    Ok(()) => continue,
                    Err(e) => {
                        let mqtt_err = MqttClientError::from_io_error(e, "flush publish buffer");
                        self.event_handler.on_error(&mqtt_err).await;
                        self.message_buffer.push_front(command);
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    /// Handle received MQTT packets
    async fn handle_mqtt_packet(&mut self, packet: MqttPacket) {
        match packet {
            MqttPacket::ConnAck5(connack) => {
                let success = connack.reason_code == 0;
                self.is_connected = success;

                if success {
                    self.reconnect_attempts = 0;

                    if !connack.session_present {
                        if let Some(session) = self.session.as_mut() {
                            session.clear();
                        }

                        let had_pending = !self.pending_subscribes.is_empty()
                            || !self.pending_unsubscribes.is_empty();
                        self.pending_subscribes.clear();
                        self.pending_unsubscribes.clear();

                        if had_pending {
                            self.event_handler.on_pending_operations_cleared().await;
                        }
                    } else if let Some(session) = self.session.as_ref() {
                        for resend_packet in session.resend_pending_messages() {
                            if let Err(e) = self.send_packet_to_broker(&resend_packet).await {
                                let mqtt_err =
                                    MqttClientError::from_io_error(e, "resend pending message");
                                self.event_handler.on_error(&mqtt_err).await;
                                self.handle_connection_lost().await;
                                return;
                            }
                        }
                    }

                    if self.config.buffer_messages {
                        self.flush_message_buffer().await;
                    }
                }

                let result = ConnectionResult {
                    reason_code: connack.reason_code,
                    session_present: connack.session_present,
                    properties: connack.properties.clone(),
                };

                // Complete synchronous connect if waiting
                if let Some(tx) = self.pending_connect.take() {
                    let _ = tx.send(result.clone());
                }

                self.event_handler.on_connected(&result).await;

                if !success {
                    self.event_handler
                        .on_disconnected(Some(connack.reason_code))
                        .await;
                    self.handle_connection_lost().await;
                }
            }
            MqttPacket::SubAck5(suback) => {
                let result = SubscribeResult {
                    packet_id: suback.packet_id,
                    reason_codes: suback.reason_codes.clone(),
                    properties: suback.properties.clone(),
                };

                self.pending_subscribes.remove(&suback.packet_id);

                // Complete synchronous subscribe if waiting
                if let Some(tx) = self.pending_subscribes_sync.remove(&suback.packet_id) {
                    let _ = tx.send(result.clone());
                }

                self.event_handler.on_subscribed(&result).await;
            }
            MqttPacket::UnsubAck5(unsuback) => {
                let result = UnsubscribeResult {
                    packet_id: unsuback.packet_id,
                    reason_codes: unsuback.reason_codes.clone(),
                    properties: unsuback.properties.clone(),
                };

                if let Some(topics) = self.pending_unsubscribes.remove(&unsuback.packet_id) {
                    if !topics.is_empty() {
                        let topic_set: HashSet<String> = topics.into_iter().collect();
                        self.pending_subscribes.retain(|_, pending_topics| {
                            pending_topics.retain(|topic| !topic_set.contains(topic));
                            !pending_topics.is_empty()
                        });
                    }
                }

                // Complete synchronous unsubscribe if waiting
                if let Some(tx) = self.pending_unsubscribes_sync.remove(&unsuback.packet_id) {
                    let _ = tx.send(result.clone());
                }

                self.event_handler.on_unsubscribed(&result).await;
            }
            MqttPacket::PubAck5(puback) => {
                if let Some(session) = self.session.as_mut() {
                    session.handle_incoming_puback(puback.clone());
                }

                let result = PublishResult {
                    packet_id: Some(puback.packet_id),
                    reason_code: Some(puback.reason_code),
                    properties: Some(puback.properties.clone()),
                    qos: 1,
                };

                // Complete synchronous publish if waiting
                if let Some(tx) = self.pending_publishes.remove(&puback.packet_id) {
                    let _ = tx.send(result.clone());
                }

                self.event_handler.on_published(&result).await;
            }
            MqttPacket::PubRec5(pubrec) => {
                let packet_id = pubrec.packet_id;
                let reason_code = pubrec.reason_code;
                let properties = pubrec.properties.clone();

                let mut pubrel_from_session = false;
                let next_pubrel = if let Some(session) = self.session.as_mut() {
                    let result = session.handle_incoming_pubrec(pubrec.clone());
                    if result.is_some() {
                        pubrel_from_session = true;
                    }
                    result
                } else if reason_code < 0x80 {
                    Some(MqttPubRel::new(packet_id, 0, Vec::new()))
                } else {
                    None
                };

                if reason_code < 0x80 {
                    if let Some(pubrel) = next_pubrel {
                        let packet = MqttPacket::PubRel5(pubrel.clone());
                        if let Err(e) = self.send_packet_to_broker(&packet).await {
                            let mqtt_err = MqttClientError::from_io_error(e, "send PUBREL packet");
                            self.event_handler.on_error(&mqtt_err).await;
                            self.handle_connection_lost().await;
                            return;
                        }

                        if pubrel_from_session {
                            if let Some(session) = self.session.as_mut() {
                                session.handle_outgoing_pubrel(pubrel);
                            }
                        }
                    }
                } else {
                    let result = PublishResult {
                        packet_id: Some(packet_id),
                        reason_code: Some(reason_code),
                        properties: Some(properties),
                        qos: 2,
                    };
                    self.event_handler.on_published(&result).await;
                }
            }
            MqttPacket::PubRel5(pubrel) => {
                if self.options.auto_ack {
                    let pubcomp = if let Some(session) = self.session.as_mut() {
                        session.handle_incoming_pubrel(pubrel)
                    } else {
                        MqttPubComp::new(pubrel.packet_id, 0, Vec::new())
                    };

                    let packet = MqttPacket::PubComp5(pubcomp.clone());
                    if let Err(e) = self.send_packet_to_broker(&packet).await {
                        let mqtt_err = MqttClientError::from_io_error(e, "send PUBCOMP packet");
                        self.event_handler.on_error(&mqtt_err).await;
                        self.handle_connection_lost().await;
                    }
                }
            }
            MqttPacket::PubComp5(pubcomp) => {
                if let Some(session) = self.session.as_mut() {
                    session.handle_incoming_pubcomp(pubcomp.clone());
                }

                let result = PublishResult {
                    packet_id: Some(pubcomp.packet_id),
                    reason_code: Some(pubcomp.reason_code),
                    properties: Some(pubcomp.properties.clone()),
                    qos: 2,
                };

                // Complete synchronous publish if waiting
                if let Some(tx) = self.pending_publishes.remove(&pubcomp.packet_id) {
                    let _ = tx.send(result.clone());
                }

                self.event_handler.on_published(&result).await;
            }
            MqttPacket::Publish5(publish) => {
                let qos = publish.qos;
                let packet_id = publish.packet_id;

                let ack_packet = if self.options.auto_ack {
                    if let Some(session) = self.session.as_mut() {
                        session.handle_incoming_publish(publish.clone())
                    } else {
                        match qos {
                            1 => packet_id.map(|pid| {
                                MqttPacket::PubAck5(MqttPubAck::new(pid, 0, Vec::new()))
                            }),
                            2 => packet_id.map(|pid| {
                                MqttPacket::PubRec5(MqttPubRec::new(pid, 0, Vec::new()))
                            }),
                            _ => None,
                        }
                    }
                } else {
                    None
                };

                self.event_handler.on_message_received(&publish).await;

                if let Some(ack) = ack_packet {
                    if let Err(e) = self.send_packet_to_broker(&ack).await {
                        let mqtt_err = MqttClientError::from_io_error(e, "send publish ACK");
                        self.event_handler.on_error(&mqtt_err).await;
                        self.handle_connection_lost().await;
                    }
                }
            }
            MqttPacket::PingResp5(_) => {
                let result = PingResult { success: true };

                // Complete synchronous ping if waiting
                if let Some(tx) = self.pending_ping.take() {
                    let _ = tx.send(result.clone());
                }

                self.event_handler.on_ping_response(&result).await;
            }
            MqttPacket::Disconnect5(disconnect) => {
                self.event_handler
                    .on_disconnected(Some(disconnect.reason_code))
                    .await;
                self.handle_connection_lost().await;
            }
            MqttPacket::PingReq5(_) => {
                // Respond to unexpected PINGREQ from broker with PINGRESP
                let response = MqttPacket::PingResp5(pingrespv5::MqttPingResp::new());
                if let Err(e) = self.send_packet_to_broker(&response).await {
                    let mqtt_err = MqttClientError::from_io_error(e, "send PINGRESP packet");
                    self.event_handler.on_error(&mqtt_err).await;
                    self.handle_connection_lost().await;
                }
            }
            MqttPacket::Auth(auth) => {
                // Handle incoming AUTH packet from broker (enhanced authentication)
                let result = AuthResult {
                    reason_code: auth.reason_code,
                    properties: auth.properties.clone(),
                };

                self.event_handler.on_auth_received(&result).await;

                // Note: Application is responsible for responding with appropriate AUTH packet
                // via client.auth() or client.auth_continue() based on authentication method
            }
            other => {
                // For unhandled packets, just log for now
                println!("TokioAsync: Unhandled MQTT packet: {:?}", other);
            }
        }
    }

    /// Handle connection lost
    async fn handle_connection_lost(&mut self) {
        let was_connected = self.is_connected;
        self.is_connected = false;
        self.stream = None;
        self.keep_alive_timer = None;

        // Clean up any pending synchronous publish operations
        for (_packet_id, tx) in self.pending_publishes.drain() {
            let result = PublishResult {
                packet_id: Some(_packet_id),
                reason_code: Some(128), // Unspecified error
                properties: Some(vec![]),
                qos: 0, // Unknown QoS
            };
            let _ = tx.send(result);
        }

        // Clean up pending synchronous connect
        if let Some(tx) = self.pending_connect.take() {
            let result = ConnectionResult {
                reason_code: 128, // Unspecified error
                session_present: false,
                properties: Some(vec![]),
            };
            let _ = tx.send(result);
        }

        // Clean up pending synchronous subscribes
        for (_packet_id, tx) in self.pending_subscribes_sync.drain() {
            let result = SubscribeResult {
                packet_id: _packet_id,
                reason_codes: vec![128],
                properties: vec![],
            };
            let _ = tx.send(result);
        }

        // Clean up pending synchronous unsubscribes
        for (_packet_id, tx) in self.pending_unsubscribes_sync.drain() {
            let result = UnsubscribeResult {
                packet_id: _packet_id,
                reason_codes: vec![128],
                properties: vec![],
            };
            let _ = tx.send(result);
        }

        // Clean up pending synchronous ping
        if let Some(tx) = self.pending_ping.take() {
            let result = PingResult { success: false };
            let _ = tx.send(result);
        }

        if was_connected {
            self.event_handler.on_connection_lost().await;

            if self.config.auto_reconnect {
                self.schedule_reconnect().await;
            }
        }
    }

    /// Schedule reconnection attempt
    async fn schedule_reconnect(&mut self) {
        // Early exit if auto-reconnect was disabled during sleep
        if !self.config.auto_reconnect {
            return;
        }

        if self.config.max_reconnect_attempts > 0
            && self.reconnect_attempts >= self.config.max_reconnect_attempts
        {
            return; // Max attempts reached
        }

        self.reconnect_attempts += 1;
        self.event_handler
            .on_reconnect_attempt(self.reconnect_attempts)
            .await;

        // Calculate delay with exponential backoff
        let delay = std::cmp::min(
            self.config.reconnect_delay_ms * (1 << (self.reconnect_attempts - 1)),
            self.config.max_reconnect_delay_ms,
        );

        // Sleep and then attempt reconnect
        tokio::time::sleep(Duration::from_millis(delay)).await;
        self.handle_connect().await;
    }

    /// Set MQTT version for packet parsing
    #[allow(dead_code)]
    fn set_mqtt_version(&mut self, version: u8) {
        self.mqtt_version = version;
    }
}

#[cfg(test)]
mod subscribe_builder_tests {
    use super::*;

    #[test]
    fn test_simple_subscription() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temp", 1)
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions.len(), 1);
        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temp");
        assert_eq!(cmd.subscriptions[0].qos, 1);
        assert!(!cmd.subscriptions[0].no_local);
        assert!(!cmd.subscriptions[0].retain_as_published);
        assert_eq!(cmd.subscriptions[0].retain_handling, 0);
        assert!(cmd.packet_id.is_none());
        assert!(cmd.properties.is_empty());
    }

    #[test]
    fn test_subscription_with_no_local() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/+/temp", 1)
            .with_no_local(true)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/+/temp");
        assert_eq!(sub.qos, 1);
        assert!(sub.no_local);
        assert!(!sub.retain_as_published);
        assert_eq!(sub.retain_handling, 0);
    }

    #[test]
    fn test_subscription_with_retain_as_published() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/#", 2)
            .with_retain_as_published(true)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/#");
        assert_eq!(sub.qos, 2);
        assert!(!sub.no_local);
        assert!(sub.retain_as_published);
        assert_eq!(sub.retain_handling, 0);
    }

    #[test]
    fn test_subscription_with_retain_handling() {
        // Test all valid retain_handling values
        for rh in 0..=2 {
            let cmd = SubscribeCommand::builder()
                .add_topic("test/topic", 1)
                .with_retain_handling(rh)
                .build()
                .unwrap();

            assert_eq!(cmd.subscriptions[0].retain_handling, rh);
        }
    }

    #[test]
    fn test_subscription_with_all_options() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/+/temp", 2)
            .with_no_local(true)
            .with_retain_as_published(true)
            .with_retain_handling(1)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/+/temp");
        assert_eq!(sub.qos, 2);
        assert!(sub.no_local);
        assert!(sub.retain_as_published);
        assert_eq!(sub.retain_handling, 1);
    }

    #[test]
    fn test_multiple_topics() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temp", 1)
            .with_no_local(true)
            .add_topic("sensors/humidity", 2)
            .with_retain_handling(1)
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions.len(), 2);

        // First subscription
        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temp");
        assert_eq!(cmd.subscriptions[0].qos, 1);
        assert!(cmd.subscriptions[0].no_local);
        assert!(!cmd.subscriptions[0].retain_as_published);
        assert_eq!(cmd.subscriptions[0].retain_handling, 0);

        // Second subscription
        assert_eq!(cmd.subscriptions[1].topic_filter, "sensors/humidity");
        assert_eq!(cmd.subscriptions[1].qos, 2);
        assert!(!cmd.subscriptions[1].no_local);
        assert!(!cmd.subscriptions[1].retain_as_published);
        assert_eq!(cmd.subscriptions[1].retain_handling, 1);
    }

    #[test]
    fn test_add_topic_with_options() {
        let cmd = SubscribeCommand::builder()
            .add_topic_with_options("sensors/temp", 2, true, true, 1)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/temp");
        assert_eq!(sub.qos, 2);
        assert!(sub.no_local);
        assert!(sub.retain_as_published);
        assert_eq!(sub.retain_handling, 1);
    }

    #[test]
    fn test_with_subscription_id() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/#", 1)
            .with_subscription_id(42)
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            cmd.properties[0],
            Property::SubscriptionIdentifier(42)
        ));
    }

    #[test]
    fn test_add_property() {
        let cmd = SubscribeCommand::builder()
            .add_topic("test/topic", 1)
            .add_property(Property::UserProperty("key".into(), "value".into()))
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            &cmd.properties[0],
            Property::UserProperty(k, v) if k == "key" && v == "value"
        ));
    }

    #[test]
    fn test_multiple_properties() {
        let cmd = SubscribeCommand::builder()
            .add_topic("test/topic", 1)
            .with_subscription_id(100)
            .add_property(Property::UserProperty("key1".into(), "value1".into()))
            .add_property(Property::UserProperty("key2".into(), "value2".into()))
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 3);
        assert!(matches!(
            cmd.properties[0],
            Property::SubscriptionIdentifier(100)
        ));
    }

    #[test]
    fn test_with_packet_id() {
        let cmd = SubscribeCommand::builder()
            .add_topic("test/topic", 1)
            .with_packet_id(123)
            .build()
            .unwrap();

        assert_eq!(cmd.packet_id, Some(123));
    }

    #[test]
    fn test_no_topics_error() {
        let result = SubscribeCommand::builder().build();

        assert!(result.is_err());
        assert!(matches!(result, Err(SubscribeBuilderError::NoTopics)));

        // Test error message
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No topics added"));
    }

    #[test]
    #[should_panic(expected = "no topics added yet")]
    fn test_with_no_local_before_topic_panics() {
        SubscribeCommand::builder().with_no_local(true);
    }

    #[test]
    #[should_panic(expected = "no topics added yet")]
    fn test_with_retain_as_published_before_topic_panics() {
        SubscribeCommand::builder().with_retain_as_published(true);
    }

    #[test]
    #[should_panic(expected = "no topics added yet")]
    fn test_with_retain_handling_before_topic_panics() {
        SubscribeCommand::builder().with_retain_handling(1);
    }

    #[test]
    #[should_panic(expected = "Invalid retain_handling value")]
    fn test_invalid_retain_handling_value() {
        SubscribeCommand::builder()
            .add_topic("test", 1)
            .with_retain_handling(3); // Invalid: max is 2
    }

    #[test]
    fn test_builder_default() {
        let builder1 = SubscribeCommandBuilder::default();
        let builder2 = SubscribeCommandBuilder::new();

        // Both should have the same initial state
        assert_eq!(builder1.topics.len(), builder2.topics.len());
        assert_eq!(builder1.properties.len(), builder2.properties.len());
        assert_eq!(builder1.packet_id, builder2.packet_id);
    }

    #[test]
    fn test_string_ownership() {
        let topic = String::from("sensors/temp");
        let cmd = SubscribeCommand::builder()
            .add_topic(topic.clone(), 1) // Clone to test Into<String>
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions[0].topic_filter, topic);
    }

    #[test]
    fn test_str_slice() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temp", 1) // &str
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temp");
    }

    #[test]
    fn test_complex_subscription() {
        // Test a realistic complex subscription scenario
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temperature/#", 1)
            .with_no_local(false)
            .with_retain_handling(0)
            .add_topic("sensors/humidity/+/data", 2)
            .with_no_local(true)
            .with_retain_as_published(true)
            .with_retain_handling(2)
            .add_topic("alerts/#", 1)
            .with_retain_handling(1)
            .with_subscription_id(999)
            .add_property(Property::UserProperty("client".into(), "test".into()))
            .with_packet_id(42)
            .build()
            .unwrap();

        // Verify structure
        assert_eq!(cmd.subscriptions.len(), 3);
        assert_eq!(cmd.packet_id, Some(42));
        assert_eq!(cmd.properties.len(), 2);

        // Verify first topic
        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temperature/#");
        assert_eq!(cmd.subscriptions[0].qos, 1);
        assert!(!cmd.subscriptions[0].no_local);
        assert_eq!(cmd.subscriptions[0].retain_handling, 0);

        // Verify second topic
        assert_eq!(cmd.subscriptions[1].topic_filter, "sensors/humidity/+/data");
        assert_eq!(cmd.subscriptions[1].qos, 2);
        assert!(cmd.subscriptions[1].no_local);
        assert!(cmd.subscriptions[1].retain_as_published);
        assert_eq!(cmd.subscriptions[1].retain_handling, 2);

        // Verify third topic
        assert_eq!(cmd.subscriptions[2].topic_filter, "alerts/#");
        assert_eq!(cmd.subscriptions[2].qos, 1);
        assert_eq!(cmd.subscriptions[2].retain_handling, 1);

        // Verify properties
        assert!(matches!(
            cmd.properties[0],
            Property::SubscriptionIdentifier(999)
        ));
        assert!(matches!(
            &cmd.properties[1],
            Property::UserProperty(k, v) if k == "client" && v == "test"
        ));
    }

    #[test]
    fn test_builder_is_clone() {
        let builder = SubscribeCommand::builder()
            .add_topic("test", 1)
            .with_subscription_id(42);

        let builder_clone = builder.clone();

        let cmd1 = builder.build().unwrap();
        let cmd2 = builder_clone.build().unwrap();

        assert_eq!(cmd1.subscriptions.len(), cmd2.subscriptions.len());
        assert_eq!(cmd1.properties.len(), cmd2.properties.len());
    }
}

#[cfg(test)]
mod config_builder_tests {
    use super::*;

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
        assert_eq!(config.reconnect_delay_ms, 1000);
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
}
