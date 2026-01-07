// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the DISCONNECT packet in MQTT v3.1.1.
///
/// The DISCONNECT packet is the final control packet sent from the Client to the Server.
/// It indicates that the Client is disconnecting cleanly.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttDisconnect {
    // MQTT v3.1.1 DISCONNECT has no variable header and no payload
}

impl MqttDisconnect {
    /// Creates a new `MqttDisconnect` packet.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for MqttDisconnect {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttControlPacket for MqttDisconnect {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::DISCONNECT as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT v3.1.1 DISCONNECT has no variable header
        Ok(Vec::new())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT v3.1.1 DISCONNECT has no payload
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::DISCONNECT as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "DISCONNECT packet has invalid fixed header flags".to_string(),
            ));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let offset = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT v3.1.1 DISCONNECT must have remaining length of 0
        if size != 0 {
            return Err(ParseError::ParseError(
                "MQTT v3.1.1 DISCONNECT packet must have remaining length of 0".to_string(),
            ));
        }

        Ok(ParseOk::Packet(
            MqttPacket::Disconnect3(MqttDisconnect::new()),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disconnect_serialization() {
        let disconnect = MqttDisconnect::new();
        let bytes = disconnect.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0xE0, // Packet type and flags (DISCONNECT with flags 0000)
                0x00, // Remaining length = 0
            ]
        );
    }

    #[test]
    fn test_disconnect_deserialization() {
        let bytes = vec![0xE0, 0x00];
        match MqttDisconnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect3(disconnect), consumed) => {
                assert_eq!(consumed, 2);
                assert_eq!(disconnect, MqttDisconnect::new());
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_disconnect_roundtrip() {
        let original = MqttDisconnect::new();
        let bytes = original.to_bytes().unwrap();
        match MqttDisconnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_disconnect_invalid_flags() {
        let bytes = vec![0xE1, 0x00]; // Invalid flags (should be 0000)
        match MqttDisconnect::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_disconnect_invalid_remaining_length() {
        let bytes = vec![0xE0, 0x01, 0x00]; // Remaining length should be 0
        match MqttDisconnect::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("must have remaining length of 0") => {
            }
            other => panic!(
                "Expected ParseError with remaining length message, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_disconnect_default() {
        let disconnect = MqttDisconnect::default();
        assert_eq!(disconnect, MqttDisconnect::new());
    }
}
