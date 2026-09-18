// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReceiveStage {
    Publish(u8),
    PubRec,
    PubRel,
}

pub(super) struct ProtocolState {
    pub reserved: HashSet<u16>,
    pub received: HashMap<u16, (ReceiveStage, Option<u64>)>,
    // Session exchanges can outlive the connection on which PUBLISH used quota.
    pub received_on_connection: HashSet<u16>,
    pub incoming_quota_used: usize,
    pub incoming_aliases: HashMap<u16, String>,
    pub outgoing_aliases: HashMap<u16, String>,
    pub incoming_alias_maximum: u16,
    pub outgoing_alias_maximum: u16,
    pub incoming_receive_maximum: u16,
    pub maximum_packet_size: Option<usize>,
    pub maximum_qos: u8,
    pub retain_available: bool,
    pub wildcard_available: bool,
    pub shared_available: bool,
    pub subscription_ids_available: bool,
    pub keep_alive: u16,
    pub session_expiry: u32,
    // CONNECT creates an ID allocator before a successful CONNACK establishes state.
    pub has_session: bool,
    pub connecting: bool,
    // Remains terminal after a reset/close until CONNECT is successfully queued.
    pub input_closed: bool,
    pub reauthenticating: bool,
    pub queued_bytes: usize,
    pub control_reserve: usize,
    pub replay_reserve: usize,
    pub responses: VecDeque<MqttPacket>,
    pub deadlines: HashMap<(OperationKind, Option<u16>), (Instant, Duration)>,
}

impl ProtocolState {
    pub fn new(keep_alive: u16) -> Self {
        Self {
            reserved: HashSet::new(),
            received: HashMap::new(),
            received_on_connection: HashSet::new(),
            incoming_quota_used: 0,
            incoming_aliases: HashMap::new(),
            outgoing_aliases: HashMap::new(),
            incoming_alias_maximum: 0,
            outgoing_alias_maximum: 0,
            incoming_receive_maximum: u16::MAX,
            maximum_packet_size: None,
            maximum_qos: 2,
            retain_available: true,
            wildcard_available: true,
            shared_available: true,
            subscription_ids_available: true,
            keep_alive,
            session_expiry: 0,
            has_session: false,
            connecting: false,
            input_closed: false,
            reauthenticating: false,
            queued_bytes: 0,
            control_reserve: 6,
            replay_reserve: 0,
            responses: VecDeque::new(),
            deadlines: HashMap::new(),
        }
    }
}

fn invalid(field: &str, reason: &str) -> MqttClientError {
    MqttClientError::InvalidConfiguration {
        field: field.into(),
        reason: reason.into(),
    }
}

impl MqttEngine {
    #[cfg(feature = "rustls-tls")]
    pub(crate) fn defer_events(&mut self, events: Vec<MqttEvent>) {
        self.events.extend(events);
    }

    pub(super) fn connect_properties(&self) -> Result<Vec<Property>, MqttClientError> {
        use Property::*;
        let mut properties = self.options.connect_properties.clone();
        if let Some(v) = self.options.session_expiry_interval {
            properties.push(SessionExpiryInterval(v));
        }
        if let Some(v) = self.options.maximum_packet_size {
            properties.push(MaximumPacketSize(v));
        }
        if let Some(v) = self.options.request_response_information {
            properties.push(RequestResponseInformation(v.into()));
        }
        if let Some(v) = self.options.request_problem_information {
            properties.push(RequestProblemInformation(v.into()));
        }
        if let Some(v) = self.options.incoming_receive_maximum {
            properties.push(ReceiveMaximum(v));
        }
        let mut merged = Vec::new();
        for property in properties {
            match &property {
                ReceiveMaximum(0) | MaximumPacketSize(0) => {
                    return Err(invalid(
                        "CONNECT",
                        "Receive Maximum and Maximum Packet Size must be nonzero",
                    ))
                }
                RequestResponseInformation(v) | RequestProblemInformation(v) if *v > 1 => {
                    return Err(invalid("CONNECT", "Boolean property must be 0 or 1"))
                }
                SessionExpiryInterval(_)
                | ReceiveMaximum(_)
                | MaximumPacketSize(_)
                | TopicAliasMaximum(_)
                | RequestResponseInformation(_)
                | RequestProblemInformation(_)
                | UserProperty(_, _)
                | AuthenticationMethod(_)
                | AuthenticationData(_) => {}
                _ => return Err(invalid("CONNECT", "Property is not valid in CONNECT")),
            }
            if !matches!(property, UserProperty(_, _)) {
                if let Some(existing) = merged
                    .iter()
                    .find(|p| std::mem::discriminant(*p) == std::mem::discriminant(&property))
                {
                    if existing != &property {
                        return Err(invalid("CONNECT", "Conflicting singleton property values"));
                    }
                    continue;
                }
            }
            merged.push(property);
        }
        if merged.iter().any(|p| matches!(p, AuthenticationData(_)))
            && !merged.iter().any(|p| matches!(p, AuthenticationMethod(_)))
        {
            return Err(invalid(
                "CONNECT",
                "Authentication Data requires Authentication Method",
            ));
        }
        Ok(merged)
    }

