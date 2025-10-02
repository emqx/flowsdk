use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, ErrorKind};
use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, error::TryRecvError, error::TrySendError, Receiver};
use tokio::time::Sleep;
use tokio_stream::wrappers::ReceiverStream;

use crate::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::{
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
    ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
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
    /// Error occurred during operation
    Error(io::Error),
    /// TLS peer certificate received (for certificate validation)
    PeerCertReceived(Vec<u8>),
    /// Connection lost (will attempt to reconnect if enabled)
    ConnectionLost,
    /// Reconnection attempt started
    ReconnectAttempt(u32),
    /// All pending operations cleared (on reconnect with clean_start)
    PendingOperationsCleared,
    /// Parse error occurred
    ParseError(String),
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

    /// Called when an error occurs
    async fn on_error(&mut self, error: &io::Error) {
        let _ = error;
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
#[derive(Debug)]
enum TokioClientCommand {
    /// Connect to the broker
    Connect,
    /// Subscribe to topics
    Subscribe(SubscribeCommand),
    /// Publish a message
    Publish(PublishCommand),
    /// Unsubscribe from topics
    Unsubscribe(UnsubscribeCommand),
    /// Send ping to broker
    Ping,
    /// Disconnect from broker
    Disconnect,
    /// Shutdown the client and worker
    Shutdown,
    /// Enable/disable automatic reconnection
    SetAutoReconnect { enabled: bool },
    /// Send a raw MQTT packet directly
    SendPacket(MqttPacket),
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
        }
    }
}

/// Async stream for outbound MQTT frame bytes.
struct AsyncEgressStream {
    #[allow(dead_code)]
    sender: mpsc::Sender<Vec<u8>>,
    receiver: Option<Receiver<Vec<u8>>>,
    #[allow(dead_code)]
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

    fn take_receiver(&mut self) -> Receiver<Vec<u8>> {
        self.receiver.take().expect("egress receiver already taken")
    }

    #[allow(dead_code)]
    fn into_stream(&mut self) -> ReceiverStream<Vec<u8>> {
        ReceiverStream::new(self.take_receiver())
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    fn into_stream(&mut self) -> ReceiverStream<MqttPacket> {
        ReceiverStream::new(self.take_receiver())
    }

    /// Push raw transport bytes, parse MQTT packets, and forward to consumers.
    fn push_raw_data(&mut self, data: &[u8], mqtt_version: u8) -> io::Result<()> {
        self.raw_buffer.extend_from_slice(data);
        self.flush_pending()?;

        loop {
            match self.try_parse_next_packet(mqtt_version)? {
                Some((packet, consumed)) => match self.sender.try_send(packet) {
                    Ok(()) => {
                        self.raw_buffer.advance(consumed);
                    }
                    Err(TrySendError::Full(packet)) => {
                        if self.pending_packets.len() >= self.max_pending {
                            return Err(io::Error::new(
                                io::ErrorKind::WouldBlock,
                                "Ingress buffer full",
                            ));
                        }
                        self.pending_packets.push_back(packet);
                        self.raw_buffer.advance(consumed);
                        break;
                    }
                    Err(TrySendError::Closed(_)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "Ingress stream closed",
                        ));
                    }
                },
                None => break,
            }
        }

