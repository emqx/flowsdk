#[cfg(feature = "quic")]
use quinn_proto::{ClientConfig, Connection, ConnectionHandle, Endpoint, EndpointConfig, StreamId};
use std::collections::VecDeque;
#[cfg(feature = "quic")]
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv3::{
    connectv3, disconnectv3, pingreqv3, pubrelv3, subscribev3, unsubscribev3,
};
use crate::mqtt_serde::mqttv5::{
    authv5, common::properties::Property, connectv5, disconnectv5, pingreqv5, pubackv5::MqttPubAck,
    pubcompv5::MqttPubComp, publishv5::MqttPublish, pubrecv5::MqttPubRec, pubrelv5::MqttPubRel,
    subscribev5, unsubscribev5,
};
use crate::mqtt_serde::parser::stream::MqttParser;
use crate::mqtt_session::ClientSession;
use crate::priority_queue::PriorityQueue;

use super::client::{
    ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use super::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use super::error::MqttClientError;
use super::inflight::InflightQueue;
use super::opts::MqttClientOptions;

/// Alias for `MqttPublish` (v5) to provide a single, unified type for received messages.
///
/// The engine normalizes all incoming PUBLISH packets (whether MQTT v3.1.1 or v5.0) into this structure.
/// This simplifies downstream consumption by providing a consistent API regardless of the protocol version used.
/// For MQTT v3.1.1 messages, the v5-specific fields (Properties) will be empty.
pub type MqttMessage = MqttPublish;

/// Events emitted by the MqttEngine to be handled by the application (I/O layer)
#[derive(Debug)]
pub enum MqttEvent {
    Connected(ConnectionResult),
    Disconnected(Option<u8>),
    Published(PublishResult),
    Subscribed(SubscribeResult),
    Unsubscribed(UnsubscribeResult),
    MessageReceived(MqttMessage),
    PingResponse(PingResult),
    Error(MqttClientError),
    /// Signal that a reconnection is needed (e.g. after keep-alive timeout)
    ReconnectNeeded,
    /// Reconnection scheduled with exponential backoff
    ReconnectScheduled {
        attempt: u32,
        delay: Duration,
    },
}

/// A "Sans-I/O" MQTTv3.1.1/v5.0 protocol engine.
///
/// This engine strictly handles the *protocol state* of an MQTT connection without directly performing any I/O operations.
/// It is designed to be embedded within an I/O runtime (like Tokio) or used in other environments (embedded firmware, FFI).
///
/// # Architecture
///
/// The engine functions as a state machine:
/// - **Input**:
///     - Bytes received from the network (`handle_incoming`).
///     - Time ticks for keep-alive/timeouts (`handle_tick`).
///     - High-level commands like `publish`, `subscribe` calls.
/// - **Output**:
///     - Bytes to be sent to the network (accessible via `take_outgoing`).
///     - Events (state changes, incoming messages) for the application (`take_events`).
///
/// # Usage
///
/// 1. Initialize with `MqttClientOptions`.
/// 2. Connect the underlying transport (TCP/TLS/QUIC/etc.).
/// 3. Call `connect()` to initiate the MQTT handshake.
/// 4. In a loop:
///     - Feed incoming bytes: `engine.handle_incoming(&buf)`.
///     - Check for outgoing bytes: `engine.take_outgoing()`.
///     - Handle events: `engine.take_events()`.
///     - Manage time: Call `engine.handle_tick(now)` and sleep until `engine.next_tick_at()`.
///
/// # Buffer Limits
///
/// The engine enforces strict buffer limits to prevent memory exhaustion:
/// - `outgoing_buffer`: Limits queued packets waiting to be sent. Returns `MqttClientError::BufferFull` if exceeded.
/// - `events`: Limits pending events. Pauses parsing (back-pressure) if limit reached.
pub struct MqttEngine {
    options: MqttClientOptions,
    session: Option<ClientSession>,
    priority_queue: PriorityQueue<u8, MqttPacket>,
    is_connected: bool,
    last_packet_sent: Instant,
    last_packet_received: Instant,

    // Buffers and Parsers
    parser: MqttParser,
    outgoing_buffer: VecDeque<Vec<u8>>,

    // Pending operations tracking (state only)
    inflight_queue: InflightQueue,

    events: Vec<MqttEvent>,

    // Reconnection state
    reconnect_attempts: u32,
    next_reconnect_at: Option<Instant>,

    // Configurable timeouts (cached from options for efficiency)
    reconnect_base_delay: Duration,
    reconnect_max_delay: Duration,
    max_reconnect_attempts: u32,
}

impl MqttEngine {
    /// Create a new `MqttEngine` with the given configuration options.
    ///
    /// The engine requires strict configuration for buffer limits and timeouts.
    /// Default buffer size for the internal parser is 16KB.
    pub fn new(options: MqttClientOptions) -> Self {
        let mqtt_version = options.mqtt_version;
        // Default buffer size 16KB
        let parser = MqttParser::new(16384, mqtt_version);

        // Cache timeout values for efficiency
        let retransmission_timeout = Duration::from_millis(options.retransmission_timeout_ms);
        let reconnect_base_delay = Duration::from_millis(options.reconnect_base_delay_ms);
        let reconnect_max_delay = Duration::from_millis(options.reconnect_max_delay_ms);
        let max_reconnect_attempts = options.max_reconnect_attempts;

        Self {
            inflight_queue: InflightQueue::new(
                options.receive_maximum,
                options.mqtt_version,
                retransmission_timeout,
            ),
            session: None,
            priority_queue: PriorityQueue::new(1000),
            is_connected: false,
            last_packet_sent: Instant::now(),
            last_packet_received: Instant::now(),
            parser,
            outgoing_buffer: VecDeque::new(),
            events: Vec::new(),
            reconnect_attempts: 0,
            next_reconnect_at: None,
            reconnect_base_delay,
            reconnect_max_delay,
            max_reconnect_attempts,
            options,
        }
    }

    /// Drain all pending events from the engine.
    ///
    /// This should be called frequently (e.g., after `handle_incoming` or `handle_tick`)
    /// to process state changes and incoming messages.
    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn options(&self) -> &MqttClientOptions {
        &self.options
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }

    pub fn handle_connection_lost(&mut self) {
        self.is_connected = false;
    }

    /// Schedule the next reconnection attempt using exponential backoff.
    ///
    /// Logic: `delay = min(base * 2^attempts, max)`.
    ///
    /// If `max_reconnect_attempts` is set and reached, no reconnection is scheduled,
    /// and the engine remains in a disconnected state essentially "giving up".
    ///
    /// Emits `MqttEvent::ReconnectScheduled` to notify the application of the next attempt.
    pub fn schedule_reconnect(&mut self, now: Instant) {
        // Check if max attempts reached
        if self.max_reconnect_attempts > 0 && self.reconnect_attempts >= self.max_reconnect_attempts
        {
            // Max attempts reached, don't schedule
            self.next_reconnect_at = None;
            return;
        }

        // Calculate exponential backoff: base_delay * 2^attempts
        // Cap exponent at 10 to prevent overflow (2^10 = 1024)
        let exponent = self.reconnect_attempts.min(10);
        let multiplier = 1u64 << exponent; // 2^exponent

        let delay_ms = self
            .reconnect_base_delay
            .as_millis()
            .saturating_mul(multiplier as u128);

        // Cap at max delay
        let delay_ms = delay_ms.min(self.reconnect_max_delay.as_millis());
        let delay = Duration::from_millis(delay_ms as u64);

        self.next_reconnect_at = Some(now + delay);
        self.reconnect_attempts += 1;

        self.events.push(MqttEvent::ReconnectScheduled {
            attempt: self.reconnect_attempts,
            delay,
        });
    }

    /// Reset reconnection state after successful connection.
    ///
    /// Call this after receiving a successful CONNACK to reset the
    /// reconnection attempt counter and clear any scheduled reconnection.
    pub fn reset_reconnect_state(&mut self) {
        self.reconnect_attempts = 0;
        self.next_reconnect_at = None;
    }

    /// Feed raw bytes received from the network into the protocol parser.
    ///
    /// This method parses the input stream into MQTT packets and updates the internal state.
    ///
    /// # Back-pressure
    ///
    /// If the internal `events` buffer reaches `max_event_count`, this method will **stop processing**
    /// and return early, leaving remaining bytes in the internal buffer. The caller should
    /// consume events via `take_events()` and call `handle_incoming(&[])` again to resume processing.
    pub fn handle_incoming(&mut self, data: &[u8]) -> Vec<MqttEvent> {
        self.parser.feed(data);
        self.last_packet_received = Instant::now();

        loop {
            if self.events.len() >= self.options.max_event_count {
                // Buffer full, stop processing for now.
                // Remaining data stays in parser/buffer.
                break;
            }

            match self.parser.next_packet() {
                Ok(Some(packet)) => {
                    let packet_events = self.handle_packet(packet);
                    self.events.extend(packet_events);
                }
                Ok(None) => break,
                Err(e) => {
                    self.events.push(MqttEvent::Error(MqttClientError::from(e)));
                    break;
                }
            }
        }

        self.take_events()
    }

    pub fn mqtt_version(&self) -> u8 {
        self.options.mqtt_version
    }

    /// Process time-dependent logic (keep-alive, timeouts, retransmissions).
    ///
    /// This should be called at every tick of the run loop or when the `next_tick_at` deadline expires.
    ///
    /// # Operations
    /// 1. **Reconnection**: If disconnected and it's time to reconnect, emits `ReconnectNeeded`.
    /// 2. **Keep-Alive**: Sends `PINGREQ` if no control packets have been sent within the Keep-Alive interval.
    /// 3. **Timeout Detection**: Detects dead connections (no data received for Keep-Alive * multiplier) -> Disconnects and schedules reconnect.
    /// 4. **Retransmissions** (MQTT v3.1.1): Resends unacknowledged QoS 1/2 packets.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        // Handle reconnection timer when disconnected
        if !self.is_connected {
            if let Some(reconnect_at) = self.next_reconnect_at {
                if now >= reconnect_at {
                    self.events.push(MqttEvent::ReconnectNeeded);
                    self.next_reconnect_at = None;
                }
            }
            return self.take_events();
        }

        let keep_alive = Duration::from_secs(self.options.keep_alive as u64);

        // 1. Keep-alive: Send PING if needed
        if keep_alive > Duration::ZERO && now.duration_since(self.last_packet_sent) >= keep_alive {
            self.send_ping();
            self.last_packet_sent = now;
        }

        // 2. Connection timeout: Detect dead connection
        if keep_alive > Duration::ZERO
            && now.duration_since(self.last_packet_received)
                >= keep_alive * self.options.ping_timeout_multiplier
        {
            self.events.push(MqttEvent::ReconnectNeeded);
            self.handle_connection_lost();
            // Schedule reconnection with backoff
            self.schedule_reconnect(now);
            return self.take_events();
        }

        // 3. Retransmissions
        let retrans_events = self.handle_retransmissions(now);
        self.events.extend(retrans_events);

        self.take_events()
    }

    fn handle_retransmissions(&mut self, now: Instant) -> Vec<MqttEvent> {
        let events = Vec::new();

        let expired = self.inflight_queue.get_expired(now);
        for packet in expired {
            if let Ok(bytes) = packet.to_bytes() {
                self.outgoing_buffer.push_back(bytes);
                self.last_packet_sent = now;
            }
        }

        events
    }

    /// Returns the exact timestamp of the next required wake-up.
    ///
    /// The runtime loop should sleep until this timestamp to avoid busy-waiting.
    ///
    /// Returns `None` if there are no scheduled timer events (sleep indefinitely or until IO).
    ///
    /// Prioritizes:
    /// 1. Reconnection attempts (if disconnected).
    /// 2. Keep-alive PINGs.
    /// 3. Connection timeout checks.
    /// 4. Packet retransmissions.
    pub fn next_tick_at(&self) -> Option<Instant> {
        // 1. Reconnection timer (highest priority when disconnected)
        if !self.is_connected {
            return self.next_reconnect_at;
        }

        let mut next = None;
        let keep_alive = Duration::from_secs(self.options.keep_alive as u64);

        // 2. Keep-alive timer (send PING)
        if keep_alive > Duration::ZERO {
            let ping_deadline = self.last_packet_sent + keep_alive;
            next = Some(ping_deadline);
        }

        // 3. Connection timeout (detect dead connection)
        if keep_alive > Duration::ZERO {
            let timeout = keep_alive * self.options.ping_timeout_multiplier;
            let timeout_deadline = self.last_packet_received + timeout;
            if next.is_none() || timeout_deadline < next.unwrap() {
                next = Some(timeout_deadline);
            }
        }

        // 4. Retransmission timeouts (QoS 1/2 messages)
        // Only for MQTT v3.1.1, as v5.0 forbids client-side retransmission
        if let Some(resend_at) = self.inflight_queue.next_expiration() {
            if next.is_none() || resend_at < next.unwrap() {
                next = Some(resend_at);
            }
        }

        next
    }

    /// Take bytes ready to be sent to the network.
    ///
    /// This should be written to the underlying transport immediately.
    /// Clears the internal outgoing buffer.
    pub fn take_outgoing(&mut self) -> Vec<u8> {
        let mut all_bytes = Vec::new();
        while let Some(packet) = self.outgoing_buffer.pop_front() {
            all_bytes.extend(packet);
        }
        all_bytes
    }

    // --- Command Methods ---

    /// Initiate the MQTT connection handshake (send CONNECT packet).
    ///
    /// Should be called after the physical connection is established.
    pub fn connect(&mut self) {
        if self.is_connected {
            return;
        }

        // Initialize session if needed
        if self.session.is_none() {
            self.session = Some(ClientSession::new());
        }

        let packet = if self.options.mqtt_version == 5 {
            let connect = connectv5::MqttConnect::new(
                self.options.client_id.clone(),
                self.options.username.clone(),
                self.options.password.clone(),
                None, // Will
                self.options.keep_alive,
                self.options.clean_start,
                Vec::new(), // Properties
            );
            MqttPacket::Connect5(connect)
        } else {
            let connect = connectv3::MqttConnect::new(
                self.options.client_id.clone(),
                self.options.keep_alive,
                self.options.clean_start,
            );
            MqttPacket::Connect3(connect)
        };

        let _ = self.enqueue_packet(packet);
    }

    /// Queue a PUBLISH packet.
    ///
    /// - **QoS 0**: ID is usually None (unless needed for tracing).
    /// - **QoS 1/2**: Returns the assigned Packet ID (or uses the one provided).
    ///
    /// The command is pushed to the `PriorityQueue` and only moved to the `outgoing_buffer`
    /// via `process_queue()` if the buffer limits allow.
    pub fn publish(&mut self, mut command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        let pid = if command.qos > 0 {
            if let Some(pid) = command.packet_id {
                Some(pid)
            } else {
                let pid = self.next_packet_id()?;
                command.packet_id = Some(pid);
                Some(pid)
            }
        } else {
            None
        };

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Publish5(command.to_mqtt_publish())
        } else {
            MqttPacket::Publish3(command.to_mqttv3_publish())
        };

        if let Some(_pid) = pid {
            // QoS > 0 messages are pushed to inflight only when they are about to be sent.
            // But we can check here if we have room in the inflight queue.
            // However, the priority queue is the one that manages the order.
            // We'll check inflight capacity in process_queue().
        }

        self.priority_queue.enqueue(command.priority, packet);
        self.process_queue();
        Ok(pid)
    }

    /// Queue a SUBSCRIBE packet.
    ///
    /// Be aware that this might fail immediately with `MqttClientError::BufferFull`
    /// if the outgoing buffer is at capacity.
    pub fn subscribe(&mut self, mut command: SubscribeCommand) -> Result<u16, MqttClientError> {
        let pid = if let Some(pid) = command.packet_id {
            pid
        } else {
            let pid = self.next_packet_id()?;
            command.packet_id = Some(pid);
            pid
        };

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Subscribe5(subscribev5::MqttSubscribe::new(
                pid,
                command.subscriptions,
                command.properties,
            ))
        } else {
            let v3_subs = command
                .subscriptions
                .into_iter()
                .map(|s| subscribev3::SubscriptionTopic {
                    topic_filter: s.topic_filter,
                    qos: s.qos,
                })
                .collect();
            MqttPacket::Subscribe3(subscribev3::MqttSubscribe::new(pid, v3_subs))
        };

        self.inflight_queue.push(pid, packet.clone(), 1)?;
        self.enqueue_packet(packet)?;
        Ok(pid)
    }

    pub fn unsubscribe(&mut self, mut command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        let pid = if let Some(pid) = command.packet_id {
            pid
        } else {
            let pid = self.next_packet_id()?;
            command.packet_id = Some(pid);
            pid
        };

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Unsubscribe5(unsubscribev5::MqttUnsubscribe::new(
                pid,
                command.topics.clone(),
                command.properties,
            ))
        } else {
            MqttPacket::Unsubscribe3(unsubscribev3::MqttUnsubscribe::new(pid, command.topics))
        };

        self.inflight_queue.push(pid, packet.clone(), 1)?;
        self.enqueue_packet(packet)?;
        Ok(pid)
    }

    /// Queue a DISCONNECT packet and update state to disconnected.
    pub fn disconnect(&mut self) {
        if !self.is_connected {
            return;
        }

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Disconnect5(disconnectv5::MqttDisconnect::new(0, Vec::new()))
        } else {
            MqttPacket::Disconnect3(disconnectv3::MqttDisconnect::new())
        };

        let _ = self.enqueue_packet(packet);
        self.is_connected = false;
    }

    pub fn auth(&mut self, reason_code: u8, properties: Vec<Property>) {
        if self.options.mqtt_version == 5 {
            let auth = authv5::MqttAuth::new(reason_code, properties);
            let _ = self.enqueue_packet(MqttPacket::Auth(auth));
        }
    }

    // --- Internal Helpers ---

    fn handle_packet(&mut self, packet: MqttPacket) -> Vec<MqttEvent> {
        let mut events = Vec::new();
        match packet {
            MqttPacket::ConnAck5(ack) => {
                self.is_connected = ack.reason_code == 0;
                if self.is_connected {
                    self.reset_reconnect_state();
                }
                events.push(MqttEvent::Connected(ConnectionResult {
                    reason_code: ack.reason_code,
                    session_present: ack.session_present,
                    properties: ack.properties.clone(),
                }));

                // Update receive_maximum from CONNACK properties if present
                if let Some(props) = &ack.properties {
                    for prop in props {
                        if let Property::ReceiveMaximum(max) = prop {
                            self.inflight_queue.update_receive_maximum(*max);
                            break;
                        }
                    }
                }

                // Handle session resumption: resend pending messages
                // Only when server confirms session resumption (session_present=true)
                if ack.session_present {
                    let pending = self.inflight_queue.get_all_for_reconnect();
                    for packet in pending {
                        if let Ok(bytes) = packet.to_bytes() {
                            self.outgoing_buffer.push_back(bytes);
                        }
                    }
                }
            }
            MqttPacket::ConnAck3(ack) => {
                self.is_connected = ack.return_code == 0;
                if self.is_connected {
                    self.reset_reconnect_state();
                }
                events.push(MqttEvent::Connected(ConnectionResult {
                    reason_code: ack.return_code,
                    session_present: ack.session_present,
                    properties: None,
                }));

                // Handle session resumption: resend pending messages
                // Only when server confirms session resumption (session_present=true)
                if ack.session_present {
                    let pending = self.inflight_queue.get_all_for_reconnect();
                    for packet in pending {
                        if let Ok(bytes) = packet.to_bytes() {
                            self.outgoing_buffer.push_back(bytes);
                        }
                    }
                }
            }
            MqttPacket::PubAck5(ack) => {
                if let Some(entry) = self.inflight_queue.acknowledge(ack.packet_id) {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(ack.packet_id),
                        reason_code: Some(ack.reason_code),
                        properties: Some(ack.properties),
                        qos: entry.qos,
                    }));
                }
            }
            MqttPacket::PubAck3(ack) => {
                if let Some(entry) = self.inflight_queue.acknowledge(ack.message_id) {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(ack.message_id),
                        reason_code: Some(0), // v3 has no reason code in PUBACK
                        properties: None,
                        qos: entry.qos,
                    }));
                }
            }
            MqttPacket::Publish5(p) => {
                let qos = p.qos;
                let pid = p.packet_id;
                events.push(MqttEvent::MessageReceived(p));

                if qos == 1 {
                    if let Some(pid) = pid {
                        let ack = MqttPacket::PubAck5(MqttPubAck::new(pid, 0, Vec::new()));
                        let _ = self.enqueue_packet(ack);
                    }
                } else if qos == 2 {
                    if let Some(pid) = pid {
                        let rec = MqttPacket::PubRec5(MqttPubRec::new(pid, 0, Vec::new()));
                        let _ = self.enqueue_packet(rec);
                    }
                }
            }
            MqttPacket::Publish3(p) => {
                let qos = p.qos;
                let pid = p.message_id;
                // Convert v3 publish to v5 for internal consumption
                let p5 = MqttPublish::new_with_prop(
                    qos,
                    p.topic_name.clone(),
                    pid,
                    p.payload.clone(),
                    p.retain,
                    p.dup,
                    Vec::new(),
                );
                events.push(MqttEvent::MessageReceived(p5));

                if qos == 1 {
                    if let Some(pid) = pid {
                        let ack = MqttPacket::PubAck3(
                            crate::mqtt_serde::mqttv3::puback::MqttPubAck::new(pid),
                        );
                        let _ = self.enqueue_packet(ack);
                    }
                } else if qos == 2 {
                    if let Some(pid) = pid {
                        let rec = MqttPacket::PubRec3(
                            crate::mqtt_serde::mqttv3::pubrec::MqttPubRec::new(pid),
                        );
                        let _ = self.enqueue_packet(rec);
                    }
                }
            }
            MqttPacket::SubAck5(ack) => {
                self.inflight_queue.acknowledge(ack.packet_id);
                events.push(MqttEvent::Subscribed(SubscribeResult {
                    packet_id: ack.packet_id,
                    reason_codes: ack.reason_codes,
                    properties: ack.properties,
                }));
            }
            MqttPacket::SubAck3(ack) => {
                self.inflight_queue.acknowledge(ack.message_id);
                events.push(MqttEvent::Subscribed(SubscribeResult {
                    packet_id: ack.message_id,
                    reason_codes: ack.return_codes,
                    properties: Vec::new(),
                }));
            }
            MqttPacket::UnsubAck5(ack) => {
                self.inflight_queue.acknowledge(ack.packet_id);
                events.push(MqttEvent::Unsubscribed(UnsubscribeResult {
                    packet_id: ack.packet_id,
                    reason_codes: ack.reason_codes,
                    properties: ack.properties,
                }));
            }
            MqttPacket::UnsubAck3(ack) => {
                self.inflight_queue.acknowledge(ack.message_id);
                events.push(MqttEvent::Unsubscribed(UnsubscribeResult {
                    packet_id: ack.message_id,
                    reason_codes: Vec::new(),
                    properties: Vec::new(),
                }));
            }
            MqttPacket::PingResp5(_) | MqttPacket::PingResp3(_) => {
                events.push(MqttEvent::PingResponse(PingResult { success: true }));
            }
            MqttPacket::PubRec5(rec) => {
                if let Some(mut entry) = self.inflight_queue.acknowledge(rec.packet_id) {
                    let rel = MqttPacket::PubRel5(MqttPubRel::new(rec.packet_id, 0, Vec::new()));
                    // Update entry to store PUBREL for retransmission
                    entry.packet = rel.clone();
                    entry.sent_at = Instant::now();
                    let _ = self.inflight_queue.push(entry.packet_id, entry.packet, 2);
                    let _ = self.enqueue_packet(rel);
                }
            }
            MqttPacket::PubRec3(rec) => {
                if let Some(mut entry) = self.inflight_queue.acknowledge(rec.message_id) {
                    let rel = MqttPacket::PubRel3(pubrelv3::MqttPubRel::new(rec.message_id));
                    // Update entry to store PUBREL for retransmission
                    entry.packet = rel.clone();
                    entry.sent_at = Instant::now();
                    let _ = self.inflight_queue.push(entry.packet_id, entry.packet, 2);
                    let _ = self.enqueue_packet(rel);
                }
            }
            MqttPacket::PubRel5(rel) => {
                let comp = MqttPacket::PubComp5(MqttPubComp::new(rel.packet_id, 0, Vec::new()));
                let _ = self.enqueue_packet(comp);
            }
            MqttPacket::PubRel3(rel) => {
                let comp = MqttPacket::PubComp3(
                    crate::mqtt_serde::mqttv3::pubcomp::MqttPubComp::new(rel.message_id),
                );
                let _ = self.enqueue_packet(comp);
            }
            MqttPacket::PubComp5(comp) => {
                if let Some(entry) = self.inflight_queue.acknowledge(comp.packet_id) {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(comp.packet_id),
                        reason_code: Some(comp.reason_code),
                        properties: Some(comp.properties),
                        qos: entry.qos,
                    }));
                }
            }
            MqttPacket::PubComp3(comp) => {
                if let Some(entry) = self.inflight_queue.acknowledge(comp.message_id) {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(comp.message_id),
                        reason_code: Some(0),
                        properties: None,
                        qos: entry.qos,
                    }));
                }
            }
            MqttPacket::Disconnect5(d) => {
                self.is_connected = false;
                events.push(MqttEvent::Disconnected(Some(d.reason_code)));
            }
            _ => {}
        }
        events
    }

    pub fn send_ping(&mut self) {
        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::PingReq5(pingreqv5::MqttPingReq::new())
        } else {
            MqttPacket::PingReq3(pingreqv3::MqttPingReq::new())
        };
        let _ = self.enqueue_packet(packet);
    }

    pub fn enqueue_packet(&mut self, packet: MqttPacket) -> Result<(), MqttClientError> {
        if self.outgoing_buffer.len() >= self.options.max_outgoing_packet_count {
            return Err(MqttClientError::BufferFull {
                buffer_type: "outgoing".to_string(),
                capacity: self.options.max_outgoing_packet_count,
            });
        }

        match packet.to_bytes() {
            Ok(bytes) => {
                self.outgoing_buffer.push_back(bytes);
                self.last_packet_sent = Instant::now();
                Ok(())
            }
            Err(e) => Err(MqttClientError::from(e)),
        }
    }

    fn process_queue(&mut self) {
        if !self.is_connected {
            return;
        }

        while self.outgoing_buffer.len() < self.options.max_outgoing_packet_count {
            if let Some((_priority, packet)) = self.priority_queue.peek() {
                let is_publish =
                    matches!(packet, MqttPacket::Publish5(_) | MqttPacket::Publish3(_));
                if is_publish && !self.inflight_queue.can_push_publish() {
                    break;
                }

                // Attempt to encode before dequeuing
                match packet.to_bytes() {
                    Ok(bytes) => {
                        // All checks pass, dequeue and track
                        if let Some((_, packet)) = self.priority_queue.dequeue() {
                            match &packet {
                                MqttPacket::Publish5(p) if p.qos > 0 => {
                                    let pid = p.packet_id.unwrap();
                                    let _ = self.inflight_queue.push(pid, packet.clone(), p.qos);
                                }
                                MqttPacket::Publish3(p) if p.qos > 0 => {
                                    let pid = p.message_id.unwrap();
                                    let _ = self.inflight_queue.push(pid, packet.clone(), p.qos);
                                }
                                _ => {}
                            }
                            self.outgoing_buffer.push_back(bytes);
                            self.last_packet_sent = Instant::now();
                        }
                    }
                    Err(e) => {
                        // Permanent failure (broken packet), dequeue and report error
                        if let Some((_, _)) = self.priority_queue.dequeue() {
                            self.events.push(MqttEvent::Error(MqttClientError::from(e)));
                        }
                    }
                }
            } else {
                break;
            }
        }
    }

    pub fn next_packet_id(&mut self) -> Result<u16, MqttClientError> {
        let session = self
            .session
            .as_mut()
            .ok_or(MqttClientError::ProtocolViolation {
                message: "No session available for packet ID allocation".into(),
            })?;
        Ok(session.next_packet_id())
    }
}

