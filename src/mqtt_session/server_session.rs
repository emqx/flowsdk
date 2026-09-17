// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::puback::MqttPubAck;
use crate::mqtt_serde::mqttv5::pubcomp::MqttPubComp;
use crate::mqtt_serde::mqttv5::publish::MqttPublish;
use crate::mqtt_serde::mqttv5::pubrec::MqttPubRec;
use crate::mqtt_serde::mqttv5::pubrel::MqttPubRel;
use crate::mqtt_serde::mqttv5::subscribe::{MqttSubscribe, TopicSubscription};
use crate::mqtt_serde::mqttv5::unsubscribe::MqttUnsubscribe;
use std::collections::{HashMap, HashSet, VecDeque};

/// In-memory MQTT 5 session with non-shared subscriptions and a local retained store.
/// The caller supplies validated packets and drives acknowledgments/transmission.
/// Broker-wide routing, shared subscriptions and persistent storage are external.
pub struct ServerSession {
    // The client's subscriptions. The key is the topic filter.
    subscriptions: HashMap<String, TopicSubscription>,

    // QoS 1 and QoS 2 messages that have been sent to the client but not acknowledged.
    // The key is the packet identifier.
    unacknowledged_publishes: HashMap<u16, MqttPublish>,

    // Application messages awaiting their first transmission to the client.
    pending_publishes: VecDeque<MqttPublish>,

    // Incoming QoS 2 identifiers remain owned until PUBREL, independently of
    // outgoing identifiers used when forwarding the application message.
    received_qos2: HashSet<u16>,

    retained_publishes: HashMap<String, MqttPublish>,
    packet_id_counter: u16,

    // QoS 2 PUBREL messages that have been sent but not yet acknowledged with PUBCOMP.
    // The key is the packet identifier.
    unacknowledged_pubrels: HashMap<u16, MqttPubRel>,

    // The Will Message.
    #[allow(dead_code)]
    will: Option<MqttPublish>,

    // The session expiry interval.
    #[allow(dead_code)]
    session_expiry_interval: u32,

    // The client's receive maximum value.
    receive_maximum: u16,
}

impl ServerSession {
    pub fn new(receive_maximum: u16) -> Self {
        ServerSession {
            subscriptions: HashMap::new(),
            unacknowledged_publishes: HashMap::new(),
            pending_publishes: VecDeque::new(),
            received_qos2: HashSet::new(),
            retained_publishes: HashMap::new(),
            packet_id_counter: 0,
            unacknowledged_pubrels: HashMap::new(),
            will: None,
            session_expiry_interval: 0,
            receive_maximum,
        }
    }

    pub fn handle_incoming_subscribe(&mut self, subscribe: MqttSubscribe) {
        for subscription in subscribe.subscriptions {
            let existed = self.subscriptions.contains_key(&subscription.topic_filter);
            if !subscription.topic_filter.starts_with("$share/")
                && (subscription.retain_handling == 0
                    || (subscription.retain_handling == 1 && !existed))
            {
                for publish in self.retained_publishes.values() {
                    if topic_matches(&subscription.topic_filter, &publish.topic_name) {
                        self.pending_publishes.push_back(forwarded_publish(
                            publish,
                            &subscription,
                            true,
                        ));
                    }
                }
            }
            self.subscriptions
                .insert(subscription.topic_filter.clone(), subscription);
        }
    }

    pub fn handle_incoming_unsubscribe(&mut self, unsubscribe: MqttUnsubscribe) {
        for topic_filter in unsubscribe.topic_filters {
            self.subscriptions.remove(&topic_filter);
        }
    }

    pub fn handle_incoming_publish(&mut self, publish: MqttPublish) -> Option<MqttPacket> {
        if publish.qos == 2 && !self.received_qos2.insert(publish.packet_id.unwrap()) {
            return Some(MqttPacket::PubRec5(MqttPubRec::new_success(
                publish.packet_id.unwrap(),
            )));
        }
        if publish.retain {
            if publish.payload.is_empty() {
                self.retained_publishes.remove(&publish.topic_name);
            } else {
                let mut retained = publish.clone();
                retained.dup = false;
                retained.packet_id = None;
                self.retained_publishes
                    .insert(publish.topic_name.clone(), retained);
            }
        }
        for subscription in self.subscriptions.values() {
            if topic_matches(&subscription.topic_filter, &publish.topic_name) {
                self.pending_publishes
                    .push_back(forwarded_publish(&publish, subscription, false));
            }
        }

        match publish.qos {
            1 => Some(MqttPacket::PubAck5(
                crate::mqtt_serde::mqttv5::puback::MqttPubAck {
                    packet_id: publish.packet_id.unwrap(),
                    reason_code: 0,
                    properties: Vec::new(),
                },
            )),
            2 => {
                let pubrec = MqttPubRec {
                    packet_id: publish.packet_id.unwrap(),
                    reason_code: 0,
                    properties: Vec::new(),
                };
                Some(MqttPacket::PubRec5(pubrec))
            }
            _ => None,
        }
    }

