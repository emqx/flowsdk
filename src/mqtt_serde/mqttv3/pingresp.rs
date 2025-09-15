use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PINGRESP packet in MQTT v3.1.1.
///
/// The PINGRESP packet is sent by the Server to the Client in response to a PINGREQ packet.
/// It indicates that the Server is alive.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPingResp;

impl MqttPingResp {
    /// Creates a new `MqttPingResp` packet.
    pub fn new() -> Self {
        Self
    }
}

impl MqttControlPacket for MqttPingResp {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PINGRESP as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        // PINGRESP has no variable header.
        Ok(Vec::new())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PINGRESP has no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PINGRESP as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "PINGRESP packet has invalid fixed header flags".to_string(),
            ));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let total_len = 1 + vbi_len + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Remaining Length MUST be 0.
        if size != 0 {
            return Err(ParseError::ParseError(
                "PINGRESP packet must have a remaining length of 0".to_string(),
            ));
        }

        Ok(ParseOk::Packet(
            MqttPacket::PingResp3(MqttPingResp::new()),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pingresp_new() {
        let pingresp = MqttPingResp::new();
        assert_eq!(
            pingresp.control_packet_type(),
            ControlPacketType::PINGRESP as u8
        );
    }

    #[test]
    fn test_pingresp_serialization() {
        let pingresp = MqttPingResp::new();
        let bytes = pingresp.to_bytes().unwrap();
        assert_eq!(bytes, vec![0xD0, 0x00]);
    }

    #[test]
    fn test_pingresp_deserialization() {
        let bytes = vec![0xD0, 0x00];
        match MqttPingResp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingResp3(pingresp), consumed) => {
                assert_eq!(consumed, 2);
                assert_eq!(pingresp, MqttPingResp::new());
            }
            _ => panic!("Expected PINGRESP packet"),
        }
    }

    #[test]
    fn test_pingresp_roundtrip() {
        let original = MqttPingResp::new();
        let bytes = original.to_bytes().unwrap();
        match MqttPingResp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingResp3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PINGRESP packet"),
        }
    }

    #[test]
    fn test_pingresp_invalid_flags() {
        let bytes = vec![0xD1, 0x00]; // PINGRESP with invalid flags
        match MqttPingResp::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_pingresp_non_zero_remaining_length() {
        let bytes = vec![0xD0, 0x01, 0x00]; // PINGRESP with Remaining Length > 0
        match MqttPingResp::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 0") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_pingresp_wrong_packet_type() {
        let bytes = vec![0xC0, 0x00]; // Wrong packet type (PINGREQ)
        match MqttPingResp::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {
                // Expected error
            }
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_pingresp_incomplete_packet() {
        let bytes = vec![0xD0]; // Incomplete PINGRESP
        match MqttPingResp::from_bytes(&bytes) {
            Ok(ParseOk::Continue(needed, _)) => {
                assert_eq!(needed, 1);
            }
            other => panic!("Expected Continue, got {:?}", other),
        }
    }
}
