use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PUBREL packet in MQTT v3.1.1.
///
/// The PUBREL packet is the response to a PUBREC packet. It is the third packet
/// of the QoS 2 protocol exchange.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubRel {
    /// The Packet Identifier from the PUBREC packet that is being acknowledged.
    pub message_id: u16,
}

impl MqttPubRel {
    /// Creates a new `MqttPubRel` packet.
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }
}

impl MqttControlPacket for MqttPubRel {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBREL as u8
    }

    fn flags(&self) -> u8 {
        // For PUBREL, bits 3,2,1,0 MUST be 0,0,1,0
        0x02
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(self.message_id.to_be_bytes().to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBREL has no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBREL as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // For PUBREL, bits 3,2,1,0 of fixed header MUST be 0,0,1,0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x02 {
            return Err(ParseError::ParseError(
                "PUBREL packet has invalid fixed header flags".to_string(),
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
                "PUBREL packet must have a remaining length of 2".to_string(),
            ));
        }

        let message_id = u16::from_be_bytes([buffer[1 + vbi_len], buffer[1 + vbi_len + 1]]);

        Ok(ParseOk::Packet(
            MqttPacket::PubRel3(MqttPubRel::new(message_id)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
