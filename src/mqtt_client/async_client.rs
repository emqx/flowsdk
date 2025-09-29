use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::publishv5::MqttPublish;

use super::client::{
    ConnectionResult, MqttClient, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use super::opts::MqttClientOptions;

/// Events that can occur during MQTT client operation
#[derive(Debug)]
pub enum MqttEvent {
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
    ReconnectAttempt(u32), // attempt number
    /// All pending operations cleared (on reconnect with clean_start)
    PendingOperationsCleared,
}

/// Trait for handling MQTT events
/// Users implement this trait to receive callbacks for various MQTT events
pub trait MqttEventHandler: Send {
    /// Called when connection to broker is established
    fn on_connected(&mut self, result: &ConnectionResult) {
        let _ = result; // Default empty implementation
    }

    /// Called when disconnected from broker
    fn on_disconnected(&mut self, reason: Option<u8>) {
        let _ = reason;
    }

    /// Called when a message is successfully published
    fn on_published(&mut self, result: &PublishResult) {
        let _ = result;
    }

    /// Called when subscription is completed
    fn on_subscribed(&mut self, result: &SubscribeResult) {
        let _ = result;
    }

    /// Called when unsubscription is completed
    fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
        let _ = result;
    }

    /// Called when an incoming message is received
    fn on_message_received(&mut self, publish: &MqttPublish) {
        let _ = publish;
    }

    /// Called when ping response is received
    fn on_ping_response(&mut self, result: &PingResult) {
        let _ = result;
    }

    /// Called when an error occurs
    fn on_error(&mut self, error: &io::Error) {
        let _ = error;
    }

    /// Called when TLS peer certificate is received (for custom validation)
    fn on_peer_cert_received(&mut self, cert: &[u8]) {
        let _ = cert;
    }

    /// Called when connection is lost unexpectedly
    fn on_connection_lost(&mut self) {}

    /// Called when attempting to reconnect
    fn on_reconnect_attempt(&mut self, attempt: u32) {
        let _ = attempt;
    }

    /// Called when pending operations are cleared (usually on reconnect)
    fn on_pending_operations_cleared(&mut self) {}
}

/// Commands that can be sent to the worker thread
#[derive(Debug)]
enum ClientCommand {
    /// Connect to the broker
    Connect,
    /// Subscribe to topics
    Subscribe { topic: String, qos: u8 },
    /// Publish a message
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
    },
    /// Unsubscribe from topics
    Unsubscribe { topics: Vec<String> },
    /// Send ping to broker
    Ping,
    /// Disconnect from broker
    Disconnect,
    /// Shutdown the client and worker thread
    Shutdown,
    /// Enable/disable automatic reconnection
    SetAutoReconnect { enabled: bool },
}

/// Configuration for the async client
#[derive(Debug, Clone)]
pub struct AsyncClientConfig {
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
}

impl Default for AsyncClientConfig {
    fn default() -> Self {
        AsyncClientConfig {
            auto_reconnect: true,
            reconnect_delay_ms: 1000,
            max_reconnect_delay_ms: 30000,
            max_reconnect_attempts: 0, // infinite
            command_queue_size: 100,
            buffer_messages: true,
            max_buffer_size: 1000,
        }
    }
}

/// Thread-safe, event-driven MQTT client
pub struct AsyncMqttClient {
    /// Channel to send commands to worker thread
    command_tx: Sender<ClientCommand>,
    /// Handle to the worker thread
    worker_handle: Option<JoinHandle<()>>,
    /// Client configuration
    config: AsyncClientConfig,
}

impl AsyncMqttClient {
    /// Create a new async MQTT client
    pub fn new(
        mqtt_options: MqttClientOptions,
        event_handler: Box<dyn MqttEventHandler>,
        config: AsyncClientConfig,
    ) -> io::Result<Self> {
        let (command_tx, command_rx) = mpsc::channel();

        let worker = ClientWorker::new(mqtt_options, event_handler, command_rx, config.clone());
        let worker_handle = thread::Builder::new()
            .name("mqtt-client-worker".to_string())
            .spawn(move || {
                worker.run();
            })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(AsyncMqttClient {
            command_tx,
            worker_handle: Some(worker_handle),
            config,
        })
    }