    pub fn handle_incoming_puback(&mut self, puback: MqttPubAck) {
        if self
            .unacknowledged_publishes
            .get(&puback.packet_id)
            .is_some_and(|publish| publish.qos == 1)
        {
            self.unacknowledged_publishes.remove(&puback.packet_id);
        }
    }

    pub fn handle_incoming_pubrec(&mut self, pubrec: MqttPubRec) -> Option<MqttPubRel> {
        if let Some(pubrel) = self.unacknowledged_pubrels.get(&pubrec.packet_id) {
            return (pubrec.reason_code < 0x80).then(|| pubrel.clone());
        }
        if self
            .unacknowledged_publishes
            .get(&pubrec.packet_id)
            .is_none_or(|publish| publish.qos != 2)
        {
            return None;
        }
        self.unacknowledged_publishes.remove(&pubrec.packet_id);
        if pubrec.reason_code < 0x80 {
            let pubrel = MqttPubRel {
                packet_id: pubrec.packet_id,
                reason_code: 0,
                properties: Vec::new(),
            };
            self.unacknowledged_pubrels
                .insert(pubrec.packet_id, pubrel.clone());
            Some(pubrel)
        } else {
            None
        }
    }

    pub fn handle_incoming_pubrel(&mut self, pubrel: MqttPubRel) -> MqttPubComp {
        let known = self.received_qos2.remove(&pubrel.packet_id);
        MqttPubComp {
            packet_id: pubrel.packet_id,
            reason_code: if known { 0 } else { 0x92 },
            properties: Vec::new(),
        }
    }

    pub fn handle_incoming_pubcomp(&mut self, pubcomp: MqttPubComp) {
        self.unacknowledged_pubrels.remove(&pubcomp.packet_id);
    }

    /// Explicitly resend outstanding exchanges and transmit queued publications.
    /// MQTT 5 callers should request retransmission only when resuming a session.
    /// Existing exchanges keep their quota reservation; PUBREL needs no new slot.
    pub fn resend_pending_messages(&mut self) -> Vec<MqttPacket> {
        let mut packets_to_resend = Vec::new();
        for publish in self.unacknowledged_publishes.values() {
            let mut publish = publish.clone();
            publish.dup = true;
            packets_to_resend.push(MqttPacket::Publish5(publish));
        }

        // Resend unacknowledged pubrels
        for pubrel in self.unacknowledged_pubrels.values() {
            packets_to_resend.push(MqttPacket::PubRel5(pubrel.clone()));
        }
        packets_to_resend.extend(self.take_pending_messages());
        packets_to_resend
    }

    /// Drain new publications allowed by the send quota without retransmitting.
    /// Call after receiving application messages, subscriptions or acknowledgments.
    pub fn take_pending_messages(&mut self) -> Vec<MqttPacket> {
        let mut packets = Vec::new();
        let inflight = self.unacknowledged_publishes.len() + self.unacknowledged_pubrels.len();
        let mut available_slots = (self.receive_maximum as usize).saturating_sub(inflight);

        // Send pending publishes
        for _ in 0..self.pending_publishes.len() {
            let mut publish = self.pending_publishes.pop_front().unwrap();
            if publish.qos > 0 {
                if available_slots == 0 {
                    self.pending_publishes.push_back(publish);
                    continue;
                }
                let Some(id) = self.next_packet_id() else {
                    self.pending_publishes.push_front(publish);
                    break;
                };
                publish.packet_id = Some(id);
                self.unacknowledged_publishes.insert(id, publish.clone());
                available_slots -= 1;
            }
            packets.push(MqttPacket::Publish5(publish));
        }
        packets
    }

    fn next_packet_id(&mut self) -> Option<u16> {
        for _ in 0..u16::MAX {
            self.packet_id_counter = self.packet_id_counter.checked_add(1).unwrap_or(1);
            if !self
                .unacknowledged_publishes
                .contains_key(&self.packet_id_counter)
                && !self
                    .unacknowledged_pubrels
                    .contains_key(&self.packet_id_counter)
            {
                return Some(self.packet_id_counter);
            }
        }
        None
    }
}

fn forwarded_publish(
    publish: &MqttPublish,
    subscription: &TopicSubscription,
    retained_replay: bool,
) -> MqttPublish {
    let mut forwarded = publish.clone();
    forwarded.qos = publish.qos.min(subscription.qos);
    forwarded.dup = false;
    forwarded.retain = retained_replay || (publish.retain && subscription.retain_as_published);
    // The subscriber's packet identifier is assigned when its send quota permits.
    forwarded.packet_id = None;
    forwarded
}

fn topic_matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && filter.starts_with(['+', '#']) {
        return false;
    }
    let mut filters = filter.split('/');
    let mut topics = topic.split('/');
    loop {
        match filters.next() {
            Some("#") => return filters.next().is_none(),
            Some("+") => {
                if topics.next().is_none() {
                    return false;
                }
            }
            Some(level) if topics.next() != Some(level) => return false,
            Some(_) => {}
            None => return topics.next().is_none(),
        }
    }
}