/// A "Sans-I/O" MQTT over QUIC protocol engine.
///
/// This engine combines the `MqttEngine` (MQTT state machine) with `quinn_proto` (QUIC state machine)
/// to provide a complete MQTT-over-QUIC implementation that does not perform any direct I/O.
#[cfg(feature = "quic")]
pub struct QuicMqttEngine {
    mqtt_engine: MqttEngine,
    endpoint: Endpoint,
    connection: Option<Connection>,
    connection_handle: Option<ConnectionHandle>,

    // The bidirectional stream used for MQTT control packets
    mqtt_stream: Option<StreamId>,

    // Outgoing UDP datagrams to be sent by the application
    outgoing_datagrams: VecDeque<(std::net::SocketAddr, Vec<u8>)>,
    // Internal buffer for reading from QUIC stream and feeding to MQTT engine
    // stream_read_buffer removed
}

#[cfg(feature = "quic")]
impl QuicMqttEngine {
    pub fn new(options: MqttClientOptions) -> Result<Self, MqttClientError> {
        // Initialize MqttEngine
        let mqtt_engine = MqttEngine::new(options);

        // Initialize QUIC Endpoint (Client)
        let endpoint_config = EndpointConfig::default();
        // Endpoint::new(config, server_config, disable_stateless_retry, reset_token_key)
        let endpoint = Endpoint::new(Arc::new(endpoint_config), None, true, None);

        Ok(Self {
            mqtt_engine,
            endpoint,
            connection: None,
            connection_handle: None,
            mqtt_stream: None,
            outgoing_datagrams: VecDeque::new(),
        })
    }