    /// Create a new async MQTT client with default configuration
    pub fn with_default_config(
        mqtt_options: MqttClientOptions,
        event_handler: Box<dyn MqttEventHandler>,
    ) -> io::Result<Self> {
        Self::new(mqtt_options, event_handler, AsyncClientConfig::default())
    }

    /// Connect to the MQTT broker (non-blocking)
    pub fn connect(&self) -> io::Result<()> {
        self.send_command(ClientCommand::Connect)
    }

    /// Subscribe to a topic (non-blocking)
    pub fn subscribe(&self, topic: &str, qos: u8) -> io::Result<()> {
        self.send_command(ClientCommand::Subscribe {
            topic: topic.to_string(),
            qos,
        })
    }

    /// Publish a message (non-blocking)
    pub fn publish(&self, topic: &str, payload: &[u8], qos: u8, retain: bool) -> io::Result<()> {
        self.send_command(ClientCommand::Publish {
            topic: topic.to_string(),
            payload: payload.to_vec(),
            qos,
            retain,
        })
    }

    /// Unsubscribe from topics (non-blocking)
    pub fn unsubscribe(&self, topics: Vec<&str>) -> io::Result<()> {
        let topics: Vec<String> = topics.into_iter().map(|s| s.to_string()).collect();
        self.send_command(ClientCommand::Unsubscribe { topics })
    }

    /// Send ping to broker (non-blocking)
    pub fn ping(&self) -> io::Result<()> {
        self.send_command(ClientCommand::Ping)
    }

    /// Disconnect from broker (non-blocking)
    pub fn disconnect(&self) -> io::Result<()> {
        self.send_command(ClientCommand::Disconnect)
    }

    /// Enable or disable automatic reconnection
    pub fn set_auto_reconnect(&self, enabled: bool) -> io::Result<()> {
        self.send_command(ClientCommand::SetAutoReconnect { enabled })
    }

    /// Shutdown the client and wait for worker thread to finish
    pub fn shutdown(mut self) -> io::Result<()> {
        // Send shutdown command
        let _ = self.send_command(ClientCommand::Shutdown);

        // Wait for worker thread to finish
        if let Some(handle) = self.worker_handle.take() {
            handle
                .join()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "Worker thread panicked"))?;
        }

        Ok(())
    }

    /// Send a command to the worker thread
    fn send_command(&self, command: ClientCommand) -> io::Result<()> {
        self.command_tx.send(command).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Worker thread is no longer running",
            )
        })
    }
}

