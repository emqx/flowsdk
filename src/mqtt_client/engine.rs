// SPDX-License-Identifier: MPL-2.0

#[cfg(feature = "quic-proto")]
use quinn_proto::{
    ClientConfig, Connection, ConnectionError, ConnectionHandle, Dir, Endpoint, EndpointConfig,
    StreamId, VarInt,
};
#[cfg(feature = "quic-proto")]
use rustls::{
    client::{
        ClientSessionMemoryCache, ClientSessionStore, Resumption, Tls12ClientSessionValue,
        Tls13ClientSessionValue,
    },
    pki_types::ServerName,
    NamedGroup,
};
#[cfg(feature = "quic-proto")]
use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
#[cfg(feature = "quic-proto")]
use std::{
    fmt,
    sync::{Arc, Mutex},
};

#[cfg(feature = "strict-protocol-compliance")]
use crate::mqtt_serde::control_packet::ControlPacketType;
use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv3::{
    connectv3, disconnectv3, pingreqv3, pubrelv3, subscribev3, unsubscribev3,
};
use crate::mqtt_serde::mqttv5::{
    authv5, common::properties::Property, connectv5, disconnectv5, pingreqv5, pubackv5::MqttPubAck,
    pubcompv5::MqttPubComp, publishv5::MqttPublish, pubrecv5::MqttPubRec, pubrelv5::MqttPubRel,
    subscribev5, unsubscribev5,
};
use crate::mqtt_serde::parser::leveled::{ParseLevel, ParsedPacket, VariableHeader};
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

mod reliability;
use reliability::ProtocolState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum OperationKind {
    Connect,
    Publish,
    Subscribe,
    Unsubscribe,
}

/// Alias for `MqttPublish` (v5) to provide a single, unified type for received messages.
///
/// The engine normalizes all incoming PUBLISH packets (whether MQTT v3.1.1 or v5.0) into this structure.
/// This simplifies downstream consumption by providing a consistent API regardless of the protocol version used.
/// For MQTT v3.1.1 messages, the v5-specific fields (Properties) will be empty.
pub type MqttMessage = MqttPublish;

/// Configuration for QUIC 0-RTT/session-ticket support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuicZeroRttConfig {
    /// Maximum number of TLS session entries retained in memory.
    pub session_cache_size: usize,
    /// Replay exact early MQTT bytes as 1-RTT if the peer rejects 0-RTT.
    pub replay_on_reject: bool,
}

impl Default for QuicZeroRttConfig {
    fn default() -> Self {
        Self {
            session_cache_size: 256,
            replay_on_reject: true,
        }
    }
}

/// Current QUIC 0-RTT state for [`QuicMqttEngine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum QuicZeroRttStatus {
    /// 0-RTT is not enabled for this engine/connection.
    Disabled,
    /// 0-RTT was enabled but no resumable session ticket was available.
    Unavailable,
    /// 0-RTT keys were available and early data is being attempted.
    Attempted,
    /// The peer accepted early data.
    Accepted,
    /// The peer rejected early data.
    Rejected,
}

/// Events emitted by the MqttEngine to be handled by the application (I/O layer)
#[derive(Debug, serde::Serialize)]
pub enum MqttEvent {
    /// An operation failed or exceeded its application deadline. A CONNECT
    /// timeout ends the connection attempt. Other acknowledgement timeouts keep
    /// the MQTT exchange active, so a late acknowledgement may still complete it.
    OperationFailed {
        operation: OperationKind,
        packet_id: Option<u16>,
        error: MqttClientError,
    },
    Connected(ConnectionResult),
    AuthReceived(super::client::AuthResult),
    Disconnected(Option<u8>),
    /// A broker MQTT 5 DISCONNECT, including its properties.
    DisconnectReceived {
        reason_code: u8,
        properties: Vec<Property>,
    },
    Published(PublishResult),
    Subscribed(SubscribeResult),
    Unsubscribed(UnsubscribeResult),
    /// Incoming PUBLISH metadata.
    ///
    /// With full parsing this is emitted immediately before `MessageReceived`.
    /// Reduced parser levels emit only this metadata event. `stream` identifies
    /// the logical channel when the packet arrived on a QUIC data stream.
    PublishReceived {
        packet_id: Option<u16>,
        stream: Option<u64>,
    },
    MessageReceived(MqttMessage),
    /// Incoming QoS 2 PUBREL received.
    ///
    /// Exposed so manual-ack clients can wait for PUBREL before sending PUBCOMP.
    /// `stream` identifies the logical channel when the packet arrived on a QUIC
    /// data stream.
    PubRelReceived {
        packet_id: u16,
        stream: Option<u64>,
    },
    PingResponse(PingResult),
    Error(MqttClientError),
    /// The underlying QUIC transport connection was lost or closed, with details
    /// from quinn-proto (peer close, idle timeout, local close, reset, etc.).
    ///
    /// `by_peer` is true if the peer initiated the close; `error_code` carries the
    /// QUIC application/transport error code when one was signalled.
    TransportClosed {
        reason: String,
        by_peer: bool,
        error_code: Option<u64>,
    },
    /// A QUIC stream was closed gracefully.
    ///
    /// `by_peer` is true when the peer finished its send side. It is false when
    /// our finished send side was acknowledged by the peer.
    StreamClosed {
        stream_id: u64,
        reason: String,
        by_peer: bool,
    },
    /// The peer aborted its send side with RESET_STREAM.
    StreamReset {
        stream_id: u64,
        error_code: u64,
    },
    /// The peer aborted our send side with STOP_SENDING.
    StreamStopped {
        stream_id: u64,
        error_code: u64,
    },
    /// Signal that a reconnection is needed (e.g. after keep-alive timeout)
    ReconnectNeeded,
    /// Reconnection scheduled with exponential backoff
    ReconnectScheduled {
        attempt: u32,
        delay: Duration,
    },
    /// QUIC 0-RTT status changed for the Sans-I/O QUIC engine.
    ZeroRttStatusChanged {
        status: QuicZeroRttStatus,
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
    reliability: ProtocolState,
    options: MqttClientOptions,
    session: Option<ClientSession>,
    priority_queue: PriorityQueue<u8, MqttPacket>,
    is_connected: bool,
    last_packet_sent: Instant,
    last_packet_received: Instant,
    /// Timestamp of the outstanding PINGREQ, if any. A connection timeout is
    /// only valid after this request has not received a PINGRESP in time.
    ping_sent_at: Option<Instant>,

    // Buffers and Parsers
    parser: MqttParser,
    outgoing_buffer: VecDeque<Vec<u8>>,

    // Retransmissions (MQTT v3) for packets originally sent on a specific logical
    // channel (e.g. a QUIC data stream). Drained by the transport via
    // `take_stream_retransmissions` and re-sent on that same channel so the QoS
    // 1/2 handshake never crosses streams. Untagged retransmissions still use
    // `outgoing_buffer`.
    stream_retransmissions: VecDeque<(u64, Vec<u8>)>,

    // Pending operations tracking (state only)
    inflight_queue: InflightQueue,
    previous_inflight: Vec<(u16, MqttPacket)>,
    transport_manages_session: bool,
    session_replay: VecDeque<(u16, MqttPacket)>,
    configured_subscriptions_pending: bool,

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
        let parser = MqttParser::new(options.parser_buffer_size, mqtt_version);

        // Cache timeout values for efficiency
        let retransmission_timeout = Duration::from_millis(options.retransmission_timeout_ms);
        let reconnect_base_delay = Duration::from_millis(options.reconnect_base_delay_ms);
        let reconnect_max_delay = Duration::from_millis(options.reconnect_max_delay_ms);
        let max_reconnect_attempts = options.max_reconnect_attempts;

        Self {
            reliability: ProtocolState::new(options.keep_alive),
            inflight_queue: InflightQueue::new(
                options.receive_maximum,
                options.mqtt_version,
                retransmission_timeout,
            ),
            session: None,
            previous_inflight: Vec::new(),
            transport_manages_session: false,
            session_replay: VecDeque::new(),
            configured_subscriptions_pending: false,
            priority_queue: PriorityQueue::new(options.max_outgoing_packet_count),
            is_connected: false,
            last_packet_sent: Instant::now(),
            last_packet_received: Instant::now(),
            ping_sent_at: None,
            parser,
            outgoing_buffer: VecDeque::new(),
            stream_retransmissions: VecDeque::new(),
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

    /// Select how deeply subsequent incoming packets are parsed.
    ///
    /// Reduced levels are intended for packet-counting workloads. Callers must
    /// complete connection setup at [`ParseLevel::Full`] before switching.
    pub fn set_parse_level(&mut self, level: ParseLevel) {
        self.parser.set_parse_level(level);
    }

    /// Change parser depth only when connection and outstanding operations permit it.
    pub fn try_set_parse_level(&mut self, level: ParseLevel) -> Result<(), MqttClientError> {
        if level != ParseLevel::Full
            && (!self.is_connected
                || !self.inflight_queue.is_empty()
                || !self.priority_queue.is_empty())
        {
            return Err(MqttClientError::InvalidState {
                expected: "connected with no outstanding operations".into(),
                actual: "connecting or waiting for acknowledgements".into(),
            });
        }
        self.set_parse_level(level);
        Ok(())
    }

    fn require_full_parser(&self) -> Result<(), MqttClientError> {
        if self.parse_level() != ParseLevel::Full {
            return Err(MqttClientError::InvalidState {
                expected: "Full parser for acknowledgement-bearing operations".into(),
                actual: format!("{:?}", self.parse_level()),
            });
        }
        Ok(())
    }

    /// Return the parser depth used for subsequent incoming packets.
    pub fn parse_level(&self) -> ParseLevel {
        self.parser.parse_level()
    }

    pub fn options(&self) -> &MqttClientOptions {
        &self.options
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }

    pub fn handle_connection_lost(&mut self) {
        self.connection_lost_at(Instant::now());
    }

    fn connection_lost_at(&mut self, now: Instant) {
        let scheduled = self.next_reconnect_at;
        self.reset_for_new_transport();
        self.next_reconnect_at = scheduled;
        if self.options.reconnect {
            self.schedule_reconnect(now);
        }
    }

    /// Reset all transport-tied state for a fresh underlying transport (e.g. a
    /// QUIC reconnect), discarding any bytes queued for the *old* connection so
    /// they cannot be replayed before the new CONNECT.
    ///
    /// Clears the outgoing byte buffer, pending per-stream retransmissions, and
    /// the inbound parser (any partial packet from the old transport). The MQTT
    /// session and the inflight queue are **retained** so QoS 1/2 messages can be
    /// resumed if the server reports `session_present`.
    pub fn reset_for_new_transport(&mut self) {
        self.close_input();
        self.outgoing_buffer.clear();
        self.configured_subscriptions_pending = false;
        self.stream_retransmissions.clear();
        self.inflight_queue
            .update_receive_maximum(self.options.receive_maximum);
        self.reliability.incoming_aliases.clear();
        self.reliability.outgoing_aliases.clear();
        // Queued application messages retain canonical topics, never old aliases.
        self.reliability.queued_bytes = 0;
        for packet in self.priority_queue.iter_mut() {
            if let MqttPacket::Publish5(p) = packet {
                p.properties
                    .retain(|property| !matches!(property, Property::TopicAlias(_)));
            }
            self.reliability.queued_bytes +=
                packet.to_bytes().expect("validated queued packet").len();
        }
        self.reliability.outgoing_alias_maximum = 0;
        self.reliability.maximum_packet_size = None;
        self.reliability.maximum_qos = 2;
        self.reliability.retain_available = true;
        self.reliability.wildcard_available = true;
        self.reliability.shared_available = true;
        self.reliability.subscription_ids_available = true;
        self.reliability.keep_alive = self.options.keep_alive;
        self.reliability
            .received
            .retain(|_, (stage, _)| !matches!(stage, reliability::ReceiveStage::Publish(1)));
        for (_, stream) in self.reliability.received.values_mut() {
            *stream = None;
        }
        if self.options.sessionless {
            self.reliability.has_session = false;
            let outstanding = self.inflight_queue.snapshot_for_reconnect();
            for (id, packet) in outstanding {
                self.fail_operation(id, &packet, MqttClientError::SessionExpired);
            }
            let replay = std::mem::take(&mut self.session_replay);
            for (id, packet) in replay {
                self.fail_operation(id, &packet, MqttClientError::SessionExpired);
            }
            self.reliability.received.clear();
        }
        self.parser = MqttParser::new(self.options.parser_buffer_size, self.options.mqtt_version);
        // Cancel any pending reconnect deadline so it cannot fire another
        // ReconnectNeeded while a new transport handshake is in progress. The
        // attempt counter is left intact until CONNACK resets it.
        self.next_reconnect_at = None;
        let now = Instant::now();
        self.last_packet_sent = now;
        self.last_packet_received = now;
        self.ping_sent_at = None;
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
        if self.next_reconnect_at.is_some() {
            return;
        }
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
        if self.reliability.input_closed {
            return self.take_events();
        }
        if let Some(capacity) = self.options.max_incoming_buffer_bytes {
            if self.parser.buffer_mut().len().saturating_add(data.len()) > capacity {
                self.fail_connection(MqttClientError::BufferFull {
                    buffer_type: "incoming bytes".into(),
                    capacity,
                });
                return self.take_events();
            }
        }
        self.parser.feed(data);
        self.flush_responses();

        loop {
            if !self.reliability.responses.is_empty() || self.reliability.input_closed {
                break;
            }
            if let Err(error) = Self::validate_input_buffer(&self.options, self.parser.buffer_mut())
            {
                self.fail_connection(error);
                break;
            }
            if self.events.len() >= self.options.max_event_count {
                // Buffer full, stop processing for now.
                // Remaining data stays in parser/buffer.
                break;
            }

            match self.parser.next_parsed() {
                Ok(Some(ParsedPacket::Full(packet))) => {
                    self.last_packet_received = Instant::now();
                    let (packet_events, responses) = self.handle_packet(packet, None);
                    self.events.extend(packet_events);
                    // Single-stream transports (TCP/TLS, QUIC control stream) send
                    // responses back over the same shared outgoing buffer.
                    self.reliability.responses.extend(responses);
                    self.flush_responses();
                }
                Ok(Some(packet)) => {
                    self.last_packet_received = Instant::now();
                    let (event, response) = self.handle_reduced_packet(packet, None);
                    if let Some(event) = event {
                        self.events.push(event);
                    }
                    if let Some(response) = response {
                        self.reliability.responses.push_back(response);
                        self.flush_responses();
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    self.fail_connection(MqttClientError::from(e));
                    break;
                }
            }
        }
        self.process_queue();
        self.take_events()
    }

    fn handle_reduced_packet(
        &mut self,
        packet: ParsedPacket,
        stream: Option<u64>,
    ) -> (Option<MqttEvent>, Option<MqttPacket>) {
        if !self.accept_incoming(packet.packet_type()) {
            return (None, None);
        }
        let mut event = None;
        let mut response = None;

        match packet {
            ParsedPacket::TypeOnly(packet) => match packet.packet_type {
                crate::mqtt_serde::control_packet::ControlPacketType::PUBLISH => {
                    let qos = (packet.flags >> 1) & 0x03;
                    if qos == 0 {
                        event = Some(MqttEvent::PublishReceived {
                            packet_id: None,
                            stream,
                        });
                    } else {
                        event = Some(MqttEvent::Error(MqttClientError::ProtocolViolation {
                            message: format!(
                                "type-only parser cannot acknowledge incoming QoS {} PUBLISH",
                                qos
                            ),
                        }));
                    }
                }
                crate::mqtt_serde::control_packet::ControlPacketType::PINGRESP => {
                    self.ping_sent_at = None;
                    event = Some(MqttEvent::PingResponse(PingResult { success: true }));
                }
                crate::mqtt_serde::control_packet::ControlPacketType::DISCONNECT => {
                    self.handle_connection_lost();
                    event = Some(MqttEvent::Disconnected(None));
                }
                _ => {}
            },
            ParsedPacket::HeadersParsed(packet) => match packet.variable_header {
                VariableHeader::PublishV5 { qos, packet_id, .. } => {
                    event = Some(MqttEvent::PublishReceived { packet_id, stream });
                    if self.options.auto_ack && qos == 1 {
                        if let Some(packet_id) = packet_id {
                            response = Some(MqttPacket::PubAck5(MqttPubAck::new(
                                packet_id,
                                0,
                                Vec::new(),
                            )));
                        }
                    } else if self.options.auto_ack && qos == 2 {
                        if let Some(packet_id) = packet_id {
                            response = Some(MqttPacket::PubRec5(MqttPubRec::new(
                                packet_id,
                                0,
                                Vec::new(),
                            )));
                        }
                    }
                }
                VariableHeader::PublishV3 {
                    qos, message_id, ..
                } => {
                    event = Some(MqttEvent::PublishReceived {
                        packet_id: message_id,
                        stream,
                    });
                    if self.options.auto_ack && qos == 1 {
                        if let Some(message_id) = message_id {
                            response = Some(MqttPacket::PubAck3(
                                crate::mqtt_serde::mqttv3::puback::MqttPubAck::new(message_id),
                            ));
                        }
                    } else if self.options.auto_ack && qos == 2 {
                        if let Some(message_id) = message_id {
                            response = Some(MqttPacket::PubRec3(
                                crate::mqtt_serde::mqttv3::pubrec::MqttPubRec::new(message_id),
                            ));
                        }
                    }
                }
                VariableHeader::PubRelV5 { packet_id, .. } => {
                    event = Some(MqttEvent::PubRelReceived { packet_id, stream });
                    if self.options.auto_ack {
                        response = Some(MqttPacket::PubComp5(MqttPubComp::new(
                            packet_id,
                            0,
                            Vec::new(),
                        )));
                    }
                }
                VariableHeader::PacketIdOnlyV3 { message_id }
                    if packet.packet_type
                        == crate::mqtt_serde::control_packet::ControlPacketType::PUBREL =>
                {
                    event = Some(MqttEvent::PubRelReceived {
                        packet_id: message_id,
                        stream,
                    });
                    if self.options.auto_ack {
                        response = Some(MqttPacket::PubComp3(
                            crate::mqtt_serde::mqttv3::pubcomp::MqttPubComp::new(message_id),
                        ));
                    }
                }
                VariableHeader::Empty
                    if packet.packet_type
                        == crate::mqtt_serde::control_packet::ControlPacketType::PINGRESP =>
                {
                    self.ping_sent_at = None;
                    event = Some(MqttEvent::PingResponse(PingResult { success: true }));
                }
                VariableHeader::AuthV5 {
                    reason_code,
                    properties,
                } => {
                    event = self.auth_received(reason_code, properties);
                }
                VariableHeader::DisconnectV5 {
                    reason_code,
                    properties,
                } => {
                    self.handle_connection_lost();
                    event = Some(MqttEvent::DisconnectReceived {
                        reason_code,
                        properties,
                    });
                }
                VariableHeader::Empty
                    if packet.packet_type
                        == crate::mqtt_serde::control_packet::ControlPacketType::DISCONNECT =>
                {
                    self.handle_connection_lost();
                    event = Some(MqttEvent::Disconnected(None));
                }
                _ => {}
            },
            ParsedPacket::RawBody(_) => {
                event = Some(MqttEvent::Error(MqttClientError::InvalidState {
                    expected: "Full, HeadersParsed, or TypeOnly parser level".to_string(),
                    actual: "RawBody parser level".to_string(),
                }));
            }
            ParsedPacket::Full(_) => unreachable!("full packets use handle_packet"),
        }

        (event, response)
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
    /// 3. **Timeout Detection**: Detects dead connections when a PINGREQ is unanswered for Keep-Alive * multiplier, then schedules reconnect.
    /// 4. **Retransmissions** (MQTT v3.1.1): Resends unacknowledged QoS 1/2 packets.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        self.check_deadlines(now);
        self.flush_responses();
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

        let keep_alive = Duration::from_secs(self.reliability.keep_alive as u64);

        // 1. A PINGRESP is only required after we actually send a PINGREQ. In
        // particular, QoS 0 publishing may be continuously outbound and have
        // no broker responses at all, so `last_packet_received` alone cannot
        // establish that the connection has timed out.
        let ping_timeout = keep_alive * self.options.ping_timeout_multiplier;
        if keep_alive > Duration::ZERO
            && self
                .ping_sent_at
                .is_some_and(|sent_at| now.duration_since(sent_at) >= ping_timeout)
        {
            self.connection_lost_at(now);
            self.events.push(MqttEvent::Disconnected(None));
            return self.take_events();
        }

        // 2. Keep-alive: Send PING only while otherwise idle, and do not send
        // another one until the outstanding PINGREQ is answered or times out.
        if self.ping_sent_at.is_none()
            && self.options.auto_keepalive
            && keep_alive > Duration::ZERO
            && now.duration_since(self.last_packet_sent) >= keep_alive
        {
            let _ = self.send_ping_at(now);
        }

        self.process_queue();

        // 3. Retransmissions
        let retrans_events = self.handle_retransmissions(now);
        self.events.extend(retrans_events);

        self.take_events()
    }

    fn handle_retransmissions(&mut self, now: Instant) -> Vec<MqttEvent> {
        let events = Vec::new();

        let expired = self.inflight_queue.get_expired_with_stream(now);
        for (mut packet, stream) in expired {
            packet.set_dup(true);
            if let Ok(bytes) = packet.to_bytes() {
                match stream {
                    // Retransmit on the originating channel (e.g. QUIC data stream)
                    // to preserve the same-stream QoS handshake.
                    Some(stream_id) => self.stream_retransmissions.push_back((stream_id, bytes)),
                    None => {
                        if let Err(error) = self.enqueue_packet(packet) {
                            if !matches!(error, MqttClientError::BufferFull { .. }) {
                                self.events.push(MqttEvent::Error(error));
                            }
                        }
                    }
                }
                self.last_packet_sent = now;
            }
        }

        events
    }

    /// Drain retransmissions that must be re-sent on a specific logical channel.
    ///
    /// Each entry is `(stream_handle, encoded_bytes)`. The transport routes each
    /// onto the named channel. Empty for single-stream transports.
    pub fn take_stream_retransmissions(&mut self) -> VecDeque<(u64, Vec<u8>)> {
        std::mem::take(&mut self.stream_retransmissions)
    }

    /// Returns the exact timestamp of the next required wake-up.
    ///
    /// The runtime loop should sleep until this timestamp to avoid busy-waiting.
    ///
    /// Returns `None` if there are no scheduled timer events (sleep indefinitely or until IO).
    ///
    /// Prioritizes:
    /// 1. Reconnection attempts (if disconnected).
    /// 2. Keep-alive PINGs and their response timeout.
    /// 4. Packet retransmissions.
    pub fn next_tick_at(&self) -> Option<Instant> {
        // 1. Reconnection timer (highest priority when disconnected)
        if !self.is_connected {
            return self
                .next_reconnect_at
                .into_iter()
                .chain(self.reliability.deadlines.values().map(|(at, _)| *at))
                .min();
        }

        let mut next = None;
        let keep_alive = Duration::from_secs(self.reliability.keep_alive as u64);

        // 2. A PINGREQ response timeout takes precedence over another PINGREQ.
        if keep_alive > Duration::ZERO {
            if let Some(ping_sent_at) = self.ping_sent_at {
                let timeout_deadline =
                    ping_sent_at + keep_alive * self.options.ping_timeout_multiplier;
                next = Some(timeout_deadline);
            } else if self.options.auto_keepalive {
                let ping_deadline = self.last_packet_sent + keep_alive;
                next = Some(ping_deadline);
            }
        }

        // 4. Retransmission timeouts (QoS 1/2 messages)
        // Only for MQTT v3.1.1, as v5.0 forbids client-side retransmission
        if let Some(resend_at) = self.inflight_queue.next_expiration() {
            if next.is_none() || resend_at < next.unwrap() {
                next = Some(resend_at);
            }
        }

        next.into_iter()
            .chain(self.reliability.deadlines.values().map(|(at, _)| *at))
            .min()
    }

    pub fn has_pending_output(&self) -> bool {
        !self.outgoing_buffer.is_empty() || !self.reliability.responses.is_empty()
    }

    /// Take bytes ready to be sent to the network.
    ///
    /// This should be written to the underlying transport immediately.
    /// Clears the internal outgoing buffer.
    pub fn take_outgoing(&mut self) -> Vec<u8> {
        let mut all_bytes = Vec::new();
        loop {
            while let Some(packet) = self.outgoing_buffer.pop_front() {
                all_bytes.extend(packet);
            }
            self.flush_responses();
            if !self.reliability.input_closed {
                let events = self.handle_incoming(&[]);
                self.events.extend(events);
            }
            self.process_queue();
            if self.outgoing_buffer.is_empty() {
                break;
            }
        }
        all_bytes
    }

    // --- Command Methods ---

    /// Initiate the MQTT connection handshake (send CONNECT packet).
    ///
    /// Should be called after the physical connection is established.
    pub fn connect(&mut self) -> Result<(), MqttClientError> {
        if self.is_connected {
            return Err(MqttClientError::AlreadyConnected);
        }
        if self.reliability.connecting {
            return Err(MqttClientError::InvalidState {
                expected: "disconnected".into(),
                actual: "awaiting CONNACK".into(),
            });
        }
        if self.options.max_outgoing_packet_count == 0 || self.options.max_event_count == 0 {
            return Err(MqttClientError::InvalidConfiguration {
                field: "buffer limits".into(),
                reason: "Packet and event limits must be nonzero".into(),
            });
        }
        let properties = if self.options.mqtt_version == 5 {
            self.connect_properties()?
        } else {
            Vec::new()
        };

        // Initialize session if needed
        if self.session.is_none() {
            self.session = Some(ClientSession::new());
        }

        let packet = if self.options.mqtt_version == 5 {
            let connect = connectv5::MqttConnect::new(
                self.options.client_id.clone(),
                self.options.username.clone(),
                self.options.password.clone(),
                self.options.will.clone(),
                self.options.keep_alive,
                self.options.clean_start,
                properties,
            );
            MqttPacket::Connect5(connect)
        } else {
            let mut connect = connectv3::MqttConnect::new(
                self.options.client_id.clone(),
                self.options.keep_alive,
                self.options.clean_start,
            );
            connect.username = self.options.username.clone();
            connect.password = self.options.password.clone();
            connect.will = self.options.will.as_ref().map(|will| connectv3::Will {
                retain: will.will_retain,
                qos: will.will_qos,
                topic: will.will_topic.clone(),
                message: will.will_message.clone(),
            });
            MqttPacket::Connect3(connect)
        };

        let reserve = packet
            .to_bytes()
            .map_err(MqttClientError::from)?
            .len()
            .max(6);
        self.enqueue_packet(packet)?;
        self.previous_inflight = self.inflight_queue.snapshot_for_reconnect();
        self.previous_inflight.extend(self.session_replay.drain(..));
        self.reliability.control_reserve = reserve;
        self.reliability.input_closed = false;
        self.reliability.connecting = true;
        self.reliability.reauthenticating = false;
        self.next_reconnect_at = None;
        self.start_deadline(OperationKind::Connect, None);
        Ok(())
    }

    /// Queue a PUBLISH packet.
    ///
    /// - **QoS 0**: ID is usually None (unless needed for tracing).
    /// - **QoS 1/2**: Returns the assigned Packet ID (or uses the one provided).
    ///
    /// The command is pushed to the `PriorityQueue` and only moved to the `outgoing_buffer`
    /// via `process_queue()` if the buffer limits allow.
    pub fn publish(&mut self, mut command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        if command.qos > 0 {
            self.require_full_parser()?;
        }
        let alias = self.validate_publish(&mut command)?;
        if self.priority_queue.len() >= self.options.max_outgoing_packet_count {
            return Err(MqttClientError::BufferFull {
                buffer_type: "publish queue".into(),
                capacity: self.options.max_outgoing_packet_count,
            });
        }
        let pid = if command.qos > 0 {
            let pid = self.candidate_id(command.packet_id)?;
            command.packet_id = Some(pid);
            Some(pid)
        } else {
            None
        };

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Publish5(command.to_mqtt_publish())
        } else {
            MqttPacket::Publish3(command.to_mqttv3_publish())
        };

        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.check_packet_size(&bytes)?;
        // Queued publishes must not consume the space needed for acknowledgements,
        // the next CONNECT, or replaying an outstanding publication after reconnect.
        let reserve = self
            .reliability
            .control_reserve
            .max(self.reliability.replay_reserve);
        self.check_output_capacity(bytes.len().saturating_add(reserve))?;
        self.reliability.queued_bytes += bytes.len();
        if let Some(id) = pid {
            self.reliability.reserved.insert(id);
            self.reliability.replay_reserve = self.reliability.replay_reserve.max(bytes.len());
        }
        if let Some((id, topic)) = alias {
            self.reliability.outgoing_aliases.insert(id, topic);
        }

        self.priority_queue.enqueue(command.priority, packet);
        self.process_queue();
        Ok(pid)
    }

    /// Encode a PUBLISH packet immediately and return its bytes, bypassing the
    /// internal priority queue / outgoing buffer.
    ///
    /// This is used by transports that need to route individual packets to a
    /// specific destination (e.g. a dedicated QUIC stream for MQTT-over-QUIC
    /// multi-stream support). Protocol state shared across streams — packet-id
    /// allocation and the inflight queue for QoS 1/2 — is still managed here, so
    /// acknowledgements continue to flow through the normal engine machinery.
    ///
    /// Returns the assigned packet id (for QoS > 0) together with the encoded
    /// bytes. The caller is responsible for actually transmitting the bytes.
    ///
    /// `stream` records the logical channel the packet is sent on so that QoS 1/2
    /// retransmissions are routed back onto the same channel (see
    /// [`take_stream_retransmissions`](Self::take_stream_retransmissions)).
    pub fn publish_encoded(
        &mut self,
        mut command: PublishCommand,
        stream: Option<u64>,
    ) -> Result<(Option<u16>, Vec<u8>), MqttClientError> {
        #[cfg(feature = "strict-protocol-compliance")]
        self.ensure_outgoing_allowed(ControlPacketType::PUBLISH)?;
        if command.qos > 0 {
            self.require_full_parser()?;
        }
        let alias = self.validate_publish(&mut command)?;
        let pid = if command.qos > 0 {
            let pid = self.candidate_id(command.packet_id)?;
            command.packet_id = Some(pid);
            Some(pid)
        } else {
            None
        };

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Publish5(command.to_mqtt_publish())
        } else {
            MqttPacket::Publish3(command.to_mqttv3_publish())
        };

        // Encode before registering inflight so a serialization failure leaves
        // no dangling inflight entry.
        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.check_packet_size(&bytes)?;

        // QoS > 0 messages must be tracked for retransmission/acknowledgement.
        // `push_with_stream` enforces the receive-maximum limit and errors if exceeded.
        if command.qos > 0 {
            self.inflight_queue.push_with_stream(
                pid.expect("QoS > 0 always assigns a packet id"),
                packet,
                command.qos,
                stream,
            )?;
        }

        if let Some(id) = pid {
            self.reliability.reserved.insert(id);
            self.reliability.replay_reserve = self.reliability.replay_reserve.max(bytes.len());
            self.start_deadline(OperationKind::Publish, Some(id));
        }
        if let Some((id, topic)) = alias {
            self.reliability.outgoing_aliases.insert(id, topic);
        }
        self.last_packet_sent = Instant::now();
        Ok((pid, bytes))
    }

    /// Process a single already-parsed MQTT packet that arrived on a specific
    /// logical channel (e.g. one QUIC stream).
    ///
    /// This mirrors the per-packet handling done inside [`handle_incoming`], but
    /// takes a decoded [`MqttPacket`] and, instead of enqueueing any protocol
    /// responses onto the shared outgoing buffer, **returns the encoded response
    /// bytes** so the caller can write them back on the *same* channel the packet
    /// arrived on. This is what guarantees no cross-stream firing for MQTT over
    /// QUIC: a PUBACK/PUBREC/PUBREL/PUBCOMP is always sent on the stream that
    /// carried the packet it answers.
    ///
    /// `stream` identifies the channel the packet arrived on, used to tag any
    /// inflight entry (QoS 2 PUBREL) created while handling it.
    ///
    /// Returns `(events, response_bytes)`.
    pub fn ingest_stream_packet(
        &mut self,
        packet: MqttPacket,
        stream: u64,
    ) -> (Vec<MqttEvent>, Vec<u8>) {
        let mut events = Vec::new();
        let response =
            self.ingest_parsed_stream_packet_into(ParsedPacket::Full(packet), stream, &mut events);
        (events, response)
    }

    fn ingest_parsed_stream_packet_into(
        &mut self,
        packet: ParsedPacket,
        stream: u64,
        events: &mut Vec<MqttEvent>,
    ) -> Vec<u8> {
        self.last_packet_received = Instant::now();
        events.extend(self.take_events());
        let mut response_bytes = Vec::new();
        match packet {
            ParsedPacket::Full(packet) => {
                let (packet_events, responses) = self.handle_packet(packet, Some(stream));
                events.extend(packet_events);
                for response in responses {
                    match response.to_bytes() {
                        Ok(bytes) => response_bytes.extend(bytes),
                        Err(e) => events.push(MqttEvent::Error(MqttClientError::from(e))),
                    }
                }
            }
            packet => {
                let (event, response) = self.handle_reduced_packet(packet, Some(stream));
                if let Some(event) = event {
                    events.push(event);
                }
                if let Some(response) = response {
                    match response.to_bytes() {
                        Ok(bytes) => response_bytes.extend(bytes),
                        Err(e) => events.push(MqttEvent::Error(MqttClientError::from(e))),
                    }
                }
            }
        }
        if !response_bytes.is_empty() {
            self.last_packet_sent = Instant::now();
        }

        // Acknowledgements may have freed inflight capacity; flush any control
        // packets waiting in the priority queue (these target the control path).
        self.process_queue();
        events.extend(self.take_events());

        response_bytes
    }

    /// Queue a SUBSCRIBE packet.
    ///
    /// Be aware that this might fail immediately with `MqttClientError::BufferFull`
    /// if the outgoing buffer is at capacity.
    pub fn subscribe(&mut self, mut command: SubscribeCommand) -> Result<u16, MqttClientError> {
        #[cfg(feature = "strict-protocol-compliance")]
        self.ensure_outgoing_allowed(ControlPacketType::SUBSCRIBE)?;
        self.require_full_parser()?;
        self.validate_subscribe(&command)?;
        let pid = self.candidate_id(command.packet_id)?;
        command.packet_id = Some(pid);

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

        self.enqueue_packet(packet.clone())?;
        self.inflight_queue.push(pid, packet, 1)?;
        self.reliability.reserved.insert(pid);
        self.start_deadline(OperationKind::Subscribe, Some(pid));
        Ok(pid)
    }

    pub fn unsubscribe(&mut self, mut command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        #[cfg(feature = "strict-protocol-compliance")]
        self.ensure_outgoing_allowed(ControlPacketType::UNSUBSCRIBE)?;
        self.require_full_parser()?;
        let pid = self.candidate_id(command.packet_id)?;
        command.packet_id = Some(pid);

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Unsubscribe5(unsubscribev5::MqttUnsubscribe::new(
                pid,
                command.topics.clone(),
                command.properties,
            ))
        } else {
            MqttPacket::Unsubscribe3(unsubscribev3::MqttUnsubscribe::new(pid, command.topics))
        };

        self.enqueue_packet(packet.clone())?;
        self.inflight_queue.push(pid, packet, 1)?;
        self.reliability.reserved.insert(pid);
        self.start_deadline(OperationKind::Unsubscribe, Some(pid));
        Ok(pid)
    }

