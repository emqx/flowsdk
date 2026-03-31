// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv3::packet_id_ack::v3_packet_id_ack;

v3_packet_id_ack!(MqttUnsubAck, UNSUBACK, 0x00, "UNSUBACK", UnsubAck3);

#[cfg(test)]
mod tests {
    use super::MqttUnsubAck;
    use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::parser::{ParseError, ParseOk};

    #[test]
    fn test_unsuback_new() {
        let unsuback = MqttUnsubAck::new(123);
        assert_eq!(unsuback.message_id, 123);
    }

    #[test]
    fn test_unsuback_serialization() {
        let unsuback = MqttUnsubAck::new(0x1234);
        let bytes = unsuback.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0xB0, // Packet type and flags (UNSUBACK with flags 0000)
                0x02, // Remaining length = 2
                0x12, 0x34, // Message ID
            ]
        );
    }

    #[test]
    fn test_unsuback_deserialization() {
        let bytes = vec![0xB0, 0x02, 0x00, 0x42];
        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck3(unsuback), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(unsuback.message_id, 0x42);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_roundtrip() {
        let original = MqttUnsubAck::new(0xABCD);
        let bytes = original.to_bytes().unwrap();
        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_invalid_flags() {
        let bytes = vec![0xB1, 0x02, 0x00, 0x01]; // Invalid flags (should be 0000)
        match MqttUnsubAck::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_unsuback_invalid_remaining_length_too_short() {
        let bytes = vec![0xB0, 0x01, 0x00]; // Remaining length should be 2
        match MqttUnsubAck::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("must have remaining length of 2") => {
            }
            other => panic!(
                "Expected ParseError with remaining length message, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_unsuback_invalid_remaining_length_too_long() {
        let bytes = vec![0xB0, 0x03, 0x00, 0x01, 0x00]; // Remaining length should be 2
        match MqttUnsubAck::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("must have remaining length of 2") => {
            }
            other => panic!(
                "Expected ParseError with remaining length message, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_unsuback_control_packet_type() {
        let unsuback = MqttUnsubAck::new(1);
        assert_eq!(
            unsuback.control_packet_type(),
            ControlPacketType::UNSUBACK as u8
        );
    }
}
