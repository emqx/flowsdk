// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PINGREQ packet in MQTT v3.1.1.
///
/// The PINGREQ packet is sent from a Client to the Server. It is used to:
/// 1. Indicate to the Server that the Client is alive in the absence of any other Control Packets being sent from the Client.
/// 2. Request that the Server responds with a PINGRESP packet, which indicates that it is also alive.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPingReq;

impl MqttPingReq {
    /// Creates a new `MqttPingReq` packet.
    pub fn new() -> Self {
        Self
    }
}

impl MqttControlPacket for MqttPingReq {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PINGREQ as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        // PINGREQ has no variable header.
        Ok(Vec::new())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PINGREQ has no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PINGREQ as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "PINGREQ packet has invalid fixed header flags".to_string(),
            ));
        }

        if buffer[1..].is_empty() {
            return Ok(ParseOk::Continue(1, 0));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let total_len = 1 + vbi_len + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Remaining Length MUST be 0.
        if size != 0 {
            return Err(ParseError::ParseError(
                "PINGREQ packet must have a remaining length of 0".to_string(),
            ));
        }

        Ok(ParseOk::Packet(
            MqttPacket::PingReq3(MqttPingReq::new()),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pingreq_new() {
        let pingreq = MqttPingReq::new();
        assert_eq!(
            pingreq.control_packet_type(),
            ControlPacketType::PINGREQ as u8
        );
    }

    #[test]
    fn test_pingreq_serialization() {
        let pingreq = MqttPingReq::new();
        let bytes = pingreq.to_bytes().unwrap();
        assert_eq!(bytes, vec![0xC0, 0x00]);
    }

    #[test]
    fn test_pingreq_deserialization() {
        let bytes = vec![0xC0, 0x00];
        match MqttPingReq::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingReq3(pingreq), consumed) => {
                assert_eq!(consumed, 2);
                assert_eq!(pingreq, MqttPingReq::new());
            }
            _ => panic!("Expected PINGREQ packet"),
        }
    }

    #[test]
    fn test_pingreq_roundtrip() {
        let original = MqttPingReq::new();
        let bytes = original.to_bytes().unwrap();
        match MqttPingReq::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingReq3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PINGREQ packet"),
        }
    }

    #[test]
    fn test_pingreq_invalid_flags() {
        let bytes = vec![0xC1, 0x00]; // PINGREQ with invalid flags
        match MqttPingReq::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_pingreq_non_zero_remaining_length() {
        let bytes = vec![0xC0, 0x01, 0x00]; // PINGREQ with Remaining Length > 0
        match MqttPingReq::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 0") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_pingreq_wrong_packet_type() {
        let bytes = vec![0xD0, 0x00]; // Wrong packet type (PINGRESP)
        match MqttPingReq::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {
                // Expected error
            }
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_pingreq_incomplete_packet() {
        let bytes = vec![0xC0]; // Incomplete PINGREQ
        match MqttPingReq::from_bytes(&bytes) {
            Ok(ParseOk::Continue(needed, _)) => {
                assert_eq!(needed, 1);
            }
            other => panic!("Expected Continue, got {:?}", other),
        }
    }
}
