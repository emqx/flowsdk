// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv3::packet_id_ack::v3_packet_id_ack;

v3_packet_id_ack!(MqttPubRel, PUBREL, 0x02, "PUBREL", PubRel3);

#[cfg(test)]
mod tests {
    use super::MqttPubRel;
    use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::parser::{ParseError, ParseOk};

    #[test]
    fn test_pubrel_new() {
        let pubrel = MqttPubRel::new(42);
        assert_eq!(pubrel.message_id, 42);
        assert_eq!(
            pubrel.control_packet_type(),
            ControlPacketType::PUBREL as u8
        );
    }

    #[test]
    fn test_pubrel_serialization() {
        let pubrel = MqttPubRel::new(1000);
        let bytes = pubrel.to_bytes().unwrap();
        // packet type (0x6) + flags (0x2) + remaining length + message_id (0x03E8)
        assert_eq!(bytes, vec![0x62, 0x02, 0x03, 0xE8]);
    }

    #[test]
    fn test_pubrel_deserialization() {
        let bytes = vec![0x62, 0x02, 0x03, 0xE8];
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel3(pubrel), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubrel.message_id, 1000);
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_roundtrip() {
        let original = MqttPubRel::new(12345);
        let bytes = original.to_bytes().unwrap();
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_invalid_flags() {
        // Flags must be 0b0010
        let bytes = vec![0x60, 0x02, 0x00, 0x01]; // Invalid flags (0)
        match MqttPubRel::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message for flags=0"),
        }

        let bytes = vec![0x61, 0x02, 0x00, 0x01]; // Invalid flags (1)
        match MqttPubRel::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message for flags=1"),
        }

        let bytes = vec![0x63, 0x02, 0x00, 0x01]; // Invalid flags (3)
        match MqttPubRel::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message for flags=3"),
        }
    }

    #[test]
    fn test_pubrel_invalid_remaining_length() {
        let bytes = vec![0x62, 0x01, 0x01]; // Length 1, should be 2
        match MqttPubRel::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 2") => {}
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_pubrel_wrong_packet_type() {
        let bytes = vec![0x72, 0x02, 0x00, 0x01]; // Wrong packet type (PUBCOMP)
        match MqttPubRel::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {}
            _ => panic!("Expected InvalidPacketType error"),
        }
    }
}
