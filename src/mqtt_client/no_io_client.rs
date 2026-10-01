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
//! # Portable usage
//!
//! ```
//! use flowsdk::mqtt_client::{MqttClientOptions, PortableNoIoMqttClient};
//! use flowsdk::time::Timestamp;
//!
//! let now = Timestamp::try_from_millis(12_345).unwrap();
//! let mut client = PortableNoIoMqttClient::try_new_at(MqttClientOptions::default(), now).unwrap();
//! client.connect_at(now).unwrap();
//! let outgoing = client.take_outgoing_at(now).unwrap();
//! assert_eq!(outgoing[0], 0x10);
//! // Send outgoing bytes through an application-owned transport. Feed responses
//! // through handle_incoming_at(bytes, now) and timers through handle_tick_at(now).
//! ```
//!
//! Use one monotonic origin for every operation. Equal timestamps are valid;
//! backward time and deadline overflow are rejected before changing state.
//! The application supplies allocation, networking and scheduling. With `std`,
//! the existing `NoIoMqttClient::new` and host-clock methods are also available.

use alloc::vec::Vec;

use crate::mqtt_client::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use crate::mqtt_client::engine::{MqttEngine, MqttEvent};
use crate::mqtt_client::error::MqttClientError;
use crate::mqtt_client::opts::MqttClientOptions;
use crate::mqtt_serde::mqttv5::common::properties::Property;
#[cfg(feature = "durable-session")]
use crate::mqtt_session::ClientSessionState;
use crate::time::{DefaultTime, TimePoint, Timestamp};
#[cfg(feature = "std")]
use std::time::Instant;

/// A pure Sans-I/O MQTT client.
///
/// This client handles all MQTT protocol logic without performing any I/O operations.
/// You are responsible for:
///
/// - Reading bytes from the network and feeding them via [`handle_incoming_at()`](Self::handle_incoming_at)
/// - Writing bytes from [`take_outgoing_at()`](Self::take_outgoing_at) to the network
/// - Calling [`handle_tick_at()`](Self::handle_tick_at) at appropriate intervals for keep-alive and retransmissions
///
/// # Thread Safety
///
/// This client is `Send` and `Sync`; protocol operations require exclusive access.
///
/// # Example
///
/// ```no_run
/// use flowsdk::mqtt_client::{PortableNoIoMqttClient as NoIoMqttClient, MqttClientOptions};
///
/// let options = MqttClientOptions::builder()
///     .peer("broker.example.com:1883")
///     .client_id("my_client")
///     .build();
///
/// let mut client = NoIoMqttClient::new_at(options, flowsdk::time::Timestamp::ZERO);
/// client.connect_at(flowsdk::time::Timestamp::ZERO).unwrap();
///
/// // Get CONNECT packet bytes
/// let bytes = client.take_outgoing_at(flowsdk::time::Timestamp::ZERO).unwrap();
/// // ... send `bytes` to your socket ...
/// ```
pub struct NoIoMqttClient<T: TimePoint = DefaultTime> {
    engine: MqttEngine<T>,
}

#[cfg(feature = "std")]
mod host;

/// No-I/O client with an explicit clock on every platform.
pub type PortableNoIoMqttClient = NoIoMqttClient<Timestamp>;

impl<T: TimePoint> NoIoMqttClient<T> {
    /// Construct a client with a caller-supplied monotonic origin.
    pub fn new_at(options: MqttClientOptions, now: T) -> Self {
        Self {
            engine: MqttEngine::new_at(options, now),
        }
    }

    /// Capture serializable MQTT state for an application-owned session store.
    /// See [`MqttEngine::snapshot_session`] for checkpoint ordering requirements.
    #[cfg(feature = "durable-session")]
    pub fn snapshot_session(&self) -> Result<ClientSessionState, MqttClientError> {
        self.engine.snapshot_session()
    }

    /// Load a checkpoint into a fresh client before calling `connect`.
    /// Requires the same peer/client ID and `clean_start(false)`.
    #[cfg(feature = "durable-session")]
    pub fn restore_session_state(
        &mut self,
        state: ClientSessionState,
    ) -> Result<(), MqttClientError> {
        self.engine.restore_session_state(state)
    }

    /// Check configuration and timestamp before constructing the client.
    pub fn try_new_at(options: MqttClientOptions, now: T) -> Result<Self, MqttClientError> {
        Ok(Self {
            engine: MqttEngine::try_new_at(options, now)?,
        })
    }
    /// Run `handle_incoming` with caller-supplied monotonic time.
    pub fn handle_incoming_at(
        &mut self,
        data: &[u8],
        now: T,
    ) -> Result<Vec<MqttEvent>, MqttClientError> {
        self.engine.handle_incoming_at(data, now)
    }
    /// Run `take_outgoing` with caller-supplied monotonic time.
    pub fn take_outgoing_at(&mut self, now: T) -> Result<Vec<u8>, MqttClientError> {
        self.engine.take_outgoing_at(now)
    }
    /// Run `handle_tick` with caller-supplied monotonic time.
    pub fn handle_tick_at(&mut self, now: T) -> Result<Vec<MqttEvent>, MqttClientError> {
        self.engine.handle_tick_at(now)
    }