    pub(super) fn negotiate(&mut self, properties: &[Property]) -> Result<(), MqttClientError> {
        use Property::*;
        let mut seen = HashSet::new();
        for p in properties {
            if !matches!(p, UserProperty(_, _)) && !seen.insert(std::mem::discriminant(p)) {
                return Err(invalid("CONNACK", "Duplicate singleton property"));
            }
            match p {
                ReceiveMaximum(0) | MaximumPacketSize(0) => {
                    return Err(invalid("CONNACK", "Limit must be nonzero"))
                }
                MaximumQoS(v) if *v > 1 => {
                    return Err(invalid("CONNACK", "Maximum QoS must be 0 or 1"))
                }
                RetainAvailable(v)
                | WildcardSubscriptionAvailable(v)
                | SharedSubscriptionAvailable(v)
                | SubscriptionIdentifierAvailable(v)
                    if *v > 1 =>
                {
                    return Err(invalid("CONNACK", "Boolean property must be 0 or 1"))
                }
                ReceiveMaximum(v) => self
                    .inflight_queue
                    .update_receive_maximum((*v).min(self.options.receive_maximum)),
                MaximumPacketSize(v) => self.reliability.maximum_packet_size = Some(*v as usize),
                ServerKeepAlive(v) => self.reliability.keep_alive = *v,
                MaximumQoS(v) => self.reliability.maximum_qos = *v,
                RetainAvailable(v) => self.reliability.retain_available = *v != 0,
                WildcardSubscriptionAvailable(v) => self.reliability.wildcard_available = *v != 0,
                SharedSubscriptionAvailable(v) => self.reliability.shared_available = *v != 0,
                SubscriptionIdentifierAvailable(v) => {
                    self.reliability.subscription_ids_available = *v != 0
                }
                TopicAliasMaximum(v) => self.reliability.outgoing_alias_maximum = *v,
                AssignedClientIdentifier(v) if !v.is_empty() => self.options.client_id = v.clone(),
                SessionExpiryInterval(v) => self.reliability.session_expiry = *v,
                AssignedClientIdentifier(_) => {
                    return Err(invalid("CONNACK", "Assigned client ID is empty"))
                }
                AuthenticationMethod(_)
                | AuthenticationData(_)
                | ResponseInformation(_)
                | ServerReference(_)
                | ReasonString(_)
                | UserProperty(_, _) => {}
                _ => return Err(invalid("CONNACK", "Property is not valid in CONNACK")),
            }
        }
        // The next CONNECT uses the assigned identifier, including any change
        // in the encoded Remaining Length. Sizing must not alter negotiated state.
        let reserve = self
            .connect_packet()?
            .to_bytes()
            .map_err(MqttClientError::from)?
            .len()
            .max(6);
        if self
            .options
            .max_outgoing_buffer_bytes
            .is_some_and(|limit| reserve > limit)
        {
            return Err(invalid(
                "max_outgoing_buffer_bytes",
                "The reconnect CONNECT packet exceeds the outgoing byte limit",
            ));
        }
        self.reliability.control_reserve = reserve;
        Ok(())
    }

    pub(super) fn candidate_id(&mut self, requested: Option<u16>) -> Result<u16, MqttClientError> {
        if let Some(id) = requested {
            if id == 0
                || self.reliability.reserved.contains(&id)
                || self.inflight_queue.contains(id)
            {
                return Err(MqttClientError::InvalidPacketId { packet_id: id });
            }
            Ok(id)
        } else {
            self.next_packet_id()
        }
    }