    pub fn connect(
        &mut self,
        server_addr: std::net::SocketAddr,
        server_name: &str,
        mut crypto_config: rustls::ClientConfig,
        now: Instant,
    ) -> Result<(), MqttClientError> {
        // Set default ALPN "mqtt" only if none are configured by the caller
        if crypto_config.alpn_protocols.is_empty() {
            crypto_config.alpn_protocols = vec![b"mqtt".to_vec()];
        }

        // Wrap in quinn config
        let mut client_config = ClientConfig::new(Arc::new(
            quinn_proto::crypto::rustls::QuicClientConfig::try_from(crypto_config).map_err(
                |e| MqttClientError::InternalError {
                    message: format!("Failed to create QUIC client config: {}", e),
                },
            )?,
        ));

        // Disable unreliable datagrams (buffer size 0 / None)
        let mut transport = quinn_proto::TransportConfig::default();
        transport.datagram_receive_buffer_size(None);
        client_config.transport_config(Arc::new(transport));

        let (ch, conn) = self
            .endpoint
            .connect(now, client_config, server_addr, server_name)
            .map_err(|e| MqttClientError::InternalError {
                message: format!("Failed to create QUIC connection: {}", e),
            })?;

        self.connection = Some(conn);
        self.connection_handle = Some(ch);

        Ok(())
    }

