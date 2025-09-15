use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PUBREC packet in MQTT v3.1.1.
///
/// The PUBREC packet is the response to a PUBLISH packet with QoS 2. It is the
/// second packet of the QoS 2 protocol exchange.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubRec {
    /// The Packet Identifier from the PUBLISH packet that is being acknowledged.
    pub message_id: u16,
}

impl MqttPubRec {
    /// Creates a new `MqttPubRec` packet.
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }
}

impl MqttControlPacket for MqttPubRec {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBREC as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(self.message_id.to_be_bytes().to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBREC has no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBREC as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "PUBREC packet has invalid fixed header flags".to_string(),
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
                "PUBREC packet must have a remaining length of 2".to_string(),
            ));
        }

        let message_id = u16::from_be_bytes([buffer[1 + vbi_len], buffer[1 + vbi_len + 1]]);

        Ok(ParseOk::Packet(
            MqttPacket::PubRec3(MqttPubRec::new(message_id)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
