// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the CONNACK packet in MQTT v3.1.1.
///
/// The CONNACK packet is the packet sent by the Server in response to a CONNECT packet
/// received from a Client.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttConnAck {
    /// If the Server accepts a connection with CleanSession set to 1, the Server MUST set
    /// Session Present to 0 in the CONNACK packet.
    /// If the Server accepts a connection with CleanSession set to 0, the value of Session Present
    /// depends on whether the Server already has stored Session State for the supplied client ID.
    pub session_present: bool,
    /// The return code for the connection attempt.
    pub return_code: u8,
}

impl MqttConnAck {
    /// Creates a new `MqttConnAck` packet.
    pub fn new(session_present: bool, return_code: u8) -> Self {
        Self {
            session_present,
            return_code,
        }
    }
}

impl MqttControlPacket for MqttConnAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::CONNACK as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        if self.return_code > 5 {
            return Err(ParseError::ParseError(
                "Invalid CONNACK return code".to_string(),
            ));
        }
        let session_present_byte = self.session_present as u8;
        Ok(vec![session_present_byte, self.return_code])
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // CONNACK has no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::CONNACK as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "CONNACK packet has invalid fixed header flags".to_string(),
            ));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let total_len = 1 + vbi_len + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        if size != 2 {
            return Err(ParseError::ParseError(
                "CONNACK packet must have a remaining length of 2".to_string(),
            ));
        }

        let vh_offset = 1 + vbi_len;
        let session_present_byte = buffer[vh_offset];
        if session_present_byte & 0xFE != 0 {
            return Err(ParseError::ParseError(
                "CONNACK session present flags reserved bits must be 0".to_string(),
            ));
        }
        let session_present = session_present_byte == 0x01;

        let return_code = buffer[vh_offset + 1];
        if return_code > 5 {
            return Err(ParseError::ParseError(
                "Invalid CONNACK return code".to_string(),
            ));
        }

        Ok(ParseOk::Packet(
            MqttPacket::ConnAck3(MqttConnAck::new(session_present, return_code)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connack_serialization() {
        let connack = MqttConnAck::new(true, 0x00);
        let bytes = connack.to_bytes().unwrap();
        assert_eq!(bytes, vec![0x20, 0x02, 0x01, 0x00]);
    }

    #[test]
    fn test_connack_deserialization() {
        let bytes = vec![0x20, 0x02, 0x01, 0x00];
        match MqttConnAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::ConnAck3(connack), len) => {
                assert_eq!(len, 4);
                assert!(connack.session_present);
                assert_eq!(connack.return_code, 0);
            }
            _ => panic!("Deserialization failed"),
        }
    }

    #[test]
    fn test_connack_roundtrip() {
        let original = MqttConnAck::new(false, 0x03);
        let bytes = original.to_bytes().unwrap();
        match MqttConnAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::ConnAck3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected CONNACK packet"),
        }
    }

    #[test]
    fn test_connack_invalid_return_code() {
        let bytes = vec![0x20, 0x02, 0x00, 0x06]; // Invalid return code
        assert!(MqttConnAck::from_bytes(&bytes).is_err());

        let connack = MqttConnAck::new(false, 6);
        assert!(connack.to_bytes().is_err());
    }

    #[test]
    fn test_connack_invalid_session_present_flags() {
        let bytes = vec![0x20, 0x02, 0x02, 0x00]; // Reserved bit is 1
        assert!(MqttConnAck::from_bytes(&bytes).is_err());
    }
}
