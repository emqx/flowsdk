use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv3::{connectv3, disconnectv3, pingreqv3, pubrelv3, unsubscribev3};
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
use super::opts::MqttClientOptions;

/// Events emitted by the MqttEngine to be handled by the application (I/O layer)
#[derive(Debug)]
pub enum MqttEvent {
    Connected(ConnectionResult),
    Disconnected(Option<u8>),
    Published(PublishResult),
    Subscribed(SubscribeResult),
    Unsubscribed(UnsubscribeResult),
    MessageReceived(MqttPublish),
    PingResponse(PingResult),
    Error(MqttClientError),
    /// Signal that a reconnection is needed (e.g. after keep-alive timeout)
    ReconnectNeeded,
}

/// A Sans-I/O MQTT protocol engine.
pub struct MqttEngine {
    options: MqttClientOptions,
    session: Option<ClientSession>,
    priority_queue: PriorityQueue<u8, PublishCommand>,
    is_connected: bool,
    last_packet_sent: Instant,
    last_packet_received: Instant,

    // Buffers and Parsers
    parser: MqttParser,
    outgoing_buffer: VecDeque<Vec<u8>>,

    // Pending operations tracking (state only)
    pending_subscribes: HashMap<u16, Vec<String>>,
    pending_unsubscribes: HashMap<u16, Vec<String>>,
    pending_publishes: HashMap<u16, Instant>,

    mqtt_version: u8,
    events: Vec<MqttEvent>,
}

impl MqttEngine {
    pub fn new(options: MqttClientOptions) -> Self {
        let mqtt_version = options.mqtt_version;
        // Default buffer size 16KB
        let parser = MqttParser::new(16384, mqtt_version);

        Self {
            options,
            session: None,
            priority_queue: PriorityQueue::new(1000),
            is_connected: false,
            last_packet_sent: Instant::now(),
            last_packet_received: Instant::now(),
            parser,
            outgoing_buffer: VecDeque::new(),
            pending_subscribes: HashMap::new(),
            pending_unsubscribes: HashMap::new(),
            pending_publishes: HashMap::new(),
            mqtt_version,
            events: Vec::new(),
        }
    }

    /// Take all pending events from the engine.
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

    /// Process raw incoming data from the network.
    pub fn handle_incoming(&mut self, data: &[u8]) -> Vec<MqttEvent> {
        self.parser.feed(data);
        self.last_packet_received = Instant::now();

        loop {
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
        self.mqtt_version
    }

    /// Handle time-sensitive operations like keep-alive.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        if !self.is_connected {
            return self.take_events();
        }

        let keep_alive = Duration::from_secs(self.options.keep_alive as u64);
        if keep_alive > Duration::ZERO {
            // Keep alive check (send Ping)
            if now.duration_since(self.last_packet_sent) >= keep_alive {
                self.send_ping();
                self.last_packet_sent = now;
            }

            // Timeout check (no response)
            if now.duration_since(self.last_packet_received) >= keep_alive * 2 {
                self.events.push(MqttEvent::ReconnectNeeded);
                self.is_connected = false;
                return self.take_events();
            }
        }

        // Retransmissions
        let retrans_events = self.handle_retransmissions(now);
        self.events.extend(retrans_events);