    /// Feed an incoming UDP datagram from the network.
    pub fn handle_datagram(
        &mut self,
        data: Vec<u8>,
        remote_addr: std::net::SocketAddr,
        now: Instant,
    ) {
        use bytes::BytesMut;
        use quinn_proto::DatagramEvent;

        // Feed to Endpoint
        let mut buf = Vec::new();

        // Convert the incoming datagram into BytesMut as required by quinn-proto::Endpoint::handle.
        let bytes = BytesMut::from(&data[..]);
        let result = self
            .endpoint
            .handle(now, remote_addr, None, None, bytes, &mut buf);

        // Handle immediate outgoing packet if buf is filled
        if !buf.is_empty() {
            self.outgoing_datagrams.push_back((remote_addr, buf));
        }

        // Process any resulting events
        if let Some(event) = result {
            match event {
                DatagramEvent::NewConnection(_incoming) => {
                    // As a client, we don't expect incoming connections usually.
                }
                DatagramEvent::ConnectionEvent(ch, event) => {
                    if Some(ch) == self.connection_handle {
                        if let Some(conn) = &mut self.connection {
                            conn.handle_event(event);
                        }
                    }
                }
                DatagramEvent::Response(_transmit) => {
                    // Metadata for sending packet. Content in buffer.
                }
            }
        }
    }

