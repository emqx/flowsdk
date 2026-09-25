// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::mqtt_session::client_store::{ClientSessionState, ReceivedExchange, ReceivedStage};
use protocol_state::ReceiveStage;
use std::collections::HashSet;

fn invalid(reason: &str) -> MqttClientError {
    MqttClientError::InvalidConfiguration {
        field: "session state".into(),
        reason: reason.into(),
    }
}

fn canonical(mut packet: MqttPacket) -> MqttPacket {
    if let MqttPacket::Publish5(p) = &mut packet {
        p.properties
            .retain(|p| !matches!(p, Property::TopicAlias(_)));
    }
    packet
}

// Check these invariants even in builds without strict-protocol-compliance:
// deserialized snapshots must not inject arbitrary packets or reuse live IDs.
fn packet_id(
    packet: &MqttPacket,
    mqtt_version: u8,
    queued: bool,
) -> Result<Option<u16>, MqttClientError> {
    let publish = match packet {
        MqttPacket::Publish5(p) if mqtt_version == 5 => {
            if p.properties
                .iter()
                .any(|p| matches!(p, Property::TopicAlias(_)))
            {
                return Err(invalid("Saved publications must not contain topic aliases"));
            }
            Some((p.qos, p.packet_id, &p.topic_name, p.dup))
        }
        MqttPacket::Publish3(p) if mqtt_version != 5 => {
            Some((p.qos, p.message_id, &p.topic_name, p.dup))
        }
        _ => None,
    };
    if let Some((qos, id, topic, dup)) = publish {
        if qos > 2
            || topic.is_empty()
            || topic.contains(['#', '+', '\0'])
            || (qos == 0 && (id.is_some() || dup || !queued))
            || (qos > 0 && !matches!(id, Some(1..=u16::MAX)))
        {
            return Err(invalid("Invalid saved publication"));
        }
        return Ok(id);
    }
    if queued {
        return Err(invalid("Only publications may be queued"));
    }
    let id = match packet {
        MqttPacket::PubRel5(p) if mqtt_version == 5 => p.packet_id,
        MqttPacket::Subscribe5(p) if mqtt_version == 5 => p.packet_id,
        MqttPacket::Unsubscribe5(p) if mqtt_version == 5 => p.packet_id,
        MqttPacket::PubRel3(p) if mqtt_version != 5 => p.message_id,
        MqttPacket::Subscribe3(p) if mqtt_version != 5 => p.message_id,
        MqttPacket::Unsubscribe3(p) if mqtt_version != 5 => p.message_id,
        _ => return Err(invalid("Invalid outstanding packet type or MQTT version")),
    };
    if id == 0 {
        return Err(invalid("Saved packet identifiers must be nonzero"));
    }
    Ok(Some(id))
}

impl MqttEngine {
    /// Capture an owned, serializable checkpoint without changing the engine.
    ///
    /// Requires a nonempty client ID and session tracking. Includes accepted
    /// queued publications and outstanding QoS exchanges, including replay that
    /// is waiting for quota or CONNACK. Connection-specific state is excluded.
    ///
    /// The application must write this through its `ClientSessionStore`. For a
    /// planned restart, stop driving the engine while saving the final snapshot.
    /// For crash recovery, save before exposing each transition's events/output.
    /// `take_outgoing` also advances state: save after draining it and before
    /// sending its bytes. On storage failure, keep output/events private and
    /// retry or stop. Incoming application work must be durably accepted with
    /// its checkpoint; application events themselves are not stored here.
    pub fn snapshot_session(&self) -> Result<ClientSessionState, MqttClientError> {
        if self.options.sessionless || self.options.client_id.is_empty() {
            return Err(invalid(
                "Checkpointing requires session tracking and a client ID",
            ));
        }
        let mut outbound: Vec<_> = self
            .previous_inflight
            .iter()
            .filter(|(id, _)| self.state.reserved.contains(id))
            .cloned()
            .collect();
        let mut positions: HashMap<_, _> = outbound
            .iter()
            .enumerate()
            .map(|(index, (id, _))| (*id, index))
            .collect();
        for (id, packet) in self
            .inflight_queue
            .snapshot_for_reconnect()
            .into_iter()
            .chain(self.session_replay.iter().cloned())
        {
            if let Some(&index) = positions.get(&id) {
                outbound[index].1 = packet;
            } else {
                positions.insert(id, outbound.len());
                outbound.push((id, packet));
            }
        }
        let mut received: Vec<_> = self
            .state
            .received
            .iter()
            .filter_map(|(&(stream, packet_id), stage)| {
                let stage = match stage {
                    ReceiveStage::Publish(2) => ReceivedStage::Publish,
                    ReceiveStage::PubRec => ReceivedStage::PubRec,
                    ReceiveStage::PubRel => ReceivedStage::PubRel,
                    _ => return None, // Incoming QoS 1 is connection-scoped.
                };
                Some(ReceivedExchange {
                    stream,
                    packet_id,
                    stage,
                })
            })
            .collect();
        received.sort_by_key(|entry| (entry.stream, entry.packet_id));
        Ok(ClientSessionState {
            version: ClientSessionState::VERSION,
            peer: self.options.peer.clone(),
            client_id: self.options.client_id.clone(),
            mqtt_version: self.options.mqtt_version,
            has_session: self.state.has_session,
            session_expiry_interval: self.state.session_expiry,
            packet_id_counter: self
                .session
                .as_ref()
                .map_or(0, ClientSession::packet_id_counter),
            outbound: outbound
                .into_iter()
                .map(|(id, p)| (id, canonical(p)))
                .collect(),
            queued: self
                .priority_queue
                .iter()
                .map(|(&priority, p)| (priority, canonical(p.clone())))
                .collect(),
            received,
        })
    }

