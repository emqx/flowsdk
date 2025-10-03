use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::puback::MqttPubAck;
use crate::mqtt_serde::mqttv5::pubcomp::MqttPubComp;
use crate::mqtt_serde::mqttv5::publish::MqttPublish;
use crate::mqtt_serde::mqttv5::pubrec::MqttPubRec;
use crate::mqtt_serde::mqttv5::pubrel::MqttPubRel;
use crate::mqtt_serde::mqttv5::subscribe::{MqttSubscribe, TopicSubscription};
use crate::mqtt_serde::mqttv5::unsubscribe::MqttUnsubscribe;
use std::collections::HashMap;

pub struct ServerSession {
    // The client's subscriptions. The key is the topic filter.
    subscriptions: HashMap<String, TopicSubscription>,

    // QoS 1 and QoS 2 messages that have been sent to the client but not acknowledged.
    // The key is the packet identifier.
    unacknowledged_publishes: HashMap<u16, MqttPublish>,

    // QoS 1 and QoS 2 messages pending transmission to the client.
    pending_publishes: Vec<MqttPublish>,

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

    // The number of messages sent in the current window.
    inflight_messages: u16,
}

impl ServerSession {
    pub fn new(receive_maximum: u16) -> Self {
        ServerSession {
            subscriptions: HashMap::new(),
            unacknowledged_publishes: HashMap::new(),
            pending_publishes: Vec::new(),
            unacknowledged_pubrels: HashMap::new(),
            will: None,
            session_expiry_interval: 0,
            receive_maximum,
            inflight_messages: 0,
        }
    }

    pub fn handle_incoming_subscribe(&mut self, subscribe: MqttSubscribe) {
        for subscription in subscribe.subscriptions {
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
        // TODO: Implement topic matching logic here.
        // For now, we will just queue the message for all subscribers.
        for _subscription in self.subscriptions.values() {
            self.pending_publishes.push(publish.clone());
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

    pub fn handle_incoming_puback(&mut self, _puback: MqttPubAck) {
        if self.inflight_messages > 0 {
            self.inflight_messages -= 1;
        }
        self.unacknowledged_publishes.remove(&_puback.packet_id);
    }

    pub fn handle_incoming_pubrec(&mut self, pubrec: MqttPubRec) -> Option<MqttPubRel> {
        if pubrec.reason_code < 0x80 {
            let pubrel = MqttPubRel {
                packet_id: pubrec.packet_id,
                reason_code: 0,
                properties: Vec::new(),
            };
            self.unacknowledged_pubrels
                .insert(pubrec.packet_id, pubrel.clone());
            self.unacknowledged_publishes.remove(&pubrec.packet_id);
            Some(pubrel)
        } else {
            self.unacknowledged_publishes.remove(&pubrec.packet_id);
            None
        }
    }

    pub fn handle_incoming_pubrel(&mut self, pubrel: MqttPubRel) -> MqttPubComp {
        MqttPubComp {
            packet_id: pubrel.packet_id,
            reason_code: 0,
            properties: Vec::new(),
        }
    }

    pub fn handle_incoming_pubcomp(&mut self, pubcomp: MqttPubComp) {
        self.unacknowledged_pubrels.remove(&pubcomp.packet_id);
        if self.inflight_messages > 0 {
            self.inflight_messages -= 1;
        }
    }

    pub fn resend_pending_messages(&mut self) -> Vec<MqttPacket> {
        let mut packets_to_resend = Vec::new();
        let mut available_slots = self.receive_maximum.saturating_sub(self.inflight_messages);

        // Resend unacknowledged publishes
        for (packet_id, publish) in &self.unacknowledged_publishes {
            if !self.unacknowledged_pubrels.contains_key(packet_id) {
                if available_slots == 0 {
                    break;
                }
                packets_to_resend.push(MqttPacket::Publish5(publish.clone()));
                available_slots -= 1;
            }
        }

        // Resend unacknowledged pubrels
        for pubrel in self.unacknowledged_pubrels.values() {
            if available_slots == 0 {
                break;
            }
            packets_to_resend.push(MqttPacket::PubRel5(pubrel.clone()));
            available_slots -= 1;
        }

        // Send pending publishes
        let mut i = 0;
        while i < self.pending_publishes.len() && available_slots > 0 {
            let publish = self.pending_publishes.remove(i);
            packets_to_resend.push(MqttPacket::Publish5(publish));
            available_slots -= 1;
            i += 1;
        }

        self.inflight_messages = self.receive_maximum - available_slots;

        packets_to_resend
    }
}