    /// Drive time-dependent logic for both QUIC and MQTT state machines.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        let mut mqtt_events = Vec::new();

        // 1. Drive QUIC Endpoint - handle_call removed in 0.11?
        // self.endpoint.handle_call(now);

        // 2. Drive QUIC Connection
        if let Some(conn) = &mut self.connection {
            conn.handle_timeout(now);

            // Check for new streams or data
            while let Some(event) = conn.poll() {
                match event {
                    quinn_proto::Event::Stream(_stream_id) => {
                        // Stream event (readable/writable etc)
                    }
                    quinn_proto::Event::Connected => {
                        // QUIC Handshake done. Open a bidirectional stream for MQTT.
                        if self.mqtt_stream.is_none() {
                            if let Some(stream_id) = conn.streams().open(quinn_proto::Dir::Bi) {
                                self.mqtt_stream = Some(stream_id);
                                self.mqtt_engine.connect();
                            }
                        }
                    }
                    quinn_proto::Event::ConnectionLost { .. } => {
                        self.mqtt_engine.handle_connection_lost();
                        mqtt_events.push(MqttEvent::Disconnected(None));
                    }
                    _ => {}
                }
            }

            // 3. Transfer data: QUIC Stream -> MqttEngine
            if let Some(stream_id) = self.mqtt_stream {
                let mut stream = conn.recv_stream(stream_id);
                // 0.11 read(ordered) -> Chunks
                // Fix lifetime by ensuring the Result temporary is dropped
                let read_result = stream.read(true);
                if let Ok(mut chunks) = read_result {
                    while let Ok(Some(chunk)) = chunks.next(16384) {
                        mqtt_events.extend(self.mqtt_engine.handle_incoming(&chunk.bytes));
                    }
                }
            }

            // 4. Transfer data: MqttEngine -> QUIC Stream
            let outgoing_bytes = self.mqtt_engine.take_outgoing();
            if !outgoing_bytes.is_empty() {
                if let Some(stream_id) = self.mqtt_stream {
                    let mut stream = conn.send_stream(stream_id);
                    // @TODO: handle error
                    let _ = stream.write(&outgoing_bytes);
                }
            }

            // 5. Collect outgoing UDP datagrams
            let mut buf = Vec::new();
            while let Some(transmit) = conn.poll_transmit(now, 1, &mut buf) {
                self.outgoing_datagrams
                    .push_back((transmit.destination, buf.clone()));
                // @TODO: Reuse the same buffer; poll_transmit writes the datagram payload into `buf` each time.
                buf.clear();
            }
        }