    pub(super) fn start_deadline(&mut self, operation: OperationKind, id: Option<u16>) {
        let timeouts = self.options.operation_timeouts;
        let duration = match operation {
            OperationKind::Connect => timeouts.connect,
            OperationKind::Publish => timeouts.publish,
            OperationKind::Subscribe => timeouts.subscribe,
            OperationKind::Unsubscribe => timeouts.unsubscribe,
        };
        if let Some(duration) = duration {
            self.reliability
                .deadlines
                .insert((operation, id), (Instant::now() + duration, duration));
        }
    }

    pub(super) fn finish_operation(&mut self, id: u16) {
        self.reliability.reserved.remove(&id);
        if self.reliability.reserved.is_empty() {
            self.reliability.replay_reserve = 0;
        }
        self.reliability
            .deadlines
            .retain(|(_, pid), _| *pid != Some(id));
    }

    pub(super) fn check_deadlines(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .reliability
            .deadlines
            .iter()
            .filter(|(_, (at, _))| now >= *at)
            .map(|(key, (_, duration))| (*key, *duration))
            .collect();
        for ((operation, packet_id), duration) in expired {
            self.reliability.deadlines.remove(&(operation, packet_id));
            self.events.push(MqttEvent::OperationFailed {
                operation,
                packet_id,
                error: MqttClientError::OperationTimeout {
                    operation: format!("{operation:?}"),
                    timeout_ms: duration.as_millis().min(u64::MAX as u128) as u64,
                },
            });
            if operation == OperationKind::Connect {
                self.connection_lost_at(now);
            }
        }
    }

    pub(super) fn validate_publish(
        &self,
        command: &mut PublishCommand,
    ) -> Result<Option<(u16, String)>, MqttClientError> {
        if command.qos > self.reliability.maximum_qos
            || (command.retain && !self.reliability.retain_available)
        {
            return Err(invalid(
                "PUBLISH",
                "Broker does not support requested QoS or retain",
            ));
        }
        let aliases: Vec<_> = command
            .properties
            .iter()
            .filter_map(|p| {
                if let Property::TopicAlias(v) = p {
                    Some(*v)
                } else {
                    None
                }
            })
            .collect();
        let mut registration = None;
        if let Some(&alias) = aliases.first() {
            if aliases.len() != 1 || alias == 0 || alias > self.reliability.outgoing_alias_maximum {
                return Err(invalid(
                    "Topic Alias",
                    "Alias exceeds negotiated maximum or is duplicated",
                ));
            }
            if command.topic_name.is_empty() {
                command.topic_name = self
                    .reliability
                    .outgoing_aliases
                    .get(&alias)
                    .cloned()
                    .ok_or_else(|| invalid("Topic Alias", "Undefined alias"))?;
            } else {
                registration = Some((alias, command.topic_name.clone()));
            }
        }
        Ok(registration)
    }

