use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PINGREQ packet in MQTT v5.0.
///
/// The PINGREQ packet is sent by the Client to the Server. It can be used to:
/// - Indicate to the Server that the Client is alive in the absence of any other MQTT Control Packets being sent from the Client to the Server.
/// - Request that the Server responds to confirm that it is alive.
/// - Exercise the network to indicate that the Network Connection is active.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPingReq;

impl MqttPingReq {
    /// Creates a new PINGREQ packet.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MqttPingReq {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttControlPacket for MqttPingReq {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PINGREQ as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT 5.0: 3.12.2 PINGREQ Variable Header
        // The PINGREQ packet has no Variable Header.
        Ok(Vec::new())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT 5.0: 3.12.3 PINGREQ Payload
        // The PINGREQ packet has no Payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PINGREQ as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let offset = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.12.1 PINGREQ Fixed Header
        // The Remaining Length field for the PINGREQ packet MUST be zero.
        #[cfg(feature = "strict-protocol-compliance")]
        if size != 0 {
            return Err(ParseError::ParseError(
                "PINGREQ packet must have remaining length of 0".to_string(),
            ));
        }

        // MQTT 5.0: 3.12.1 Reserved bits in the Fixed Header
        // Bits 3,2,1 and 0 of the Fixed Header in the PINGREQ packet are reserved and MUST be set to 0,0,0,0.
        #[cfg(feature = "strict-protocol-compliance")]
        {
            let flags = buffer[0] & 0x0F;
            if flags != 0x00 {
                return Err(ParseError::ParseError(
                    "PINGREQ packet has invalid fixed header flags".to_string(),
                ));
            }
        }

        let ping_req = MqttPingReq::new();
        Ok(ParseOk::Packet(MqttPacket::PingReq(ping_req), total_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pingreq_creation() {
        let pingreq = MqttPingReq::new();
        assert_eq!(
            pingreq.control_packet_type(),
            ControlPacketType::PINGREQ as u8
        );
    }

    #[test]
    fn test_pingreq_default() {
        let pingreq = MqttPingReq;
        assert_eq!(
            pingreq.control_packet_type(),
            ControlPacketType::PINGREQ as u8
        );
    }

    #[test]
    fn test_pingreq_serialization() {
        let pingreq = MqttPingReq::new();
        let bytes = pingreq.to_bytes().unwrap();

        // Expected: Fixed header only: 0xC0 (packet type 12, flags 0) + 0x00 (remaining length 0)
        assert_eq!(bytes, vec![0xC0, 0x00]);
    }

    #[test]
    fn test_pingreq_deserialization() {
        let bytes = vec![0xC0, 0x00]; // PINGREQ with remaining length 0

        match MqttPingReq::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingReq(_), consumed) => {
                assert_eq!(consumed, 2);
            }
            _ => panic!("Expected PINGREQ packet"),
        }
    }

    #[test]
    fn test_pingreq_roundtrip() {
        let original = MqttPingReq::new();
        let bytes = original.to_bytes().unwrap();

        match MqttPingReq::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingReq(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PINGREQ packet"),
        }
    }

    #[test]
    fn test_pingreq_invalid_remaining_length() {
        let bytes = vec![0xC0, 0x01, 0xFF]; // PINGREQ with invalid remaining length 1

        match MqttPingReq::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 0") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_pingreq_invalid_flags() {
        let bytes = vec![0xC1, 0x00]; // PINGREQ with invalid flags (should be 0x00)

        match MqttPingReq::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_pingreq_incomplete_packet() {
        let bytes = vec![0xC0]; // Incomplete PINGREQ (missing remaining length)

        match MqttPingReq::from_bytes(&bytes) {
            Ok(ParseOk::Continue(needed, _)) => {
                assert_eq!(needed, 1); // Need 1 more byte for remaining length
            }
            Err(ParseError::BufferTooShort) => {
                // This is also acceptable for incomplete packets
            }
            other => panic!(
                "Expected Continue parse result or BufferTooShort, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_pingreq_wrong_packet_type() {
        let bytes = vec![0xD0, 0x00]; // PINGRESP packet type, not PINGREQ

        match MqttPingReq::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {
                // Expected error
            }
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_pingreq_empty_variable_header_and_payload() {
        let pingreq = MqttPingReq::new();

        assert!(pingreq.variable_header().unwrap().is_empty());
        assert!(pingreq.payload().unwrap().is_empty());
    }
}
