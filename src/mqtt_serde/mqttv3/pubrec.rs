// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv3::packet_id_ack::v3_packet_id_ack;

v3_packet_id_ack!(MqttPubRec, PUBREC, 0x00, "PUBREC", PubRec3);

#[cfg(test)]
mod tests {
    use super::MqttPubRec;
    use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::parser::{ParseError, ParseOk};

    #[test]
    fn test_pubrec_new() {
        let pubrec = MqttPubRec::new(42);
        assert_eq!(pubrec.message_id, 42);
        assert_eq!(
            pubrec.control_packet_type(),
            ControlPacketType::PUBREC as u8
        );
    }

    #[test]
    fn test_pubrec_serialization() {
        let pubrec = MqttPubRec::new(1000);
        let bytes = pubrec.to_bytes().unwrap();
        // packet type + remaining length + message_id (0x03E8)
        assert_eq!(bytes, vec![0x50, 0x02, 0x03, 0xE8]);
    }

    #[test]
    fn test_pubrec_deserialization() {
        let bytes = vec![0x50, 0x02, 0x03, 0xE8];
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec3(pubrec), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubrec.message_id, 1000);
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_roundtrip() {
        let original = MqttPubRec::new(12345);
        let bytes = original.to_bytes().unwrap();
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_invalid_flags() {
        let bytes = vec![0x51, 0x02, 0x00, 0x01];
        match MqttPubRec::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_pubrec_invalid_remaining_length() {
        let bytes = vec![0x50, 0x03, 0x01, 0x02, 0x03]; // Length 3, should be 2
        match MqttPubRec::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 2") => {}
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_pubrec_wrong_packet_type() {
        let bytes = vec![0x40, 0x02, 0x00, 0x01]; // Wrong packet type (PUBACK)
        match MqttPubRec::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {}
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_pubrec_incomplete_packet() {
        let bytes = vec![0x50, 0x02]; // Incomplete PUBREC
        match MqttPubRec::from_bytes(&bytes) {
            Ok(ParseOk::Continue(needed, _)) => {
                assert_eq!(needed, 2);
            }
            other => panic!("Expected Continue, got {:?}", other),
        }
    }
}