    /// Restore a checkpoint into a fresh engine, before CONNECT or commands.
    ///
    /// Requires the same peer, client ID and MQTT version, `clean_start(false)`
    /// and session tracking. Supply credentials and other connection options
    /// separately. A validation error leaves the engine unchanged. Reconnect
    /// starts new timers, negotiates new limits and waits for CONNACK to decide
    /// whether to replay or fail outstanding operations.
    pub fn restore_session_state(
        &mut self,
        saved: ClientSessionState,
    ) -> Result<(), MqttClientError> {
        if self.session.is_some()
            || self.is_connected
            || self.state.connecting
            || !self.state.reserved.is_empty()
            || !self.priority_queue.is_empty()
            || !self.outgoing_buffer.is_empty()
        {
            return Err(MqttClientError::InvalidState {
                expected: "a fresh engine before CONNECT or commands".into(),
                actual: "engine already in use".into(),
            });
        }
        if saved.version != ClientSessionState::VERSION {
            return Err(invalid("Unsupported session state version"));
        }
        if self.options.sessionless || self.options.clean_start {
            return Err(invalid(
                "Resumption requires session tracking and clean_start(false)",
            ));
        }
        if saved.client_id.is_empty()
            || saved.client_id != self.options.client_id
            || saved.peer != self.options.peer
            || saved.mqtt_version != self.options.mqtt_version
            || !matches!(saved.mqtt_version, 3..=5)
        {
            return Err(invalid(
                "Saved broker/client identity or MQTT version does not match",
            ));
        }
        if saved.queued.len() > self.options.max_outgoing_packet_count {
            return Err(invalid(
                "Saved publications exceed the configured queue capacity",
            ));
        }
        let mut reserved = HashSet::new();
        let mut replay_reserve = 0;
        for (id, packet) in &saved.outbound {
            if packet_id(packet, saved.mqtt_version, false)? != Some(*id) || !reserved.insert(*id) {
                return Err(invalid(
                    "Duplicate or inconsistent outstanding packet identifier",
                ));
            }
            replay_reserve = replay_reserve.max(packet.to_bytes()?.len());
        }
        let mut queue = PriorityQueue::new(self.options.max_outgoing_packet_count);
        let mut queued_bytes = 0usize;
        for (priority, packet) in saved.queued {
            let id = packet_id(&packet, saved.mqtt_version, true)?;
            if let Some(id) = id {
                if !reserved.insert(id) {
                    return Err(invalid("Duplicate queued packet identifier"));
                }
            }
            let size = packet.to_bytes()?.len();
            queued_bytes = queued_bytes.saturating_add(size);
            if id.is_some() {
                replay_reserve = replay_reserve.max(size);
            }
            queue.enqueue(priority, packet);
        }
        let control_reserve = self.connect_packet()?.to_bytes()?.len().max(6);
        self.check_output_capacity(
            queued_bytes.saturating_add(control_reserve.max(replay_reserve)),
        )?;
        let mut received = HashMap::new();
        for entry in saved.received {
            let stage = match entry.stage {
                ReceivedStage::Publish => ReceiveStage::Publish(2),
                ReceivedStage::PubRec => ReceiveStage::PubRec,
                ReceivedStage::PubRel => ReceiveStage::PubRel,
            };
            if !saved.has_session
                || entry.packet_id == 0
                || received
                    .insert((entry.stream, entry.packet_id), stage)
                    .is_some()
            {
                return Err(invalid("Invalid or duplicate incoming QoS 2 exchange"));
            }
        }

        // Commit only after the complete checkpoint has passed validation.
        let mut session = ClientSession::new();
        session.restore_packet_id_counter(saved.packet_id_counter);
        self.session = Some(session);
        self.previous_inflight = saved.outbound;
        self.priority_queue = queue;
        self.state.reserved = reserved;
        self.state.queued_bytes = queued_bytes;
        self.state.control_reserve = control_reserve;
        self.state.replay_reserve = replay_reserve;
        self.state.has_session = saved.has_session;
        self.state.session_expiry = saved.session_expiry_interval;
        self.state.received_unbound = received.keys().copied().collect();
        self.state.received = received;
        self.state.input_closed = true;
        Ok(())
    }
}