    /// Get the next time when [`handle_tick_at()`](Self::handle_tick_at) should be called.
    ///
    /// Use this to optimize your event loop by only waking up when necessary.
    /// Returns `None` if no timer is needed, including during disconnection.
    ///
    /// # Returns
    ///
    /// The next timestamp when a protocol timer needs to fire, or `None`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{PortableNoIoMqttClient as NoIoMqttClient, MqttClientOptions};
    /// # use flowsdk::time::Timestamp;
    /// # let client = NoIoMqttClient::new_at(MqttClientOptions::default(), flowsdk::time::Timestamp::ZERO);
    /// if let Some(next_tick) = client.next_tick_at() {
    ///     let now = Timestamp::ZERO;
    ///     if next_tick > now {
    ///         let sleep_duration = next_tick.saturating_duration_since(now);
    ///         // Sleep for `sleep_duration` or until network data arrives
    ///     }
    /// }
    /// ```
    pub fn next_tick_at(&self) -> Option<T> {
        self.engine.next_tick_at()
    }

    /// Take all pending events from the client.
    ///
    /// Events returned by `handle_incoming()` and `handle_tick()` are already drained.
    /// This retrieves additional events from commands and output-drain processing.
    ///
    /// # Returns
    ///
    /// A vector of all pending [`MqttEvent`]s.
    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        self.engine.take_events()
    }

    /// Select how deeply subsequent incoming MQTT packets are parsed.
    pub fn set_parse_level(&mut self, level: crate::mqtt_serde::ParseLevel) {
        self.engine.set_parse_level(level);
    }

    pub fn parse_level(&self) -> crate::mqtt_serde::ParseLevel {
        self.engine.parse_level()
    }
    /// Run `connect` with caller-supplied monotonic time.
    pub fn connect_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.engine.connect_at(now)
    }
    /// Run `publish` with caller-supplied monotonic time.
    pub fn publish_at(
        &mut self,
        command: PublishCommand,
        now: T,
    ) -> Result<Option<u16>, MqttClientError> {
        self.engine.publish_at(command, now)
    }
    /// Run `subscribe` with caller-supplied monotonic time.
    pub fn subscribe_at(
        &mut self,
        command: SubscribeCommand,
        now: T,
    ) -> Result<u16, MqttClientError> {
        self.engine.subscribe_at(command, now)
    }
    /// Run `unsubscribe` with caller-supplied monotonic time.
    pub fn unsubscribe_at(
        &mut self,
        command: UnsubscribeCommand,
        now: T,
    ) -> Result<u16, MqttClientError> {
        self.engine.unsubscribe_at(command, now)
    }
    /// Run `ping` with caller-supplied monotonic time.
    pub fn ping_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.engine.try_send_ping_at(now)
    }
    /// Run `disconnect` with caller-supplied monotonic time.
    pub fn disconnect_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.engine.disconnect_at(now)
    }
    /// Run `auth` with caller-supplied monotonic time.
    pub fn auth_at(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.engine.auth_at(reason_code, properties, now)
    }
    /// Run `reset_for_new_transport` with caller-supplied monotonic time.
    pub fn reset_for_new_transport_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.engine.reset_for_new_transport_at(now)
    }
    /// Run `schedule_reconnect` with caller-supplied monotonic time.
    pub fn schedule_reconnect_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.engine.schedule_reconnect_at(now)
    }

    /// Whether wire bytes or protocol responses are waiting to be drained.
    pub fn has_pending_output(&self) -> bool {
        self.engine.has_pending_output()
    }
    /// Run `disconnect_with` with caller-supplied monotonic time.
    pub fn disconnect_with_at(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.engine
            .try_disconnect_with_at(reason_code, properties, now)
    }
    /// Run `puback` with caller-supplied monotonic time.
    pub fn puback_at(
        &mut self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.engine
            .puback_at(packet_id, reason_code, properties, now)
    }
    /// Run `pubrec` with caller-supplied monotonic time.
    pub fn pubrec_at(
        &mut self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.engine
            .pubrec_at(packet_id, reason_code, properties, now)
    }
    /// Run `pubcomp` with caller-supplied monotonic time.
    pub fn pubcomp_at(
        &mut self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.engine
            .pubcomp_at(packet_id, reason_code, properties, now)
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
    /// # use flowsdk::mqtt_client::{PortableNoIoMqttClient as NoIoMqttClient, MqttClientOptions};
    /// # let client = NoIoMqttClient::new_at(MqttClientOptions::default(), flowsdk::time::Timestamp::ZERO);
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
    /// - `3` is also MQTT v3.1.1
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{PortableNoIoMqttClient as NoIoMqttClient, MqttClientOptions};
    /// # let client = NoIoMqttClient::new_at(MqttClientOptions::default(), flowsdk::time::Timestamp::ZERO);
    /// match client.mqtt_version() {
    ///     5 => println!("Using MQTT v5.0"),
    ///     4 => println!("Using MQTT v3.1.1"),
    ///     3 => println!("Using MQTT v3.1.1"),
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
    /// # use flowsdk::mqtt_client::{PortableNoIoMqttClient as NoIoMqttClient, MqttClientOptions};
    /// # let client = NoIoMqttClient::new_at(MqttClientOptions::default(), flowsdk::time::Timestamp::ZERO);
    /// let options = client.options();
    /// println!("Client ID: {}", options.client_id);
    /// println!("Keep alive: {}", options.keep_alive);
    /// ```
    pub fn options(&self) -> &MqttClientOptions {
        self.engine.options()
    }
    /// Run `handle_connection_lost` with caller-supplied monotonic time.
    pub fn handle_connection_lost_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.engine.handle_connection_lost_at(now)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(all(test, feature = "std"))]
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
        client.connect().unwrap();

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
        client.connect().unwrap();
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
        client.connect().unwrap();
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
        client.connect().unwrap();
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
