// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv3::packet_id_ack::v3_packet_id_ack;

v3_packet_id_ack!(MqttPubComp, PUBCOMP, 0x00, "PUBCOMP", PubComp3);

#[cfg(test)]
mod tests {
    use super::MqttPubComp;
    use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::parser::{ParseError, ParseOk};

    #[test]
    fn test_pubcomp_new() {
        let pubcomp = MqttPubComp::new(42);
        assert_eq!(pubcomp.message_id, 42);
        assert_eq!(
            pubcomp.control_packet_type(),
            ControlPacketType::PUBCOMP as u8
        );
    }

    #[test]
    fn test_pubcomp_serialization() {
        let pubcomp = MqttPubComp::new(1000);
        let bytes = pubcomp.to_bytes().unwrap();
        // packet type + remaining length + message_id (0x03E8)
        assert_eq!(bytes, vec![0x70, 0x02, 0x03, 0xE8]);
    }

    #[test]
    fn test_pubcomp_deserialization() {
        let bytes = vec![0x70, 0x02, 0x03, 0xE8];
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp3(pubcomp), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubcomp.message_id, 1000);
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_roundtrip() {
        let original = MqttPubComp::new(12345);
        let bytes = original.to_bytes().unwrap();
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_invalid_flags() {
        let bytes = vec![0x71, 0x02, 0x00, 0x01];
        match MqttPubComp::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_pubcomp_invalid_remaining_length() {
        let bytes = vec![0x70, 0x01, 0x01]; // Length 1, should be 2
        match MqttPubComp::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 2") => {}
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_pubcomp_wrong_packet_type() {
        let bytes = vec![0x60, 0x02, 0x00, 0x01]; // Wrong packet type (PUBREL)
        match MqttPubComp::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {}
            _ => panic!("Expected InvalidPacketType error"),
        }
    }
}