    /// Encode a SUBSCRIBE packet immediately and return its bytes, bypassing the
    /// shared outgoing buffer.
    ///
    /// Like [`publish_encoded`](Self::publish_encoded), this is used by the QUIC
    /// transport to route a SUBSCRIBE onto a dedicated data stream so the SUBACK
    /// (and subsequently delivered messages) flow on that same stream. Packet-id
    /// allocation and inflight tracking remain centralized.
    pub fn subscribe_encoded(
        &mut self,
        mut command: SubscribeCommand,
        stream: Option<u64>,
    ) -> Result<(u16, Vec<u8>), MqttClientError> {
        #[cfg(feature = "strict-protocol-compliance")]
        self.ensure_outgoing_allowed(ControlPacketType::SUBSCRIBE)?;
        self.require_full_parser()?;
        self.validate_subscribe(&command)?;
        let pid = self.candidate_id(command.packet_id)?;
        command.packet_id = Some(pid);

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

        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.check_packet_size(&bytes)?;
        self.inflight_queue
            .push_with_stream(pid, packet, 1, stream)?;
        self.reliability.reserved.insert(pid);
        self.start_deadline(OperationKind::Subscribe, Some(pid));
        self.last_packet_sent = Instant::now();
        Ok((pid, bytes))
    }

