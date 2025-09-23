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
        if let Some(packet_id) = publish.packet_id {
            if publish.qos > 0 && !self.unacknowledged_publishes.contains_key(&packet_id) {
                self.unacknowledged_publishes.insert(packet_id, publish);
            }
        }
    }

    pub fn handle_outgoing_pubrel(&mut self, pubrel: MqttPubRel) {
        self.unacknowledged_pubrels.insert(pubrel.packet_id, pubrel);
    }

    pub fn handle_incoming_publish(&mut self, publish: MqttPublish) -> Option<MqttPacket> {
        match publish.qos {
            1 => Some(MqttPacket::PubAck5(MqttPubAck {
                packet_id: publish.packet_id.unwrap(),
                reason_code: 0,
                properties: Vec::new(),
            })),
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
        self.unacknowledged_publishes.remove(&puback.packet_id);
    }

    pub fn handle_incoming_pubrec(&mut self, pubrec: MqttPubRec) -> Option<MqttPubRel> {
        if pubrec.reason_code < 0x80 {
            let pubrel = MqttPubRel {
                packet_id: pubrec.packet_id,
                reason_code: 0,
                properties: Vec::new(),
            };
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
    }

    pub fn resend_pending_messages(&self) -> Vec<MqttPacket> {
        let mut packets_to_resend = Vec::new();

        for (packet_id, publish) in &self.unacknowledged_publishes {
            if !self.unacknowledged_pubrels.contains_key(packet_id) {
                packets_to_resend.push(MqttPacket::Publish5(publish.clone()));
            }
        }

        for pubrel in self.unacknowledged_pubrels.values() {
            packets_to_resend.push(MqttPacket::PubRel5(pubrel.clone()));
        }

        packets_to_resend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_serde::mqttv5::publish::MqttPublish;

    fn create_qos1_publish(packet_id: u16) -> MqttPublish {
        MqttPublish {
            dup: false,
            qos: 1,
            retain: false,
            topic_name: "test/topic".to_string(),
            packet_id: Some(packet_id),
            payload: b"hello".to_vec(),
            properties: Vec::new(),
        }
    }

    fn create_qos2_publish(packet_id: u16) -> MqttPublish {
        MqttPublish {
            dup: false,
            qos: 2,
            retain: false,
            topic_name: "test/topic".to_string(),
            packet_id: Some(packet_id),
            payload: b"hello".to_vec(),
            properties: Vec::new(),
        }
    }

    fn create_qos0_publish() -> MqttPublish {
        MqttPublish {
            dup: false,
            qos: 0,
            retain: false,
            topic_name: "test/topic".to_string(),
            packet_id: None,
            payload: b"hello".to_vec(),
            properties: Vec::new(),
        }
    }

    #[test]
    fn test_handle_outgoing_publish_qos1() {
        let mut session = ClientSession::new();
        let publish = create_qos1_publish(1);
        session.handle_outgoing_publish(publish.clone());
        assert_eq!(session.unacknowledged_publishes.len(), 1);
        assert_eq!(session.unacknowledged_publishes.get(&1), Some(&publish));
    }

    #[test]
    fn test_handle_outgoing_publish_qos2() {
        let mut session = ClientSession::new();
        let publish = create_qos2_publish(1);
        session.handle_outgoing_publish(publish.clone());
        assert_eq!(session.unacknowledged_publishes.len(), 1);
        assert_eq!(session.unacknowledged_publishes.get(&1), Some(&publish));
    }

    #[test]
    fn test_handle_outgoing_publish_qos0() {
        let mut session = ClientSession::new();
        let publish = create_qos0_publish();
        session.handle_outgoing_publish(publish);
        assert!(session.unacknowledged_publishes.is_empty());
    }

    #[test]
    fn test_handle_incoming_puback() {
        let mut session = ClientSession::new();
        let publish = create_qos1_publish(1);
        session.handle_outgoing_publish(publish);

        let puback = MqttPubAck {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        };
        session.handle_incoming_puback(puback);
        assert!(session.unacknowledged_publishes.is_empty());
    }

    #[test]
    fn test_qos2_full_flow_successful() {
        let mut session = ClientSession::new();
        let publish = create_qos2_publish(1);

        // Client sends PUBLISH (QoS 2)
        session.handle_outgoing_publish(publish.clone());
        assert_eq!(session.unacknowledged_publishes.get(&1), Some(&publish));

        // Client receives PUBREC
        let pubrec = MqttPubRec {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        };
        let pubrel = session.handle_incoming_pubrec(pubrec).unwrap();
        assert!(session.unacknowledged_publishes.is_empty());

        // Client sends PUBREL
        session.handle_outgoing_pubrel(pubrel.clone());
        assert_eq!(session.unacknowledged_pubrels.get(&1), Some(&pubrel));

        // Client receives PUBCOMP
        let pubcomp = MqttPubComp {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        };
        session.handle_incoming_pubcomp(pubcomp);
        assert!(session.unacknowledged_pubrels.is_empty());
    }

    #[test]
    fn test_handle_incoming_pubrec_with_error() {
        let mut session = ClientSession::new();
        let publish = create_qos2_publish(1);
        session.handle_outgoing_publish(publish);

        let pubrec = MqttPubRec {
            packet_id: 1,
            reason_code: 0x80, // Error
            properties: Vec::new(),
        };
        let result = session.handle_incoming_pubrec(pubrec);
        assert!(result.is_none());
        assert!(session.unacknowledged_publishes.is_empty());
    }

    #[test]
    fn test_handle_incoming_publish() {
        let mut session = ClientSession::new();

        // QoS 1
        let qos1_publish = create_qos1_publish(1);
        let response = session.handle_incoming_publish(qos1_publish).unwrap();
        match response {
            MqttPacket::PubAck5(puback) => assert_eq!(puback.packet_id, 1),
            _ => panic!("Expected PubAck5"),
        }

        // QoS 2
        let qos2_publish = create_qos2_publish(2);
        let response = session.handle_incoming_publish(qos2_publish).unwrap();
        match response {
            MqttPacket::PubRec5(pubrec) => assert_eq!(pubrec.packet_id, 2),
            _ => panic!("Expected PubRec5"),
        }

        // QoS 0
        let qos0_publish = create_qos0_publish();
        let response = session.handle_incoming_publish(qos0_publish);
        assert!(response.is_none());
    }

    #[test]
    fn test_handle_incoming_pubrel() {
        let mut session = ClientSession::new();
        let pubrel = MqttPubRel {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        };
        let pubcomp = session.handle_incoming_pubrel(pubrel);
        assert_eq!(pubcomp.packet_id, 1);
    }

    #[test]
    fn test_resend_pending_messages() {
        let mut session = ClientSession::new();

        // Add QoS 1 publish
        let qos1_publish = create_qos1_publish(1);
        session.handle_outgoing_publish(qos1_publish.clone());

        // Add QoS 2 publish and process up to PUBREL
        let qos2_publish = create_qos2_publish(2);
        session.handle_outgoing_publish(qos2_publish);
        let pubrec = MqttPubRec {
            packet_id: 2,
            reason_code: 0,
            properties: Vec::new(),
        };
        let pubrel = session.handle_incoming_pubrec(pubrec).unwrap();
        session.handle_outgoing_pubrel(pubrel.clone());

        let packets_to_resend = session.resend_pending_messages();
        assert_eq!(packets_to_resend.len(), 2);

        let mut has_qos1_publish = false;
        let mut has_pubrel = false;
        for packet in packets_to_resend {
            match packet {
                MqttPacket::Publish5(p) if p.packet_id == Some(1) => has_qos1_publish = true,
                MqttPacket::PubRel5(p) if p.packet_id == 2 => has_pubrel = true,
                _ => {}
            }
        }
        assert!(has_qos1_publish);
        assert!(has_pubrel);
    }

    #[test]
    fn test_no_resend_for_qos0() {
        let mut session = ClientSession::new();
        let qos0_publish = create_qos0_publish();
        session.handle_outgoing_publish(qos0_publish);
        let packets_to_resend = session.resend_pending_messages();
        assert!(packets_to_resend.is_empty());
    }
}
