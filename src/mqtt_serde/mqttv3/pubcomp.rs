use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PUBCOMP packet in MQTT v3.1.1.
///
/// The PUBCOMP packet is the response to a PUBREL packet. It is the fourth and
/// final packet of the QoS 2 protocol exchange.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubComp {
    /// The Packet Identifier from the PUBREL packet that is being acknowledged.
    pub message_id: u16,
}

impl MqttPubComp {
    /// Creates a new `MqttPubComp` packet.
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }
}

impl MqttControlPacket for MqttPubComp {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBCOMP as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(self.message_id.to_be_bytes().to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBCOMP has no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBCOMP as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "PUBCOMP packet has invalid fixed header flags".to_string(),
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
                "PUBCOMP packet must have a remaining length of 2".to_string(),
            ));
        }

        let message_id = u16::from_be_bytes([buffer[1 + vbi_len], buffer[1 + vbi_len + 1]]);

        Ok(ParseOk::Packet(
            MqttPacket::PubComp3(MqttPubComp::new(message_id)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