        // 6. Drive MqttEngine tick
        let tick_events = self.mqtt_engine.handle_tick(now);
        mqtt_events.extend(tick_events);

        mqtt_events
    }

    pub fn take_outgoing_datagrams(&mut self) -> VecDeque<(std::net::SocketAddr, Vec<u8>)> {
        std::mem::take(&mut self.outgoing_datagrams)
    }

    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        self.mqtt_engine.take_events()
    }

    /// Delegate: Queue a PUBLISH packet.
    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.mqtt_engine.publish(command)
    }

    /// Delegate: Queue a SUBSCRIBE packet.
    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.mqtt_engine.subscribe(command)
    }

    /// Delegate: Queue an UNSUBSCRIBE packet.
    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.mqtt_engine.unsubscribe(command)
    }

    /// Delegate: Queue a DISCONNECT packet.
    pub fn disconnect(&mut self) {
        self.mqtt_engine.disconnect();
    }

    /// Check if the MQTT session is connected.
    pub fn is_connected(&self) -> bool {
        self.mqtt_engine.is_connected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_client::opts::MqttClientOptions;
    use crate::mqtt_serde::mqttv5::pingreqv5;

    #[test]
    fn test_outgoing_buffer_limit() {
        let options = MqttClientOptions::builder()
            .max_outgoing_packet_count(2)
            .build();
        let mut engine = MqttEngine::new(options);

        // Manually enqueue packets (simulate CONNECT, etc.)
        let packet = MqttPacket::PingReq5(pingreqv5::MqttPingReq::new());

        // 1. Fill buffer
        assert!(engine.enqueue_packet(packet.clone()).is_ok());
        assert_eq!(engine.outgoing_buffer.len(), 1);

        assert!(engine.enqueue_packet(packet.clone()).is_ok());
        assert_eq!(engine.outgoing_buffer.len(), 2);

        // 2. Overfill - should fail
        let result = engine.enqueue_packet(packet.clone());
        assert!(result.is_err());
        match result {
            Err(MqttClientError::BufferFull {
                buffer_type,
                capacity,
            }) => {
                assert_eq!(buffer_type, "outgoing");
                assert_eq!(capacity, 2);
            }
            _ => panic!("Expected BufferFull error"),
        }

        // 3. Drain and retry
        let _ = engine.take_outgoing();
        assert_eq!(engine.outgoing_buffer.len(), 0);

        assert!(engine.enqueue_packet(packet).is_ok());
    }

    #[test]
    fn test_event_buffer_limit() {
        let options = MqttClientOptions::builder().max_event_count(1).build();
        let mut engine = MqttEngine::new(options);

        // Mock incoming data: 2 PINGRESP packets
        // PINGRESP (v5) is fixed header 0xD0, length 0x00.
        let data = vec![0xD0, 0x00, 0xD0, 0x00];

        // Feed data
        let events = engine.handle_incoming(&data);

        // Should only process 1 packet because limit is 1
        assert_eq!(events.len(), 1);
        match events[0] {
            MqttEvent::PingResponse(_) => {}
            _ => panic!("Expected PingResponse"),
        }

        // The second packet should remain in parser
        // Resume processing with empty data to flush/continue parsing
        let events2 = engine.handle_incoming(&[]);
        assert_eq!(events2.len(), 1);
        match events2[0] {
            MqttEvent::PingResponse(_) => {}
            _ => panic!("Expected second PingResponse"),
        }
    }
}
