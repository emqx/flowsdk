// SPDX-License-Identifier: MPL-2.0

//! Pure Sans-I/O MQTT Client
//!
//! This module provides a protocol-only MQTT client that is completely independent
//! of any I/O runtime or networking implementation. It's designed for:
//!
//! - **Embedded systems** with custom I/O
//! - **FFI bindings** (C, Python, Java, etc.)
//! - **Custom runtimes** (WASM, bare metal, etc.)
//! - **Testing** without network dependencies
//! - **Protocol analysis** and debugging
//!
//! # Architecture
//!
//! The `NoIoMqttClient` is a thin wrapper around [`MqttEngine`] that provides
//! a clean Sans-I/O interface. You are responsible for:
//!
//! 1. **Network I/O**: Reading/writing bytes to/from sockets
//! 2. **Timers**: Calling `handle_tick()` at appropriate intervals
//! 3. **Event Loop**: Coordinating between network, timers, and protocol
//!
//! # Example: Basic Usage
//!
//! ```no_run
//! use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, PublishCommand};
//! use std::time::Instant;
//!
//! // Create client
//! let options = MqttClientOptions::builder()
//!     .peer("broker.example.com:1883")
//!     .client_id("my_client")
//!     .keep_alive(60)
//!     .build();
//!
//! let mut client = NoIoMqttClient::new(options);
//!
//! // Initiate connection
//! client.connect();
//!
//! // Get bytes to send to network
//! let outgoing = client.take_outgoing();
//! // ... write `outgoing` to your socket ...
//!
//! // Feed incoming bytes from network
//! let incoming_bytes = vec![/* bytes from socket */];
//! let events = client.handle_incoming(&incoming_bytes);
//!
//! // Process events
//! for event in events {
//!     match event {
//!         flowsdk::mqtt_client::MqttEvent::Connected(result) => {
//!             println!("Connected! Session present: {}", result.session_present);
//!         }
//!         flowsdk::mqtt_client::MqttEvent::MessageReceived(msg) => {
//!             println!("Received: {:?}", msg);
//!         }
//!         _ => {}
//!     }
//! }
//!
//! // Publish a message
//! let cmd = PublishCommand::simple("sensors/temp", b"23.5".to_vec(), 1, false);
//! client.publish(cmd).unwrap();
//!
//! // Get bytes to send
//! let outgoing = client.take_outgoing();
//! // ... write to socket ...
//! ```
//!
//! # Example: Event Loop Integration
//!
//! ```no_run
//! use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
//! use std::time::Instant;
//!
//! fn run_event_loop(mut client: NoIoMqttClient) {
//!     loop {
//!         // 1. Check if we need to handle protocol timers
//!         if let Some(next_tick) = client.next_tick_at() {
//!             let now = Instant::now();
//!             if now >= next_tick {
//!                 let events = client.handle_tick(now);
//!                 // Process events...
//!             }
//!         }
//!
//!         // 2. Read from socket (pseudo-code)
//!         // let bytes = socket.read()?;
//!         // let events = client.handle_incoming(&bytes);
//!         // Process events...
//!
//!         // 3. Write to socket
//!         let outgoing = client.take_outgoing();
//!         if !outgoing.is_empty() {
//!             // socket.write_all(&outgoing)?;
//!         }
//!
//!         // 4. Sleep until next event
//!         // (network data, timer, or user command)
//!     }
//! }
//! ```

use crate::mqtt_client::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use crate::mqtt_client::engine::{MqttEngine, MqttEvent};
use crate::mqtt_client::error::MqttClientError;
use crate::mqtt_client::opts::MqttClientOptions;
use crate::mqtt_serde::mqttv5::common::properties::Property;
use std::time::Instant;

/// A pure Sans-I/O MQTT client.
///
/// This client handles all MQTT protocol logic without performing any I/O operations.
/// You are responsible for:
///
/// - Reading bytes from the network and feeding them via [`handle_incoming()`](Self::handle_incoming)
/// - Writing bytes from [`take_outgoing()`](Self::take_outgoing) to the network
/// - Calling [`handle_tick()`](Self::handle_tick) at appropriate intervals for keep-alive and retransmissions
///
/// # Thread Safety
///
/// This client is `!Send` and `!Sync` by design. If you need to use it across threads,
/// wrap it in appropriate synchronization primitives for your use case.
///
/// # Example
///
/// ```no_run
/// use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
///
/// let options = MqttClientOptions::builder()
///     .peer("broker.example.com:1883")
///     .client_id("my_client")
///     .build();
///
/// let mut client = NoIoMqttClient::new(options);
/// client.connect();
///
/// // Get CONNECT packet bytes
/// let bytes = client.take_outgoing();
/// // ... send `bytes` to your socket ...
/// ```
pub struct NoIoMqttClient {
    engine: MqttEngine,
}