        Ok(())
    }

    /// Allow consumers to wake pending packets after they drain channel capacity.
    fn flush_pending(&mut self) -> io::Result<()> {
        while let Some(packet) = self.pending_packets.pop_front() {
            match self.sender.try_send(packet) {
                Ok(()) => continue,
                Err(TrySendError::Full(packet)) => {
                    self.pending_packets.push_front(packet);
                    break;
                }
                Err(TrySendError::Closed(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Ingress stream closed",
                    ));
                }
            }
        }
        Ok(())
    }

    fn try_parse_next_packet(&self, mqtt_version: u8) -> io::Result<Option<(MqttPacket, usize)>> {
        if self.raw_buffer.is_empty() {
            return Ok(None);
        }

        match MqttPacket::from_bytes_with_version(&self.raw_buffer[..], mqtt_version) {
            Ok(ParseOk::Packet(packet, consumed)) => Ok(Some((packet, consumed))),
            Ok(ParseOk::Continue(_, _)) => Ok(None),
            Ok(ParseOk::TopicName(_, _)) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unexpected ParseOk variant: TopicName",
            )),
            Err(ParseError::More(_, _))
            | Err(ParseError::BufferTooShort)
            | Err(ParseError::BufferEmpty) => Ok(None),
            Err(e) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("MQTT packet parsing failed: {:?}", e),
            )),
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

    fn push_ingress_data(&mut self, data: &[u8], mqtt_version: u8) -> io::Result<()> {
        self.ingress.push_raw_data(data, mqtt_version)
    }

    fn flush_ingress_pending(&mut self) -> io::Result<()> {
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
                        if let Err(e) = self.handle_egress_frame(frame, &mut egress_rx).await {
                            self.event_handler.on_error(&e).await;
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
                            if let Err(e) = self.streams.push_ingress_data(&bytes, self.mqtt_version) {
                                self.event_handler.on_error(&e).await;
                                self.handle_connection_lost().await;
                                continue;
                            }
                        }
                        Ok(None) => {
                            let error = io::Error::new(io::ErrorKind::UnexpectedEof, "Connection closed");
                            self.event_handler.on_error(&error).await;
                            self.handle_connection_lost().await;
                            continue;
                        }
                        Err(e) => {
                            self.event_handler.on_error(&e).await;
                            self.handle_connection_lost().await;
                            continue;
                        }
                    }
                }

                // Consume parsed MQTT packets from ingress stream
                packet = ingress_rx.recv() => {
                    if let Some(packet) = packet {
                        self.handle_mqtt_packet(packet).await;
                        if let Err(e) = self.streams.flush_ingress_pending() {
                            self.event_handler.on_error(&e).await;
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
            TokioClientCommand::Subscribe(command) => {
                self.handle_subscribe(command).await;
            }
            TokioClientCommand::Publish(command) => {
                self.handle_publish(command).await;
            }
            TokioClientCommand::Unsubscribe(command) => {
                self.handle_unsubscribe(command).await;
            }
            TokioClientCommand::Ping => {
                self.handle_ping().await;
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
                    self.event_handler.on_error(&e).await;
                }
            }
        }
        true
    }

    /// Drain the egress channel and push MQTT frames onto the transport.
    async fn handle_egress_frame(
        &mut self,
        first: Vec<u8>,
        egress_rx: &mut Receiver<Vec<u8>>,
    ) -> io::Result<()> {
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

    async fn write_to_transport(&mut self, frame: &[u8]) -> io::Result<()> {
        if let Some(stream) = &mut self.stream {
            let result = stream.write_all(frame).await;
            if result.is_ok() {
                // Update last send time for keep-alive tracking
                self.last_packet_sent = Instant::now();
            }
            result
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Transport not connected",
            ))
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
                        self.event_handler.on_error(&e).await;
                        self.handle_connection_lost().await;
                        return; // Don't reset timer if connection lost
                    }
                    // last_packet_sent will be updated in write_to_transport
                }
                Err(err) => {
                    let error = io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Failed to serialize PINGREQ: {:?}", err),
                    );
                    self.event_handler.on_error(&error).await;
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
                        self.event_handler.on_error(&e).await;
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
                let connect_packet = connectv5::MqttConnect::new(
                    self.options.client_id.clone(),
                    self.options.username.clone(),
                    self.options.password.clone(),
                    self.options.will.clone(),
                    self.options.keep_alive,
                    self.options.clean_start,
                    vec![],
                );

                let connect_bytes = match connect_packet.to_bytes() {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        let error = io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Failed to serialize CONNECT packet: {:?}", err),
                        );
                        self.event_handler.on_error(&error).await;
                        return;
                    }
                };

                if let Err(e) = self.streams.send_egress(connect_bytes).await {
                    self.event_handler.on_error(&e).await;
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
                self.event_handler.on_error(&e).await;
            }
        }
    }

    /// Handle subscribe command
    async fn handle_subscribe(&mut self, mut command: SubscribeCommand) {
        if command.subscriptions.is_empty() {
            let error = io::Error::new(
                ErrorKind::InvalidInput,
                "SUBSCRIBE requires at least one topic subscription",
            );
            self.event_handler.on_error(&error).await;
            return;
        }

        let packet_id = if let Some(id) = command.packet_id {
            id
        } else if let Some(session) = self.session.as_mut() {
            session.next_packet_id()
        } else {
            let error = io::Error::new(
                io::ErrorKind::Other,
                "No active session available for SUBSCRIBE",
            );
            self.event_handler.on_error(&error).await;
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
                    self.event_handler.on_error(&e).await;
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
                let error = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to serialize SUBSCRIBE packet: {:?}", err),
                );
                self.event_handler.on_error(&error).await;
            }
        }
    }

    /// Handle publish command
    async fn handle_publish(&mut self, command: PublishCommand) {
        if self.stream.is_none() && self.config.buffer_messages {
            if self.message_buffer.len() >= self.config.max_buffer_size {
                let error = io::Error::new(
                    ErrorKind::WouldBlock,
                    "Publish buffer full; dropping message",
                );
                self.event_handler.on_error(&error).await;
                return;
            }

            self.message_buffer.push_back(command);
            return;
        }

        if self.stream.is_none() {
            let error = io::Error::new(
                io::ErrorKind::NotConnected,
                "Cannot publish while disconnected",
            );
            self.event_handler.on_error(&error).await;
            return;
        }

        if let Err(e) = self.send_publish_command(command).await {
            self.event_handler.on_error(&e).await;
        }
    }

    /// Handle unsubscribe command
    async fn handle_unsubscribe(&mut self, mut command: UnsubscribeCommand) {
        if command.topics.is_empty() {
            let error = io::Error::new(
                ErrorKind::InvalidInput,
                "UNSUBSCRIBE requires at least one topic",
            );
            self.event_handler.on_error(&error).await;
            return;
        }

        let packet_id = if let Some(id) = command.packet_id {
            id
        } else if let Some(session) = self.session.as_mut() {
            session.next_packet_id()
        } else {
            let error = io::Error::new(
                io::ErrorKind::Other,
                "No active session available for UNSUBSCRIBE",
            );
            self.event_handler.on_error(&error).await;
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
                    self.event_handler.on_error(&e).await;
                    return;
                }

                self.pending_unsubscribes
                    .insert(packet_id, command.topics.clone());
            }
            Err(err) => {
                let error = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to serialize UNSUBSCRIBE packet: {:?}", err),
                );
                self.event_handler.on_error(&error).await;
            }
        }
    }

    /// Handle ping command
    async fn handle_ping(&mut self) {
        if self.stream.is_none() {
            let error = io::Error::new(
                io::ErrorKind::NotConnected,
                "Cannot send PING while disconnected",
            );
            self.event_handler.on_error(&error).await;
            return;
        }

        let pingreq_packet = pingreqv5::MqttPingReq::new();

        match pingreq_packet.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = self.streams.send_egress(bytes).await {
                    self.event_handler.on_error(&e).await;
                }
            }
            Err(err) => {
                let error = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to serialize PINGREQ packet: {:?}", err),
                );
                self.event_handler.on_error(&error).await;
            }
        }
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
                    self.event_handler.on_error(&e).await;
                    return;
                }
            }
            Err(err) => {
                let error = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to serialize DISCONNECT packet: {:?}", err),
                );
                self.event_handler.on_error(&error).await;
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
            let session = self.session.as_mut().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Other,
                    "No active session available for QoS publish",
                )
            })?;

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
                        self.event_handler.on_error(&e).await;
                        self.message_buffer.push_front(command);

                        if matches!(
                            e.kind(),
                            ErrorKind::WouldBlock | ErrorKind::NotConnected | ErrorKind::BrokenPipe
                        ) {
                            break;
                        }
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
                                self.event_handler.on_error(&e).await;
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
                            self.event_handler.on_error(&e).await;
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
                        self.event_handler.on_error(&e).await;
                        self.handle_connection_lost().await;
                        return;
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
                        self.event_handler.on_error(&e).await;
                        self.handle_connection_lost().await;
                        return;
                    }
                }
            }
            MqttPacket::PingResp5(_) => {
                let result = PingResult { success: true };
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
                    self.event_handler.on_error(&e).await;
                    self.handle_connection_lost().await;
                }
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

        if was_connected {
            self.event_handler.on_connection_lost().await;

            if self.config.auto_reconnect {
                self.schedule_reconnect().await;
            }
        }
    }

    /// Schedule reconnection attempt
    // @TODO: Implement exponential backoff and max attempts with timer
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
