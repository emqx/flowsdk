use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the PINGRESP packet in MQTT v5.0.
///
/// The PINGRESP packet is sent by the Server to the Client in response to a PINGREQ packet.
/// It indicates that the Server is alive.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPingResp;

impl MqttPingResp {
    /// Creates a new PINGRESP packet.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MqttPingResp {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttControlPacket for MqttPingResp {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PINGRESP as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT 5.0: 3.13.2 PINGRESP Variable Header
        // The PINGRESP packet has no Variable Header.
        Ok(Vec::new())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT 5.0: 3.13.3 PINGRESP Payload
        // The PINGRESP packet has no Payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PINGRESP as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let offset = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.13.1 PINGRESP Fixed Header
        // The Remaining Length field for the PINGRESP packet MUST be zero.
        #[cfg(feature = "strict-protocol-compliance")]
        if size != 0 {
            return Err(ParseError::ParseError(
                "PINGRESP packet must have remaining length of 0".to_string(),
            ));
        }

        // MQTT 5.0: 3.13.1 Reserved bits in the Fixed Header
        // Bits 3,2,1 and 0 of the Fixed Header in the PINGRESP packet are reserved and MUST be set to 0,0,0,0.
        #[cfg(feature = "strict-protocol-compliance")]
        {
            let flags = buffer[0] & 0x0F;
            if flags != 0x00 {
                return Err(ParseError::ParseError(
                    "PINGRESP packet has invalid fixed header flags".to_string(),
                ));
            }
        }

        let ping_resp = MqttPingResp::new();
        Ok(ParseOk::Packet(MqttPacket::PingResp5(ping_resp), total_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_serde::mqttv5::pingreq::MqttPingReq;

    #[test]
    fn test_pingresp_creation() {
        let pingresp = MqttPingResp::new();
        assert_eq!(
            pingresp.control_packet_type(),
            ControlPacketType::PINGRESP as u8
        );
    }

    #[test]
    fn test_pingresp_default() {
        let pingresp = MqttPingResp;
        assert_eq!(
            pingresp.control_packet_type(),
            ControlPacketType::PINGRESP as u8
        );
    }

    #[test]
    fn test_pingresp_serialization() {
        let pingresp = MqttPingResp::new();
        let bytes = pingresp.to_bytes().unwrap();

        // Expected: Fixed header only: 0xD0 (packet type 13, flags 0) + 0x00 (remaining length 0)
        assert_eq!(bytes, vec![0xD0, 0x00]);
    }

    #[test]
    fn test_pingresp_deserialization() {
        let bytes = vec![0xD0, 0x00]; // PINGRESP with remaining length 0

        match MqttPingResp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingResp5(_), consumed) => {
                assert_eq!(consumed, 2);
            }
            _ => panic!("Expected PINGRESP packet"),
        }
    }

    #[test]
    fn test_pingresp_roundtrip() {
        let original = MqttPingResp::new();
        let bytes = original.to_bytes().unwrap();

        match MqttPingResp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingResp5(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PINGRESP packet"),
        }
    }

    #[test]
    fn test_pingresp_invalid_remaining_length() {
        let bytes = vec![0xD0, 0x01, 0xFF]; // PINGRESP with invalid remaining length 1

        match MqttPingResp::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("remaining length of 0") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with remaining length message"),
        }
    }

    #[test]
    fn test_pingresp_invalid_flags() {
        let bytes = vec![0xD1, 0x00]; // PINGRESP with invalid flags (should be 0x00)

        match MqttPingResp::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_pingresp_incomplete_packet() {
        let bytes = vec![0xD0]; // Incomplete PINGRESP (missing remaining length)

        match MqttPingResp::from_bytes(&bytes) {
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
    fn test_pingresp_wrong_packet_type() {
        let bytes = vec![0xC0, 0x00]; // PINGREQ packet type, not PINGRESP

        match MqttPingResp::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {
                // Expected error
            }
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_pingresp_empty_variable_header_and_payload() {
        let pingresp = MqttPingResp::new();

        assert!(pingresp.variable_header().unwrap().is_empty());
        assert!(pingresp.payload().unwrap().is_empty());
    }

    #[test]
    fn test_ping_request_response_workflow() {
        // Test that PINGREQ and PINGRESP work together
        let pingreq = MqttPingReq::new();
        let pingreq_bytes = pingreq.to_bytes().unwrap();

        // Simulate server receiving PINGREQ and responding with PINGRESP
        let pingresp = MqttPingResp::new();
        let pingresp_bytes = pingresp.to_bytes().unwrap();

        // Verify both packets serialize correctly
        assert_eq!(pingreq_bytes, vec![0xC0, 0x00]);
        assert_eq!(pingresp_bytes, vec![0xD0, 0x00]);

        // Verify both can be parsed back
        match MqttPingReq::from_bytes(&pingreq_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingReq5(_), _) => {}
            _ => panic!("Expected PINGREQ"),
        }

        match MqttPingResp::from_bytes(&pingresp_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingResp5(_), _) => {}
            _ => panic!("Expected PINGRESP"),
        }
    }
}