        self.take_events()
    }

    fn handle_retransmissions(&mut self, now: Instant) -> Vec<MqttEvent> {
        let events = Vec::new();
        let timeout = Duration::from_secs(5); // Default retransmission timeout

        let mut to_resend = Vec::new();
        for (&pid, &sent_at) in &self.pending_publishes {
            if now.duration_since(sent_at) >= timeout {
                to_resend.push(pid);
            }
        }

        for pid in to_resend {
            // In a real implementation, we would store the original MqttPublish
            // and resend it with dup = true.
            // For now, this is a placeholder.
            self.pending_publishes.insert(pid, now);
            // events.push(MqttEvent::Error(MqttClientError::ProtocolViolation { message: format!("Retransmitting packet {}", pid) }));
        }

        events
    }

    pub fn next_tick_at(&self) -> Option<Instant> {
        if !self.is_connected {
            return None;
        }
        let keep_alive = Duration::from_secs(self.options.keep_alive as u64);
        let mut next = None;

        if keep_alive > Duration::ZERO {
            next = Some(self.last_packet_sent + keep_alive);
        }

        // Also check retransmission timeouts
        for &sent_at in self.pending_publishes.values() {
            let resend_at = sent_at + Duration::from_secs(5);
            if next.is_none() || resend_at < next.unwrap() {
                next = Some(resend_at);
            }
        }

        next
    }

    pub fn take_outgoing(&mut self) -> Vec<u8> {
        let mut all_bytes = Vec::new();
        while let Some(packet) = self.outgoing_buffer.pop_front() {
            all_bytes.extend(packet);
        }
        all_bytes
    }

    // --- Command Methods ---

    pub fn connect(&mut self) {
        if self.is_connected {
            return;
        }

        // Initialize session if needed
        if self.session.is_none() {
            self.session = Some(ClientSession::new());
        }

        let packet = if self.mqtt_version == 5 {
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

        self.enqueue_packet(packet);
    }

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

        if let Some(pid) = pid {
            self.pending_publishes.insert(pid, Instant::now());
        }

        self.priority_queue.enqueue(command.priority, command);
        self.process_queue();
        Ok(pid)
    }

    pub fn subscribe(&mut self, mut command: SubscribeCommand) -> Result<u16, MqttClientError> {
        let pid = if let Some(pid) = command.packet_id {
            pid
        } else {
            let pid = self.next_packet_id()?;
            command.packet_id = Some(pid);
            pid
        };

        let topics: Vec<String> = command
            .subscriptions
            .iter()
            .map(|s| s.topic_filter.clone())
            .collect();
        self.pending_subscribes.insert(pid, topics);

        let packet = MqttPacket::Subscribe5(subscribev5::MqttSubscribe::new(
            pid,
            command.subscriptions,
            command.properties,
        ));

        self.enqueue_packet(packet);
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

        self.pending_unsubscribes
            .insert(pid, command.topics.clone());

        let packet = if self.mqtt_version == 5 {
            MqttPacket::Unsubscribe5(unsubscribev5::MqttUnsubscribe::new(
                pid,
                command.topics.clone(),
                command.properties,
            ))
        } else {
            MqttPacket::Unsubscribe3(unsubscribev3::MqttUnsubscribe::new(pid, command.topics))
        };

        self.enqueue_packet(packet);
        Ok(pid)
    }

    pub fn disconnect(&mut self) {
        if !self.is_connected {
            return;
        }

        let packet = if self.mqtt_version == 5 {
            MqttPacket::Disconnect5(disconnectv5::MqttDisconnect::new(0, Vec::new()))
        } else {
            MqttPacket::Disconnect3(disconnectv3::MqttDisconnect::new())
        };

        self.enqueue_packet(packet);
        self.is_connected = false;
    }

    pub fn auth(&mut self, reason_code: u8, properties: Vec<Property>) {
        if self.mqtt_version == 5 {
            let auth = authv5::MqttAuth::new(reason_code, properties);
            self.enqueue_packet(MqttPacket::Auth(auth));
        }
    }

    // --- Internal Helpers ---

    fn handle_packet(&mut self, packet: MqttPacket) -> Vec<MqttEvent> {
        let mut events = Vec::new();
        match packet {
            MqttPacket::ConnAck5(ack) => {
                self.is_connected = ack.reason_code == 0;
                events.push(MqttEvent::Connected(ConnectionResult {
                    reason_code: ack.reason_code,
                    session_present: ack.session_present,
                    properties: ack.properties,
                }));
            }
            MqttPacket::ConnAck3(ack) => {
                self.is_connected = ack.return_code == 0;
                events.push(MqttEvent::Connected(ConnectionResult {
                    reason_code: ack.return_code,
                    session_present: ack.session_present,
                    properties: None,
                }));
            }
            MqttPacket::PubAck5(ack) => {
                if self.pending_publishes.remove(&ack.packet_id).is_some() {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(ack.packet_id),
                        reason_code: Some(ack.reason_code),
                        properties: Some(ack.properties),
                        qos: 1,
                    }));
                }
            }
            MqttPacket::PubAck3(ack) => {
                if self.pending_publishes.remove(&ack.message_id).is_some() {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(ack.message_id),
                        reason_code: Some(0), // v3 has no reason code in PUBACK
                        properties: None,
                        qos: 1,
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
                        self.enqueue_packet(ack);
                    }
                } else if qos == 2 {
                    if let Some(pid) = pid {
                        let rec = MqttPacket::PubRec5(MqttPubRec::new(pid, 0, Vec::new()));
                        self.enqueue_packet(rec);
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
                        self.enqueue_packet(ack);
                    }
                } else if qos == 2 {
                    if let Some(pid) = pid {
                        let rec = MqttPacket::PubRec3(
                            crate::mqtt_serde::mqttv3::pubrec::MqttPubRec::new(pid),
                        );
                        self.enqueue_packet(rec);
                    }
                }
            }
            MqttPacket::SubAck5(ack) => {
                let _topics = self.pending_subscribes.remove(&ack.packet_id);
                events.push(MqttEvent::Subscribed(SubscribeResult {
                    packet_id: ack.packet_id,
                    reason_codes: ack.reason_codes,
                    properties: ack.properties,
                }));
            }
            MqttPacket::SubAck3(ack) => {
                let _topics = self.pending_subscribes.remove(&ack.message_id);
                events.push(MqttEvent::Subscribed(SubscribeResult {
                    packet_id: ack.message_id,
                    reason_codes: ack.return_codes,
                    properties: Vec::new(),
                }));
            }
            MqttPacket::UnsubAck5(ack) => {
                let _topics = self.pending_unsubscribes.remove(&ack.packet_id);
                events.push(MqttEvent::Unsubscribed(UnsubscribeResult {
                    packet_id: ack.packet_id,
                    reason_codes: ack.reason_codes,
                    properties: ack.properties,
                }));
            }
            MqttPacket::UnsubAck3(ack) => {
                let _topics = self.pending_unsubscribes.remove(&ack.message_id);
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
                let rel = MqttPacket::PubRel5(MqttPubRel::new(rec.packet_id, 0, Vec::new()));
                self.enqueue_packet(rel);
            }
            MqttPacket::PubRec3(rec) => {
                let rel = MqttPacket::PubRel3(pubrelv3::MqttPubRel::new(rec.message_id));
                self.enqueue_packet(rel);
            }
            MqttPacket::PubRel5(rel) => {
                let comp = MqttPacket::PubComp5(MqttPubComp::new(rel.packet_id, 0, Vec::new()));
                self.enqueue_packet(comp);
            }
            MqttPacket::PubRel3(rel) => {
                let comp = MqttPacket::PubComp3(
                    crate::mqtt_serde::mqttv3::pubcomp::MqttPubComp::new(rel.message_id),
                );
                self.enqueue_packet(comp);
            }
            MqttPacket::PubComp5(comp) => {
                if self.pending_publishes.remove(&comp.packet_id).is_some() {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(comp.packet_id),
                        reason_code: Some(comp.reason_code),
                        properties: Some(comp.properties),
                        qos: 2,
                    }));
                }
            }
            MqttPacket::PubComp3(comp) => {
                if self.pending_publishes.remove(&comp.message_id).is_some() {
                    events.push(MqttEvent::Published(PublishResult {
                        packet_id: Some(comp.message_id),
                        reason_code: Some(0),
                        properties: None,
                        qos: 2,
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
        let packet = if self.mqtt_version == 5 {
            MqttPacket::PingReq5(pingreqv5::MqttPingReq::new())
        } else {
            MqttPacket::PingReq3(pingreqv3::MqttPingReq::new())
        };
        self.enqueue_packet(packet);
    }

    pub fn enqueue_packet(&mut self, packet: MqttPacket) {
        if let Ok(bytes) = packet.to_bytes() {
            self.outgoing_buffer.push_back(bytes);
            self.last_packet_sent = Instant::now();
        }
    }

    fn process_queue(&mut self) {
        if !self.is_connected {
            return;
        }

        while let Some((_priority, mut command)) = self.priority_queue.dequeue() {
            let _pid = if command.qos > 0 {
                if let Some(pid) = command.packet_id {
                    Some(pid)
                } else {
                    match self.next_packet_id() {
                        Ok(id) => {
                            self.pending_publishes.insert(id, Instant::now());
                            command.packet_id = Some(id);
                            Some(id)
                        }
                        Err(e) => {
                            self.events.push(MqttEvent::Error(e));
                            continue;
                        }
                    }
                }
            } else {
                None
            };

            let packet = if self.mqtt_version == 5 {
                MqttPacket::Publish5(command.to_mqtt_publish())
            } else {
                MqttPacket::Publish3(command.to_mqttv3_publish())
            };
            self.enqueue_packet(packet);
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