impl NoIoMqttClient {
    /// Create a new Sans-I/O MQTT client with the given options.
    ///
    /// # Arguments
    ///
    /// * `options` - MQTT client configuration
    ///
    /// # Example
    ///
    /// ```no_run
    /// use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    ///
    /// let options = MqttClientOptions::builder()
    ///     .peer("broker.example.com:1883")
    ///     .client_id("my_client")
    ///     .keep_alive(60)
    ///     .clean_start(true)
    ///     .build();
    ///
    /// let client = NoIoMqttClient::new(options);
    /// ```
    pub fn new(options: MqttClientOptions) -> Self {
        Self {
            engine: MqttEngine::new(options),
        }
    }

    // ========================================================================
    // I/O Interface
    // ========================================================================

    /// Process incoming bytes from the network.
    ///
    /// Feed raw bytes received from your socket into this method. The client will
    /// parse MQTT packets and return a list of events.
    ///
    /// # Arguments
    ///
    /// * `data` - Raw bytes from the network
    ///
    /// # Returns
    ///
    /// A vector of [`MqttEvent`]s generated from the incoming data.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, MqttEvent};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// // Bytes received from socket
    /// let incoming_bytes = vec![0x20, 0x02, 0x00, 0x00]; // CONNACK
    ///
    /// let events = client.handle_incoming(&incoming_bytes);
    /// for event in events {
    ///     match event {
    ///         MqttEvent::Connected(result) => {
    ///             println!("Connected! Reason code: {}", result.reason_code);
    ///         }
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub fn handle_incoming(&mut self, data: &[u8]) -> Vec<MqttEvent> {
        self.engine.handle_incoming(data)
    }

    /// Take outgoing bytes to be written to the network.
    ///
    /// Call this method after any operation that might generate outgoing packets
    /// (connect, publish, subscribe, etc.) and write the returned bytes to your socket.
    ///
    /// # Returns
    ///
    /// A vector of bytes to send to the network. Returns an empty vector if there's
    /// nothing to send.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.connect();
    ///
    /// let outgoing = client.take_outgoing();
    /// if !outgoing.is_empty() {
    ///     // Write to your socket
    ///     // socket.write_all(&outgoing)?;
    /// }
    /// ```
    pub fn take_outgoing(&mut self) -> Vec<u8> {
        self.engine.take_outgoing()
    }

