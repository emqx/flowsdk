use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::puback::MqttPubAck;
use crate::mqtt_serde::mqttv5::pubcomp::MqttPubComp;
use crate::mqtt_serde::mqttv5::publish::MqttPublish;
use crate::mqtt_serde::mqttv5::pubrec::MqttPubRec;
use crate::mqtt_serde::mqttv5::pubrel::MqttPubRel;
use std::collections::HashMap;

pub struct ClientSession {
    // QoS 1 and QoS 2 messages that have been sent but not acknowledged.
    // The key is the packet identifier.
    unacknowledged_publishes: HashMap<u16, MqttPublish>,

    // QoS 2 PUBREL messages that have been sent but not yet acknowledged with PUBCOMP.
    // The key is the packet identifier.
    unacknowledged_pubrels: HashMap<u16, MqttPubRel>,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientSession {
    pub fn new() -> Self {
        ClientSession {
            unacknowledged_publishes: HashMap::new(),
            unacknowledged_pubrels: HashMap::new(),
        }
    }

    pub fn handle_outgoing_publish(&mut self, publish: MqttPublish) {
        let packet_id = publish.packet_id.unwrap();
        // Store the publish message in unacknowledged_publishes if QoS > 0
        // This allows us to track messages that need acknowledgment.
        // QoS 0 messages do not require acknowledgment and are not stored.
        // QoS 1 messages will be acknowledged with a PUBACK.
        // QoS 2 messages will go through PUBREC, PUBREL, and PUBCOMP.
        if publish.qos > 0 && !self.unacknowledged_publishes.contains_key(&packet_id) {
            self.unacknowledged_publishes.insert(packet_id, publish);
        }
    }

    pub fn handle_incoming_publish(&mut self, publish: MqttPublish) {
        if publish.qos == 2 {
            // The client should respond with a PUBREC packet.
            // This logic will be handled by the caller.
        }
    }

    pub fn handle_incoming_puback(&mut self, puback: MqttPubAck) {
        self.unacknowledged_publishes.remove(&puback.packet_id);
    }

    pub fn handle_incoming_pubrec(&mut self, pubrec: MqttPubRec) -> Option<MqttPubRel> {
        if pubrec.reason_code < 0x80 {
            let pubrel = MqttPubRel {
                packet_id: pubrec.packet_id,
                reason_code: 0,
                properties: Vec::new(),
            };
            self.unacknowledged_pubrels.insert(pubrec.packet_id, pubrel.clone());
            self.unacknowledged_publishes.remove(&pubrec.packet_id);
            Some(pubrel)
        } else {
            self.unacknowledged_publishes.remove(&pubrec.packet_id);
            None
        }
    }

    pub fn handle_incoming_pubrel(&mut self, _pubrel: MqttPubRel) {
        // This is not expected on the client side in a simple implementation
    }

    pub fn handle_incoming_pubcomp(&mut self, pubcomp: MqttPubComp) {
        self.unacknowledged_pubrels.remove(&pubcomp.packet_id);
    }

    pub fn resend_pending_messages(&self) -> Vec<MqttPacket> {
        let mut packets_to_resend = Vec::new();

        for (packet_id, publish) in &self.unacknowledged_publishes {
            if !self.unacknowledged_pubrels.contains_key(packet_id) {
                packets_to_resend.push(MqttPacket::Publish(publish.clone()));
            }
        }

        for pubrel in self.unacknowledged_pubrels.values() {
            packets_to_resend.push(MqttPacket::PubRel(pubrel.clone()));
        }

        packets_to_resend
    }
}