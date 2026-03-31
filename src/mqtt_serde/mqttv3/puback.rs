// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv3::packet_id_ack::v3_packet_id_ack;

v3_packet_id_ack!(MqttPubAck, PUBACK, 0x00, "PUBACK", PubAck3);

#[cfg(test)]
mod tests {
    use super::MqttPubAck;
    use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::parser::{ParseError, ParseOk};

    #[test]
    fn test_puback_new() {
        let puback = MqttPubAck::new(42);
        assert_eq!(puback.message_id, 42);
        assert_eq!(
            puback.control_packet_type(),
            ControlPacketType::PUBACK as u8
        );
    }

    #[test]
    fn test_puback_serialization() {
        let puback = MqttPubAck::new(1000);
        let bytes = puback.to_bytes().unwrap();
        // packet type + remaining length + message_id (0x03E8)
        assert_eq!(bytes, vec![0x40, 0x02, 0x03, 0xE8]);
    }

    #[test]
    fn test_puback_deserialization() {
        let bytes = vec![0x40, 0x02, 0x03, 0xE8];
        match MqttPubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubAck3(puback), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(puback.message_id, 1000);
            }
            _ => panic!("Expected PUBACK packet"),
        }
    }

    #[test]
    fn test_puback_roundtrip() {
        let original = MqttPubAck::new(12345);
        let bytes = original.to_bytes().unwrap();
        match MqttPubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubAck3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PUBACK packet"),
        }
    }

    #[test]
    fn test_puback_invalid_flags() {
        let bytes = vec![0x41, 0x02, 0x00, 0x01];
        match MqttPubAck::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_puback_invalid_remaining_length() {
        let bytes = vec![0x40, 0x01, 0x01]; // Length 1, should be 2
        match MqttPubAck::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 2") => {}
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_puback_wrong_packet_type() {
        let bytes = vec![0x50, 0x02, 0x00, 0x01]; // Wrong packet type (PUBREC)
        match MqttPubAck::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {}
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_puback_incomplete_packet() {
        let bytes = vec![0x40, 0x02, 0x01]; // Incomplete PUBACK
        match MqttPubAck::from_bytes(&bytes) {
            Ok(ParseOk::Continue(needed, _)) => {
                assert_eq!(needed, 1);
            }
            other => panic!("Expected Continue, got {:?}", other),
        }
    }
}