    /// Process protocol timer ticks for keep-alive and retransmissions.
    ///
    /// Call this method periodically based on the time returned by [`next_tick_at()`](Self::next_tick_at).
    /// This handles:
    ///
    /// - Keep-alive PING packets
    /// - Connection timeout detection
    /// - QoS message retransmissions
    ///
    /// # Arguments
    ///
    /// * `now` - Current time
    ///
    /// # Returns
    ///
    /// A vector of [`MqttEvent`]s generated (e.g., `ReconnectNeeded` on timeout).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # use std::time::Instant;
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let now = Instant::now();
    ///
    /// if let Some(next_tick) = client.next_tick_at() {
    ///     if now >= next_tick {
    ///         let events = client.handle_tick(now);
    ///         // Process events...
    ///     }
    /// }
    /// ```
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        self.engine.handle_tick(now)
    }

    /// Get the next time when [`handle_tick()`](Self::handle_tick) should be called.
    ///
    /// Use this to optimize your event loop by only waking up when necessary.
    /// Returns `None` if not connected or no timer is needed.
    ///
    /// # Returns
    ///
    /// The next `Instant` when a protocol timer needs to fire, or `None`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # use std::time::{Instant, Duration};
    /// # let client = NoIoMqttClient::new(MqttClientOptions::default());
    /// if let Some(next_tick) = client.next_tick_at() {
    ///     let now = Instant::now();
    ///     if next_tick > now {
    ///         let sleep_duration = next_tick - now;
    ///         // Sleep for `sleep_duration` or until network data arrives
    ///     }
    /// }
    /// ```
    pub fn next_tick_at(&self) -> Option<Instant> {
        self.engine.next_tick_at()
    }

    /// Take all pending events from the client.
    ///
    /// This is an alternative to processing events returned by `handle_incoming()`
    /// and `handle_tick()`. Useful if you want to poll for events separately.
    ///
    /// # Returns
    ///
    /// A vector of all pending [`MqttEvent`]s.
    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        self.engine.take_events()
    }

    // ========================================================================
    // MQTT Operations
    // ========================================================================

    /// Initiate a connection to the MQTT broker.
    ///
    /// This enqueues a CONNECT packet. Call [`take_outgoing()`](Self::take_outgoing)
    /// to get the bytes to send to the network.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.connect();
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn connect(&mut self) {
        self.engine.connect();
    }

    /// Publish a message to a topic.
    ///
    /// # Arguments
    ///
    /// * `command` - Publish command with topic, payload, QoS, etc.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(packet_id))` for QoS 1/2 messages
    /// - `Ok(None)` for QoS 0 messages
    /// - `Err(...)` if not connected or packet ID allocation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, PublishCommand};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let cmd = PublishCommand::builder()
    ///     .topic("sensors/temperature")
    ///     .payload(b"23.5")
    ///     .qos(1)
    ///     .build()
    ///     .unwrap();
    ///
    /// match client.publish(cmd) {
    ///     Ok(Some(packet_id)) => println!("Published with ID: {}", packet_id),
    ///     Ok(None) => println!("Published (QoS 0)"),
    ///     Err(e) => eprintln!("Publish failed: {}", e),
    /// }
    ///
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.engine.publish(command)
    }

    /// Subscribe to one or more topics.
    ///
    /// # Arguments
    ///
    /// * `command` - Subscribe command with topics and QoS levels
    ///
    /// # Returns
    ///
    /// - `Ok(packet_id)` on success
    /// - `Err(...)` if not connected or packet ID allocation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, SubscribeCommand};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/+/temperature", 1)
    ///     .add_topic("alerts/#", 2)
    ///     .build()
    ///     .unwrap();
    ///
    /// match client.subscribe(cmd) {
    ///     Ok(packet_id) => println!("Subscribe packet ID: {}", packet_id),
    ///     Err(e) => eprintln!("Subscribe failed: {}", e),
    /// }
    ///
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.engine.subscribe(command)
    }

    /// Unsubscribe from one or more topics.
    ///
    /// # Arguments
    ///
    /// * `command` - Unsubscribe command with topics
    ///
    /// # Returns
    ///
    /// - `Ok(packet_id)` on success
    /// - `Err(...)` if not connected or packet ID allocation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, UnsubscribeCommand};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let cmd = UnsubscribeCommand::from_topics(vec!["sensors/+/temperature".to_string()]);
    ///
    /// match client.unsubscribe(cmd) {
    ///     Ok(packet_id) => println!("Unsubscribe packet ID: {}", packet_id),
    ///     Err(e) => eprintln!("Unsubscribe failed: {}", e),
    /// }
    ///
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.engine.unsubscribe(command)
    }

    /// Send a PINGREQ packet to the broker.
    ///
    /// Normally you don't need to call this manually as [`handle_tick()`](Self::handle_tick)
    /// will send pings automatically based on the keep-alive interval.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.ping();
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn ping(&mut self) {
        self.engine.send_ping();
    }

    /// Send a DISCONNECT packet to the broker.
    ///
    /// This gracefully closes the connection. After calling this, you should
    /// send the outgoing bytes and close your socket.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.disconnect();
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker, then close socket
    /// ```
    pub fn disconnect(&mut self) {
        self.engine.disconnect();
    }

    /// Send an AUTH packet for enhanced authentication (MQTT v5 only).
    ///
    /// Used for multi-step authentication flows like SCRAM, OAuth, etc.
    ///
    /// # Arguments
    ///
    /// * `reason_code` - Authentication reason code
    /// * `properties` - Authentication properties
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// // Send authentication response
    /// client.auth(0x18, vec![/* auth properties */]);
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn auth(&mut self, reason_code: u8, properties: Vec<Property>) {
        self.engine.auth(reason_code, properties);
    }

    // ========================================================================
    // State Queries
    // ========================================================================

    /// Check if the client is logically connected.
    ///
    /// Returns `true` after receiving a successful CONNACK, `false` otherwise.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let client = NoIoMqttClient::new(MqttClientOptions::default());
    /// if client.is_connected() {
    ///     println!("Client is connected");
    /// }
    /// ```
    pub fn is_connected(&self) -> bool {
        self.engine.is_connected()
    }

    /// Get the MQTT protocol version being used.
    ///
    /// # Returns
    ///
    /// - `5` for MQTT v5.0
    /// - `4` for MQTT v3.1.1
    /// - `3` for MQTT v3.1
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let client = NoIoMqttClient::new(MqttClientOptions::default());
    /// match client.mqtt_version() {
    ///     5 => println!("Using MQTT v5.0"),
    ///     4 => println!("Using MQTT v3.1.1"),
    ///     3 => println!("Using MQTT v3.1"),
    ///     _ => println!("Unknown version"),
    /// }
    /// ```
    pub fn mqtt_version(&self) -> u8 {
        self.engine.mqtt_version()
    }

    /// Get a reference to the client options.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let options = client.options();
    /// println!("Client ID: {}", options.client_id);
    /// println!("Keep alive: {}", options.keep_alive);
    /// ```
    pub fn options(&self) -> &MqttClientOptions {
        self.engine.options()
    }

    // ========================================================================
    // Connection Management
    // ========================================================================

    /// Handle connection lost state.
    ///
    /// Call this when your socket disconnects or encounters an error.
    /// This resets the protocol state and clears pending operations.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// // Socket error detected
    /// client.handle_connection_lost();
    ///
    /// // Later, reconnect
    /// // ... create new socket ...
    /// client.connect();
    /// ```
    pub fn handle_connection_lost(&mut self) {
        self.engine.handle_connection_lost();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_client() {
        let options = MqttClientOptions::builder()
            .peer("localhost:1883")
            .client_id("test_client")
            .build();

        let client = NoIoMqttClient::new(options);
        assert!(!client.is_connected());
        assert_eq!(client.mqtt_version(), 5); // Default is v5
    }

    #[test]
    fn test_connect_generates_packet() {
        let options = MqttClientOptions::builder()
            .peer("localhost:1883")
            .client_id("test_client")
            .build();

        let mut client = NoIoMqttClient::new(options);
        client.connect();

        let outgoing = client.take_outgoing();
        assert!(!outgoing.is_empty());
        // CONNECT packet starts with 0x10 (packet type)
        assert_eq!(outgoing[0] & 0xF0, 0x10);
    }

    #[test]
    fn test_handle_connack() {
        let options = MqttClientOptions::builder()
            .peer("localhost:1883")
            .client_id("test_client")
            .build();

        let mut client = NoIoMqttClient::new(options);
        client.connect();
        let _ = client.take_outgoing();

        // Simulate CONNACK for MQTT v5: 0x20 (type), 0x03 (length), 0x00 (flags), 0x00 (reason code), 0x00 (properties length)
        let connack = vec![0x20, 0x03, 0x00, 0x00, 0x00];
        let events = client.handle_incoming(&connack);

        assert!(!events.is_empty(), "Should have received events");
        assert!(
            matches!(events[0], MqttEvent::Connected(_)),
            "First event should be Connected, got: {:?}",
            events[0]
        );
        assert!(client.is_connected());
    }

    #[test]
    fn test_publish_qos0() {
        let options = MqttClientOptions::builder()
            .peer("localhost:1883")
            .client_id("test_client")
            .build();

        let mut client = NoIoMqttClient::new(options);

        // Connect first
        client.connect();
        let _ = client.take_outgoing();

        // Simulate CONNACK
        let connack = vec![0x20, 0x03, 0x00, 0x00, 0x00];
        let _ = client.handle_incoming(&connack);
        assert!(client.is_connected());

        // Now publish
        let cmd = PublishCommand::simple("test/topic", b"payload".to_vec(), 0, false);
        let result = client.publish(cmd);

        // QoS 0 returns None for packet ID
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);

        let outgoing = client.take_outgoing();
        assert!(!outgoing.is_empty());
    }

    #[test]
    fn test_next_tick_at_when_disconnected() {
        let options = MqttClientOptions::builder()
            .peer("localhost:1883")
            .client_id("test_client")
            .build();

        let client = NoIoMqttClient::new(options);
        assert!(client.next_tick_at().is_none());
    }

    #[test]
    fn test_connection_lost() {
        let options = MqttClientOptions::builder()
            .peer("localhost:1883")
            .client_id("test_client")
            .build();

        let mut client = NoIoMqttClient::new(options);
        client.connect();
        let _ = client.take_outgoing();

        // Simulate CONNACK
        let connack = vec![0x20, 0x03, 0x00, 0x00, 0x00];
        let _ = client.handle_incoming(&connack);
        assert!(client.is_connected());

        // Simulate connection lost
        client.handle_connection_lost();
        assert!(!client.is_connected());
    }
}