    pub(super) fn validate_subscribe(
        &self,
        command: &SubscribeCommand,
    ) -> Result<(), MqttClientError> {
        if !self.reliability.subscription_ids_available
            && command
                .properties
                .iter()
                .any(|p| matches!(p, Property::SubscriptionIdentifier(_)))
        {
            return Err(invalid(
                "SUBSCRIBE",
                "Broker does not support subscription identifiers",
            ));
        }
        for s in &command.subscriptions {
            if (!self.reliability.wildcard_available && s.topic_filter.contains(['+', '#']))
                || (!self.reliability.shared_available && s.topic_filter.starts_with("$share/"))
            {
                return Err(invalid(
                    "SUBSCRIBE",
                    "Broker does not support this subscription",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_packet_capabilities(
        &self,
        packet: &MqttPacket,
    ) -> Result<(), MqttClientError> {
        let (qos, retain) = match packet {
            MqttPacket::Publish5(p) => (p.qos, p.retain),
            MqttPacket::Publish3(p) => (p.qos, p.retain),
            _ => return Ok(()),
        };
        if qos > self.reliability.maximum_qos || (retain && !self.reliability.retain_available) {
            return Err(invalid(
                "PUBLISH",
                "Broker does not support requested QoS or retain",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_input_buffer(
        options: &MqttClientOptions,
        bytes: &[u8],
    ) -> Result<(), MqttClientError> {
        if let Some(capacity) = options.max_incoming_buffer_bytes {
            if bytes.len() > capacity {
                return Err(MqttClientError::BufferFull {
                    buffer_type: "incoming bytes".into(),
                    capacity,
                });
            }
        }
        let advertised = options.maximum_packet_size.map(|v| v as usize).or_else(|| {
            options.connect_properties.iter().find_map(|p| {
                if let Property::MaximumPacketSize(v) = p {
                    Some(*v as usize)
                } else {
                    None
                }
            })
        });
        if let Some(capacity) = options
            .max_incoming_packet_size
            .into_iter()
            .chain(advertised)
            .min()
        {
            if bytes.len() >= 2 {
                if let Ok((length, n)) = crate::mqtt_serde::parser::parse_vbi(&bytes[1..]) {
                    if length + n + 1 > capacity {
                        return Err(MqttClientError::BufferFull {
                            buffer_type: "incoming packet".into(),
                            capacity,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn check_packet_size(&self, bytes: &[u8]) -> Result<(), MqttClientError> {
        if let Some(capacity) = self.options.max_outgoing_buffer_bytes {
            if bytes.len() > capacity {
                return Err(MqttClientError::BufferFull {
                    buffer_type: "outgoing packet".into(),
                    capacity,
                });
            }
        }
        if self
            .reliability
            .maximum_packet_size
            .is_some_and(|max| bytes.len() > max)
        {
            return Err(invalid(
                "packet size",
                "Packet exceeds the broker's Maximum Packet Size",
            ));
        }
        Ok(())
    }

    pub(super) fn outgoing_bytes(&self) -> usize {
        self.reliability.queued_bytes + self.outgoing_buffer.iter().map(Vec::len).sum::<usize>()
    }

    pub(super) fn check_output_capacity(&self, additional: usize) -> Result<(), MqttClientError> {
        if let Some(capacity) = self.options.max_outgoing_buffer_bytes {
            if self.outgoing_bytes().saturating_add(additional) > capacity {
                return Err(MqttClientError::BufferFull {
                    buffer_type: "outgoing bytes".into(),
                    capacity,
                });
            }
        }
        Ok(())
    }

    pub(super) fn flush_responses(&mut self) {
        while let Some(packet) = self.reliability.responses.front().cloned() {
            if let Some(capacity) = self.options.max_outgoing_buffer_bytes {
                if packet.to_bytes().is_ok_and(|bytes| bytes.len() > capacity) {
                    self.fail_connection(MqttClientError::BufferFull {
                        buffer_type: "protocol response".into(),
                        capacity,
                    });
                    break;
                }
            }
            match self.enqueue_packet(packet) {
                Ok(()) => {
                    self.reliability.responses.pop_front();
                }
                Err(MqttClientError::BufferFull { .. }) => break,
                Err(error) => {
                    self.fail_connection(error);
                    break;
                }
            }
        }
    }

    pub(super) fn close_input(&mut self) {
        self.is_connected = false;
        self.reliability.connecting = false;
        self.reliability.input_closed = true;
        self.reliability.reauthenticating = false;
        self.reliability.responses.clear();
        self.reliability.deadlines.clear();
        self.reliability.received_on_connection.clear();
        self.reliability.incoming_quota_used = 0;
        self.parser.buffer_mut().clear();
        self.ping_sent_at = None;
    }

    pub(super) fn accept_incoming(
        &mut self,
        packet_type: crate::mqtt_serde::control_packet::ControlPacketType,
    ) -> bool {
        use crate::mqtt_serde::control_packet::ControlPacketType::*;
        if self.reliability.input_closed {
            return false;
        }
        let valid = match packet_type {
            CONNACK => self.reliability.connecting && !self.is_connected,
            AUTH | DISCONNECT => {
                self.is_connected || (self.mqtt_version() == 5 && self.reliability.connecting)
            }
            _ => self.is_connected,
        };
        if !valid {
            self.fail_connection(MqttClientError::ProtocolViolation {
                message: format!("Unexpected {packet_type:?} in the current connection state"),
            });
        }
        valid
    }

    pub(super) fn fail_connection(&mut self, error: MqttClientError) {
        self.connection_lost_at(Instant::now());
        self.events.push(MqttEvent::Error(error));
        self.events.push(MqttEvent::Disconnected(None));
    }

    pub(super) fn resolve_incoming(
        &mut self,
        publish: &mut MqttPublish,
    ) -> Result<(), MqttClientError> {
        let aliases: Vec<_> = publish
            .properties
            .iter()
            .filter_map(|p| {
                if let Property::TopicAlias(v) = p {
                    Some(*v)
                } else {
                    None
                }
            })
            .collect();
        if let Some(&alias) = aliases.first() {
            if aliases.len() != 1 || alias == 0 || alias > self.reliability.incoming_alias_maximum {
                return Err(invalid("Topic Alias", "Invalid incoming alias"));
            }
            if publish.topic_name.is_empty() {
                publish.topic_name = self
                    .reliability
                    .incoming_aliases
                    .get(&alias)
                    .cloned()
                    .ok_or_else(|| invalid("Topic Alias", "Undefined incoming alias"))?;
            } else {
                self.reliability
                    .incoming_aliases
                    .insert(alias, publish.topic_name.clone());
            }
        }
        if publish.topic_name.is_empty() {
            return Err(invalid("PUBLISH", "Empty topic without a defined alias"));
        }
        Ok(())
    }

    pub(super) fn receive_publish(
        &mut self,
        qos: u8,
        id: Option<u16>,
        stream: Option<u64>,
    ) -> Result<bool, MqttClientError> {
        let Some(id) = id.filter(|_| qos > 0) else {
            return Ok(true);
        };
        if id == 0 {
            return Err(MqttClientError::InvalidPacketId { packet_id: id });
        }
        if let Some((stage, previous_stream)) = self.reliability.received.get(&id) {
            if (qos == 1) != matches!(stage, ReceiveStage::Publish(1)) {
                return Err(MqttClientError::InvalidPacketId { packet_id: id });
            }
            if previous_stream.is_some() && *previous_stream != stream {
                return Err(invalid("PUBLISH", "QoS handshake moved between streams"));
            }
        }
        if !self.reliability.received_on_connection.contains(&id) {
            if self.reliability.incoming_quota_used
                >= self.reliability.incoming_receive_maximum as usize
            {
                return Err(MqttClientError::ProtocolViolation {
                    message: "Incoming Receive Maximum exceeded".into(),
                });
            }
            self.reliability.received_on_connection.insert(id);
            self.reliability.incoming_quota_used += 1;
        }
        if let Some((_, previous_stream)) = self.reliability.received.get_mut(&id) {
            *previous_stream = stream;
            return Ok(false);
        }
        self.reliability
            .received
            .insert(id, (ReceiveStage::Publish(qos), stream));
        Ok(true)
    }

    pub(super) fn manual_ack_packet(
        &self,
        id: u16,
        kind: u8,
        reason: u8,
        properties: Vec<Property>,
        stream: Option<u64>,
    ) -> Result<MqttPacket, MqttClientError> {
        if id == 0 {
            return Err(MqttClientError::InvalidPacketId { packet_id: id });
        }
        let valid = self
            .reliability
            .received
            .get(&id)
            .is_some_and(|(stage, channel)| {
                *channel == stream
                    && matches!(
                        (kind, stage),
                        (4, ReceiveStage::Publish(1))
                            | (5, ReceiveStage::Publish(2))
                            | (5, ReceiveStage::PubRec)
                            | (7, ReceiveStage::PubRel)
                    )
            });
        if !valid {
            return Err(MqttClientError::InvalidPacketId { packet_id: id });
        }
        if self.mqtt_version() != 5 && (reason != 0 || !properties.is_empty()) {
            return Err(invalid(
                "acknowledgement",
                "MQTT 3 does not support reason codes or properties",
            ));
        }
        if (kind == 7 && !matches!(reason, 0 | 0x92))
            || (kind != 7
                && !matches!(
                    reason,
                    0 | 0x10 | 0x80 | 0x83 | 0x87 | 0x90 | 0x91 | 0x97 | 0x99
                ))
        {
            return Err(invalid("acknowledgement", "Invalid reason code"));
        }
        if properties
            .iter()
            .any(|p| !matches!(p, Property::ReasonString(_) | Property::UserProperty(_, _)))
            || properties
                .iter()
                .filter(|p| matches!(p, Property::ReasonString(_)))
                .count()
                > 1
        {
            return Err(invalid("acknowledgement", "Invalid properties"));
        }
        Ok(match (self.mqtt_version(), kind) {
            (5, 4) => MqttPacket::PubAck5(MqttPubAck::new(id, reason, properties)),
            (5, 5) => MqttPacket::PubRec5(MqttPubRec::new(id, reason, properties)),
            (5, 7) => MqttPacket::PubComp5(MqttPubComp::new(id, reason, properties)),
            (_, 4) => MqttPacket::PubAck3(crate::mqtt_serde::mqttv3::pubackv3::MqttPubAck::new(id)),
            (_, 5) => MqttPacket::PubRec3(crate::mqtt_serde::mqttv3::pubrecv3::MqttPubRec::new(id)),
            (_, 7) => {
                MqttPacket::PubComp3(crate::mqtt_serde::mqttv3::pubcompv3::MqttPubComp::new(id))
            }
            _ => unreachable!(),
        })
    }

    pub(super) fn commit_receive_ack(&mut self, id: u16, kind: u8, reason: u8) {
        if kind == 5 && reason < 0x80 {
            if let Some((stage, _)) = self.reliability.received.get_mut(&id) {
                *stage = ReceiveStage::PubRec;
            }
        } else {
            self.reliability.received.remove(&id);
            self.reliability.received_on_connection.remove(&id);
            // Even PUBCOMP for a resumed PUBREL replenishes the peer's quota,
            // capped at the initial limit (MQTT 5 section 4.9).
            self.reliability.incoming_quota_used =
                self.reliability.incoming_quota_used.saturating_sub(1);
        }
    }

    fn manual_ack(
        &mut self,
        id: u16,
        kind: u8,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        let packet = self.manual_ack_packet(id, kind, reason, properties, None)?;
        self.enqueue_packet(packet)?;
        self.commit_receive_ack(id, kind, reason);
        Ok(())
    }

    pub fn puback(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.manual_ack(id, 4, reason, properties)
    }
    pub fn pubrec(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.manual_ack(id, 5, reason, properties)
    }
    pub fn pubcomp(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.manual_ack(id, 7, reason, properties)
    }
}

impl MqttEngine {
    pub(super) fn receive_message(
        &mut self,
        mut publish: MqttPublish,
        stream: Option<u64>,
    ) -> (Vec<MqttEvent>, Vec<MqttPacket>) {
        if let Err(error) = self.resolve_incoming(&mut publish) {
            self.fail_connection(error);
            return (self.take_events(), vec![]);
        }
        let qos = publish.qos;
        let pid = publish.packet_id;
        let deliver = match self.receive_publish(qos, pid, stream) {
            Ok(deliver) => deliver,
            Err(error) => {
                self.fail_connection(error);
                return (self.take_events(), vec![]);
            }
        };
        let mut events = Vec::new();
        if deliver {
            events.push(MqttEvent::PublishReceived {
                packet_id: pid,
                stream,
            });
            events.push(MqttEvent::MessageReceived(publish));
        }
        let mut responses = Vec::new();
        if let Some(id) = pid.filter(|_| qos > 0) {
            let already_acknowledged = self
                .reliability
                .received
                .get(&id)
                .is_some_and(|(stage, _)| *stage == ReceiveStage::PubRec);
            if self.options.auto_ack || already_acknowledged {
                let kind = if qos == 1 { 4 } else { 5 };
                match self.manual_ack_packet(id, kind, 0, vec![], stream) {
                    Ok(packet) => {
                        responses.push(packet);
                        self.commit_receive_ack(id, kind, 0);
                    }
                    Err(error) => events.push(MqttEvent::Error(error)),
                }
            }
        }
        (events, responses)
    }

    pub(super) fn receive_pubrel(
        &mut self,
        id: u16,
        stream: Option<u64>,
    ) -> (Vec<MqttEvent>, Vec<MqttPacket>) {
        if id == 0 {
            self.fail_connection(MqttClientError::InvalidPacketId { packet_id: id });
            return (self.take_events(), vec![]);
        }
        let mut reason = 0;
        match self.reliability.received.get_mut(&id) {
            Some((stage, channel))
                if matches!(stage, ReceiveStage::PubRec | ReceiveStage::PubRel)
                    && (*channel == stream || channel.is_none()) =>
            {
                *stage = ReceiveStage::PubRel;
                *channel = stream;
            }
            None => {
                reason = if self.mqtt_version() == 5 { 0x92 } else { 0 };
                self.reliability
                    .received
                    .insert(id, (ReceiveStage::PubRel, stream));
            }
            _ => {
                return (
                    vec![MqttEvent::Error(MqttClientError::InvalidPacketId {
                        packet_id: id,
                    })],
                    vec![],
                )
            }
        }
        let events = vec![MqttEvent::PubRelReceived {
            packet_id: id,
            stream,
        }];
        let responses = if self.options.auto_ack {
            let packet = self
                .manual_ack_packet(id, 7, reason, vec![], stream)
                .expect("validated PUBREL state");
            self.commit_receive_ack(id, 7, reason);
            vec![packet]
        } else {
            vec![]
        };
        (events, responses)
    }

    pub(super) fn validate_ack(
        &self,
        packet: &MqttPacket,
        stream: Option<u64>,
    ) -> Result<(), MqttClientError> {
        let (id, kind) = match packet {
            MqttPacket::PubAck5(p) => (p.packet_id, 4),
            MqttPacket::PubAck3(p) => (p.message_id, 4),
            MqttPacket::PubRec5(p) => (p.packet_id, 5),
            MqttPacket::PubRec3(p) => (p.message_id, 5),
            MqttPacket::PubComp5(p) => (p.packet_id, 7),
            MqttPacket::PubComp3(p) => (p.message_id, 7),
            MqttPacket::SubAck5(p) => (p.packet_id, 9),
            MqttPacket::SubAck3(p) => (p.message_id, 9),
            MqttPacket::UnsubAck5(p) => (p.packet_id, 11),
            MqttPacket::UnsubAck3(p) => (p.message_id, 11),
            _ => return Ok(()),
        };
        let Some(entry) = self.inflight_queue.get(id) else {
            return Err(MqttClientError::InvalidPacketId { packet_id: id });
        };
        let valid = match (&entry.packet, kind) {
            (MqttPacket::Publish5(p), 4 | 5) => p.qos == if kind == 4 { 1 } else { 2 },
            (MqttPacket::Publish3(p), 4 | 5) => p.qos == if kind == 4 { 1 } else { 2 },
            (MqttPacket::PubRel5(_) | MqttPacket::PubRel3(_), 5 | 7) => true,
            (MqttPacket::Subscribe5(_) | MqttPacket::Subscribe3(_), 9) => true,
            (MqttPacket::Unsubscribe5(_) | MqttPacket::Unsubscribe3(_), 11) => true,
            _ => false,
        };
        if !valid || entry.stream != stream {
            return Err(MqttClientError::InvalidPacketId { packet_id: id });
        }
        let counts = match (&entry.packet, packet) {
            (MqttPacket::Subscribe3(sent), MqttPacket::SubAck3(ack)) => {
                Some((sent.subscriptions.len(), ack.return_codes.len()))
            }
            (MqttPacket::Subscribe5(sent), MqttPacket::SubAck5(ack)) => {
                Some((sent.subscriptions.len(), ack.reason_codes.len()))
            }
            _ => None,
        };
        if let Some((expected, actual)) = counts {
            if expected != actual {
                return Err(MqttClientError::ProtocolViolation {
                    message: format!(
                        "SUBACK has {actual} results for {expected} subscription filters"
                    ),
                });
            }
        }
        Ok(())
    }

    pub(super) fn fail_operation(&mut self, id: u16, packet: &MqttPacket, error: MqttClientError) {
        let operation = match packet {
            MqttPacket::Subscribe3(_) | MqttPacket::Subscribe5(_) => OperationKind::Subscribe,
            MqttPacket::Unsubscribe3(_) | MqttPacket::Unsubscribe5(_) => OperationKind::Unsubscribe,
            _ => OperationKind::Publish,
        };
        self.inflight_queue.acknowledge(id);
        self.finish_operation(id);
        self.events.push(MqttEvent::OperationFailed {
            operation,
            packet_id: Some(id),
            error,
        });
    }

    pub(super) fn process_replay(&mut self) {
        if self.transport_manages_session {
            return;
        }
        while let Some((id, packet)) = self.session_replay.front().cloned() {
            let qos = match &packet {
                MqttPacket::Publish5(p) => p.qos,
                MqttPacket::Publish3(p) => p.qos,
                _ => 2,
            };
            if matches!(packet, MqttPacket::Publish5(_) | MqttPacket::Publish3(_))
                && !self.inflight_queue.can_push_publish()
            {
                break;
            }
            match self.enqueue_packet(packet.clone()) {
                Ok(()) => {
                    self.session_replay.pop_front();
                    if let Err(error) = self.inflight_queue.push(id, packet.clone(), qos) {
                        self.fail_operation(id, &packet, error);
                    } else {
                        self.start_deadline(OperationKind::Publish, Some(id));
                    }
                }
                Err(MqttClientError::BufferFull { .. }) => break,
                Err(error) => {
                    self.session_replay.pop_front();
                    self.fail_operation(id, &packet, error);
                }
            }
        }
    }
}

#[cfg(feature = "quic-proto")]
impl QuicMqttEngine {
    pub(super) fn check_stream_bytes(&self, additional: usize) -> Result<(), MqttClientError> {
        if let Some(capacity) = self.mqtt_engine.options.max_outgoing_buffer_bytes {
            let used = self.mqtt_engine.outgoing_bytes()
                + self.control_outgoing.len()
                + self
                    .data_streams
                    .values()
                    .map(|s| s.outgoing.len())
                    .sum::<usize>();
            if used.saturating_add(additional) > capacity {
                return Err(MqttClientError::BufferFull {
                    buffer_type: "QUIC outgoing bytes".into(),
                    capacity,
                });
            }
        }
        Ok(())
    }

    pub(super) fn preflight_publish(
        &self,
        command: &PublishCommand,
    ) -> Result<(), MqttClientError> {
        let mut command = command.clone();
        self.mqtt_engine.validate_publish(&mut command)?;
        if command.qos > 0 {
            command.packet_id = Some(command.packet_id.unwrap_or(1));
        }
        let packet = if self.mqtt_engine.mqtt_version() == 5 {
            MqttPacket::Publish5(command.to_mqtt_publish())
        } else {
            MqttPacket::Publish3(command.to_mqttv3_publish())
        };
        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.mqtt_engine.check_packet_size(&bytes)?;
        self.check_stream_bytes(bytes.len())
    }

    pub(super) fn preflight_subscribe(
        &self,
        command: &SubscribeCommand,
    ) -> Result<(), MqttClientError> {
        self.mqtt_engine.validate_subscribe(command)?;
        let id = command.packet_id.unwrap_or(1);
        let packet = if self.mqtt_engine.mqtt_version() == 5 {
            MqttPacket::Subscribe5(subscribev5::MqttSubscribe::new(
                id,
                command.subscriptions.clone(),
                command.properties.clone(),
            ))
        } else {
            MqttPacket::Subscribe3(subscribev3::MqttSubscribe::new(
                id,
                command
                    .subscriptions
                    .iter()
                    .map(|s| subscribev3::SubscriptionTopic {
                        topic_filter: s.topic_filter.clone(),
                        qos: s.qos,
                    })
                    .collect(),
            ))
        };
        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.mqtt_engine.check_packet_size(&bytes)?;
        self.check_stream_bytes(bytes.len())
    }

    pub(super) fn preflight_unsubscribe(
        &self,
        command: &UnsubscribeCommand,
    ) -> Result<(), MqttClientError> {
        let id = command.packet_id.unwrap_or(1);
        let packet = if self.mqtt_engine.mqtt_version() == 5 {
            MqttPacket::Unsubscribe5(unsubscribev5::MqttUnsubscribe::new(
                id,
                command.topics.clone(),
                command.properties.clone(),
            ))
        } else {
            MqttPacket::Unsubscribe3(unsubscribev3::MqttUnsubscribe::new(
                id,
                command.topics.clone(),
            ))
        };
        let bytes = packet.to_bytes().map_err(MqttClientError::from)?;
        self.mqtt_engine.check_packet_size(&bytes)?;
        self.check_stream_bytes(bytes.len())
    }
}