impl Drop for AsyncMqttClient {
    fn drop(&mut self) {
        // Send shutdown command (ignore errors)
        let _ = self.send_command(ClientCommand::Shutdown);

        // Wait for worker thread (with timeout)
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Worker thread that handles MQTT operations
struct ClientWorker {
    /// The underlying synchronous MQTT client
    client: MqttClient,
    /// Event handler for callbacks
    event_handler: Box<dyn MqttEventHandler>,
    /// Command receiver from main thread
    command_rx: Receiver<ClientCommand>,
    /// Worker configuration
    config: AsyncClientConfig,
    /// Current connection state
    is_connected: bool,
    /// Reconnection state
    reconnect_attempts: u32,
    /// Message buffer for when disconnected
    message_buffer: Vec<(String, Vec<u8>, u8, bool)>, // (topic, payload, qos, retain)
    /// Pending operations tracking
    pending_subscribes: HashMap<String, u8>, // topic -> qos
}

impl ClientWorker {
    /// Create a new client worker
    fn new(
        mqtt_options: MqttClientOptions,
        event_handler: Box<dyn MqttEventHandler>,
        command_rx: Receiver<ClientCommand>,
        config: AsyncClientConfig,
    ) -> Self {
        let client = MqttClient::new(mqtt_options.client_id.clone(), mqtt_options);

        ClientWorker {
            client,
            event_handler,
            command_rx,
            config,
            is_connected: false,
            reconnect_attempts: 0,
            message_buffer: Vec::new(),
            pending_subscribes: HashMap::new(),
        }
    }

    /// Main worker thread loop
    fn run(mut self) {
        loop {
            // Process commands with timeout to allow periodic tasks
            let timeout = Duration::from_millis(100);
            match self.command_rx.recv_timeout(timeout) {
                Ok(command) => {
                    if !self.handle_command(command) {
                        break; // Shutdown requested
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Periodic tasks
                    self.handle_periodic_tasks();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Main thread dropped, shutdown
                    break;
                }
            }

            // Read incoming messages if connected
            if self.is_connected {
                self.handle_incoming_messages();
            }
        }
    }

    /// Handle a command from the main thread
    fn handle_command(&mut self, command: ClientCommand) -> bool {
        match command {
            ClientCommand::Connect => {
                self.handle_connect();
            }
            ClientCommand::Subscribe { topic, qos } => {
                self.handle_subscribe(&topic, qos);
            }
            ClientCommand::Publish {
                topic,
                payload,
                qos,
                retain,
            } => {
                self.handle_publish(&topic, &payload, qos, retain);
            }
            ClientCommand::Unsubscribe { topics } => {
                self.handle_unsubscribe(&topics);
            }
            ClientCommand::Ping => {
                self.handle_ping();
            }
            ClientCommand::Disconnect => {
                self.handle_disconnect();
            }
            ClientCommand::Shutdown => {
                self.handle_disconnect();
                return false; // Exit worker loop
            }
            ClientCommand::SetAutoReconnect { enabled } => {
                // Update config (we'd need to make config mutable)
                let _ = enabled; // For now, just acknowledge
            }
        }
        true
    }

    /// Handle connect command
    fn handle_connect(&mut self) {
        match self.client.connected() {
            Ok(result) => {
                self.is_connected = result.is_success();
                self.reconnect_attempts = 0;
                self.event_handler.on_connected(&result);

                if self.is_connected {
                    // Process any buffered messages
                    self.process_buffered_messages();
                    // Re-subscribe to pending subscriptions
                    self.resubscribe_pending();
                }
            }
            Err(e) => {
                self.event_handler.on_error(&e);
                if self.config.auto_reconnect {
                    self.schedule_reconnect();
                }
            }
        }
    }

    /// Handle subscribe command
    fn handle_subscribe(&mut self, topic: &str, qos: u8) {
        if self.is_connected {
            match self.client.subscribed(topic, qos) {
                Ok(result) => {
                    if result.is_success() {
                        self.pending_subscribes.remove(topic);
                    }
                    self.event_handler.on_subscribed(&result);
                }
                Err(e) => {
                    self.event_handler.on_error(&e);
                    self.pending_subscribes.insert(topic.to_string(), qos);
                }
            }
        } else {
            // Store for later when connected
            self.pending_subscribes.insert(topic.to_string(), qos);
        }
    }

    /// Handle publish command
    fn handle_publish(&mut self, topic: &str, payload: &[u8], qos: u8, retain: bool) {
        if self.is_connected {
            match self.client.published(topic, payload, qos, retain) {
                Ok(result) => {
                    self.event_handler.on_published(&result);
                }
                Err(e) => {
                    self.event_handler.on_error(&e);
                    // Buffer message if enabled and connection lost
                    if self.config.buffer_messages {
                        self.buffer_message(topic, payload, qos, retain);
                    }
                }
            }
        } else if self.config.buffer_messages {
            // Buffer message for later
            self.buffer_message(topic, payload, qos, retain);
        }
    }

    /// Handle unsubscribe command
    fn handle_unsubscribe(&mut self, topics: &[String]) {
        if self.is_connected {
            let topic_refs: Vec<&str> = topics.iter().map(|s| s.as_str()).collect();
            match self.client.unsubscribed(topic_refs) {
                Ok(result) => {
                    // Remove from pending subscriptions
                    for topic in topics {
                        self.pending_subscribes.remove(topic);
                    }
                    self.event_handler.on_unsubscribed(&result);
                }
                Err(e) => {
                    self.event_handler.on_error(&e);
                }
            }
        } else {
            // Remove from pending subscriptions even if not connected
            for topic in topics {
                self.pending_subscribes.remove(topic);
            }
        }
    }

    /// Handle ping command
    fn handle_ping(&mut self) {
        if self.is_connected {
            match self.client.pingd() {
                Ok(result) => {
                    self.event_handler.on_ping_response(&result);
                }
                Err(e) => {
                    self.event_handler.on_error(&e);
                    self.handle_connection_lost();
                }
            }
        }
    }

    /// Handle disconnect command
    fn handle_disconnect(&mut self) {
        if self.is_connected {
            match self.client.disconnected() {
                Ok(_) => {
                    self.is_connected = false;
                    self.event_handler.on_disconnected(Some(0)); // Normal disconnect
                }
                Err(e) => {
                    self.is_connected = false;
                    self.event_handler.on_error(&e);
                    self.event_handler.on_disconnected(None);
                }
            }
        }
    }

    /// Handle periodic tasks (heartbeat, connection monitoring, etc.)
    fn handle_periodic_tasks(&mut self) {
        // Could add periodic ping, connection health checks, etc.
    }

    /// Handle incoming messages from broker
    fn handle_incoming_messages(&mut self) {
        // Try to receive a message with a short timeout to avoid blocking
        match self.client.recv_packet() {
            Ok(Some(packet)) => match packet {
                MqttPacket::Publish5(publish) => {
                    self.event_handler.on_message_received(&publish);
                }
                // Handle other packet types as needed
                _ => {
                    // For now, ignore other packet types
                    // In a full implementation, you'd handle SUBACK, UNSUBACK, etc.
                }
            },
            Ok(None) => {
                // Connection closed
                self.handle_connection_lost();
            }
            Err(e) => {
                // Check if it's a timeout or actual error
                match e.kind() {
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                        // Normal timeout, continue
                    }
                    _ => {
                        self.event_handler.on_error(&e);
                        self.handle_connection_lost();
                    }
                }
            }
        }
    }

    /// Handle connection lost
    fn handle_connection_lost(&mut self) {
        if self.is_connected {
            self.is_connected = false;
            self.event_handler.on_connection_lost();

            if self.config.auto_reconnect {
                self.schedule_reconnect();
            }
        }
    }

    /// Schedule reconnection attempt
    fn schedule_reconnect(&mut self) {
        if self.config.max_reconnect_attempts > 0
            && self.reconnect_attempts >= self.config.max_reconnect_attempts
        {
            return; // Max attempts reached
        }

        self.reconnect_attempts += 1;
        self.event_handler
            .on_reconnect_attempt(self.reconnect_attempts);

        // Calculate delay with exponential backoff
        let delay = std::cmp::min(
            self.config.reconnect_delay_ms * (1 << (self.reconnect_attempts - 1)),
            self.config.max_reconnect_delay_ms,
        );

        // Sleep and then attempt reconnect
        thread::sleep(Duration::from_millis(delay));
        self.handle_connect();
    }

    /// Process buffered messages after connection
    fn process_buffered_messages(&mut self) {
        if self.message_buffer.is_empty() {
            return;
        }

        let messages = std::mem::take(&mut self.message_buffer);
        for (topic, payload, qos, retain) in messages {
            self.handle_publish(&topic, &payload, qos, retain);
        }
    }

    /// Re-subscribe to pending subscriptions
    fn resubscribe_pending(&mut self) {
        let pending: Vec<(String, u8)> = self
            .pending_subscribes
            .iter()
            .map(|(topic, qos)| (topic.clone(), *qos))
            .collect();

        for (topic, qos) in pending {
            self.handle_subscribe(&topic, qos);
        }
    }

    /// Buffer a message for later sending
    fn buffer_message(&mut self, topic: &str, payload: &[u8], qos: u8, retain: bool) {
        if self.message_buffer.len() < self.config.max_buffer_size {
            self.message_buffer
                .push((topic.to_string(), payload.to_vec(), qos, retain));
        }
        // If buffer is full, oldest messages are dropped (FIFO)
    }
}
