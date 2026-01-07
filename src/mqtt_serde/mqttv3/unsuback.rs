// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the UNSUBACK packet in MQTT v3.1.1.
///
/// The UNSUBACK packet is sent by the Server to the Client to confirm receipt and processing
/// of an UNSUBSCRIBE packet.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttUnsubAck {
    /// The Packet Identifier from the UNSUBSCRIBE packet that is being acknowledged.
    pub message_id: u16,
}

impl MqttUnsubAck {
    /// Creates a new `MqttUnsubAck` packet.
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }
}

impl MqttControlPacket for MqttUnsubAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::UNSUBACK as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(self.message_id.to_be_bytes().to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT v3.1.1 UNSUBACK has no payload
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::UNSUBACK as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "UNSUBACK packet has invalid fixed header flags".to_string(),
            ));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let offset = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Variable Header: Packet Identifier (2 bytes)
        if size != 2 {
            return Err(ParseError::ParseError(
                "MQTT v3.1.1 UNSUBACK packet must have remaining length of 2".to_string(),
            ));
        }

        let message_id = u16::from_be_bytes([buffer[offset], buffer[offset + 1]]);

        Ok(ParseOk::Packet(
            MqttPacket::UnsubAck3(MqttUnsubAck::new(message_id)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
