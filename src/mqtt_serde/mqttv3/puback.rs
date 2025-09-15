use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PUBACK packet in MQTT v3.1.1.
///
/// The PUBACK packet is the response to a PUBLISH packet with QoS level 1.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubAck {
    /// The Packet Identifier from the PUBLISH packet that is being acknowledged.
    pub message_id: u16,
}

impl MqttPubAck {
    /// Creates a new `MqttPubAck` packet.
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }
}

impl MqttControlPacket for MqttPubAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBACK as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(self.message_id.to_be_bytes().to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBACK has no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBACK as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "PUBACK packet has invalid fixed header flags".to_string(),
            ));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let total_len = 1 + vbi_len + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Remaining Length MUST be 2.
        if size != 2 {
            return Err(ParseError::ParseError(
                "PUBACK packet must have a remaining length of 2".to_string(),
            ));
        }

        let message_id = u16::from_be_bytes([buffer[1 + vbi_len], buffer[1 + vbi_len + 1]]);

        Ok(ParseOk::Packet(
            MqttPacket::PubAck3(MqttPubAck::new(message_id)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