    /// Encode an UNSUBSCRIBE packet immediately and return its bytes, bypassing
    /// the shared outgoing buffer. See [`subscribe_encoded`](Self::subscribe_encoded).
    pub fn unsubscribe_encoded(
        &mut self,
        mut command: UnsubscribeCommand,
        stream: Option<u64>,
    ) -> Result<(u16, Vec<u8>), MqttClientError> {
        #[cfg(feature = "strict-protocol-compliance")]
        self.ensure_outgoing_allowed(ControlPacketType::UNSUBSCRIBE)?;
        self.require_full_parser()?;
        let pid = self.candidate_id(command.packet_id)?;
        command.packet_id = Some(pid);

        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Unsubscribe5(unsubscribev5::MqttUnsubscribe::new(
                pid,
                command.topics.clone(),
                command.properties,
            ))
        } else {
            MqttPacket::Unsubscribe3(unsubscribev3::MqttUnsubscribe::new(pid, command.topics))
        };

        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.check_packet_size(&bytes)?;
        self.inflight_queue
            .push_with_stream(pid, packet, 1, stream)?;
        self.reliability.reserved.insert(pid);
        self.start_deadline(OperationKind::Unsubscribe, Some(pid));
        self.last_packet_sent = Instant::now();
        Ok((pid, bytes))
    }

    fn disconnect_packet(&self) -> MqttPacket {
        if self.options.mqtt_version == 5 {
            MqttPacket::Disconnect5(disconnectv5::MqttDisconnect::new(0, Vec::new()))
        } else {
            MqttPacket::Disconnect3(disconnectv3::MqttDisconnect::new())
        }
    }

    /// Queue a DISCONNECT packet and mark the session disconnected on success.
    /// Queueing errors are returned without marking a connected session disconnected.
    pub fn disconnect(&mut self) -> Result<(), MqttClientError> {
        self.try_disconnect()
    }

    /// Queue a DISCONNECT packet, propagating [`MqttClientError::BufferFull`] if
    /// the outgoing buffer is full. The session is marked disconnected only once
    /// the packet has actually been queued. A no-op (returns `Ok`) if already
    /// disconnected.
    pub fn try_disconnect(&mut self) -> Result<(), MqttClientError> {
        self.try_disconnect_with(0, Vec::new())
    }

    pub fn try_disconnect_with(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        // MQTT 5 section 3.14.2.1, client-sent reason codes.
        if !matches!(reason_code, 0 | 4 | 0x80..=0x83 | 0x90 | 0x93..=0x99) {
            return Err(MqttClientError::InvalidConfiguration {
                field: "DISCONNECT reason code".into(),
                reason: "Not a client DISCONNECT reason code".into(),
            });
        }
        if self.options.mqtt_version != 5 && (reason_code != 0 || !properties.is_empty()) {
            return Err(MqttClientError::InvalidConfiguration {
                field: "DISCONNECT".into(),
                reason: "Reason codes and properties require MQTT 5".into(),
            });
        }
        if self.reliability.session_expiry == 0
            && properties
                .iter()
                .any(|p| matches!(p, Property::SessionExpiryInterval(value) if *value != 0))
        {
            return Err(MqttClientError::ProtocolViolation {
                message: "DISCONNECT cannot extend a zero session expiry interval".into(),
            });
        }
        if !self.is_connected {
            self.next_reconnect_at = None;
            self.close_input();
            return Ok(());
        }
        let packet = if self.options.mqtt_version == 5 {
            MqttPacket::Disconnect5(disconnectv5::MqttDisconnect::new(reason_code, properties))
        } else {
            self.disconnect_packet()
        };
        self.enqueue_packet(packet)?;
        self.next_reconnect_at = None;
        self.close_input();
        Ok(())
    }

    pub fn auth(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.try_auth(reason_code, properties)
    }

    pub fn try_auth(
        &mut self,
        reason_code: u8,
        mut properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        if self.options.mqtt_version != 5 || !matches!(reason_code, 0x18 | 0x19) {
            return Err(MqttClientError::InvalidConfiguration {
                field: "AUTH".into(),
                reason: "Client AUTH requires MQTT 5 and reason code 0x18 or 0x19".into(),
            });
        }
        let method = self
            .options
            .connect_properties
            .iter()
            .find_map(|p| {
                if let Property::AuthenticationMethod(method) = p {
                    Some(method)
                } else {
                    None
                }
            })
            .ok_or_else(|| MqttClientError::InvalidConfiguration {
                field: "AUTH".into(),
                reason: "CONNECT must specify an authentication method".into(),
            })?;
        if properties
            .iter()
            .any(|p| matches!(p, Property::AuthenticationMethod(value) if value != method))
        {
            return Err(MqttClientError::ProtocolViolation {
                message: "Authentication method cannot change during a connection".into(),
            });
        }
        let valid_state = match reason_code {
            0x19 => self.is_connected && !self.reliability.reauthenticating,
            0x18 => self.reliability.connecting || self.reliability.reauthenticating,
            _ => false,
        };
        if !valid_state || self.reliability.input_closed {
            return Err(MqttClientError::ProtocolViolation {
                message: "AUTH is not valid in the current authentication state".into(),
            });
        }
        if !properties
            .iter()
            .any(|p| matches!(p, Property::AuthenticationMethod(_)))
        {
            properties.insert(0, Property::AuthenticationMethod(method.clone()));
        }
        self.enqueue_packet(MqttPacket::Auth(authv5::MqttAuth::new(
            reason_code,
            properties,
        )))?;
        if reason_code == 0x19 {
            self.reliability.reauthenticating = true;
        }
        Ok(())
    }

    fn auth_received(&mut self, reason_code: u8, properties: Vec<Property>) -> Option<MqttEvent> {
        let expected = self
            .options
            .connect_properties
            .iter()
            .find_map(|p| match p {
                Property::AuthenticationMethod(value) => Some(value),
                _ => None,
            });
        let methods: Vec<_> = properties
            .iter()
            .filter_map(|p| match p {
                Property::AuthenticationMethod(value) => Some(value),
                _ => None,
            })
            .collect();
        let valid_state = match reason_code {
            0 => self.is_connected && self.reliability.reauthenticating,
            0x18 => self.reliability.connecting || self.reliability.reauthenticating,
            _ => false,
        };
        if expected.is_none() || !valid_state || methods.len() != 1 || Some(methods[0]) != expected
        {
            self.fail_connection(MqttClientError::ProtocolViolation {
                message: "Server AUTH must use the CONNECT authentication method during an active authentication exchange".into(),
            });
            return None;
        }
        if reason_code == 0 {
            self.reliability.reauthenticating = false;
        }
        Some(MqttEvent::AuthReceived(super::client::AuthResult {
            reason_code,
            properties,
        }))
    }

    // --- Internal Helpers ---

    /// Handle a single decoded incoming packet.
    ///
    /// Returns the application-facing events together with any **direct response
    /// packets** that the protocol requires us to send in reply to *this* packet
    /// (e.g. PUBACK for an incoming QoS 1 PUBLISH, PUBREC/PUBCOMP for QoS 2,
    /// PUBREL after a PUBREC). These responses are returned rather than enqueued
    /// so the caller can route them to the correct destination — for MQTT over
    /// QUIC this means replying on the *same stream* the packet arrived on,
    /// avoiding cross-stream firing.
    ///
    /// Session-level resends (session resumption after CONNACK) are not direct
    /// responses and are still pushed onto the shared outgoing buffer.
    ///
    /// `stream` is the logical channel the packet arrived on; it is recorded on
    /// any inflight entry created while handling it (the QoS 2 PUBREL), so that
    /// retransmission stays on the same channel.
    fn handle_packet(
        &mut self,
        packet: MqttPacket,
        stream: Option<u64>,
    ) -> (Vec<MqttEvent>, Vec<MqttPacket>) {
        let mut events = Vec::new();
        let mut responses: Vec<MqttPacket> = Vec::new();
        if !self.accept_incoming(packet.packet_type()) {
            return (self.take_events(), responses);
        }
        if let Err(error) = self.validate_ack(&packet, stream) {
            if self.mqtt_version() != 5
                || matches!(error, MqttClientError::ProtocolViolation { .. })
            {
                self.fail_connection(error);
                return (self.take_events(), responses);
            }
            return (vec![MqttEvent::Error(error)], responses);
        }
        match packet {
            MqttPacket::ConnAck5(ack) => {
                if ack.session_present && !self.reliability.has_session {
                    self.fail_connection(MqttClientError::ProtocolViolation {
                        message: "CONNACK reports Session Present without local session state"
                            .into(),
                    });
                    return (self.take_events(), responses);
                }
                #[cfg(feature = "strict-protocol-compliance")]
                {
                    let expected = self
                        .options
                        .connect_properties
                        .iter()
                        .find_map(|p| match p {
                            Property::AuthenticationMethod(method) => Some(method),
                            _ => None,
                        });
                    let received = ack
                        .properties
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .find_map(|p| match p {
                            Property::AuthenticationMethod(method) => Some(method),
                            _ => None,
                        });
                    // MQTT-4.12.0-5 requires the original method on success;
                    // MQTT-4.12.0-6 forbids unsolicited methods on any CONNACK.
                    // Check before negotiation can change connection/session state.
                    if (ack.reason_code == 0 && received != expected)
                        || (expected.is_none() && received.is_some())
                    {
                        self.fail_connection(MqttClientError::ProtocolViolation {
                            message: "CONNACK authentication method does not match CONNECT".into(),
                        });
                        return (self.take_events(), responses);
                    }
                }
                self.reliability.connecting = false;
                self.reliability
                    .deadlines
                    .remove(&(OperationKind::Connect, None));
                if let Err(error) = self.negotiate(ack.properties.as_deref().unwrap_or_default()) {
                    self.fail_connection(error);
                    return (self.take_events(), responses);
                }
                self.is_connected = ack.reason_code == 0;
                if self.is_connected {
                    self.reliability.has_session = true;
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
                            self.inflight_queue
                                .update_receive_maximum((*max).min(self.options.receive_maximum));
                            break;
                        }
                    }
                }

                if self.is_connected {
                    self.restore_session(ack.session_present, &mut events);
                } else {
                    self.handle_connection_lost();
                }
            }
            MqttPacket::ConnAck3(ack) => {
                if ack.session_present && !self.reliability.has_session {
                    self.fail_connection(MqttClientError::ProtocolViolation {
                        message: "CONNACK reports Session Present without local session state"
                            .into(),
                    });
                    return (self.take_events(), responses);
                }
                self.reliability.connecting = false;
                self.reliability
                    .deadlines
                    .remove(&(OperationKind::Connect, None));
                self.is_connected = ack.return_code == 0;
                if self.is_connected {
                    self.reliability.has_session = true;
                    self.reset_reconnect_state();
                }
                events.push(MqttEvent::Connected(ConnectionResult {
                    reason_code: ack.return_code,
                    session_present: ack.session_present,
                    properties: None,
                }));

                if self.is_connected {
                    self.restore_session(ack.session_present, &mut events);
                } else {
                    self.handle_connection_lost();
                }
            }
            MqttPacket::PubAck5(ack) => {
                if let Some(entry) = self.inflight_queue.acknowledge(ack.packet_id) {
                    self.finish_operation(ack.packet_id);
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
                    self.finish_operation(ack.message_id);
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(ack.message_id),
                        reason_code: Some(0), // v3 has no reason code in PUBACK
                        properties: None,
                        qos: entry.qos,
                    }));
                }
            }
            MqttPacket::Publish5(p) => return self.receive_message(p, stream),
            MqttPacket::Publish3(p) => {
                return self.receive_message(
                    MqttPublish::new_with_prop(
                        p.qos,
                        p.topic_name,
                        p.message_id,
                        p.payload,
                        p.retain,
                        p.dup,
                        Vec::new(),
                    ),
                    stream,
                )
            }
            MqttPacket::SubAck5(ack) => {
                self.inflight_queue.acknowledge(ack.packet_id);
                self.finish_operation(ack.packet_id);
                events.push(MqttEvent::Subscribed(SubscribeResult {
                    packet_id: ack.packet_id,
                    reason_codes: ack.reason_codes,
                    properties: ack.properties,
                }));
            }
            MqttPacket::SubAck3(ack) => {
                self.inflight_queue.acknowledge(ack.message_id);
                self.finish_operation(ack.message_id);
                events.push(MqttEvent::Subscribed(SubscribeResult {
                    packet_id: ack.message_id,
                    reason_codes: ack.return_codes,
                    properties: Vec::new(),
                }));
            }
            MqttPacket::UnsubAck5(ack) => {
                self.inflight_queue.acknowledge(ack.packet_id);
                self.finish_operation(ack.packet_id);
                events.push(MqttEvent::Unsubscribed(UnsubscribeResult {
                    packet_id: ack.packet_id,
                    reason_codes: ack.reason_codes,
                    properties: ack.properties,
                }));
            }
            MqttPacket::UnsubAck3(ack) => {
                self.inflight_queue.acknowledge(ack.message_id);
                self.finish_operation(ack.message_id);
                events.push(MqttEvent::Unsubscribed(UnsubscribeResult {
                    packet_id: ack.message_id,
                    reason_codes: Vec::new(),
                    properties: Vec::new(),
                }));
            }
            MqttPacket::PingResp5(_) | MqttPacket::PingResp3(_) => {
                self.ping_sent_at = None;
                events.push(MqttEvent::PingResponse(PingResult { success: true }));
            }
            MqttPacket::PubRec5(rec) => {
                if rec.reason_code >= 0x80 {
                    if let Some(entry) = self.inflight_queue.acknowledge(rec.packet_id) {
                        self.finish_operation(rec.packet_id);
                        events.push(MqttEvent::Published(PublishResult {
                            packet_id: Some(rec.packet_id),
                            reason_code: Some(rec.reason_code),
                            properties: Some(rec.properties),
                            qos: entry.qos,
                        }));
                    }
                } else {
                    let rel = MqttPacket::PubRel5(MqttPubRel::new(rec.packet_id, 0, Vec::new()));
                    self.inflight_queue
                        .transition_pubrel(rec.packet_id, rel.clone());
                    responses.push(rel);
                }
            }
            MqttPacket::PubRec3(rec) => {
                let rel = MqttPacket::PubRel3(pubrelv3::MqttPubRel::new(rec.message_id));
                self.inflight_queue
                    .transition_pubrel(rec.message_id, rel.clone());
                responses.push(rel);
            }
            MqttPacket::PubRel5(rel) => return self.receive_pubrel(rel.packet_id, stream),
            MqttPacket::PubRel3(rel) => return self.receive_pubrel(rel.message_id, stream),
            MqttPacket::PubComp5(comp) => {
                if let Some(entry) = self.inflight_queue.acknowledge(comp.packet_id) {
                    self.finish_operation(comp.packet_id);
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
                    self.finish_operation(comp.message_id);
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(comp.message_id),
                        reason_code: Some(0),
                        properties: None,
                        qos: entry.qos,
                    }));
                }
            }
            MqttPacket::Auth(auth) => {
                if let Some(event) = self.auth_received(auth.reason_code, auth.properties) {
                    events.push(event);
                }
            }
            MqttPacket::Disconnect5(d) => {
                self.handle_connection_lost();
                events.push(MqttEvent::DisconnectReceived {
                    reason_code: d.reason_code,
                    properties: d.properties,
                });
            }
            _ => {}
        }
        (events, responses)
    }

    fn restore_session(&mut self, session_present: bool, events: &mut Vec<MqttEvent>) {
        let mut previous = std::mem::take(&mut self.previous_inflight);
        let resumed = session_present && !self.options.sessionless;
        let replay_unacknowledged = resumed
            || (self.mqtt_version() != 5 && !self.options.clean_start && !self.options.sessionless);
        if replay_unacknowledged {
            // A PUBLISH waiting for quota must not block PUBREL recovery. Keep the
            // original order within each packet type while replaying PUBREL first.
            previous.sort_by_key(|(_, packet)| {
                !matches!(packet, MqttPacket::PubRel3(_) | MqttPacket::PubRel5(_))
            });
        }
        for (id, mut packet) in previous {
            if replay_unacknowledged
                && matches!(
                    packet,
                    MqttPacket::Publish3(_)
                        | MqttPacket::Publish5(_)
                        | MqttPacket::PubRel3(_)
                        | MqttPacket::PubRel5(_)
                )
            {
                packet.set_dup(true);
                if let MqttPacket::Publish5(p) = &mut packet {
                    p.properties
                        .retain(|p| !matches!(p, Property::TopicAlias(_)));
                }
                self.inflight_queue.acknowledge(id);
                self.session_replay.push_back((id, packet));
            } else {
                self.fail_operation(id, &packet, MqttClientError::SessionExpired);
            }
        }
        if !resumed {
            self.reliability.received.clear();
            if self.transport_manages_session {
                self.configured_subscriptions_pending =
                    !self.options.subscription_topics.is_empty();
            } else if !self.options.subscription_topics.is_empty() {
                let command =
                    SubscribeCommand::new(None, self.options.subscription_topics.clone(), vec![]);
                if let Err(error) = self.subscribe(command) {
                    events.push(MqttEvent::Error(error));
                }
            }
        }
        self.process_replay();
    }

    fn pingreq_packet(&self) -> MqttPacket {
        if self.options.mqtt_version == 5 {
            MqttPacket::PingReq5(pingreqv5::MqttPingReq::new())
        } else {
            MqttPacket::PingReq3(pingreqv3::MqttPingReq::new())
        }
    }

    /// Queue a PINGREQ, returning an error if it cannot be queued.
    pub fn send_ping(&mut self) -> Result<(), MqttClientError> {
        self.send_ping_at(Instant::now())
    }

    /// Queue a PINGREQ, propagating [`MqttClientError::BufferFull`] if the
    /// outgoing buffer is full, so callers can confirm it was actually queued.
    pub fn try_send_ping(&mut self) -> Result<(), MqttClientError> {
        self.send_ping_at(Instant::now())
    }

    fn send_ping_at(&mut self, now: Instant) -> Result<(), MqttClientError> {
        let packet = self.pingreq_packet();
        self.enqueue_packet(packet)?;
        self.last_packet_sent = now;
        self.ping_sent_at = Some(now);
        Ok(())
    }

    #[cfg(feature = "strict-protocol-compliance")]
    fn ensure_outgoing_allowed(
        &self,
        packet_type: ControlPacketType,
    ) -> Result<(), MqttClientError> {
        // Enforce MQTT-3.1.2-30 in strict mode. Reauthentication permits normal
        // traffic; queued PUBLISHes wait for CONNACK in process_queue.
        if self.mqtt_version() == 5
            && self.reliability.connecting
            && self
                .options
                .connect_properties
                .iter()
                .any(|p| matches!(p, Property::AuthenticationMethod(_)))
            && !matches!(
                packet_type,
                ControlPacketType::AUTH | ControlPacketType::DISCONNECT
            )
        {
            return Err(MqttClientError::InvalidState {
                expected: "AUTH or DISCONNECT until CONNACK".into(),
                actual: format!("{packet_type:?} during initial enhanced authentication"),
            });
        }
        Ok(())
    }

    pub fn enqueue_packet(&mut self, packet: MqttPacket) -> Result<(), MqttClientError> {
        #[cfg(feature = "strict-protocol-compliance")]
        self.ensure_outgoing_allowed(packet.packet_type())?;
        if self.outgoing_buffer.len() >= self.options.max_outgoing_packet_count {
            return Err(MqttClientError::BufferFull {
                buffer_type: "outgoing".to_string(),
                capacity: self.options.max_outgoing_packet_count,
            });
        }

        self.validate_packet_capabilities(&packet)?;
        match packet.to_bytes() {
            Ok(bytes) => {
                self.check_packet_size(&bytes)?;
                self.check_output_capacity(bytes.len())?;
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

        self.flush_responses();
        self.process_replay();
        if !self.reliability.responses.is_empty() || !self.session_replay.is_empty() {
            return;
        }
        while self.outgoing_buffer.len() < self.options.max_outgoing_packet_count {
            if let Some((_priority, packet)) = self.priority_queue.peek() {
                let needs_ack = match packet {
                    MqttPacket::Publish5(packet) => packet.qos > 0,
                    MqttPacket::Publish3(packet) => packet.qos > 0,
                    _ => false,
                };
                if needs_ack && !self.inflight_queue.can_push_publish() {
                    break;
                }

                // Attempt to encode before dequeuing
                match packet.to_bytes() {
                    Ok(bytes) => {
                        // Queued byte reservations move to the serialized output buffer.
                        self.reliability.queued_bytes =
                            self.reliability.queued_bytes.saturating_sub(bytes.len());
                        if let Err(error) = self
                            .validate_packet_capabilities(packet)
                            .and_then(|()| self.check_packet_size(&bytes))
                        {
                            if let Some((_, packet)) = self.priority_queue.dequeue() {
                                let id = match &packet {
                                    MqttPacket::Publish5(p) => p.packet_id,
                                    MqttPacket::Publish3(p) => p.message_id,
                                    _ => None,
                                };
                                if let Some(id) = id {
                                    self.fail_operation(id, &packet, error);
                                } else {
                                    self.events.push(MqttEvent::Error(error));
                                }
                            }
                            continue;
                        }
                        // All checks pass, dequeue and track
                        if let Some((_, packet)) = self.priority_queue.dequeue() {
                            match &packet {
                                MqttPacket::Publish5(p) if p.qos > 0 => {
                                    let pid = p.packet_id.unwrap();
                                    self.inflight_queue
                                        .push(pid, packet.clone(), p.qos)
                                        .expect("reserved packet ID and checked quota");
                                    self.start_deadline(OperationKind::Publish, Some(pid));
                                }
                                MqttPacket::Publish3(p) if p.qos > 0 => {
                                    let pid = p.message_id.unwrap();
                                    self.inflight_queue
                                        .push(pid, packet.clone(), p.qos)
                                        .expect("reserved packet ID and checked quota");
                                    self.start_deadline(OperationKind::Publish, Some(pid));
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
        for _ in 0..u16::MAX {
            let id = session.next_packet_id();
            if !self.reliability.reserved.contains(&id) && !self.inflight_queue.contains(id) {
                return Ok(id);
            }
        }
        Err(MqttClientError::PacketIdExhausted)
    }
}

/// State for a single QUIC data stream used by [`QuicMqttEngine`].
///
/// Every QUIC stream is an independent, ordered byte stream that carries its own
/// sequence of MQTT packets. Because partial packets from different streams must
/// never be concatenated, each stream owns a dedicated [`MqttParser`] for framing
/// inbound bytes and a buffer for outbound bytes awaiting transmission.
#[cfg(feature = "quic-proto")]
struct QuicStream {
    priority: u8,
    publish_priority: Option<u8>,
    parser: MqttParser,
    outgoing: Vec<u8>,
    /// Number of logical packets currently buffered in `outgoing` (reset to 0 once
    /// the buffer fully drains). Used to bound the buffer like the engine's
    /// `max_outgoing_packet_count`, so a stalled stream cannot grow without limit.
    pending_packets: usize,
    /// Maximum number of buffered packets before [`MqttClientError::BufferFull`].
    max_packets: usize,
    /// A FIN has been requested; the send side is finished once `outgoing` drains.
    finishing: bool,
    /// The send side has been finished or reset; further writes are rejected.
    finished: bool,
    /// The receive side has seen FIN or RESET; further reads are skipped.
    recv_closed: bool,
}

#[cfg(feature = "quic-proto")]
impl QuicStream {
    fn new(parser_buffer_size: usize, mqtt_version: u8, max_packets: usize) -> Self {
        Self {
            priority: 128,
            publish_priority: None,
            parser: MqttParser::new(parser_buffer_size, mqtt_version),
            outgoing: Vec::new(),
            pending_packets: 0,
            max_packets,
            finishing: false,
            finished: false,
            recv_closed: false,
        }
    }

    fn with_outgoing(
        parser_buffer_size: usize,
        mqtt_version: u8,
        max_packets: usize,
        outgoing: Vec<u8>,
        pending_packets: usize,
    ) -> Self {
        Self {
            priority: 128,
            publish_priority: None,
            parser: MqttParser::new(parser_buffer_size, mqtt_version),
            outgoing,
            pending_packets,
            max_packets,
            finishing: false,
            finished: false,
            recv_closed: false,
        }
    }
}

#[cfg(feature = "quic-proto")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EarlyStreamRole {
    Control,
    DefaultPub,
    PriorityPub(u8),
    DefaultSub,
    ExplicitData,
}

#[cfg(feature = "quic-proto")]
#[derive(Debug, Clone)]
struct EarlyStreamJournal {
    priority: u8,
    original_stream_id: StreamId,
    role: EarlyStreamRole,
    bytes: Vec<u8>,
    packet_count: usize,
}

#[cfg(feature = "quic-proto")]
struct ClearableQuicSessionCache {
    size: usize,
    inner: Mutex<ClientSessionMemoryCache>,
}

#[cfg(feature = "quic-proto")]
impl ClearableQuicSessionCache {
    fn new(size: usize) -> Self {
        Self {
            size,
            inner: Mutex::new(ClientSessionMemoryCache::new(size)),
        }
    }

    fn size(&self) -> usize {
        self.size
    }

    fn clear(&self) {
        let mut guard = self.inner.lock().unwrap();
        let _ = std::mem::replace(&mut *guard, ClientSessionMemoryCache::new(self.size));
    }
}

#[cfg(feature = "quic-proto")]
impl fmt::Debug for ClearableQuicSessionCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClearableQuicSessionCache")
            .field("size", &self.size)
            .field("sessions", &"[redacted]")
            .finish()
    }
}

#[cfg(feature = "quic-proto")]
impl ClientSessionStore for ClearableQuicSessionCache {
    fn set_kx_hint(&self, server_name: ServerName<'static>, group: NamedGroup) {
        self.inner.lock().unwrap().set_kx_hint(server_name, group);
    }

    fn kx_hint(&self, server_name: &ServerName<'_>) -> Option<NamedGroup> {
        self.inner.lock().unwrap().kx_hint(server_name)
    }

    fn set_tls12_session(&self, server_name: ServerName<'static>, value: Tls12ClientSessionValue) {
        self.inner
            .lock()
            .unwrap()
            .set_tls12_session(server_name, value);
    }

    fn tls12_session(&self, server_name: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        self.inner.lock().unwrap().tls12_session(server_name)
    }

    fn remove_tls12_session(&self, server_name: &ServerName<'static>) {
        self.inner.lock().unwrap().remove_tls12_session(server_name);
    }

    fn insert_tls13_ticket(
        &self,
        server_name: ServerName<'static>,
        value: Tls13ClientSessionValue,
    ) {
        self.inner
            .lock()
            .unwrap()
            .insert_tls13_ticket(server_name, value);
    }

    fn take_tls13_ticket(
        &self,
        server_name: &ServerName<'static>,
    ) -> Option<Tls13ClientSessionValue> {
        self.inner.lock().unwrap().take_tls13_ticket(server_name)
    }
}

/// Error returned when writing to a stream whose send side has been finished or
/// reset.
#[cfg(feature = "quic-proto")]
fn stream_finished_error() -> MqttClientError {
    MqttClientError::InvalidState {
        expected: "an open (non-finished) stream".to_string(),
        actual: "stream finished or reset".to_string(),
    }
}

/// Convert an application error code into a QUIC [`VarInt`], rejecting values
/// that exceed the 62-bit QUIC varint range.
#[cfg(feature = "quic-proto")]
fn quic_error_code(error_code: u64) -> Result<VarInt, MqttClientError> {
    VarInt::from_u64(error_code).map_err(|_| MqttClientError::InvalidConfiguration {
        field: "error_code".to_string(),
        reason: "QUIC error code exceeds 2^62-1".to_string(),
    })
}

/// Map a quinn-proto [`ConnectionError`] to `(by_peer, error_code)` for the
/// [`MqttEvent::TransportClosed`] event.
#[cfg(feature = "quic-proto")]
fn transport_close_details(reason: &ConnectionError) -> (bool, Option<u64>) {
    match reason {
        ConnectionError::ApplicationClosed(c) => (true, Some(u64::from(c.error_code))),
        // Peer's QUIC stack aborted with a transport error code — preserve it.
        ConnectionError::ConnectionClosed(c) => (true, Some(u64::from(c.error_code))),
        ConnectionError::Reset => (true, None),
        ConnectionError::VersionMismatch
        | ConnectionError::TransportError(_)
        | ConnectionError::TimedOut
        | ConnectionError::LocallyClosed
        | ConnectionError::CidsExhausted => (false, None),
    }
}

/// A "Sans-I/O" MQTT over QUIC protocol engine.
///
/// This engine combines the `MqttEngine` (MQTT state machine) with `quinn_proto` (QUIC state machine)
/// to provide a complete MQTT-over-QUIC implementation that does not perform any direct I/O.
///
/// # Multi-stream support
///
/// MQTT-over-QUIC can spread traffic across multiple QUIC streams to avoid
/// head-of-line blocking. This engine opens a primary **control stream** for the
/// MQTT handshake and control packets, and supports additional **data streams**:
///
/// - [`open_data_stream`](Self::open_data_stream) opens a new client-initiated
///   bidirectional stream and returns an opaque handle.
/// - [`publish_on`](Self::publish_on) routes a PUBLISH onto a specific data stream.
/// - Server-initiated bidirectional streams are accepted automatically and their
///   inbound packets are fed into the shared protocol state machine.
///
/// MQTT protocol state (packet-id allocation, the inflight queue for QoS 1/2,
/// session, keep-alive) is shared across all streams; only the wire framing is
/// per-stream.
#[cfg(feature = "quic-proto")]
pub struct QuicMqttEngine {
    mqtt_engine: MqttEngine,
    endpoint: Endpoint,
    connection: Option<Connection>,
    connection_handle: Option<ConnectionHandle>,

    // The primary bidirectional stream used for the MQTT handshake and control packets.
    control_stream: Option<StreamId>,

    // Additional data streams (client- or server-initiated), keyed by raw QUIC stream id.
    // Each carries its own framing parser and outgoing buffer.
    data_streams: HashMap<StreamId, QuicStream>,

    // Lazily-opened default data streams backing the convenience `publish` /
    // `subscribe` / `unsubscribe` methods, so that PUBLISH and SUBSCRIBE traffic
    // never lands on the session control stream.
    default_pub_stream: Option<StreamId>,
    default_sub_stream: Option<StreamId>,

    // Raw bytes injected onto the control stream via the low-level escape hatch
    // (`send_raw_on` / `send_packet_on`), flushed alongside the engine's session
    // packets. Used for negative testing.
    control_outgoing: Vec<u8>,

    // Connection parameters retained from the last `connect` so `reconnect` can
    // re-establish on the same endpoint without the caller rebuilding the config.
    client_config: Option<ClientConfig>,
    server_addr: Option<std::net::SocketAddr>,
    server_name: Option<String>,

    // Optional QUIC 0-RTT/session-ticket policy and cache. The cache is retained
    // across connections until explicitly cleared or replaced with another size.
    zero_rtt_config: Option<QuicZeroRttConfig>,
    zero_rtt_status: QuicZeroRttStatus,
    zero_rtt_cache: Option<Arc<ClearableQuicSessionCache>>,
    pending_transport_events: VecDeque<MqttEvent>,
    early_stream_journal: Vec<EarlyStreamJournal>,

    // A deferred graceful QUIC close, applied after the queued MQTT DISCONNECT has
    // been flushed (see `disconnect_and_close`).
    pending_close: Option<(VarInt, bytes::Bytes)>,
    // A FIN requested for the control stream's send side, applied after its
    // buffered bytes are flushed.
    control_finishing: bool,
    // The control stream's send side has been finished or reset; further writes
    // (including raw injection) are rejected.
    control_finished: bool,

    // Outgoing UDP datagrams to be sent by the application
    outgoing_datagrams: VecDeque<(std::net::SocketAddr, Vec<u8>)>,
}

#[cfg(feature = "quic-proto")]
impl QuicMqttEngine {
    pub fn new(options: MqttClientOptions) -> Result<Self, MqttClientError> {
        // Session operations must be routed onto QUIC data streams.
        let mut mqtt_engine = MqttEngine::new(options);
        mqtt_engine.transport_manages_session = true;

        // Initialize QUIC Endpoint (Client)
        let endpoint_config = EndpointConfig::default();
        // quinn-proto 0.11 (mainstream) has a 4th reset_token_key parameter;
        // the fork (0.12) removed it.
        #[cfg(feature = "quic-proto-openssl")]
        let endpoint = Endpoint::new(Arc::new(endpoint_config), None, true);
        #[cfg(not(feature = "quic-proto-openssl"))]
        let endpoint = Endpoint::new(Arc::new(endpoint_config), None, true, None);

        Ok(Self {
            mqtt_engine,
            endpoint,
            connection: None,
            connection_handle: None,
            control_stream: None,
            data_streams: HashMap::new(),
            default_pub_stream: None,
            default_sub_stream: None,
            control_outgoing: Vec::new(),
            client_config: None,
            server_addr: None,
            server_name: None,
            zero_rtt_config: None,
            zero_rtt_status: QuicZeroRttStatus::Disabled,
            zero_rtt_cache: None,
            pending_transport_events: VecDeque::new(),
            early_stream_journal: Vec::new(),
            pending_close: None,
            control_finishing: false,
            control_finished: false,
            outgoing_datagrams: VecDeque::new(),
        })
    }

    /// Select the parser depth for the control stream and all QUIC data streams.
    pub fn set_parse_level(&mut self, level: ParseLevel) {
        self.mqtt_engine.set_parse_level(level);
        for stream in self.data_streams.values_mut() {
            stream.parser.set_parse_level(level);
        }
    }

    pub fn parse_level(&self) -> ParseLevel {
        self.mqtt_engine.parse_level()
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

        let client_config = Self::client_config_from_crypto(crypto_config)?;

        // Retain parameters so `reconnect` can re-establish without rebuilding.
        self.client_config = Some(client_config);
        self.server_addr = Some(server_addr);
        self.server_name = Some(server_name.to_string());
        self.zero_rtt_config = None;
        self.zero_rtt_status = QuicZeroRttStatus::Disabled;
        self.pending_transport_events.clear();
        self.early_stream_journal.clear();

        self.establish(now)
    }

    /// Connect with QUIC 0-RTT/session-ticket support enabled.
    ///
    /// Early MQTT commands are only accepted while the returned status is
    /// [`QuicZeroRttStatus::Attempted`]. If the peer rejects 0-RTT and
    /// `replay_on_reject` is true, the engine replays the exact early bytes as
    /// 1-RTT data on replacement streams.
    pub fn connect_with_zero_rtt(
        &mut self,
        server_addr: std::net::SocketAddr,
        server_name: &str,
        mut crypto_config: rustls::ClientConfig,
        zero_rtt_config: QuicZeroRttConfig,
        now: Instant,
    ) -> Result<(), MqttClientError> {
        if crypto_config.alpn_protocols.is_empty() {
            crypto_config.alpn_protocols = vec![b"mqtt".to_vec()];
        }

        crypto_config.enable_early_data = true;
        let cache = self.zero_rtt_cache_for_size(zero_rtt_config.session_cache_size);
        crypto_config.resumption = Resumption::store(cache);

        let client_config = Self::client_config_from_crypto(crypto_config)?;

        self.client_config = Some(client_config);
        self.server_addr = Some(server_addr);
        self.server_name = Some(server_name.to_string());
        self.zero_rtt_config = Some(zero_rtt_config);
        self.zero_rtt_status = QuicZeroRttStatus::Unavailable;
        self.pending_transport_events.clear();
        self.early_stream_journal.clear();

        self.establish(now)
    }

    /// Current QUIC 0-RTT state.
    pub fn zero_rtt_status(&self) -> QuicZeroRttStatus {
        self.zero_rtt_status
    }

    /// Clear any retained QUIC TLS resumption/session tickets.
    pub fn clear_quic_session_cache(&mut self) {
        if let Some(cache) = &self.zero_rtt_cache {
            cache.clear();
        }
    }

    /// Re-establish the QUIC connection on the existing endpoint, reusing the
    /// configuration captured by the previous [`connect`](Self::connect).
    ///
    /// Resets all stream state (the new connection starts fresh streams) and the
    /// MQTT connected flag; the MQTT handshake is re-driven once the new QUIC
    /// handshake completes. Returns an error if [`connect`](Self::connect) has not
    /// been called yet.
    pub fn reconnect(&mut self, now: Instant) -> Result<(), MqttClientError> {
        if self.client_config.is_none() {
            return Err(MqttClientError::InvalidState {
                expected: "a prior successful connect()".to_string(),
                actual: "reconnect called before connect".to_string(),
            });
        }
        self.reset_connection_state();
        self.establish(now)
    }

    /// Notify quinn-proto that the embedding runtime has switched to a new local UDP address.
    ///
    /// The sans-I/O engine does not own sockets, so the caller is responsible for
    /// rebinding/replacing the UDP socket used to send [`take_outgoing_datagrams`](Self::take_outgoing_datagrams).
    /// This hook mirrors Quinn's async `Endpoint::rebind` behavior for active
    /// connections: it rotates to a fresh remote connection ID when available and
    /// queues a PING so the peer can validate the new path.
    pub fn notify_local_address_changed(&mut self) -> Result<(), MqttClientError> {
        if let Some(conn) = &mut self.connection {
            conn.local_address_changed();
        }
        Ok(())
    }

    fn client_config_from_crypto(
        crypto_config: rustls::ClientConfig,
    ) -> Result<ClientConfig, MqttClientError> {
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
        // Set max_idle_timeout to prevent QUIC from timing out before MQTT keepalive mechanism
        // Use 120 seconds to accommodate MQTT keepalive (typically 30-60s) with 2x multiplier for safety
        let idle_timeout = std::time::Duration::from_secs(120)
            .try_into()
            .map_err(|e| MqttClientError::InternalError {
                message: format!("Failed to convert QUIC idle timeout: {}", e),
            })?;
        transport.max_idle_timeout(Some(idle_timeout));
        client_config.transport_config(Arc::new(transport));
        Ok(client_config)
    }

    fn zero_rtt_cache_for_size(&mut self, size: usize) -> Arc<ClearableQuicSessionCache> {
        if let Some(cache) = &self.zero_rtt_cache {
            if cache.size() == size {
                return Arc::clone(cache);
            }
        }

        let cache = Arc::new(ClearableQuicSessionCache::new(size));
        self.zero_rtt_cache = Some(Arc::clone(&cache));
        cache
    }

    fn journal_stream_open(
        early_stream_journal: &mut Vec<EarlyStreamJournal>,
        stream_id: StreamId,
        role: EarlyStreamRole,
    ) {
        early_stream_journal.push(EarlyStreamJournal {
            priority: if let EarlyStreamRole::PriorityPub(priority) = role {
                priority
            } else {
                128
            },
            original_stream_id: stream_id,
            role,
            bytes: Vec::new(),
            packet_count: 0,
        });
    }

    fn journal_stream_bytes(
        early_stream_journal: &mut [EarlyStreamJournal],
        stream_id: StreamId,
        bytes: &[u8],
        packet_count_delta: usize,
    ) {
        if bytes.is_empty() && packet_count_delta == 0 {
            return;
        }
        if let Some(entry) = early_stream_journal
            .iter_mut()
            .find(|entry| entry.original_stream_id == stream_id)
        {
            entry.bytes.extend_from_slice(bytes);
            entry.packet_count += packet_count_delta;
        }
    }

    fn append_control_outgoing_bytes(
        control_outgoing: &mut Vec<u8>,
        early_stream_journal: &mut [EarlyStreamJournal],
        zero_rtt_status: QuicZeroRttStatus,
        control_stream: Option<StreamId>,
        bytes: &[u8],
        packet_count_delta: usize,
    ) {
        if bytes.is_empty() {
            return;
        }
        control_outgoing.extend_from_slice(bytes);
        if zero_rtt_status == QuicZeroRttStatus::Attempted {
            if let Some(stream_id) = control_stream {
                Self::journal_stream_bytes(
                    early_stream_journal,
                    stream_id,
                    bytes,
                    packet_count_delta,
                );
            }
        }
    }

    fn mark_zero_rtt_unavailable(&mut self) {
        self.zero_rtt_status = QuicZeroRttStatus::Unavailable;
        self.pending_transport_events
            .push_back(MqttEvent::ZeroRttStatusChanged {
                status: QuicZeroRttStatus::Unavailable,
            });
    }

    fn begin_zero_rtt_attempt(&mut self, control_stream: Option<StreamId>) {
        let Some(stream_id) = control_stream else {
            self.mark_zero_rtt_unavailable();
            return;
        };

        self.zero_rtt_status = QuicZeroRttStatus::Attempted;
        self.pending_transport_events
            .push_back(MqttEvent::ZeroRttStatusChanged {
                status: QuicZeroRttStatus::Attempted,
            });
        self.control_stream = Some(stream_id);
        Self::journal_stream_open(
            &mut self.early_stream_journal,
            stream_id,
            EarlyStreamRole::Control,
        );
        if let Err(error) = self.mqtt_engine.connect() {
            self.mqtt_engine.events.push(MqttEvent::Error(error));
            return;
        }
        let connect_bytes = self.mqtt_engine.take_outgoing();
        Self::append_control_outgoing_bytes(
            &mut self.control_outgoing,
            &mut self.early_stream_journal,
            self.zero_rtt_status,
            self.control_stream,
            &connect_bytes,
            usize::from(!connect_bytes.is_empty()),
        );
    }

    fn ensure_command_allowed_before_mqtt_connected(&self) -> Result<(), MqttClientError> {
        if self.mqtt_engine.is_connected() || self.zero_rtt_status == QuicZeroRttStatus::Attempted {
            return Ok(());
        }

        Err(MqttClientError::InvalidState {
            expected: "a connected MQTT session or attempted QUIC 0-RTT".to_string(),
            actual: format!(
                "MQTT not connected; 0-RTT status {:?}",
                self.zero_rtt_status
            ),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_early_stream_journals(
        conn: &mut Connection,
        journals: Vec<EarlyStreamJournal>,
        parser_buffer_size: usize,
        mqtt_version: u8,
        max_packets: usize,
        parse_level: ParseLevel,
        control_stream: &mut Option<StreamId>,
        data_streams: &mut HashMap<StreamId, QuicStream>,
        default_pub_stream: &mut Option<StreamId>,
        default_sub_stream: &mut Option<StreamId>,
        control_outgoing: &mut Vec<u8>,
        events: &mut Vec<MqttEvent>,
    ) {
        for journal in journals {
            let Some(stream_id) = conn.streams().open(Dir::Bi) else {
                events.push(MqttEvent::Error(MqttClientError::InternalError {
                    message: "QUIC stream limit reached while replaying rejected 0-RTT data"
                        .to_string(),
                }));
                break;
            };

            if stream_id != journal.original_stream_id {
                events.push(MqttEvent::Error(MqttClientError::InternalError {
                    message: format!(
                        "0-RTT replay stream id changed from {} to {}",
                        u64::from(journal.original_stream_id),
                        u64::from(stream_id)
                    ),
                }));
            }

            match journal.role {
                EarlyStreamRole::Control => {
                    *control_stream = Some(stream_id);
                    control_outgoing.extend_from_slice(&journal.bytes);
                }
                EarlyStreamRole::DefaultPub
                | EarlyStreamRole::PriorityPub(_)
                | EarlyStreamRole::DefaultSub
                | EarlyStreamRole::ExplicitData => {
                    let mut stream = QuicStream::with_outgoing(
                        parser_buffer_size,
                        mqtt_version,
                        max_packets,
                        journal.bytes,
                        journal.packet_count,
                    );
                    stream.parser.set_parse_level(parse_level);
                    stream.priority = journal.priority;
                    if let EarlyStreamRole::PriorityPub(priority) = journal.role {
                        stream.publish_priority = Some(priority);
                    }
                    data_streams.insert(stream_id, stream);
                    match journal.role {
                        EarlyStreamRole::DefaultPub => *default_pub_stream = Some(stream_id),
                        EarlyStreamRole::DefaultSub => *default_sub_stream = Some(stream_id),
                        EarlyStreamRole::PriorityPub(_)
                        | EarlyStreamRole::ExplicitData
                        | EarlyStreamRole::Control => {}
                    }
                }
            }
        }
    }

    /// Clear all per-connection state (streams, buffers, MQTT connected flag).
    fn reset_connection_state(&mut self) {
        self.connection = None;
        self.connection_handle = None;
        self.control_stream = None;
        self.data_streams.clear();
        self.default_pub_stream = None;
        self.default_sub_stream = None;
        self.control_outgoing.clear();
        self.outgoing_datagrams.clear();
        self.pending_close = None;
        self.control_finishing = false;
        self.control_finished = false;
        self.pending_transport_events.clear();
        self.early_stream_journal.clear();
        // Discard any MQTT bytes queued for the old transport so they are not
        // replayed before the next CONNECT.
        self.mqtt_engine.reset_for_new_transport();
    }

    fn clear_data_stream_refs_fields(
        data_streams: &mut HashMap<StreamId, QuicStream>,
        default_pub_stream: &mut Option<StreamId>,
        default_sub_stream: &mut Option<StreamId>,
        stream_id: StreamId,
    ) {
        data_streams.remove(&stream_id);
        if *default_pub_stream == Some(stream_id) {
            *default_pub_stream = None;
        }
        if *default_sub_stream == Some(stream_id) {
            *default_sub_stream = None;
        }
    }

    fn handle_control_stream_terminal_fields(
        control_stream: &mut Option<StreamId>,
        control_outgoing: &mut Vec<u8>,
        control_finishing: &mut bool,
        control_finished: &mut bool,
        mqtt_engine: &mut MqttEngine,
        mqtt_events: &mut Vec<MqttEvent>,
    ) {
        *control_stream = None;
        control_outgoing.clear();
        *control_finishing = false;
        *control_finished = true;
        mqtt_engine.handle_connection_lost();
        mqtt_events.push(MqttEvent::Disconnected(None));
    }

    #[cfg(test)]
    fn handle_stream_stopped(
        &mut self,
        stream_id: StreamId,
        error_code: VarInt,
        mqtt_events: &mut Vec<MqttEvent>,
    ) {
        Self::handle_stream_stopped_fields(
            &mut self.control_stream,
            &mut self.data_streams,
            &mut self.default_pub_stream,
            &mut self.default_sub_stream,
            &mut self.control_outgoing,
            &mut self.control_finishing,
            &mut self.control_finished,
            &mut self.mqtt_engine,
            stream_id,
            error_code,
            mqtt_events,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_stream_stopped_fields(
        control_stream: &mut Option<StreamId>,
        data_streams: &mut HashMap<StreamId, QuicStream>,
        default_pub_stream: &mut Option<StreamId>,
        default_sub_stream: &mut Option<StreamId>,
        control_outgoing: &mut Vec<u8>,
        control_finishing: &mut bool,
        control_finished: &mut bool,
        mqtt_engine: &mut MqttEngine,
        stream_id: StreamId,
        error_code: VarInt,
        mqtt_events: &mut Vec<MqttEvent>,
    ) {
        if Some(stream_id) == *control_stream {
            mqtt_events.push(MqttEvent::StreamStopped {
                stream_id: u64::from(stream_id),
                error_code: u64::from(error_code),
            });
            Self::handle_control_stream_terminal_fields(
                control_stream,
                control_outgoing,
                control_finishing,
                control_finished,
                mqtt_engine,
                mqtt_events,
            );
        } else if data_streams.contains_key(&stream_id) {
            mqtt_events.push(MqttEvent::StreamStopped {
                stream_id: u64::from(stream_id),
                error_code: u64::from(error_code),
            });
            Self::clear_data_stream_refs_fields(
                data_streams,
                default_pub_stream,
                default_sub_stream,
                stream_id,
            );
        }
    }

    #[cfg(test)]
    fn handle_stream_reset(
        &mut self,
        stream_id: StreamId,
        error_code: VarInt,
        mqtt_events: &mut Vec<MqttEvent>,
    ) {
        Self::handle_stream_reset_fields(
            &mut self.control_stream,
            &mut self.data_streams,
            &mut self.default_pub_stream,
            &mut self.default_sub_stream,
            &mut self.control_outgoing,
            &mut self.control_finishing,
            &mut self.control_finished,
            &mut self.mqtt_engine,
            stream_id,
            error_code,
            mqtt_events,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_stream_reset_fields(
        control_stream: &mut Option<StreamId>,
        data_streams: &mut HashMap<StreamId, QuicStream>,
        default_pub_stream: &mut Option<StreamId>,
        default_sub_stream: &mut Option<StreamId>,
        control_outgoing: &mut Vec<u8>,
        control_finishing: &mut bool,
        control_finished: &mut bool,
        mqtt_engine: &mut MqttEngine,
        stream_id: StreamId,
        error_code: VarInt,
        mqtt_events: &mut Vec<MqttEvent>,
    ) {
        if Some(stream_id) == *control_stream {
            mqtt_events.push(MqttEvent::StreamReset {
                stream_id: u64::from(stream_id),
                error_code: u64::from(error_code),
            });
            Self::handle_control_stream_terminal_fields(
                control_stream,
                control_outgoing,
                control_finishing,
                control_finished,
                mqtt_engine,
                mqtt_events,
            );
        } else if data_streams.contains_key(&stream_id) {
            mqtt_events.push(MqttEvent::StreamReset {
                stream_id: u64::from(stream_id),
                error_code: u64::from(error_code),
            });
            Self::clear_data_stream_refs_fields(
                data_streams,
                default_pub_stream,
                default_sub_stream,
                stream_id,
            );
        }
    }

    #[cfg(test)]
    fn handle_stream_closed(
        &mut self,
        stream_id: StreamId,
        reason: &'static str,
        by_peer: bool,
        mqtt_events: &mut Vec<MqttEvent>,
    ) {
        Self::handle_stream_closed_fields(
            &mut self.control_stream,
            &mut self.data_streams,
            &mut self.control_outgoing,
            &mut self.control_finishing,
            &mut self.control_finished,
            &mut self.mqtt_engine,
            stream_id,
            reason,
            by_peer,
            mqtt_events,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_stream_closed_fields(
        control_stream: &mut Option<StreamId>,
        data_streams: &mut HashMap<StreamId, QuicStream>,
        control_outgoing: &mut Vec<u8>,
        control_finishing: &mut bool,
        control_finished: &mut bool,
        mqtt_engine: &mut MqttEngine,
        stream_id: StreamId,
        reason: &'static str,
        by_peer: bool,
        mqtt_events: &mut Vec<MqttEvent>,
    ) {
        if Some(stream_id) == *control_stream {
            mqtt_events.push(MqttEvent::StreamClosed {
                stream_id: u64::from(stream_id),
                reason: reason.to_string(),
                by_peer,
            });
            if by_peer {
                Self::handle_control_stream_terminal_fields(
                    control_stream,
                    control_outgoing,
                    control_finishing,
                    control_finished,
                    mqtt_engine,
                    mqtt_events,
                );
            }
        } else if let Some(ds) = data_streams.get_mut(&stream_id) {
            mqtt_events.push(MqttEvent::StreamClosed {
                stream_id: u64::from(stream_id),
                reason: reason.to_string(),
                by_peer,
            });
            if by_peer {
                ds.recv_closed = true;
            } else {
                ds.finished = true;
            }
        }
    }

    /// Open a fresh QUIC connection using the retained configuration.
    fn establish(&mut self, now: Instant) -> Result<(), MqttClientError> {
        let client_config =
            self.client_config
                .clone()
                .ok_or_else(|| MqttClientError::InvalidState {
                    expected: "stored client config".to_string(),
                    actual: "none".to_string(),
                })?;
        let server_addr = self
            .server_addr
            .ok_or_else(|| MqttClientError::InvalidState {
                expected: "stored server address".to_string(),
                actual: "none".to_string(),
            })?;
        let server_name =
            self.server_name
                .clone()
                .ok_or_else(|| MqttClientError::InvalidState {
                    expected: "stored server name".to_string(),
                    actual: "none".to_string(),
                })?;

        let (ch, mut conn) = self
            .endpoint
            .connect(now, client_config, server_addr, &server_name)
            .map_err(|e| MqttClientError::InternalError {
                message: format!("Failed to create QUIC connection: {}", e),
            })?;

        if self.zero_rtt_config.is_some() {
            if conn.has_0rtt() {
                let control_stream = conn.streams().open(Dir::Bi);
                self.begin_zero_rtt_attempt(control_stream);
            } else {
                self.mark_zero_rtt_unavailable();
            }
        }

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
        let mut mqtt_events: Vec<MqttEvent> = self.pending_transport_events.drain(..).collect();

        let parser_buffer_size = self.mqtt_engine.options().parser_buffer_size;
        let mqtt_version = self.mqtt_engine.mqtt_version();
        let max_packets = self.mqtt_engine.options().max_outgoing_packet_count;
        let parse_level = self.mqtt_engine.parse_level();

        // Drive QUIC Connection
        if let Some(conn) = &mut self.connection {
            conn.handle_timeout(now);

            // 1. Drain connection events.
            while let Some(event) = conn.poll() {
                match event {
                    quinn_proto::Event::Stream(stream_event) => match stream_event {
                        quinn_proto::StreamEvent::Finished { id } => {
                            Self::handle_stream_closed_fields(
                                &mut self.control_stream,
                                &mut self.data_streams,
                                &mut self.control_outgoing,
                                &mut self.control_finishing,
                                &mut self.control_finished,
                                &mut self.mqtt_engine,
                                id,
                                "send_finished",
                                false,
                                &mut mqtt_events,
                            );
                        }
                        quinn_proto::StreamEvent::Stopped { id, error_code } => {
                            Self::handle_stream_stopped_fields(
                                &mut self.control_stream,
                                &mut self.data_streams,
                                &mut self.default_pub_stream,
                                &mut self.default_sub_stream,
                                &mut self.control_outgoing,
                                &mut self.control_finishing,
                                &mut self.control_finished,
                                &mut self.mqtt_engine,
                                id,
                                error_code,
                                &mut mqtt_events,
                            );
                        }
                        quinn_proto::StreamEvent::Opened { .. }
                        | quinn_proto::StreamEvent::Readable { .. }
                        | quinn_proto::StreamEvent::Writable { .. }
                        | quinn_proto::StreamEvent::Available { .. } => {
                            // Readable/writable/open events are handled by polling
                            // the streams directly below.
                        }
                    },
                    quinn_proto::Event::Connected => {
                        let zero_rtt_was_attempted =
                            self.zero_rtt_status == QuicZeroRttStatus::Attempted;

                        if zero_rtt_was_attempted {
                            if conn.accepted_0rtt() {
                                self.zero_rtt_status = QuicZeroRttStatus::Accepted;
                                self.early_stream_journal.clear();
                                mqtt_events.push(MqttEvent::ZeroRttStatusChanged {
                                    status: QuicZeroRttStatus::Accepted,
                                });
                            } else {
                                self.zero_rtt_status = QuicZeroRttStatus::Rejected;
                                mqtt_events.push(MqttEvent::ZeroRttStatusChanged {
                                    status: QuicZeroRttStatus::Rejected,
                                });

                                let replay_on_reject = self
                                    .zero_rtt_config
                                    .map(|config| config.replay_on_reject)
                                    .unwrap_or(false);
                                let journals = std::mem::take(&mut self.early_stream_journal);
                                self.control_stream = None;
                                self.data_streams.clear();
                                self.default_pub_stream = None;
                                self.default_sub_stream = None;
                                self.control_outgoing.clear();
                                self.pending_close = None;
                                self.control_finishing = false;
                                self.control_finished = false;

                                if replay_on_reject {
                                    Self::replay_early_stream_journals(
                                        conn,
                                        journals,
                                        parser_buffer_size,
                                        mqtt_version,
                                        max_packets,
                                        parse_level,
                                        &mut self.control_stream,
                                        &mut self.data_streams,
                                        &mut self.default_pub_stream,
                                        &mut self.default_sub_stream,
                                        &mut self.control_outgoing,
                                        &mut mqtt_events,
                                    );
                                }
                            }
                        }

                        // QUIC handshake done. Open the control stream for MQTT
                        // unless it already exists from 0-RTT or a rejected
                        // attempt intentionally left retry to the caller.
                        if self.control_stream.is_none() && !zero_rtt_was_attempted {
                            if let Some(stream_id) = conn.streams().open(Dir::Bi) {
                                self.control_stream = Some(stream_id);
                                if let Err(error) = self.mqtt_engine.connect() {
                                    mqtt_events.push(MqttEvent::Error(error));
                                }
                            }
                        }
                    }
                    quinn_proto::Event::ConnectionLost { reason } => {
                        // Drop transport-tied MQTT output so a stale packet is not
                        // replayed before CONNECT on the next transport.
                        self.mqtt_engine.reset_for_new_transport();
                        self.control_stream = None;
                        self.data_streams.clear();
                        self.default_pub_stream = None;
                        self.default_sub_stream = None;
                        self.control_outgoing.clear();
                        self.early_stream_journal.clear();
                        self.pending_close = None;
                        self.control_finishing = false;
                        self.control_finished = false;
                        // Surface the QUIC-level detail in addition to the
                        // MQTT-level Disconnected signal.
                        let (by_peer, error_code) = transport_close_details(&reason);
                        mqtt_events.push(MqttEvent::TransportClosed {
                            reason: reason.to_string(),
                            by_peer,
                            error_code,
                        });
                        mqtt_events.push(MqttEvent::Disconnected(None));
                    }
                    _ => {}
                }
            }

            // 2. Accept server-initiated bidirectional streams as data streams.
            while let Some(stream_id) = conn.streams().accept(Dir::Bi) {
                self.data_streams.entry(stream_id).or_insert_with(|| {
                    let mut stream = QuicStream::new(parser_buffer_size, mqtt_version, max_packets);
                    stream.parser.set_parse_level(parse_level);
                    stream
                });
            }

            // 3a. Read the control stream into the shared internal parser.
            if let Some(stream_id) = self.control_stream {
                let reset_code = {
                    let mut stream = conn.recv_stream(stream_id);
                    stream.received_reset().ok().flatten()
                };
                if let Some(error_code) = reset_code {
                    Self::handle_stream_reset_fields(
                        &mut self.control_stream,
                        &mut self.data_streams,
                        &mut self.default_pub_stream,
                        &mut self.default_sub_stream,
                        &mut self.control_outgoing,
                        &mut self.control_finishing,
                        &mut self.control_finished,
                        &mut self.mqtt_engine,
                        stream_id,
                        error_code,
                        &mut mqtt_events,
                    );
                }
            }
            if let Some(stream_id) = self.control_stream {
                let mut recv_finished = false;
                let mut read_reset = None;
                {
                    let mut stream = conn.recv_stream(stream_id);
                    // Bind the Result to a local so its temporary (which borrows
                    // `stream`) is dropped before `stream` itself.
                    let read_result = stream.read(true);
                    if let Ok(mut chunks) = read_result {
                        loop {
                            match chunks.next(16384) {
                                Ok(Some(chunk)) => {
                                    mqtt_events
                                        .extend(self.mqtt_engine.handle_incoming(&chunk.bytes));
                                }
                                Ok(None) => {
                                    recv_finished = true;
                                    break;
                                }
                                Err(quinn_proto::ReadError::Reset(error_code)) => {
                                    read_reset = Some(error_code);
                                    break;
                                }
                                Err(_) => break,
                            }
                        }
                    }
                }
                if let Some(error_code) = read_reset {
                    Self::handle_stream_reset_fields(
                        &mut self.control_stream,
                        &mut self.data_streams,
                        &mut self.default_pub_stream,
                        &mut self.default_sub_stream,
                        &mut self.control_outgoing,
                        &mut self.control_finishing,
                        &mut self.control_finished,
                        &mut self.mqtt_engine,
                        stream_id,
                        error_code,
                        &mut mqtt_events,
                    );
                } else if recv_finished {
                    Self::handle_stream_closed_fields(
                        &mut self.control_stream,
                        &mut self.data_streams,
                        &mut self.control_outgoing,
                        &mut self.control_finishing,
                        &mut self.control_finished,
                        &mut self.mqtt_engine,
                        stream_id,
                        "recv_finished",
                        true,
                        &mut mqtt_events,
                    );
                }
            }

            // 3b. Read each data stream through its own parser, feeding complete
            //     packets into the shared protocol state machine.
            let data_ids: Vec<StreamId> = self.data_streams.keys().copied().collect();
            let max_event_count = self.mqtt_engine.options().max_event_count;
            for stream_id in data_ids {
                if self.mqtt_engine.reliability.input_closed {
                    break;
                }
                if !self.data_streams.contains_key(&stream_id) {
                    continue;
                }

                // Back-pressure: stop reading inbound data once the application
                // event buffer is full (mirrors `handle_incoming`). Undecoded bytes
                // stay in quinn-proto's receive buffer, flow-controlling the peer,
                // and are processed on a later tick after events are drained.
                if mqtt_events.len() >= max_event_count {
                    break;
                }

                let reset_code = {
                    let mut stream = conn.recv_stream(stream_id);
                    stream.received_reset().ok().flatten()
                };
                if let Some(error_code) = reset_code {
                    Self::handle_stream_reset_fields(
                        &mut self.control_stream,
                        &mut self.data_streams,
                        &mut self.default_pub_stream,
                        &mut self.default_sub_stream,
                        &mut self.control_outgoing,
                        &mut self.control_finishing,
                        &mut self.control_finished,
                        &mut self.mqtt_engine,
                        stream_id,
                        error_code,
                        &mut mqtt_events,
                    );
                    continue;
                }

                // Back-pressure: if this stream's bounded send buffer is already
                // full, don't pull more inbound data this tick. quinn-proto then
                // stops granting the peer flow-control credit, so the buffer cannot
                // grow without bound while we are unable to emit the matching
                // QoS 1/2 acknowledgements.
                let at_capacity = self
                    .data_streams
                    .get(&stream_id)
                    .map(|ds| ds.pending_packets >= ds.max_packets)
                    .unwrap_or(true);
                if at_capacity {
                    continue;
                }

                let recv_closed = self
                    .data_streams
                    .get(&stream_id)
                    .map(|ds| ds.recv_closed)
                    .unwrap_or(true);
                if !recv_closed {
                    let mut recv_finished = false;
                    let mut read_reset = None;
                    {
                        let mut stream = conn.recv_stream(stream_id);
                        let read_result = stream.read(true);
                        if let Ok(mut chunks) = read_result {
                            loop {
                                match chunks.next(16384) {
                                    Ok(Some(chunk)) => {
                                        if let Some(ds) = self.data_streams.get_mut(&stream_id) {
                                            if self
                                                .mqtt_engine
                                                .options
                                                .max_incoming_buffer_bytes
                                                .is_some_and(|limit| {
                                                    ds.parser
                                                        .buffer_mut()
                                                        .len()
                                                        .saturating_add(chunk.bytes.len())
                                                        > limit
                                                })
                                            {
                                                self.mqtt_engine.fail_connection(
                                                    MqttClientError::BufferFull {
                                                        buffer_type: "QUIC incoming bytes".into(),
                                                        capacity: self
                                                            .mqtt_engine
                                                            .options
                                                            .max_incoming_buffer_bytes
                                                            .unwrap(),
                                                    },
                                                );
                                                mqtt_events.extend(self.mqtt_engine.take_events());
                                                ds.recv_closed = true;
                                                break;
                                            }
                                            ds.parser.feed(&chunk.bytes);
                                        }
                                    }
                                    Ok(None) => {
                                        recv_finished = true;
                                        break;
                                    }
                                    Err(quinn_proto::ReadError::Reset(error_code)) => {
                                        read_reset = Some(error_code);
                                        break;
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                    if let Some(error_code) = read_reset {
                        Self::handle_stream_reset_fields(
                            &mut self.control_stream,
                            &mut self.data_streams,
                            &mut self.default_pub_stream,
                            &mut self.default_sub_stream,
                            &mut self.control_outgoing,
                            &mut self.control_finishing,
                            &mut self.control_finished,
                            &mut self.mqtt_engine,
                            stream_id,
                            error_code,
                            &mut mqtt_events,
                        );
                        continue;
                    } else if recv_finished {
                        Self::handle_stream_closed_fields(
                            &mut self.control_stream,
                            &mut self.data_streams,
                            &mut self.control_outgoing,
                            &mut self.control_finishing,
                            &mut self.control_finished,
                            &mut self.mqtt_engine,
                            stream_id,
                            "recv_finished",
                            true,
                            &mut mqtt_events,
                        );
                    }
                }
                // `chunks`/`stream` are dropped here, releasing the connection
                // borrow needed to drive the MQTT engine. Disjoint field borrows
                // (data_streams + mqtt_engine) keep this valid while `conn` lives.
                Self::drain_data_stream(
                    &mut self.data_streams,
                    &mut self.mqtt_engine,
                    stream_id,
                    &mut mqtt_events,
                );
            }

            // 4a. Flush the control stream. `control_outgoing` is a persistent
            //     buffer: the engine's session packets are appended to it, then it
            //     is written with partial-write retention (like the data streams),
            //     so a flow-controlled or partial write never silently drops bytes
            //     such as a queued MQTT DISCONNECT.
            // Stop pulling new session output once a FIN has been requested or
            // applied — those bytes could never be written on the finished send
            // side and would block the deferred close below forever.
            if !self.control_finishing && !self.control_finished {
                let session_bytes = self.mqtt_engine.take_outgoing();
                Self::append_control_outgoing_bytes(
                    &mut self.control_outgoing,
                    &mut self.early_stream_journal,
                    self.zero_rtt_status,
                    self.control_stream,
                    &session_bytes,
                    0,
                );
            }
            if let Some(stream_id) = self.control_stream {
                let _ = conn.send_stream(stream_id).set_priority(i32::MAX);
                // Flush buffered bytes (partial-write retention) until the send side
                // is finished; once finished we no longer write.
                if !self.control_finished && !self.control_outgoing.is_empty() {
                    let mut stream = conn.send_stream(stream_id);
                    if let Ok(written) = stream.write(&self.control_outgoing) {
                        self.control_outgoing.drain(..written);
                    }
                }
                // Deferred FIN: finish only once the control buffer is fully
                // flushed, so no queued bytes are lost.
                if self.control_finishing
                    && !self.control_finished
                    && self.control_outgoing.is_empty()
                {
                    let _ = conn.send_stream(stream_id).finish();
                    self.control_finishing = false;
                    self.control_finished = true;
                }
            }

            // 4b. Flush each data stream's outgoing buffer, retaining any bytes
            //     that could not be written yet (stream-level back-pressure).
            for stream_id in Self::stream_send_order(&self.data_streams) {
                let ds = self.data_streams.get_mut(&stream_id).unwrap();
                let _ = conn
                    .send_stream(stream_id)
                    .set_priority(i32::from(ds.priority));
                if !ds.outgoing.is_empty() {
                    let mut stream = conn.send_stream(stream_id);
                    if let Ok(written) = stream.write(&ds.outgoing) {
                        ds.outgoing.drain(..written);
                    }
                    // Once fully drained the buffered-packet count is cleared,
                    // freeing capacity for new publishes/subscribes on this stream.
                    if ds.outgoing.is_empty() {
                        ds.pending_packets = 0;
                    }
                }
                // Deferred FIN: finish the send side only once all buffered bytes
                // have actually been written, so queued publishes are not lost.
                if ds.finishing && !ds.finished && ds.outgoing.is_empty() {
                    let _ = conn.send_stream(stream_id).finish();
                    ds.finishing = false;
                    ds.finished = true;
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

            // 6. Deferred graceful close: apply only once the control buffer
            //    (including the queued MQTT DISCONNECT) has been fully written and
            //    transmitted above. If the control stream is still draining under
            //    flow control, defer to a later tick so the DISCONNECT is not
            //    dropped. Then emit the CONNECTION_CLOSE after the DISCONNECT.
            if self.pending_close.is_some() && self.control_outgoing.is_empty() {
                if let Some((code, reason)) = self.pending_close.take() {
                    conn.close(now, code, reason);
                    self.mqtt_engine.handle_connection_lost();
                    let mut close_buf = Vec::new();
                    while let Some(transmit) = conn.poll_transmit(now, 1, &mut close_buf) {
                        self.outgoing_datagrams
                            .push_back((transmit.destination, close_buf.clone()));
                        close_buf.clear();
                    }
                }
            }
        }

        if self.is_connected() {
            if let Err(error) = self.route_session_work() {
                if !matches!(error, MqttClientError::BufferFull { .. }) {
                    mqtt_events.push(MqttEvent::Error(error));
                }
            }
        }

        // 7. Drive MqttEngine tick
        let tick_events = self.mqtt_engine.handle_tick(now);
        mqtt_events.extend(tick_events);

        // 8. Route per-stream (MQTT v3) retransmissions back onto their
        //    originating data stream; they are flushed on the next tick.
        //    Best-effort: skipped if the stream is gone or its buffer is full —
        //    the inflight timer will retry on a later tick.
        let retransmissions = self.mqtt_engine.take_stream_retransmissions();
        for (stream, bytes) in retransmissions {
            if let Some(stream_id) = self
                .data_streams
                .keys()
                .copied()
                .find(|id| u64::from(*id) == stream)
            {
                let _ = self.enqueue_on_stream(stream_id, &bytes);
            }
        }

        mqtt_events
    }

    pub fn take_outgoing_datagrams(&mut self) -> VecDeque<(std::net::SocketAddr, Vec<u8>)> {
        std::mem::take(&mut self.outgoing_datagrams)
    }

    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        let mut events: Vec<MqttEvent> = self.pending_transport_events.drain(..).collect();
        events.extend(self.mqtt_engine.take_events());
        events
    }

    /// Open a new client-initiated bidirectional QUIC data stream.
    ///
    /// Returns an opaque handle (the raw QUIC stream id) that can be passed to
    /// [`publish_on`](Self::publish_on), [`subscribe_on`](Self::subscribe_on) and
    /// [`unsubscribe_on`](Self::unsubscribe_on). A stream becomes a "pub stream"
    /// or "sub stream" simply by virtue of the traffic routed onto it; the broker
    /// replies (SUBACK, and QoS 1/2 acknowledgements) on the same stream.
    ///
    /// Fails if the connection has not been established yet, or if the peer's
    /// stream limit (`initial_max_streams_bidi`) has been reached.
    pub fn open_data_stream(&mut self) -> Result<u64, MqttClientError> {
        Ok(self
            .open_bidi_stream_with_role(EarlyStreamRole::ExplicitData)?
            .into())
    }

    /// Internal: open a bidirectional stream and register per-stream state.
    fn open_bidi_stream_with_role(
        &mut self,
        role: EarlyStreamRole,
    ) -> Result<StreamId, MqttClientError> {
        self.ensure_command_allowed_before_mqtt_connected()?;

        let parser_buffer_size = self.mqtt_engine.options().parser_buffer_size;
        let mqtt_version = self.mqtt_engine.mqtt_version();
        let max_packets = self.mqtt_engine.options().max_outgoing_packet_count;
        let parse_level = self.mqtt_engine.parse_level();

        let conn = self
            .connection
            .as_mut()
            .ok_or_else(|| MqttClientError::InvalidState {
                expected: "an established QUIC connection".to_string(),
                actual: "no connection".to_string(),
            })?;

        let stream_id =
            conn.streams()
                .open(Dir::Bi)
                .ok_or_else(|| MqttClientError::InternalError {
                    message: "QUIC stream limit reached: cannot open new data stream".to_string(),
                })?;

        let mut stream = QuicStream::new(parser_buffer_size, mqtt_version, max_packets);
        stream.parser.set_parse_level(parse_level);
        if let EarlyStreamRole::PriorityPub(priority) = role {
            stream.priority = priority;
            stream.publish_priority = Some(priority);
        }
        self.data_streams.insert(stream_id, stream);
        if self.zero_rtt_status == QuicZeroRttStatus::Attempted {
            Self::journal_stream_open(&mut self.early_stream_journal, stream_id, role);
        }
        Ok(stream_id)
    }

    /// Resolve a public stream handle to a live data-stream id, rejecting unknown
    /// handles and the control stream.
    fn resolve_stream(&self, handle: u64) -> Result<StreamId, MqttClientError> {
        self.data_streams
            .keys()
            .copied()
            .find(|id| u64::from(*id) == handle)
            .ok_or_else(|| MqttClientError::InvalidState {
                expected: "a handle from open_data_stream".to_string(),
                actual: format!("unknown data stream {}", handle),
            })
    }

    /// Reject the operation if the target stream's outgoing buffer is already at
    /// its packet-count limit, so callers do not allocate packet ids / inflight
    /// slots for a packet that cannot be buffered.
    fn ensure_stream_capacity(&self, stream_id: StreamId) -> Result<(), MqttClientError> {
        if let Some(ds) = self.data_streams.get(&stream_id) {
            if ds.finishing || ds.finished {
                return Err(stream_finished_error());
            }
            if ds.pending_packets >= ds.max_packets {
                return Err(MqttClientError::BufferFull {
                    buffer_type: "quic_stream_outgoing".to_string(),
                    capacity: ds.max_packets,
                });
            }
        }
        Ok(())
    }

    /// Append one already-encoded packet to a data stream's bounded outgoing
    /// buffer, returning [`MqttClientError::BufferFull`] if the stream is at its
    /// packet-count limit, or an error if the stream has been finished/reset.
    fn enqueue_on_stream(
        &mut self,
        stream_id: StreamId,
        bytes: &[u8],
    ) -> Result<(), MqttClientError> {
        self.check_stream_bytes(bytes.len())?;
        if let Some(ds) = self.data_streams.get_mut(&stream_id) {
            if ds.finishing || ds.finished {
                return Err(stream_finished_error());
            }
            if ds.pending_packets >= ds.max_packets {
                return Err(MqttClientError::BufferFull {
                    buffer_type: "quic_stream_outgoing".to_string(),
                    capacity: ds.max_packets,
                });
            }
            ds.outgoing.extend_from_slice(bytes);
            ds.pending_packets += 1;
            if self.zero_rtt_status == QuicZeroRttStatus::Attempted {
                Self::journal_stream_bytes(&mut self.early_stream_journal, stream_id, bytes, 1);
            }
        }
        Ok(())
    }

    /// Decode and process complete packets buffered on a data stream's parser,
    /// routing any protocol response (PUBACK/PUBREC/PUBREL/PUBCOMP) back onto the
    /// *same* stream and counting it against the stream's bounded send buffer.
    ///
    /// Stops once the send buffer reaches `max_packets`, leaving the undecoded
    /// bytes in the parser for a later tick (back-pressure). This is an associated
    /// function taking the two fields directly so it can run while the QUIC
    /// `connection` field is mutably borrowed elsewhere in `handle_tick`.
    fn drain_data_stream(
        data_streams: &mut HashMap<StreamId, QuicStream>,
        mqtt_engine: &mut MqttEngine,
        stream_id: StreamId,
        events: &mut Vec<MqttEvent>,
    ) {
        let max_event_count = mqtt_engine.options().max_event_count;
        loop {
            if mqtt_engine.reliability.input_closed {
                if let Some(ds) = data_streams.get_mut(&stream_id) {
                    ds.parser.buffer_mut().clear();
                }
                break;
            }
            // Back-pressure: stop once the event buffer is full (mirrors
            // `handle_incoming`). This bounds the number of events produced in a
            // single tick even for packets that emit events but no response bytes
            // (e.g. a stream full of QoS 0 PUBLISHes). The undecoded bytes remain
            // in this stream's parser for a later tick.
            if events.len() >= max_event_count {
                break;
            }

            if let Some(ds) = data_streams.get_mut(&stream_id) {
                if let Err(error) =
                    MqttEngine::validate_input_buffer(mqtt_engine.options(), ds.parser.buffer_mut())
                {
                    mqtt_engine.fail_connection(error);
                    events.extend(mqtt_engine.take_events());
                    ds.recv_closed = true;
                    break;
                }
            }

            if let Some(capacity) = mqtt_engine.options.max_outgoing_buffer_bytes {
                let first = data_streams
                    .get_mut(&stream_id)
                    .and_then(|ds| ds.parser.buffer_mut().first().copied());
                let needs_response = first.is_some_and(|byte| {
                    matches!(byte >> 4, 5 | 6)
                        || (byte >> 4 == 3 && byte & 6 != 0 && mqtt_engine.options.auto_ack)
                });
                if needs_response {
                    let used = mqtt_engine.outgoing_bytes()
                        + data_streams
                            .values()
                            .map(|ds| ds.outgoing.len())
                            .sum::<usize>();
                    if used.saturating_add(6) > capacity {
                        break;
                    }
                }
            }

            // Fetch the next packet only while there is buffer capacity; drops the
            // `data_streams` borrow before calling into the engine.
            let packet = match data_streams.get_mut(&stream_id) {
                Some(ds) if !ds.finished && ds.pending_packets < ds.max_packets => {
                    match ds.parser.next_parsed() {
                        Ok(Some(packet)) => packet,
                        Ok(None) => break,
                        Err(e) => {
                            events.push(MqttEvent::Error(MqttClientError::from(e)));
                            break;
                        }
                    }
                }
                // Unknown stream, or send buffer full → stop (back-pressure).
                _ => break,
            };

            let resp =
                mqtt_engine.ingest_parsed_stream_packet_into(packet, u64::from(stream_id), events);
            if !resp.is_empty() {
                if let Some(ds) = data_streams.get_mut(&stream_id) {
                    // If the send side has been finished/reset we can no longer
                    // transmit on this stream; surface the message event but drop
                    // the unsendable response rather than letting it accumulate.
                    if !ds.finished {
                        ds.outgoing.extend_from_slice(&resp);
                        ds.pending_packets += 1;
                    }
                }
            }
        }
    }

    /// Set the scheduling priority of a data stream. Higher values are sent first.
    pub fn set_stream_priority(
        &mut self,
        stream: u64,
        priority: u8,
    ) -> Result<(), MqttClientError> {
        let stream_id = self.resolve_stream(stream)?;
        let ds = self.data_streams.get_mut(&stream_id).unwrap();
        if ds.finishing || ds.finished {
            return Err(stream_finished_error());
        }
        ds.priority = priority;
        if ds.publish_priority.is_some() {
            ds.publish_priority = Some(priority);
        }
        if let Some(journal) = self
            .early_stream_journal
            .iter_mut()
            .find(|entry| entry.original_stream_id == stream_id)
        {
            journal.priority = priority;
            if matches!(journal.role, EarlyStreamRole::PriorityPub(_)) {
                journal.role = EarlyStreamRole::PriorityPub(priority);
            }
        }
        Ok(())
    }

    fn ensure_priority_pub_stream(&mut self, priority: u8) -> Result<StreamId, MqttClientError> {
        if priority == 128 {
            return self.ensure_default_pub_stream();
        }
        if let Some((&id, _)) = self
            .data_streams
            .iter()
            .find(|(_, ds)| ds.publish_priority == Some(priority) && !ds.finishing && !ds.finished)
        {
            return Ok(id);
        }
        self.open_bidi_stream_with_role(EarlyStreamRole::PriorityPub(priority))
    }

    fn stream_send_order(streams: &HashMap<StreamId, QuicStream>) -> Vec<StreamId> {
        let mut order: Vec<_> = streams.keys().copied().collect();
        order.sort_by_key(|id| (std::cmp::Reverse(streams[id].priority), u64::from(*id)));
        order
    }

    fn route_session_work(&mut self) -> Result<(), MqttClientError> {
        if self.mqtt_engine.configured_subscriptions_pending {
            let command = SubscribeCommand::new(
                None,
                self.mqtt_engine.options().subscription_topics.clone(),
                vec![],
            );
            self.subscribe(command)?;
            self.mqtt_engine.configured_subscriptions_pending = false;
        }
        while let Some((packet_id, packet)) = self.mqtt_engine.session_replay.front().cloned() {
            if matches!(packet, MqttPacket::Publish5(_) | MqttPacket::Publish3(_))
                && !self.mqtt_engine.inflight_queue.can_push_publish()
            {
                break;
            }
            let stream = self.ensure_default_pub_stream()?;
            self.ensure_stream_capacity(stream)?;
            let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
            self.mqtt_engine.validate_packet_capabilities(&packet)?;
            self.mqtt_engine.check_packet_size(&bytes)?;
            self.enqueue_on_stream(stream, &bytes)?;
            let qos = match &packet {
                MqttPacket::Publish5(p) => p.qos,
                MqttPacket::Publish3(p) => p.qos,
                _ => 2,
            };
            self.mqtt_engine.inflight_queue.push_with_stream(
                packet_id,
                packet,
                qos,
                Some(stream.into()),
            )?;
            self.mqtt_engine
                .start_deadline(OperationKind::Publish, Some(packet_id));
            self.mqtt_engine.session_replay.pop_front();
        }
        Ok(())
    }

    /// Lazily open (once) the default data stream used by [`publish`](Self::publish).
    fn ensure_default_pub_stream(&mut self) -> Result<StreamId, MqttClientError> {
        if let Some(id) = self.default_pub_stream {
            return Ok(id);
        }
        let id = self.open_bidi_stream_with_role(EarlyStreamRole::DefaultPub)?;
        self.default_pub_stream = Some(id);
        Ok(id)
    }

    /// Lazily open (once) the default data stream used by
    /// [`subscribe`](Self::subscribe) / [`unsubscribe`](Self::unsubscribe).
    fn ensure_default_sub_stream(&mut self) -> Result<StreamId, MqttClientError> {
        if let Some(id) = self.default_sub_stream {
            return Ok(id);
        }
        let id = self.open_bidi_stream_with_role(EarlyStreamRole::DefaultSub)?;
        self.default_sub_stream = Some(id);
        Ok(id)
    }

    /// Publish on the default pub data stream, opening it on first use.
    ///
    /// PUBLISH traffic is never placed on the session control stream. The QoS 1/2
    /// acknowledgement handshake completes on this same stream.
    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.ensure_command_allowed_before_mqtt_connected()?;
        self.preflight_publish(&command)?;
        let stream_id = self.ensure_priority_pub_stream(command.priority)?;
        self.ensure_stream_capacity(stream_id)?;
        let (pid, bytes) = self
            .mqtt_engine
            .publish_encoded(command, Some(u64::from(stream_id)))?;
        self.enqueue_on_stream(stream_id, &bytes)?;
        Ok(pid)
    }

    /// Publish onto a specific data stream previously returned by
    /// [`open_data_stream`](Self::open_data_stream).
    ///
    /// Packet-id allocation and QoS 1/2 inflight tracking are still handled by the
    /// shared MQTT state machine, so acknowledgements are processed normally. The
    /// entire QoS 1/2 handshake (PUBACK, or PUBREC/PUBREL/PUBCOMP) completes on
    /// this same stream — there is no cross-stream firing.
    pub fn publish_on(
        &mut self,
        stream: u64,
        command: PublishCommand,
    ) -> Result<Option<u16>, MqttClientError> {
        self.ensure_command_allowed_before_mqtt_connected()?;
        self.preflight_publish(&command)?;
        let stream_id = self.resolve_stream(stream)?;
        self.ensure_stream_capacity(stream_id)?;
        let (pid, bytes) = self.mqtt_engine.publish_encoded(command, Some(stream))?;
        self.enqueue_on_stream(stream_id, &bytes)?;
        Ok(pid)
    }

    /// Subscribe on the default sub data stream, opening it on first use.
    ///
    /// SUBSCRIBE traffic is never placed on the session control stream; the SUBACK
    /// and any messages delivered for the subscription flow on this same stream.
    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.ensure_command_allowed_before_mqtt_connected()?;
        self.preflight_subscribe(&command)?;
        let stream_id = self.ensure_default_sub_stream()?;
        self.ensure_stream_capacity(stream_id)?;
        let (pid, bytes) = self
            .mqtt_engine
            .subscribe_encoded(command, Some(u64::from(stream_id)))?;
        self.enqueue_on_stream(stream_id, &bytes)?;
        Ok(pid)
    }

    /// Subscribe onto a specific data stream. The SUBACK and delivered messages
    /// for this subscription arrive on the same stream.
    pub fn subscribe_on(
        &mut self,
        stream: u64,
        command: SubscribeCommand,
    ) -> Result<u16, MqttClientError> {
        self.ensure_command_allowed_before_mqtt_connected()?;
        self.preflight_subscribe(&command)?;
        let stream_id = self.resolve_stream(stream)?;
        self.ensure_stream_capacity(stream_id)?;
        let (pid, bytes) = self.mqtt_engine.subscribe_encoded(command, Some(stream))?;
        self.enqueue_on_stream(stream_id, &bytes)?;
        Ok(pid)
    }

    /// Unsubscribe on the default sub data stream.
    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.ensure_command_allowed_before_mqtt_connected()?;
        self.preflight_unsubscribe(&command)?;
        let stream_id = self.ensure_default_sub_stream()?;
        self.ensure_stream_capacity(stream_id)?;
        let (pid, bytes) = self
            .mqtt_engine
            .unsubscribe_encoded(command, Some(u64::from(stream_id)))?;
        self.enqueue_on_stream(stream_id, &bytes)?;
        Ok(pid)
    }

    /// Unsubscribe onto a specific data stream.
    pub fn unsubscribe_on(
        &mut self,
        stream: u64,
        command: UnsubscribeCommand,
    ) -> Result<u16, MqttClientError> {
        self.ensure_command_allowed_before_mqtt_connected()?;
        self.preflight_unsubscribe(&command)?;
        let stream_id = self.resolve_stream(stream)?;
        self.ensure_stream_capacity(stream_id)?;
        let (pid, bytes) = self
            .mqtt_engine
            .unsubscribe_encoded(command, Some(stream))?;
        self.enqueue_on_stream(stream_id, &bytes)?;
        Ok(pid)
    }

    /// Number of active data streams, excluding the control stream.
    pub fn data_stream_count(&self) -> usize {
        self.data_streams.len()
    }

    // --- Connection close controls ---

    /// Gracefully close the QUIC connection immediately, sending a
    /// CONNECTION_CLOSE frame with the given application error code and reason.
    ///
    /// Use `error_code = 0` for a normal close. This is an **immediate** QUIC
    /// close: quinn-proto abandons any unacknowledged stream data, so it does not
    /// guarantee delivery of buffered bytes — use [`disconnect_and_close`](Self::disconnect_and_close)
    /// when an MQTT DISCONNECT must reach the peer first. The connection enters its
    /// closing state and the CONNECTION_CLOSE frame is emitted on the next
    /// [`handle_tick`](Self::handle_tick); the MQTT session is marked disconnected.
    pub fn close(&mut self, error_code: u64, reason: &[u8]) -> Result<(), MqttClientError> {
        let code = quic_error_code(error_code)?;
        // Supersede any deferred close so we don't close twice.
        self.pending_close = None;
        let conn = self.require_connection()?;
        conn.close(Instant::now(), code, bytes::Bytes::copy_from_slice(reason));
        self.mqtt_engine.handle_connection_lost();
        Ok(())
    }

    /// Send an MQTT DISCONNECT on the control stream, then gracefully close the
    /// QUIC connection **after** the DISCONNECT has been transmitted.
    ///
    /// The QUIC close is deferred to [`handle_tick`](Self::handle_tick): the queued
    /// DISCONNECT is written and put on the wire first, then CONNECTION_CLOSE is
    /// emitted, so the peer has a chance to observe the MQTT-level disconnect.
    /// (Delivery is still best-effort — QUIC does not wait for the DISCONNECT to be
    /// acknowledged.) The MQTT session is marked disconnected immediately.
    pub fn disconnect_and_close(
        &mut self,
        error_code: u64,
        reason: &[u8],
    ) -> Result<(), MqttClientError> {
        self.disconnect_and_close_with(0, Vec::new(), error_code, reason)
    }

    pub fn disconnect_and_close_with(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
        error_code: u64,
        reason: &[u8],
    ) -> Result<(), MqttClientError> {
        let code = quic_error_code(error_code)?;
        // Validate a connection exists before queuing anything.
        let _ = self.require_connection()?;
        // Queue the DISCONNECT first; if the outgoing buffer is full, surface the
        // error and do NOT arm the close, so we never close the QUIC connection
        // having silently dropped the MQTT DISCONNECT. (No-op if not connected.)
        self.mqtt_engine
            .try_disconnect_with(reason_code, properties)?;
        self.pending_close = Some((code, bytes::Bytes::copy_from_slice(reason)));
        Ok(())
    }

    /// Whether the deferred DISCONNECT and QUIC close have been driven.
    pub fn disconnect_complete(&self) -> bool {
        self.pending_close.is_none() && self.connection.as_ref().is_none_or(|conn| conn.is_closed())
    }

    /// Silently/abruptly drop the QUIC connection without sending CONNECTION_CLOSE.
    ///
    /// The peer is left to detect the loss via its idle timeout. Useful for
    /// simulating dead-peer / abrupt-disconnect scenarios. All local stream state
    /// is cleared. Connection parameters are retained so [`reconnect`](Self::reconnect)
    /// still works afterwards.
    pub fn close_silent(&mut self) {
        self.reset_connection_state();
    }

    // --- Per-stream controls ---

    /// Cleanly finish (FIN) the send side of a stream — the control stream or a
    /// data stream.
    ///
    /// The FIN is **deferred** to [`handle_tick`](Self::handle_tick) and applied
    /// only once the stream's buffered outbound bytes have been written, so bytes
    /// queued by `publish_on`/`send_raw_on` before the next tick are not lost.
    /// After finishing, further writes to the stream are rejected.
    ///
    /// Finishing the **control stream** ends the MQTT session: the layer is marked
    /// disconnected so it produces no further keep-alive/control packets that could
    /// never be sent on the finished send side.
    pub fn finish_stream(&mut self, stream: u64) -> Result<(), MqttClientError> {
        let stream_id = self.resolve_any_stream(stream)?;
        // Validate a connection exists; the FIN itself is applied on the next tick.
        let _ = self.require_connection()?;
        if Some(stream_id) == self.control_stream {
            // Capture anything the engine has already queued so it is flushed
            // before the FIN, then stop the MQTT layer from producing more control
            // traffic (auto-PINGREQ, etc.).
            let pending = self.mqtt_engine.take_outgoing();
            Self::append_control_outgoing_bytes(
                &mut self.control_outgoing,
                &mut self.early_stream_journal,
                self.zero_rtt_status,
                self.control_stream,
                &pending,
                0,
            );
            self.mqtt_engine.handle_connection_lost();
            self.control_finishing = true;
        } else if let Some(ds) = self.data_streams.get_mut(&stream_id) {
            ds.finishing = true;
        }
        Ok(())
    }

    /// Reset (RESET_STREAM) the send side of a stream with the given error code,
    /// discarding any buffered outbound bytes. After resetting, further writes to
    /// the stream are rejected.
    pub fn reset_stream(&mut self, stream: u64, error_code: u64) -> Result<(), MqttClientError> {
        let stream_id = self.resolve_any_stream(stream)?;
        let code = quic_error_code(error_code)?;
        if Some(stream_id) == self.control_stream {
            self.control_outgoing.clear();
            self.control_finishing = false;
            self.control_finished = true;
            // Resetting the control send side ends the MQTT session.
            self.mqtt_engine.handle_connection_lost();
        } else if let Some(ds) = self.data_streams.get_mut(&stream_id) {
            ds.outgoing.clear();
            ds.pending_packets = 0;
            ds.finishing = false;
            ds.finished = true;
        }
        let conn = self.require_connection()?;
        conn.send_stream(stream_id)
            .reset(code)
            .map_err(|e| MqttClientError::InternalError {
                message: format!("QUIC stream reset failed: {:?}", e),
            })
    }

    /// Ask the peer to stop sending on a stream (STOP_SENDING) with the given
    /// error code.
    pub fn stop_stream(&mut self, stream: u64, error_code: u64) -> Result<(), MqttClientError> {
        let stream_id = self.resolve_any_stream(stream)?;
        let code = quic_error_code(error_code)?;
        let conn = self.require_connection()?;
        conn.recv_stream(stream_id)
            .stop(code)
            .map_err(|e| MqttClientError::InternalError {
                message: format!("QUIC stream stop failed: {:?}", e),
            })
    }

    // --- Keep-alive / ping ---

    /// Queue an MQTT PINGREQ on the control stream (manual keep-alive).
    ///
    /// Rejected until the MQTT session is connected (CONNACK received), so a
    /// PINGREQ can never be queued ahead of CONNECT on a fresh transport, and
    /// rejected once the control stream's send side has been finished or reset.
    pub fn ping(&mut self) -> Result<(), MqttClientError> {
        if self.control_finishing || self.control_finished {
            return Err(stream_finished_error());
        }
        if !self.mqtt_engine.is_connected() {
            return Err(MqttClientError::InvalidState {
                expected: "a connected MQTT session".to_string(),
                actual: "not connected".to_string(),
            });
        }
        // Propagate a full outgoing buffer instead of silently dropping the ping.
        self.mqtt_engine.try_send_ping()
    }

    /// Send a QUIC-level PING frame (transport keep-alive), independent of MQTT.
    pub fn quic_ping(&mut self) -> Result<(), MqttClientError> {
        self.require_connection()?.ping();
        Ok(())
    }

    // --- Internal helpers shared by the controls above ---

    fn require_connection(&mut self) -> Result<&mut Connection, MqttClientError> {
        self.connection
            .as_mut()
            .ok_or_else(|| MqttClientError::InvalidState {
                expected: "an established QUIC connection".to_string(),
                actual: "no connection".to_string(),
            })
    }

    /// Resolve a handle to either the control stream or a known data stream.
    fn resolve_any_stream(&self, handle: u64) -> Result<StreamId, MqttClientError> {
        if Some(handle) == self.control_stream.map(u64::from) {
            return Ok(self.control_stream.unwrap());
        }
        self.resolve_stream(handle)
    }

    /// The control stream handle, available once the QUIC handshake has completed.
    ///
    /// Can be passed to [`send_raw_on`](Self::send_raw_on) /
    /// [`send_packet_on`](Self::send_packet_on) to target the control stream
    /// directly (including with packets that do not normally belong there).
    pub fn control_stream_id(&self) -> Option<u64> {
        self.control_stream.map(u64::from)
    }

    // --- Low-level escape hatch (negative / conformance testing) ---
    //
    // The routing helpers above keep traffic spec-correct by default. flowSDK is
    // also used to exercise *bad* behaviour — e.g. sending a CONNECT on a data
    // stream, or PUBLISH on the control stream — so the following middle-layer
    // API deliberately bypasses all routing rules and protocol-state bookkeeping.

    /// Write raw bytes onto an arbitrary stream (the control stream or any data
    /// stream), bypassing all MQTT routing rules and protocol state.
    ///
    /// Intended for negative testing: malformed frames, packets sent on the
    /// "wrong" stream, etc. The bytes are buffered on the target stream and
    /// flushed on the next [`handle_tick`](Self::handle_tick).
    ///
    /// Errors if `stream` is neither the control stream nor a known data stream,
    /// or if the stream's send side has already been finished or reset.
    pub fn send_raw_on(&mut self, stream: u64, bytes: &[u8]) -> Result<(), MqttClientError> {
        if Some(stream) == self.control_stream.map(u64::from) {
            if self.control_finishing || self.control_finished {
                return Err(stream_finished_error());
            }
            Self::append_control_outgoing_bytes(
                &mut self.control_outgoing,
                &mut self.early_stream_journal,
                self.zero_rtt_status,
                self.control_stream,
                bytes,
                0,
            );
            return Ok(());
        }
        let stream_id = self.resolve_stream(stream)?;
        if let Some(ds) = self.data_streams.get_mut(&stream_id) {
            if ds.finishing || ds.finished {
                return Err(stream_finished_error());
            }
            ds.outgoing.extend_from_slice(bytes);
            if self.zero_rtt_status == QuicZeroRttStatus::Attempted {
                Self::journal_stream_bytes(&mut self.early_stream_journal, stream_id, bytes, 0);
            }
        }
        Ok(())
    }

    /// Encode an arbitrary MQTT packet and write it onto an arbitrary stream,
    /// bypassing routing rules. Unlike the high-level methods this does **not**
    /// allocate packet ids or track inflight state, so the caller has full control
    /// (e.g. to send `MqttPacket::Connect5(..)` on a data stream).
    pub fn send_packet_on(
        &mut self,
        stream: u64,
        packet: MqttPacket,
    ) -> Result<(), MqttClientError> {
        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.send_raw_on(stream, &bytes)
    }

    /// Queue a manual receive acknowledgement with normal stream backpressure.
    pub fn acknowledge_on(
        &mut self,
        stream: u64,
        packet: MqttPacket,
    ) -> Result<(), MqttClientError> {
        let (id, kind, reason, properties) = match packet {
            MqttPacket::PubAck5(p) => (p.packet_id, 4, p.reason_code, p.properties),
            MqttPacket::PubRec5(p) => (p.packet_id, 5, p.reason_code, p.properties),
            MqttPacket::PubComp5(p) => (p.packet_id, 7, p.reason_code, p.properties),
            MqttPacket::PubAck3(p) => (p.message_id, 4, 0, vec![]),
            MqttPacket::PubRec3(p) => (p.message_id, 5, 0, vec![]),
            MqttPacket::PubComp3(p) => (p.message_id, 7, 0, vec![]),
            _ => {
                return Err(MqttClientError::ProtocolViolation {
                    message: "Expected a receive acknowledgement".into(),
                })
            }
        };
        let control = Some(stream) == self.control_stream.map(u64::from);
        let channel = if control { None } else { Some(stream) };
        let packet = self
            .mqtt_engine
            .manual_ack_packet(id, kind, reason, properties, channel)?;
        if control {
            if self.control_finishing || self.control_finished {
                return Err(stream_finished_error());
            }
            self.mqtt_engine.enqueue_packet(packet)?;
        } else {
            let stream_id = self.resolve_stream(stream)?;
            self.ensure_stream_capacity(stream_id)?;
            let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
            self.mqtt_engine.check_packet_size(&bytes)?;
            self.enqueue_on_stream(stream_id, &bytes)?;
        }
        self.mqtt_engine.commit_receive_ack(id, kind, reason);
        Ok(())
    }

    /// Send a success PUBACK on a specific QUIC stream.
    ///
    /// This is a typed convenience wrapper around [`send_packet_on`](Self::send_packet_on)
    /// for manual-ack tests. It does not update protocol/inflight state.
    pub fn puback_on(&mut self, stream: u64, packet_id: u16) -> Result<(), MqttClientError> {
        self.send_packet_on(stream, self.puback_packet(packet_id))
    }

    /// Send a success PUBREC on a specific QUIC stream.
    ///
    /// This is a typed convenience wrapper around [`send_packet_on`](Self::send_packet_on)
    /// for manual-ack tests. It does not update protocol/inflight state.
    pub fn pubrec_on(&mut self, stream: u64, packet_id: u16) -> Result<(), MqttClientError> {
        self.send_packet_on(stream, self.pubrec_packet(packet_id))
    }

    /// Send a success PUBREL on a specific QUIC stream.
    ///
    /// This is a typed convenience wrapper around [`send_packet_on`](Self::send_packet_on)
    /// for manual-ack tests. It does not update protocol/inflight state.
    pub fn pubrel_on(&mut self, stream: u64, packet_id: u16) -> Result<(), MqttClientError> {
        self.send_packet_on(stream, self.pubrel_packet(packet_id))
    }

    /// Send a success PUBCOMP on a specific QUIC stream.
    ///
    /// This is a typed convenience wrapper around [`send_packet_on`](Self::send_packet_on)
    /// for manual-ack tests. It does not update protocol/inflight state.
    pub fn pubcomp_on(&mut self, stream: u64, packet_id: u16) -> Result<(), MqttClientError> {
        self.send_packet_on(stream, self.pubcomp_packet(packet_id))
    }

    fn puback_packet(&self, packet_id: u16) -> MqttPacket {
        if self.mqtt_engine.mqtt_version() == 5 {
            MqttPacket::PubAck5(MqttPubAck::new(packet_id, 0, Vec::new()))
        } else {
            MqttPacket::PubAck3(crate::mqtt_serde::mqttv3::puback::MqttPubAck::new(
                packet_id,
            ))
        }
    }

    fn pubrec_packet(&self, packet_id: u16) -> MqttPacket {
        if self.mqtt_engine.mqtt_version() == 5 {
            MqttPacket::PubRec5(MqttPubRec::new(packet_id, 0, Vec::new()))
        } else {
            MqttPacket::PubRec3(crate::mqtt_serde::mqttv3::pubrec::MqttPubRec::new(
                packet_id,
            ))
        }
    }

    fn pubrel_packet(&self, packet_id: u16) -> MqttPacket {
        if self.mqtt_engine.mqtt_version() == 5 {
            MqttPacket::PubRel5(MqttPubRel::new(packet_id, 0, Vec::new()))
        } else {
            MqttPacket::PubRel3(crate::mqtt_serde::mqttv3::pubrel::MqttPubRel::new(
                packet_id,
            ))
        }
    }

    fn pubcomp_packet(&self, packet_id: u16) -> MqttPacket {
        if self.mqtt_engine.mqtt_version() == 5 {
            MqttPacket::PubComp5(MqttPubComp::new(packet_id, 0, Vec::new()))
        } else {
            MqttPacket::PubComp3(crate::mqtt_serde::mqttv3::pubcomp::MqttPubComp::new(
                packet_id,
            ))
        }
    }

    /// Borrow the underlying sans-I/O MQTT protocol engine (the "middle layer").
    ///
    /// Exposed so tests can drive the protocol state machine directly — inspect
    /// state, or enqueue packets that the routing helpers would place elsewhere.
    pub fn engine(&self) -> &MqttEngine {
        &self.mqtt_engine
    }

    /// Mutably borrow the underlying sans-I/O MQTT protocol engine.
    ///
    /// Note: packets enqueued via [`MqttEngine::enqueue_packet`] flow on the
    /// control stream. To place arbitrary packets on a data stream, encode them
    /// and use [`send_packet_on`](Self::send_packet_on) / [`send_raw_on`](Self::send_raw_on).
    pub fn engine_mut(&mut self) -> &mut MqttEngine {
        &mut self.mqtt_engine
    }

    /// Delegate: Queue a DISCONNECT packet.
    pub fn disconnect(&mut self) -> Result<(), MqttClientError> {
        self.mqtt_engine.disconnect()
    }

    /// Check if the MQTT session is connected.
    pub fn is_connected(&self) -> bool {
        self.mqtt_engine.is_connected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn establish_connection(engine: &mut MqttEngine) {
        engine.connect().unwrap();
        engine.take_outgoing();
        let connack: &[u8] = if engine.mqtt_version() == 5 {
            &[0x20, 3, 0, 0, 0]
        } else {
            &[0x20, 2, 0, 0]
        };
        let events = engine.handle_incoming(connack);
        assert!(engine.is_connected(), "{events:?}");
        assert!(matches!(events.as_slice(), [MqttEvent::Connected(_)]));
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn closed_mqtt_connection_discards_quic_stream_input() {
        use quinn_proto::{Dir, Side, StreamId};
        let mut engine = QuicMqttEngine::new(MqttClientOptions::default()).unwrap();
        establish_connection(&mut engine.mqtt_engine);
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        let mut stream = QuicStream::new(1024, 5, 10);
        let publication = MqttPacket::Publish5(MqttPublish::new(
            1,
            "t".into(),
            Some(7),
            vec![],
            false,
            false,
        ));
        stream.parser.feed(&publication.to_bytes().unwrap());
        engine.data_streams.insert(data, stream);
        engine.mqtt_engine.disconnect().unwrap();
        engine.mqtt_engine.take_outgoing();
        let mut events = Vec::new();
        QuicMqttEngine::drain_data_stream(
            &mut engine.data_streams,
            &mut engine.mqtt_engine,
            data,
            &mut events,
        );
        assert!(events.is_empty());
        let stream = engine.data_streams.get_mut(&data).unwrap();
        assert!(stream.parser.buffer_mut().is_empty());
        assert!(stream.outgoing.is_empty());
    }
    use crate::mqtt_client::opts::MqttClientOptions;
    use crate::mqtt_serde::mqttv5::pingreqv5;

    #[cfg(feature = "quic-proto")]
    #[test]
    fn quic_byte_limit_rejects_before_reserving_packet_id() {
        use quinn_proto::{Dir, Side, StreamId};
        let mut engine = QuicMqttEngine::new(
            MqttClientOptions::builder()
                .max_outgoing_buffer_bytes(128)
                .build(),
        )
        .unwrap();
        engine.mqtt_engine.connect().unwrap();
        engine.mqtt_engine.take_outgoing();
        engine.mqtt_engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
        let stream = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(stream, QuicStream::new(1024, 5, 10));
        let mut command = PublishCommand::simple("data", vec![1; 256], 1, false);
        command.packet_id = Some(7);
        assert!(matches!(
            engine.publish_on(stream.into(), command.clone()),
            Err(MqttClientError::BufferFull { .. })
        ));
        command.payload.clear();
        assert_eq!(engine.publish_on(stream.into(), command).unwrap(), Some(7));
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn quic_recovery_observes_new_broker_quota() {
        use quinn_proto::{Dir, Side, StreamId};
        let mut engine = QuicMqttEngine::new(MqttClientOptions::default()).unwrap();
        engine.mqtt_engine.connect().unwrap();
        engine.mqtt_engine.take_outgoing();
        engine.mqtt_engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
        let stream = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(stream, QuicStream::new(1024, 5, 10));
        for _ in 0..2 {
            engine
                .publish_on(
                    stream.into(),
                    PublishCommand::simple("data", vec![], 1, false),
                )
                .unwrap();
        }
        engine.mqtt_engine.reset_for_new_transport();
        engine.mqtt_engine.connect().unwrap();
        engine.mqtt_engine.take_outgoing();
        engine
            .mqtt_engine
            .handle_incoming(&[0x20, 6, 1, 0, 3, 0x21, 0, 1]);
        engine.data_streams.clear();
        let replacement = StreamId::new(Side::Client, Dir::Bi, 2);
        engine
            .data_streams
            .insert(replacement, QuicStream::new(1024, 5, 10));
        engine.default_pub_stream = Some(replacement);
        engine.route_session_work().unwrap();
        assert_eq!(engine.data_streams[&replacement].pending_packets, 1);
        assert_eq!(engine.mqtt_engine.session_replay.len(), 1);
        let MqttPacket::Publish5(first) =
            &parse_packets(&engine.data_streams[&replacement].outgoing, 5)[0]
        else {
            panic!()
        };
        engine.mqtt_engine.ingest_stream_packet(
            MqttPacket::PubAck5(MqttPubAck::new(first.packet_id.unwrap(), 0, vec![])),
            replacement.into(),
        );
        engine.route_session_work().unwrap();
        assert!(engine.mqtt_engine.session_replay.is_empty());
        assert_eq!(engine.data_streams[&replacement].pending_packets, 2);
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn quic_pubrel_replay_ignores_quota_and_tracks_replacement_stream() {
        use quinn_proto::{Dir, Side, StreamId};
        for pending_publishes in [0, 2] {
            let mut engine = QuicMqttEngine::new(MqttClientOptions::default()).unwrap();
            engine.mqtt_engine.connect().unwrap();
            engine.mqtt_engine.take_outgoing();
            engine.mqtt_engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
            let old = StreamId::new(Side::Client, Dir::Bi, 1);
            engine
                .data_streams
                .insert(old, QuicStream::new(1024, 5, 10));
            for _ in 0..pending_publishes {
                engine
                    .publish_on(old.into(), PublishCommand::simple("t", vec![], 1, false))
                    .unwrap();
            }
            let mut ids = Vec::new();
            for _ in 0..2 {
                let id = engine
                    .publish_on(old.into(), PublishCommand::simple("t", vec![], 2, false))
                    .unwrap()
                    .unwrap();
                let (_, response) = engine.mqtt_engine.ingest_stream_packet(
                    MqttPacket::PubRec5(MqttPubRec::new(id, 0, vec![])),
                    old.into(),
                );
                assert!(matches!(parse_packets(&response, 5).as_slice(),
                    [MqttPacket::PubRel5(rel)] if rel.packet_id == id));
                ids.push(id);
            }
            engine.reset_connection_state();
            engine.mqtt_engine.connect().unwrap();
            engine.mqtt_engine.take_outgoing();
            engine
                .mqtt_engine
                .handle_incoming(&[0x20, 6, 1, 0, 3, 0x21, 0, 1]);
            let replacement = StreamId::new(Side::Client, Dir::Bi, 2);
            engine
                .data_streams
                .insert(replacement, QuicStream::new(1024, 5, 10));
            engine.default_pub_stream = Some(replacement);
            engine.route_session_work().unwrap();
            let output = parse_packets(&engine.data_streams[&replacement].outgoing, 5);
            let replayed: Vec<_> = output
                .iter()
                .filter_map(|packet| match packet {
                    MqttPacket::PubRel5(rel) => Some(rel.packet_id),
                    _ => None,
                })
                .collect();
            assert_eq!(replayed, ids);
            assert_eq!(
                output
                    .iter()
                    .filter(|p| matches!(p, MqttPacket::Publish5(_)))
                    .count(),
                usize::from(pending_publishes > 0)
            );
            assert!(engine.control_outgoing.is_empty());
            for id in ids {
                assert_eq!(
                    engine.mqtt_engine.inflight_queue.get(id).unwrap().stream,
                    Some(replacement.into())
                );
                let (events, _) = engine.mqtt_engine.ingest_stream_packet(
                    MqttPacket::PubComp5(MqttPubComp::new(id, 0, vec![])),
                    replacement.into(),
                );
                assert!(events.iter().any(|event| matches!(event,
                    MqttEvent::Published(result) if result.packet_id == Some(id) && result.qos == 2)));
            }
        }
    }

    #[test]
    fn connect_encodes_configured_will_for_each_version() {
        use crate::mqtt_serde::mqttv5::willv5::Will;
        use crate::mqtt_serde::parser::ParseOk;
        for version in [3, 4, 5] {
            let mut will = Will::new("client/status".into(), vec![0, 255], 2, true);
            will.properties.will_delay_interval = Some(30);
            let options = MqttClientOptions::builder()
                .mqtt_version(version)
                .will(will.clone())
                .build();
            let mut engine = MqttEngine::new(options);
            engine.connect().unwrap();
            let bytes = engine.take_outgoing();
            match MqttPacket::from_bytes_with_version(&bytes, version).unwrap() {
                ParseOk::Packet(MqttPacket::Connect5(packet), _) => {
                    assert_eq!(packet.will, Some(will))
                }
                ParseOk::Packet(MqttPacket::Connect3(packet), _) => {
                    let actual = packet.will.unwrap();
                    assert_eq!(actual.topic, will.will_topic);
                    assert_eq!(actual.message, will.will_message);
                    assert_eq!(actual.qos, will.will_qos);
                    assert_eq!(actual.retain, will.will_retain);
                }
                packet => panic!("Expected CONNECT, got {packet:?}"),
            }
        }
    }

    #[test]
    fn connack_preserves_new_early_operations_without_replaying_them() {
        for resumed in [false, true] {
            let mut engine = MqttEngine::new(MqttClientOptions::builder().build());
            if resumed {
                establish_connection(&mut engine);
                engine.handle_connection_lost();
            }
            engine.connect().unwrap();
            engine.take_outgoing();
            let command = PublishCommand::simple("early", vec![1], 1, false);
            let (packet_id, _) = engine.publish_encoded(command, Some(4)).unwrap();
            let events = engine.handle_incoming(&[0x20, 3, u8::from(resumed), 0, 0]);
            assert!(engine.is_connected(), "{events:?}");
            assert!(engine.take_outgoing().is_empty());
            let pid = packet_id.unwrap();
            let (events, _) = engine
                .ingest_stream_packet(MqttPacket::PubAck5(MqttPubAck::new(pid, 0, vec![])), 4);
            assert!(
                matches!(&events[..], [MqttEvent::Published(result)] if result.packet_id == packet_id)
            );
        }
    }

    #[test]
    fn new_or_sessionless_connections_discard_old_inflight_state() {
        for sessionless in [false, true] {
            let mut engine = MqttEngine::new(
                MqttClientOptions::builder()
                    .sessionless(sessionless)
                    .build(),
            );
            engine.connect().unwrap();
            engine.take_outgoing();
            engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
            let command = PublishCommand::builder()
                .topic("test")
                .payload(vec![1])
                .qos(1)
                .build()
                .unwrap();
            let pid = engine.publish(command).unwrap().unwrap();
            engine.take_outgoing();
            engine.reset_for_new_transport();
            if sessionless {
                assert!(engine.inflight_queue.acknowledge(pid).is_none());
            }
            engine.connect().unwrap();
            engine.take_outgoing();
            engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
            assert!(engine.inflight_queue.acknowledge(pid).is_none());
            assert!(engine.take_outgoing().is_empty());
        }
    }

    #[test]
    fn full_publish_queue_rejects_without_evicting_accepted_messages() {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .max_outgoing_packet_count(1)
                .build(),
        );
        engine.connect().unwrap();
        engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
        let command = |payload: u8| {
            PublishCommand::builder()
                .topic("test")
                .payload(vec![payload])
                .build()
                .unwrap()
        };
        engine.publish(command(1)).unwrap(); // CONNECT occupies the outgoing slot.
        assert!(matches!(
            engine.publish(command(2)),
            Err(MqttClientError::BufferFull { .. })
        ));
        let packet = engine.take_outgoing();
        assert_eq!(packet.last(), Some(&1));
        assert!(engine.take_outgoing().is_empty());
        assert!(engine.publish(command(3)).is_ok());
    }

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
        establish_connection(&mut engine);

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

    #[test]
    fn reduced_type_only_parser_counts_fragmented_qos0_publish_without_payload_event() {
        let mut engine = MqttEngine::new(MqttClientOptions::builder().mqtt_version(5).build());
        establish_connection(&mut engine);
        engine.set_parse_level(ParseLevel::TypeOnly);
        let publish = MqttPublish::new_with_prop(
            0,
            "bench/events".to_string(),
            None,
            b"payload".to_vec(),
            false,
            false,
            Vec::new(),
        );
        let bytes = MqttPacket::Publish5(publish).to_bytes().unwrap();

        assert!(engine.handle_incoming(&bytes[..2]).is_empty());
        let events = engine.handle_incoming(&bytes[2..]);

        assert!(matches!(
            events.as_slice(),
            [MqttEvent::PublishReceived {
                packet_id: None,
                stream: None
            }]
        ));
        assert!(engine.take_outgoing().is_empty());
    }

    #[test]
    fn reduced_headers_parser_generates_qos2_handshake_without_payload_event() {
        let mut engine = MqttEngine::new(MqttClientOptions::builder().mqtt_version(5).build());
        establish_connection(&mut engine);
        engine.set_parse_level(ParseLevel::HeadersParsed);
        let publish = MqttPublish::new_with_prop(
            2,
            "bench/events".to_string(),
            Some(42),
            b"payload".to_vec(),
            false,
            false,
            Vec::new(),
        );

        let events = engine.handle_incoming(&MqttPacket::Publish5(publish).to_bytes().unwrap());
        assert!(matches!(
            events.as_slice(),
            [MqttEvent::PublishReceived {
                packet_id: Some(42),
                stream: None
            }]
        ));
        assert_eq!(engine.take_outgoing()[0] & 0xf0, 0x50);

        let pubrel = MqttPacket::PubRel5(MqttPubRel::new(42, 0, Vec::new()));
        let events = engine.handle_incoming(&pubrel.to_bytes().unwrap());
        assert!(matches!(
            events.as_slice(),
            [MqttEvent::PubRelReceived {
                packet_id: 42,
                stream: None
            }]
        ));
        assert_eq!(engine.take_outgoing()[0] & 0xf0, 0x70);
    }

    #[test]
    fn reduced_headers_parser_handles_fragmented_v3_qos1_publish() {
        let mut engine = MqttEngine::new(MqttClientOptions::builder().mqtt_version(4).build());
        establish_connection(&mut engine);
        engine.set_parse_level(ParseLevel::HeadersParsed);
        let publish = crate::mqtt_serde::mqttv3::publish::MqttPublish::new(
            "bench/events".to_string(),
            1,
            b"payload".to_vec(),
            Some(7),
            false,
            false,
        );
        let bytes = MqttPacket::Publish3(publish).to_bytes().unwrap();
        let split = bytes.len() / 2;

        assert!(engine.handle_incoming(&bytes[..split]).is_empty());
        let events = engine.handle_incoming(&bytes[split..]);

        assert!(matches!(
            events.as_slice(),
            [MqttEvent::PublishReceived {
                packet_id: Some(7),
                stream: None
            }]
        ));
        assert_eq!(engine.take_outgoing()[0] & 0xf0, 0x40);
    }

    use crate::mqtt_client::commands::PublishCommand;

    #[test]
    fn test_publish_encoded_qos0_returns_bytes_without_inflight() {
        let mut engine = MqttEngine::new(MqttClientOptions::builder().build());
        engine.connect().unwrap(); // initialize session for packet-id allocation
        let _ = engine.take_outgoing(); // discard the CONNECT bytes

        let cmd = PublishCommand::builder()
            .topic("a/b")
            .payload("hi".to_string())
            .qos(0)
            .build()
            .unwrap();

        let (pid, bytes) = engine.publish_encoded(cmd, None).unwrap();
        assert!(pid.is_none(), "QoS 0 must not allocate a packet id");
        assert!(!bytes.is_empty());
        // QoS 0 is never tracked for retransmission.
        assert!(engine.inflight_queue.is_empty());
        // publish_encoded bypasses the outgoing buffer entirely.
        assert!(engine.outgoing_buffer.is_empty());
    }

    #[test]
    fn test_publish_encoded_qos1_tracks_inflight_and_acks() {
        let mut engine = MqttEngine::new(MqttClientOptions::builder().build());
        engine.connect().unwrap();
        engine.is_connected = true;
        let _ = engine.take_outgoing();

        let cmd = PublishCommand::builder()
            .topic("a/b")
            .payload("hi".to_string())
            .qos(1)
            .build()
            .unwrap();

        let (pid, bytes) = engine.publish_encoded(cmd, Some(7)).unwrap();
        let pid = pid.expect("QoS 1 must allocate a packet id");
        assert!(!bytes.is_empty());
        assert!(!engine.inflight_queue.is_empty());

        // Acknowledge it through ingest_stream_packet (as if it arrived on a data stream).
        let ack = MqttPacket::PubAck5(MqttPubAck::new(pid, 0, Vec::new()));
        let (events, resp) = engine.ingest_stream_packet(ack, 7);
        assert!(
            events.iter().any(|e| matches!(e, MqttEvent::Published(_))),
            "expected a Published event after PUBACK"
        );
        // A terminal PUBACK requires no response on the stream.
        assert!(resp.is_empty());
        assert!(
            engine.inflight_queue.is_empty(),
            "inflight entry must be cleared after PUBACK"
        );
    }

    #[test]
    fn test_ingest_stream_packet_emits_event() {
        let mut engine = MqttEngine::new(MqttClientOptions::builder().build());
        establish_connection(&mut engine);
        let (events, resp) = engine.ingest_stream_packet(
            MqttPacket::PingResp5(crate::mqtt_serde::mqttv5::pingrespv5::MqttPingResp::new()),
            0,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], MqttEvent::PingResponse(_)));
        assert!(resp.is_empty());
    }

    #[test]
    fn test_v3_retransmission_routes_to_origin_stream() {
        // MQTT v3 QoS 1 publish sent on data stream 9 must, on retransmission,
        // be routed back to stream 9 — never the shared/control outgoing buffer.
        let options = MqttClientOptions::builder()
            .mqtt_version(3)
            .keep_alive(0) // disable keep-alive PING so the buffer stays clean
            .retransmission_timeout_ms(10)
            .build();
        let mut engine = MqttEngine::new(options);
        engine.connect().unwrap();
        engine.is_connected = true;
        let _ = engine.take_outgoing(); // discard CONNECT

        let cmd = PublishCommand::builder()
            .topic("t")
            .payload("x".to_string())
            .qos(1)
            .build()
            .unwrap();
        engine.publish_encoded(cmd, Some(9)).unwrap();

        // Drive the tick well past the retransmission timeout.
        let future = Instant::now() + Duration::from_secs(3600);
        let _ = engine.handle_tick(future);

        let retrans = engine.take_stream_retransmissions();
        assert_eq!(
            retrans.len(),
            1,
            "expected one stream-routed retransmission"
        );
        assert_eq!(
            retrans[0].0, 9,
            "retransmission must target the origin stream"
        );
        assert!(
            engine.outgoing_buffer.is_empty(),
            "v3 stream retransmission must not land on the shared/control buffer"
        );
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_send_raw_on_routes_to_target_stream() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();

        // Simulate an established control stream and one data stream.
        let control = StreamId::new(Side::Client, Dir::Bi, 0);
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine.control_stream = Some(control);
        engine
            .data_streams
            .insert(data, QuicStream::new(1024, 5, 1000));

        // Raw bytes targeting the data stream land in that stream's buffer.
        engine.send_raw_on(u64::from(data), &[1, 2, 3]).unwrap();
        assert_eq!(
            engine.data_streams.get(&data).unwrap().outgoing,
            vec![1, 2, 3]
        );
        assert!(engine.control_outgoing.is_empty());

        // Raw bytes targeting the control stream land in the control buffer only —
        // no cross-fire to the data stream.
        engine.send_raw_on(u64::from(control), &[9]).unwrap();
        assert_eq!(engine.control_outgoing, vec![9]);
        assert_eq!(
            engine.data_streams.get(&data).unwrap().outgoing,
            vec![1, 2, 3]
        );

        // Sending a CONNECT (a control packet) on the data stream is allowed —
        // this is exactly the kind of bad behaviour flowSDK must be able to drive.
        let connect = connectv5::MqttConnect::new(
            "bad-client".to_string(),
            None,
            None,
            None,
            60,
            true,
            Vec::new(),
        );
        engine
            .send_packet_on(u64::from(data), MqttPacket::Connect5(connect))
            .unwrap();
        assert!(engine.data_streams.get(&data).unwrap().outgoing.len() > 3);

        // An unknown stream handle is rejected.
        assert!(engine.send_raw_on(9999, &[0]).is_err());

        // After the send side is finished/reset, raw writes are rejected rather
        // than silently buffering unsendable bytes.
        engine.data_streams.get_mut(&data).unwrap().finished = true;
        assert!(engine.send_raw_on(u64::from(data), &[0]).is_err());
        engine.control_finished = true;
        assert!(engine.send_raw_on(u64::from(control), &[0]).is_err());
    }

    #[cfg(feature = "quic-proto")]
    fn parse_packets(bytes: &[u8], mqtt_version: u8) -> Vec<MqttPacket> {
        use crate::mqtt_serde::parser::ParseOk;

        let mut packets = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            match MqttPacket::from_bytes_with_version(&bytes[offset..], mqtt_version).unwrap() {
                ParseOk::Packet(packet, consumed) => {
                    packets.push(packet);
                    offset += consumed;
                }
                ParseOk::Continue(_, _) => panic!("unexpected partial packet"),
                ParseOk::TopicName(_, _) => panic!("unexpected topic-name parse result"),
            }
        }
        packets
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_manual_ack_helpers_route_v5_packets_to_target_stream() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(
            MqttClientOptions::builder()
                .mqtt_version(5)
                .auto_ack(false)
                .build(),
        )
        .unwrap();
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(data, QuicStream::new(1024, 5, 1000));

        engine.puback_on(u64::from(data), 11).unwrap();
        engine.pubrec_on(u64::from(data), 12).unwrap();
        engine.pubrel_on(u64::from(data), 13).unwrap();
        engine.pubcomp_on(u64::from(data), 14).unwrap();

        let bytes = &engine.data_streams.get(&data).unwrap().outgoing;
        let packets = parse_packets(bytes, 5);
        assert_eq!(packets.len(), 4);
        assert!(
            matches!(&packets[0], MqttPacket::PubAck5(p) if p.packet_id == 11 && p.reason_code == 0)
        );
        assert!(
            matches!(&packets[1], MqttPacket::PubRec5(p) if p.packet_id == 12 && p.reason_code == 0)
        );
        assert!(
            matches!(&packets[2], MqttPacket::PubRel5(p) if p.packet_id == 13 && p.reason_code == 0)
        );
        assert!(
            matches!(&packets[3], MqttPacket::PubComp5(p) if p.packet_id == 14 && p.reason_code == 0)
        );
        assert!(engine.control_outgoing.is_empty());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_manual_ack_helpers_route_v3_packets_to_target_stream() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(
            MqttClientOptions::builder()
                .mqtt_version(3)
                .auto_ack(false)
                .build(),
        )
        .unwrap();
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(data, QuicStream::new(1024, 3, 1000));

        engine.puback_on(u64::from(data), 21).unwrap();
        engine.pubrec_on(u64::from(data), 22).unwrap();
        engine.pubrel_on(u64::from(data), 23).unwrap();
        engine.pubcomp_on(u64::from(data), 24).unwrap();

        let bytes = &engine.data_streams.get(&data).unwrap().outgoing;
        let packets = parse_packets(bytes, 3);
        assert_eq!(packets.len(), 4);
        assert!(matches!(&packets[0], MqttPacket::PubAck3(p) if p.message_id == 21));
        assert!(matches!(&packets[1], MqttPacket::PubRec3(p) if p.message_id == 22));
        assert!(matches!(&packets[2], MqttPacket::PubRel3(p) if p.message_id == 23));
        assert!(matches!(&packets[3], MqttPacket::PubComp3(p) if p.message_id == 24));
        assert!(engine.control_outgoing.is_empty());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_manual_ack_helpers_preserve_send_packet_on_errors() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(data, QuicStream::new(1024, 5, 1000));

        assert!(engine.puback_on(9999, 1).is_err());

        engine.data_streams.get_mut(&data).unwrap().finished = true;
        assert!(engine.pubrec_on(u64::from(data), 2).is_err());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn quic_configured_subscriptions_use_a_data_stream() {
        use quinn_proto::{Dir, Side, StreamId};
        let mut options = MqttClientOptions::builder().build();
        options.subscription_topics = SubscribeCommand::single("configured", 1).subscriptions;
        let mut engine = QuicMqttEngine::new(options).unwrap();
        let stream = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(stream, QuicStream::new(1024, 5, 10));
        engine.default_sub_stream = Some(stream);
        engine.mqtt_engine.connect().unwrap();
        engine.mqtt_engine.take_outgoing();
        engine.mqtt_engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
        assert!(engine.mqtt_engine.take_outgoing().is_empty());
        engine.route_session_work().unwrap();
        let packets = parse_packets(&engine.data_streams[&stream].outgoing, 5);
        assert!(
            matches!(&packets[..], [MqttPacket::Subscribe5(packet)] if packet.subscriptions[0].topic_filter == "configured")
        );
        assert!(engine.control_outgoing.is_empty());
        assert!(!engine.mqtt_engine.configured_subscriptions_pending);
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn quic_session_replay_uses_replacement_data_stream_with_backpressure() {
        use quinn_proto::{Dir, Side, StreamId};
        let mut engine = mqtt_connected_quic_engine();
        let old = StreamId::new(Side::Client, Dir::Bi, 1);
        engine.data_streams.insert(old, QuicStream::new(1024, 5, 1));
        let pid = engine
            .publish_on(
                old.into(),
                PublishCommand::simple("resume", vec![1], 1, false),
            )
            .unwrap()
            .unwrap();
        engine.reset_connection_state();
        engine.mqtt_engine.connect().unwrap();
        engine.mqtt_engine.take_outgoing();
        engine.mqtt_engine.handle_incoming(&[0x20, 3, 1, 0, 0]);
        assert!(engine.mqtt_engine.take_outgoing().is_empty());
        let replacement = StreamId::new(Side::Client, Dir::Bi, 2);
        let mut stream = QuicStream::new(1024, 5, 1);
        stream.pending_packets = 1;
        engine.data_streams.insert(replacement, stream);
        engine.default_pub_stream = Some(replacement);
        assert!(matches!(
            engine.route_session_work(),
            Err(MqttClientError::BufferFull { .. })
        ));
        assert_eq!(engine.mqtt_engine.session_replay.len(), 1);
        engine
            .data_streams
            .get_mut(&replacement)
            .unwrap()
            .pending_packets = 0;
        engine.route_session_work().unwrap();
        let packets = parse_packets(&engine.data_streams[&replacement].outgoing, 5);
        assert!(
            matches!(&packets[..], [MqttPacket::Publish5(packet)] if packet.packet_id == Some(pid) && packet.dup)
        );
        let entry = engine.mqtt_engine.inflight_queue.acknowledge(pid).unwrap();
        assert_eq!(entry.stream, Some(replacement.into()));
        assert!(engine.control_outgoing.is_empty());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn quic_publish_priorities_route_to_separate_ordered_streams() {
        use quinn_proto::{Dir, Side, StreamId};
        let mut engine = mqtt_connected_quic_engine();
        let low = StreamId::new(Side::Client, Dir::Bi, 1);
        let high = StreamId::new(Side::Client, Dir::Bi, 2);
        for (id, priority) in [(low, 8), (high, 240)] {
            let mut stream = QuicStream::new(1024, 5, 4);
            stream.priority = priority;
            stream.publish_priority = Some(priority);
            engine.data_streams.insert(id, stream);
        }
        for priority in [8, 240, 8] {
            let mut command = PublishCommand::simple("priority", vec![priority], 0, false);
            command.priority = priority;
            engine.publish(command).unwrap();
        }
        assert_eq!(
            QuicMqttEngine::stream_send_order(&engine.data_streams),
            vec![high, low]
        );
        assert_eq!(
            parse_packets(&engine.data_streams[&low].outgoing, 5).len(),
            2
        );
        assert_eq!(
            parse_packets(&engine.data_streams[&high].outgoing, 5).len(),
            1
        );
        QuicMqttEngine::journal_stream_open(
            &mut engine.early_stream_journal,
            low,
            EarlyStreamRole::PriorityPub(8),
        );
        engine.set_stream_priority(low.into(), 255).unwrap();
        assert_eq!(
            QuicMqttEngine::stream_send_order(&engine.data_streams),
            vec![low, high]
        );
        assert_eq!(engine.early_stream_journal[0].priority, 255);
        assert_eq!(
            engine.early_stream_journal[0].role,
            EarlyStreamRole::PriorityPub(255)
        );
        assert!(engine.set_stream_priority(9999, 0).is_err());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn manual_acknowledgements_preserve_stream_and_backpressure() {
        use quinn_proto::{Dir, Side, StreamId};
        let mut engine =
            QuicMqttEngine::new(MqttClientOptions::builder().auto_ack(false).build()).unwrap();
        establish_connection(&mut engine.mqtt_engine);
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(data, QuicStream::new(1024, 5, 1));
        engine.mqtt_engine.ingest_stream_packet(
            MqttPacket::Publish5(MqttPublish::new(
                2,
                "t".into(),
                Some(7),
                vec![],
                false,
                false,
            )),
            data.into(),
        );
        let packet = MqttPacket::PubRec5(MqttPubRec::new(7, 0, vec![]));
        engine.acknowledge_on(data.into(), packet.clone()).unwrap();
        assert!(matches!(
            engine.acknowledge_on(data.into(), packet),
            Err(MqttClientError::BufferFull { .. })
        ));
        let stream = engine.data_streams.get(&data).unwrap();
        assert_eq!(stream.pending_packets, 1);
        assert!(
            matches!(&parse_packets(&stream.outgoing, 5)[0], MqttPacket::PubRec5(p) if p.packet_id == 7)
        );
        assert!(engine.control_outgoing.is_empty());
        assert!(engine.mqtt_engine.take_outgoing().is_empty());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_drain_data_stream_bounds_response_buffer() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        engine.mqtt_engine.is_connected = true;

        let max_packets = 3;
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(data, QuicStream::new(16384, 5, max_packets));

        // Feed many incoming QoS 1 PUBLISH packets into this stream's parser; each
        // would normally generate a PUBACK response buffered on the same stream.
        let mut bytes = Vec::new();
        for pid in 1..=10u16 {
            let publish = MqttPublish::new_with_prop(
                1,
                "t/x".to_string(),
                Some(pid),
                b"p".to_vec(),
                false,
                false,
                Vec::new(),
            );
            bytes.extend(MqttPacket::Publish5(publish).to_bytes().unwrap());
        }
        engine
            .data_streams
            .get_mut(&data)
            .unwrap()
            .parser
            .feed(&bytes);

        let mut events = Vec::new();
        QuicMqttEngine::drain_data_stream(
            &mut engine.data_streams,
            &mut engine.mqtt_engine,
            data,
            &mut events,
        );

        let ds = engine.data_streams.get(&data).unwrap();
        // Back-pressure: ingestion stops at the buffer cap rather than growing
        // unbounded, and the packet count reflects the buffered responses.
        assert_eq!(ds.pending_packets, max_packets);
        assert!(!ds.outgoing.is_empty());
        // Only `max_packets` publishes were processed; the rest stay in the parser.
        let received = events
            .iter()
            .filter(|e| matches!(e, MqttEvent::MessageReceived(_)))
            .count();
        assert_eq!(received, max_packets);

        // Draining again without freeing the buffer makes no further progress.
        let mut more = Vec::new();
        QuicMqttEngine::drain_data_stream(
            &mut engine.data_streams,
            &mut engine.mqtt_engine,
            data,
            &mut more,
        );
        assert_eq!(
            engine.data_streams.get(&data).unwrap().pending_packets,
            max_packets
        );
        assert!(more.is_empty());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_drain_data_stream_bounds_events_for_qos0() {
        use quinn_proto::{Dir, Side, StreamId};

        // QoS 0 PUBLISHes produce events but no response bytes, so the send-buffer
        // cap alone would not bound them — the event-count cap must.
        let max_event_count = 4;
        let opts = MqttClientOptions::builder()
            .max_event_count(max_event_count)
            .build();
        let mut engine = QuicMqttEngine::new(opts).unwrap();
        engine.mqtt_engine.is_connected = true;

        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(data, QuicStream::new(16384, 5, 1000));

        let mut bytes = Vec::new();
        for _ in 0..20 {
            let publish = MqttPublish::new_with_prop(
                0,
                "t/x".to_string(),
                None,
                b"p".to_vec(),
                false,
                false,
                Vec::new(),
            );
            bytes.extend(MqttPacket::Publish5(publish).to_bytes().unwrap());
        }
        engine
            .data_streams
            .get_mut(&data)
            .unwrap()
            .parser
            .feed(&bytes);

        let mut events = Vec::new();
        QuicMqttEngine::drain_data_stream(
            &mut engine.data_streams,
            &mut engine.mqtt_engine,
            data,
            &mut events,
        );

        // Event production is bounded by max_event_count even though no response
        // bytes are generated (pending_packets stays 0).
        assert_eq!(events.len(), max_event_count);
        assert_eq!(engine.data_streams.get(&data).unwrap().pending_packets, 0);
    }

    #[test]
    fn test_ingest_stream_packet_qos1_publish_acks_on_same_stream() {
        // An incoming QoS 1 PUBLISH must produce PUBACK bytes for the caller to
        // write back on the originating stream — not enqueued to the shared buffer.
        let mut engine = MqttEngine::new(MqttClientOptions::builder().build());
        engine.is_connected = true;

        let publish = MqttPublish::new_with_prop(
            1,
            "t/1".to_string(),
            Some(42),
            b"payload".to_vec(),
            false,
            false,
            Vec::new(),
        );
        let (events, resp) = engine.ingest_stream_packet(MqttPacket::Publish5(publish), 5);

        assert!(events
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        // The response is a PUBACK (v5 fixed header 0x40) and must be returned to
        // the caller, never placed on the shared/control outgoing buffer.
        assert!(
            !resp.is_empty(),
            "QoS 1 PUBLISH must yield a PUBACK response"
        );
        assert_eq!(resp[0] & 0xF0, 0x40, "response must be a PUBACK");
        assert!(
            engine.outgoing_buffer.is_empty(),
            "ack must not cross-fire onto the shared/control buffer"
        );
    }

    #[test]
    fn test_auto_ack_false_suppresses_shared_publish_acks() {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .auto_ack(false)
                .mqtt_version(5)
                .build(),
        );
        engine.is_connected = true;

        let qos1 = MqttPublish::new_with_prop(
            1,
            "t/qos1".to_string(),
            Some(42),
            b"payload".to_vec(),
            false,
            false,
            Vec::new(),
        );
        let events = engine.handle_incoming(&MqttPacket::Publish5(qos1).to_bytes().unwrap());
        assert!(events
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        assert!(
            engine.take_outgoing().is_empty(),
            "auto_ack(false) must suppress PUBACK"
        );

        let qos2 = MqttPublish::new_with_prop(
            2,
            "t/qos2".to_string(),
            Some(43),
            b"payload".to_vec(),
            false,
            false,
            Vec::new(),
        );
        let events = engine.handle_incoming(&MqttPacket::Publish5(qos2).to_bytes().unwrap());
        assert!(events
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        assert!(
            engine.take_outgoing().is_empty(),
            "auto_ack(false) must suppress PUBREC"
        );
    }

    #[test]
    fn test_auto_ack_false_suppresses_shared_pubrel_ack() {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .auto_ack(false)
                .mqtt_version(5)
                .build(),
        );
        engine.is_connected = true;

        let pubrel = MqttPacket::PubRel5(MqttPubRel::new(44, 0, Vec::new()));
        let events = engine.handle_incoming(&pubrel.to_bytes().unwrap());
        assert!(matches!(
            events.as_slice(),
            [MqttEvent::PubRelReceived {
                packet_id: 44,
                stream: None
            }]
        ));
        assert!(
            engine.take_outgoing().is_empty(),
            "auto_ack(false) must suppress PUBCOMP"
        );
    }

    #[test]
    fn test_auto_ack_false_suppresses_stream_publish_and_pubrel_acks() {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .auto_ack(false)
                .mqtt_version(5)
                .build(),
        );
        engine.is_connected = true;

        let publish = MqttPublish::new_with_prop(
            1,
            "t/1".to_string(),
            Some(45),
            b"payload".to_vec(),
            false,
            false,
            Vec::new(),
        );
        let (events, resp) = engine.ingest_stream_packet(MqttPacket::Publish5(publish), 5);
        assert!(events
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        assert!(resp.is_empty(), "auto_ack(false) must suppress PUBACK");

        let pubrel = MqttPacket::PubRel5(MqttPubRel::new(46, 0, Vec::new()));
        let (events, resp) = engine.ingest_stream_packet(pubrel, 5);
        assert!(matches!(
            events.as_slice(),
            [MqttEvent::PubRelReceived {
                packet_id: 46,
                stream: Some(5)
            }]
        ));
        assert!(resp.is_empty(), "auto_ack(false) must suppress PUBCOMP");
    }

    #[test]
    fn test_auto_ack_false_suppresses_v3_publish_ack() {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .auto_ack(false)
                .mqtt_version(3)
                .build(),
        );
        engine.is_connected = true;

        let publish = crate::mqtt_serde::mqttv3::publish::MqttPublish::new(
            "t/v3".to_string(),
            1,
            b"payload".to_vec(),
            Some(47),
            false,
            false,
        );
        let events = engine.handle_incoming(&MqttPacket::Publish3(publish).to_bytes().unwrap());
        assert!(events
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        assert!(
            engine.take_outgoing().is_empty(),
            "auto_ack(false) must suppress v3 PUBACK"
        );
    }

    #[test]
    fn test_try_send_ping_and_disconnect_report_buffer_full() {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .max_outgoing_packet_count(1)
                .build(),
        );
        engine.connect().unwrap(); // queues CONNECT, filling the single-slot buffer
        engine.is_connected = true;
        assert_eq!(engine.outgoing_buffer.len(), 1);

        // Fallible paths must report BufferFull rather than silently dropping.
        assert!(matches!(
            engine.try_send_ping(),
            Err(MqttClientError::BufferFull { .. })
        ));
        assert!(matches!(
            engine.try_disconnect(),
            Err(MqttClientError::BufferFull { .. })
        ));
        // A failed try_disconnect must not mark the session disconnected.
        assert!(engine.is_connected());

        // After draining, both succeed and disconnect updates state.
        let _ = engine.take_outgoing();
        assert!(engine.try_send_ping().is_ok());
        let _ = engine.take_outgoing();
        assert!(engine.try_disconnect().is_ok());
        assert!(!engine.is_connected());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_quic_ping_propagates_buffer_full() {
        let mut engine = QuicMqttEngine::new(
            MqttClientOptions::builder()
                .max_outgoing_packet_count(1)
                .build(),
        )
        .unwrap();
        engine.mqtt_engine.is_connected = true;
        // Fill the single outgoing slot, then ping() must report BufferFull
        // instead of returning Ok without queuing a PINGREQ.
        engine.mqtt_engine.send_ping().unwrap();
        assert!(matches!(
            engine.ping(),
            Err(MqttClientError::BufferFull { .. })
        ));
    }

    #[test]
    fn test_reset_for_new_transport_clears_outbound_keeps_inflight() {
        let mut engine = MqttEngine::new(MqttClientOptions::builder().build());
        engine.connect().unwrap(); // queues CONNECT into outgoing_buffer
        engine.is_connected = true;

        // A QoS 1 publish (tracked inflight) and a PINGREQ (queued outbound).
        let cmd = PublishCommand::builder()
            .topic("t")
            .payload("x".to_string())
            .qos(1)
            .build()
            .unwrap();
        engine.publish_encoded(cmd, Some(3)).unwrap();
        engine.send_ping().unwrap();
        // A pending reconnect deadline from a prior timeout.
        engine.next_reconnect_at = Some(Instant::now() + Duration::from_secs(30));
        assert!(!engine.outgoing_buffer.is_empty());
        assert!(!engine.inflight_queue.is_empty());

        engine.reset_for_new_transport();

        // The stale reconnect deadline must be cancelled so it cannot fire another
        // ReconnectNeeded during the new handshake.
        assert!(engine.next_reconnect_at.is_none());

        // Transport-tied output is discarded so it cannot be replayed before the
        // next CONNECT, but the inflight queue is retained for session resumption.
        assert!(
            engine.outgoing_buffer.is_empty(),
            "stale outbound bytes must be cleared on reset"
        );
        assert!(engine.stream_retransmissions.is_empty());
        assert!(!engine.is_connected());
        assert!(
            !engine.inflight_queue.is_empty(),
            "inflight must be retained for session resumption"
        );
    }

    #[test]
    fn test_auto_keepalive_toggle() {
        let tick_at = Instant::now() + Duration::from_millis(1100);

        // Disabled: no PINGREQ emitted even though keep_alive has elapsed.
        let mut off = MqttEngine::new(
            MqttClientOptions::builder()
                .keep_alive(1)
                .auto_keepalive(false)
                .build(),
        );
        off.connect().unwrap();
        off.is_connected = true;
        let _ = off.take_outgoing();
        let _ = off.handle_tick(tick_at);
        assert!(
            off.outgoing_buffer.is_empty(),
            "no PINGREQ when auto_keepalive is disabled"
        );

        // Enabled: PINGREQ is emitted on the keep-alive timer.
        let mut on = MqttEngine::new(
            MqttClientOptions::builder()
                .keep_alive(1)
                .auto_keepalive(true)
                .build(),
        );
        on.connect().unwrap();
        on.is_connected = true;
        let _ = on.take_outgoing();
        let _ = on.handle_tick(tick_at);
        assert!(
            !on.outgoing_buffer.is_empty(),
            "PINGREQ expected when auto_keepalive is enabled"
        );
    }

    #[test]
    fn keepalive_does_not_timeout_active_qos0_publisher() {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .keep_alive(1)
                .ping_timeout_multiplier(2)
                .build(),
        );
        engine.is_connected = true;

        let now = Instant::now();
        engine.last_packet_sent = now;
        // QoS 0 publishes do not receive acknowledgements, so this timestamp
        // may be stale while the client is actively transmitting.
        engine.last_packet_received = now - Duration::from_secs(3);

        assert!(engine.handle_tick(now).is_empty());
        assert!(engine.ping_sent_at.is_none());
        assert!(engine.is_connected());

        let ping_at = now + Duration::from_millis(1100);
        assert!(engine.handle_tick(ping_at).is_empty());
        assert_eq!(engine.ping_sent_at, Some(ping_at));

        assert!(engine
            .handle_tick(ping_at + Duration::from_secs(1))
            .is_empty());
        let events = engine.handle_tick(ping_at + Duration::from_millis(2100));
        assert!(events
            .iter()
            .any(|event| matches!(event, MqttEvent::Disconnected(_))));
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_transport_close_details_mapping() {
        use quinn_proto::{ApplicationClose, ConnectionClose, TransportErrorCode, VarInt};

        let (by_peer, code) = transport_close_details(&ConnectionError::TimedOut);
        assert!(!by_peer);
        assert_eq!(code, None);

        let (by_peer, _) = transport_close_details(&ConnectionError::Reset);
        assert!(by_peer, "Reset is peer-initiated");

        let (by_peer, _) = transport_close_details(&ConnectionError::LocallyClosed);
        assert!(!by_peer, "LocallyClosed is local");

        // Peer application close preserves the application error code.
        let app = ConnectionError::ApplicationClosed(ApplicationClose {
            error_code: VarInt::from_u32(5),
            reason: bytes::Bytes::new(),
        });
        assert_eq!(transport_close_details(&app), (true, Some(5)));

        // Peer transport close preserves the transport error code (P3 regression).
        let transport = ConnectionError::ConnectionClosed(ConnectionClose {
            error_code: TransportErrorCode::APPLICATION_ERROR,
            frame_type: None,
            reason: bytes::Bytes::new(),
        });
        let (by_peer, code) = transport_close_details(&transport);
        assert!(by_peer);
        assert_eq!(code, Some(u64::from(TransportErrorCode::APPLICATION_ERROR)));
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_quic_error_code_rejects_overflow() {
        assert!(quic_error_code(0).is_ok());
        assert!(quic_error_code((1u64 << 62) - 1).is_ok());
        assert!(quic_error_code(u64::MAX).is_err());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_stream_abort_events_clear_data_stream_refs() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        let reset_stream = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(reset_stream, QuicStream::new(1024, 5, 1000));
        engine.default_pub_stream = Some(reset_stream);

        let mut events = Vec::new();
        engine.handle_stream_reset(reset_stream, VarInt::from_u32(42), &mut events);

        assert!(matches!(
            events.as_slice(),
            [MqttEvent::StreamReset {
                stream_id,
                error_code: 42
            }] if *stream_id == u64::from(reset_stream)
        ));
        assert!(!engine.data_streams.contains_key(&reset_stream));
        assert_eq!(engine.default_pub_stream, None);

        let stopped_stream = StreamId::new(Side::Client, Dir::Bi, 2);
        engine
            .data_streams
            .insert(stopped_stream, QuicStream::new(1024, 5, 1000));
        engine.default_sub_stream = Some(stopped_stream);

        events.clear();
        engine.handle_stream_stopped(stopped_stream, VarInt::from_u32(7), &mut events);

        assert!(matches!(
            events.as_slice(),
            [MqttEvent::StreamStopped {
                stream_id,
                error_code: 7
            }] if *stream_id == u64::from(stopped_stream)
        ));
        assert!(!engine.data_streams.contains_key(&stopped_stream));
        assert_eq!(engine.default_sub_stream, None);
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_stream_closed_events_mark_half_close_state() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        let data = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(data, QuicStream::new(1024, 5, 1000));

        let mut events = Vec::new();
        engine.handle_stream_closed(data, "recv_finished", true, &mut events);

        assert!(matches!(
            events.as_slice(),
            [MqttEvent::StreamClosed {
                stream_id,
                reason,
                by_peer: true
            }] if *stream_id == u64::from(data) && reason == "recv_finished"
        ));
        assert!(engine.data_streams.get(&data).unwrap().recv_closed);
        assert!(!engine.data_streams.get(&data).unwrap().finished);

        events.clear();
        engine.handle_stream_closed(data, "send_finished", false, &mut events);

        assert!(matches!(
            events.as_slice(),
            [MqttEvent::StreamClosed {
                stream_id,
                reason,
                by_peer: false
            }] if *stream_id == u64::from(data) && reason == "send_finished"
        ));
        assert!(engine.data_streams.get(&data).unwrap().finished);
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_control_stream_abort_emits_disconnect() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        let control = StreamId::new(Side::Client, Dir::Bi, 0);
        engine.control_stream = Some(control);
        engine.control_outgoing.extend_from_slice(&[1, 2, 3]);
        engine.mqtt_engine.is_connected = true;

        let mut events = Vec::new();
        engine.handle_stream_reset(control, VarInt::from_u32(9), &mut events);

        assert!(matches!(
            events.as_slice(),
            [
                MqttEvent::StreamReset {
                    stream_id,
                    error_code: 9
                },
                MqttEvent::Disconnected(None)
            ] if *stream_id == u64::from(control)
        ));
        assert_eq!(engine.control_stream, None);
        assert!(engine.control_outgoing.is_empty());
        assert!(!engine.mqtt_engine.is_connected());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_control_stream_stop_and_peer_close_emit_disconnect() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        let control = StreamId::new(Side::Client, Dir::Bi, 0);
        engine.control_stream = Some(control);
        engine.control_outgoing.extend_from_slice(&[1, 2, 3]);
        engine.control_finishing = true;
        engine.mqtt_engine.is_connected = true;

        let mut events = Vec::new();
        engine.handle_stream_stopped(control, VarInt::from_u32(11), &mut events);

        assert!(matches!(
            events.as_slice(),
            [
                MqttEvent::StreamStopped {
                    stream_id,
                    error_code: 11
                },
                MqttEvent::Disconnected(None)
            ] if *stream_id == u64::from(control)
        ));
        assert_eq!(engine.control_stream, None);
        assert!(engine.control_outgoing.is_empty());
        assert!(!engine.control_finishing);
        assert!(engine.control_finished);
        assert!(!engine.mqtt_engine.is_connected());

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        engine.control_stream = Some(control);
        engine.mqtt_engine.is_connected = true;

        events.clear();
        engine.handle_stream_closed(control, "recv_finished", true, &mut events);

        assert!(matches!(
            events.as_slice(),
            [
                MqttEvent::StreamClosed {
                    stream_id,
                    reason,
                    by_peer: true
                },
                MqttEvent::Disconnected(None)
            ] if *stream_id == u64::from(control) && reason == "recv_finished"
        ));
        assert_eq!(engine.control_stream, None);
        assert!(engine.control_finished);
        assert!(!engine.mqtt_engine.is_connected());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_controls_require_connection() {
        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();

        // reconnect before any connect() has stored config.
        assert!(engine.reconnect(Instant::now()).is_err());

        // Connection-level / stream-level controls error without a connection.
        assert!(engine.close(0, b"bye").is_err());
        assert!(engine.quic_ping().is_err());
        assert!(engine.finish_stream(0).is_err());
        assert!(engine.reset_stream(0, 1).is_err());
        assert!(engine.stop_stream(0, 1).is_err());

        // close_silent is always safe and clears state.
        engine.close_silent();

        // ping() is rejected until the MQTT session is connected, so a PINGREQ
        // can never be queued ahead of CONNECT on a fresh transport.
        assert!(engine.ping().is_err());

        // Once connected, ping() queues a PINGREQ.
        engine.mqtt_engine.is_connected = true;
        engine.ping().unwrap();
        assert!(
            !engine.mqtt_engine.outgoing_buffer.is_empty(),
            "ping() must queue a PINGREQ once connected"
        );

        // Rejected again once the control stream is finished.
        engine.control_finished = true;
        assert!(engine.ping().is_err());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_notify_local_address_changed_noop_and_connected() {
        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();

        assert!(engine.notify_local_address_changed().is_ok());
        assert!(engine.connection.is_none());

        engine
            .connect(
                "127.0.0.1:4433".parse().unwrap(),
                "localhost",
                quic_test_crypto_config(),
                Instant::now(),
            )
            .unwrap();
        assert!(engine.connection.is_some());
        assert!(engine.notify_local_address_changed().is_ok());
        assert!(engine.connection.is_some());
    }

    #[cfg(feature = "quic-proto")]
    fn quic_test_crypto_config() -> rustls::ClientConfig {
        #[cfg(feature = "quic-proto-openssl")]
        let _ = rustls_openssl::default_provider().install_default();
        #[cfg(not(feature = "quic-proto-openssl"))]
        let _ = rustls::crypto::ring::default_provider().install_default();

        rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth()
    }

    #[cfg(feature = "quic-proto")]
    fn connected_quic_engine_without_peer_limits() -> QuicMqttEngine {
        let mut engine = QuicMqttEngine::new(
            MqttClientOptions::builder()
                .max_outgoing_packet_count(1)
                .build(),
        )
        .unwrap();
        engine
            .connect(
                "127.0.0.1:4433".parse().unwrap(),
                "localhost",
                quic_test_crypto_config(),
                Instant::now(),
            )
            .unwrap();
        engine.mqtt_engine.is_connected = true;
        engine
    }

    #[cfg(feature = "quic-proto")]
    fn mqtt_connected_quic_engine() -> QuicMqttEngine {
        let mut engine = QuicMqttEngine::new(
            MqttClientOptions::builder()
                .max_outgoing_packet_count(1)
                .build(),
        )
        .unwrap();
        engine.mqtt_engine.connect().unwrap();
        let _ = engine.mqtt_engine.take_outgoing();
        let events = engine
            .mqtt_engine
            .handle_incoming(&[0x20, 0x03, 0x00, 0x00, 0x00]);
        assert!(matches!(events.as_slice(), [MqttEvent::Connected(_)]));
        engine
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn reduced_parse_level_applies_to_quic_data_streams() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = mqtt_connected_quic_engine();
        let stream_id = StreamId::new(Side::Server, Dir::Bi, 0);
        engine
            .data_streams
            .insert(stream_id, QuicStream::new(1024, 5, 2));
        engine.set_parse_level(ParseLevel::HeadersParsed);

        let publish = MqttPublish::new_with_prop(
            1,
            "bench/events".to_string(),
            Some(12),
            b"payload".to_vec(),
            false,
            false,
            Vec::new(),
        );
        engine
            .data_streams
            .get_mut(&stream_id)
            .unwrap()
            .parser
            .feed(&MqttPacket::Publish5(publish).to_bytes().unwrap());
        let mut events = Vec::new();
        QuicMqttEngine::drain_data_stream(
            &mut engine.data_streams,
            &mut engine.mqtt_engine,
            stream_id,
            &mut events,
        );

        assert_eq!(engine.parse_level(), ParseLevel::HeadersParsed);
        assert!(matches!(
            events.as_slice(),
            [MqttEvent::PublishReceived {
                packet_id: Some(12),
                stream: Some(_)
            }]
        ));
        let response = &engine.data_streams.get(&stream_id).unwrap().outgoing;
        assert_eq!(response[0] & 0xf0, 0x40);
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_quic_data_stream_api_routes_commands_and_reuses_defaults() {
        use quinn_proto::{Dir, Side, StreamId};

        // GIVEN: A QUIC connection exists, but no peer transport parameters have
        // been received to grant client-initiated bidirectional stream capacity.
        let mut engine = connected_quic_engine_without_peer_limits();

        // WHEN: The caller asks the public API to open a new data stream.
        // THEN: The request fails instead of creating untracked stream state.
        assert!(matches!(
            engine.open_data_stream(),
            Err(MqttClientError::InternalError { .. })
        ));

        // GIVEN: A connected MQTT session with one known QUIC data stream.
        let mut engine = mqtt_connected_quic_engine();
        let stream_id = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(stream_id, QuicStream::new(1024, 5, 1));
        let stream = u64::from(stream_id);
        assert_eq!(engine.data_stream_count(), 1);
        assert!(engine.control_stream_id().is_none());

        // WHEN: A publish is targeted at that explicit stream.
        // THEN: The MQTT packet is encoded, tracked, and buffered on the stream.
        let publish_pid = engine
            .publish_on(
                stream,
                PublishCommand::simple("topic/a", b"payload".to_vec(), 1, false),
            )
            .unwrap();
        assert_eq!(publish_pid, Some(1));
        assert!(engine
            .data_streams
            .values()
            .any(|ds| ds.pending_packets == 1 && !ds.outgoing.is_empty()));

        let bad_stream = stream + 4;
        // WHEN: A publish targets an unknown stream handle.
        // THEN: The API rejects the command before allocating MQTT state.
        assert!(matches!(
            engine.publish_on(
                bad_stream,
                PublishCommand::simple("topic/b", b"payload".to_vec(), 0, false)
            ),
            Err(MqttClientError::InvalidState { .. })
        ));

        // GIVEN: A fresh engine that has not registered the previous stream id.
        let mut engine = mqtt_connected_quic_engine();
        // WHEN: A subscribe uses a handle from another engine instance.
        // THEN: The handle is treated as unknown and rejected.
        let sub_pid = engine
            .subscribe_on(stream, SubscribeCommand::single("topic/sub", 0))
            .expect_err("stream handle from another engine must be rejected");
        assert!(matches!(sub_pid, MqttClientError::InvalidState { .. }));

        // GIVEN: The stream handle is registered on this engine.
        engine
            .data_streams
            .insert(stream_id, QuicStream::new(1024, 5, 1));
        // WHEN: A subscribe is targeted at the explicit stream.
        // THEN: The SUBSCRIBE packet is allocated and buffered on that stream.
        let sub_pid = engine
            .subscribe_on(stream, SubscribeCommand::single("topic/sub", 0))
            .unwrap();
        assert_eq!(sub_pid, 1);

        // GIVEN: A connected MQTT session with one known QUIC data stream.
        let mut engine = mqtt_connected_quic_engine();
        engine
            .data_streams
            .insert(stream_id, QuicStream::new(1024, 5, 1));
        // WHEN: An unsubscribe is targeted at the explicit stream.
        // THEN: The UNSUBSCRIBE packet is allocated and buffered on that stream.
        let unsub_pid = engine
            .unsubscribe_on(
                stream,
                UnsubscribeCommand::from_topics(vec!["topic/sub".to_string()]),
            )
            .unwrap();
        assert_eq!(unsub_pid, 1);

        // GIVEN: The default publish stream is already known.
        let mut engine = mqtt_connected_quic_engine();
        engine
            .data_streams
            .insert(stream_id, QuicStream::new(1024, 5, 1));
        engine.default_pub_stream = Some(stream_id);
        // WHEN: The high-level publish API is used.
        // THEN: It reuses the default publish stream instead of opening another.
        let publish_pid = engine
            .publish(PublishCommand::simple(
                "topic/default",
                b"x".to_vec(),
                1,
                false,
            ))
            .unwrap();
        assert_eq!(publish_pid, Some(1));
        let default_pub = engine.default_pub_stream.unwrap();
        assert_eq!(engine.ensure_default_pub_stream().unwrap(), default_pub);

        // GIVEN: The default subscribe stream is already known.
        let mut engine = mqtt_connected_quic_engine();
        engine
            .data_streams
            .insert(stream_id, QuicStream::new(1024, 5, 2));
        engine.default_sub_stream = Some(stream_id);
        // WHEN: The high-level subscribe and unsubscribe APIs are used.
        // THEN: Both commands reuse the default subscribe stream.
        let sub_pid = engine
            .subscribe(SubscribeCommand::single("topic/default", 0))
            .unwrap();
        assert_eq!(sub_pid, 1);
        let default_sub = engine.default_sub_stream.unwrap();
        let unsub_pid = engine
            .unsubscribe(UnsubscribeCommand::from_topics(vec![
                "topic/default".to_string()
            ]))
            .unwrap();
        assert_eq!(unsub_pid, 2);
        assert_eq!(engine.ensure_default_sub_stream().unwrap(), default_sub);
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_quic_data_stream_capacity_and_finished_errors() {
        use quinn_proto::{Dir, Side, StreamId};

        let mut engine = mqtt_connected_quic_engine();
        let stream_id = StreamId::new(Side::Client, Dir::Bi, 1);
        engine
            .data_streams
            .insert(stream_id, QuicStream::new(1024, 5, 1));
        let stream = u64::from(stream_id);

        engine
            .data_streams
            .get_mut(&stream_id)
            .unwrap()
            .pending_packets = 1;
        assert!(matches!(
            engine.publish_on(
                stream,
                PublishCommand::simple("topic/full", b"payload".to_vec(), 1, false)
            ),
            Err(MqttClientError::BufferFull {
                buffer_type,
                capacity: 1,
            }) if buffer_type == "quic_stream_outgoing"
        ));

        let ds = engine.data_streams.get_mut(&stream_id).unwrap();
        ds.pending_packets = 0;
        ds.finished = true;
        assert!(matches!(
            engine.subscribe_on(stream, SubscribeCommand::single("topic/finished", 0)),
            Err(MqttClientError::InvalidState { .. })
        ));
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_zero_rtt_config_default_and_connect_disables() {
        let default = QuicZeroRttConfig::default();
        assert_eq!(default.session_cache_size, 256);
        assert!(default.replay_on_reject);

        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        engine.zero_rtt_config = Some(default);
        engine.zero_rtt_status = QuicZeroRttStatus::Attempted;
        engine
            .pending_transport_events
            .push_back(MqttEvent::ZeroRttStatusChanged {
                status: QuicZeroRttStatus::Attempted,
            });
        QuicMqttEngine::journal_stream_open(
            &mut engine.early_stream_journal,
            StreamId::new(quinn_proto::Side::Client, Dir::Bi, 0),
            EarlyStreamRole::Control,
        );

        engine
            .connect(
                "127.0.0.1:4433".parse().unwrap(),
                "localhost",
                quic_test_crypto_config(),
                Instant::now(),
            )
            .unwrap();

        assert_eq!(engine.zero_rtt_config, None);
        assert_eq!(engine.zero_rtt_status(), QuicZeroRttStatus::Disabled);
        assert!(engine.pending_transport_events.is_empty());
        assert!(engine.early_stream_journal.is_empty());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_connect_with_zero_rtt_emits_unavailable_without_ticket() {
        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        let config = QuicZeroRttConfig {
            session_cache_size: 8,
            replay_on_reject: false,
        };

        engine
            .connect_with_zero_rtt(
                "127.0.0.1:4433".parse().unwrap(),
                "localhost",
                quic_test_crypto_config(),
                config,
                Instant::now(),
            )
            .unwrap();

        assert_eq!(engine.zero_rtt_config, Some(config));
        assert_eq!(engine.zero_rtt_status(), QuicZeroRttStatus::Unavailable);
        assert!(engine.zero_rtt_cache.is_some());

        let events = engine.take_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            MqttEvent::ZeroRttStatusChanged {
                status: QuicZeroRttStatus::Unavailable
            }
        ));
        assert!(engine.take_events().is_empty());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_zero_rtt_early_control_open_failure_falls_back_to_unavailable() {
        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        engine.zero_rtt_config = Some(QuicZeroRttConfig::default());

        engine.begin_zero_rtt_attempt(None);

        assert_eq!(engine.zero_rtt_status(), QuicZeroRttStatus::Unavailable);
        assert_eq!(engine.control_stream, None);
        assert!(engine.control_outgoing.is_empty());
        assert!(engine.early_stream_journal.is_empty());

        let events = engine.take_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            MqttEvent::ZeroRttStatusChanged {
                status: QuicZeroRttStatus::Unavailable
            }
        ));
        assert!(engine
            .publish(PublishCommand::simple(
                "topic",
                b"payload".to_vec(),
                0,
                false
            ))
            .is_err());
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_clearable_quic_session_cache_delegates_and_redacts() {
        let cache = ClearableQuicSessionCache::new(16);
        let name = ServerName::try_from("example.com").unwrap().to_owned();

        cache.set_kx_hint(name.clone(), NamedGroup::X25519);
        assert_eq!(cache.kx_hint(&name), Some(NamedGroup::X25519));

        let debug = format!("{:?}", cache);
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("example.com"));

        cache.clear();
        assert_eq!(cache.kx_hint(&name), None);
    }

    #[cfg(feature = "quic-proto")]
    #[test]
    fn test_early_stream_journal_records_enqueue_not_write() {
        let mut engine = QuicMqttEngine::new(MqttClientOptions::builder().build()).unwrap();
        engine.zero_rtt_status = QuicZeroRttStatus::Attempted;

        let control = StreamId::new(quinn_proto::Side::Client, Dir::Bi, 0);
        let data = StreamId::new(quinn_proto::Side::Client, Dir::Bi, 1);
        engine.control_stream = Some(control);
        QuicMqttEngine::journal_stream_open(
            &mut engine.early_stream_journal,
            control,
            EarlyStreamRole::Control,
        );
        QuicMqttEngine::journal_stream_open(
            &mut engine.early_stream_journal,
            data,
            EarlyStreamRole::DefaultPub,
        );
        engine
            .data_streams
            .insert(data, QuicStream::new(1024, 5, 10));

        QuicMqttEngine::append_control_outgoing_bytes(
            &mut engine.control_outgoing,
            &mut engine.early_stream_journal,
            engine.zero_rtt_status,
            engine.control_stream,
            &[0x10, 0x00],
            1,
        );
        engine.enqueue_on_stream(data, &[0x30, 0x01, b'x']).unwrap();

        let control_entry = engine
            .early_stream_journal
            .iter()
            .find(|entry| entry.original_stream_id == control)
            .unwrap();
        assert_eq!(control_entry.bytes, vec![0x10, 0x00]);
        assert_eq!(control_entry.packet_count, 1);

        let data_entry = engine
            .early_stream_journal
            .iter()
            .find(|entry| entry.original_stream_id == data)
            .unwrap();
        assert_eq!(data_entry.bytes, vec![0x30, 0x01, b'x']);
        assert_eq!(data_entry.packet_count, 1);

        // Simulate a partial QUIC write draining the stream buffer. The replay
        // journal must keep the full original enqueue bytes.
        engine
            .data_streams
            .get_mut(&data)
            .unwrap()
            .outgoing
            .drain(..1);
        let data_entry = engine
            .early_stream_journal
            .iter()
            .find(|entry| entry.original_stream_id == data)
            .unwrap();
        assert_eq!(data_entry.bytes, vec![0x30, 0x01, b'x']);
    }
}
