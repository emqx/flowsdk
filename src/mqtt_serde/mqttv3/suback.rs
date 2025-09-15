use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the SUBACK packet in MQTT v3.1.1.
///
/// The SUBACK packet is sent by the Server to the Client to confirm receipt and processing
/// of a SUBSCRIBE packet.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttSubAck {
    /// The Packet Identifier from the SUBSCRIBE packet that is being acknowledged.
    pub message_id: u16,
    /// The list of return codes. Each code corresponds to a topic filter in the SUBSCRIBE packet.
    pub return_codes: Vec<u8>,
}

impl MqttSubAck {
    /// Creates a new `MqttSubAck` packet.
    pub fn new(message_id: u16, return_codes: Vec<u8>) -> Self {
        Self {
            message_id,
            return_codes,
        }
    }
}

impl MqttControlPacket for MqttSubAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::SUBACK as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(self.message_id.to_be_bytes().to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        for &code in &self.return_codes {
            if !matches!(code, 0x00 | 0x01 | 0x02 | 0x80) {
                return Err(ParseError::ParseError(format!(
                    "Invalid SUBACK return code: {}",
                    code
                )));
            }
        }
        Ok(self.return_codes.clone())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::SUBACK as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // Bits 3,2,1,0 of fixed header MUST be 0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x00 {
            return Err(ParseError::ParseError(
                "SUBACK packet has invalid fixed header flags".to_string(),
            ));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Variable Header: Packet Identifier
        if size < 2 {
            return Err(ParseError::ParseError(
                "SUBACK packet must have a 2-byte message identifier".to_string(),
            ));
        }
        let message_id = u16::from_be_bytes([buffer[offset], buffer[offset + 1]]);
        offset += 2;

        // Payload: Return Codes
        let return_codes = buffer[offset..total_len].to_vec();
        for &code in &return_codes {
            if !matches!(code, 0x00 | 0x01 | 0x02 | 0x80) {
                return Err(ParseError::ParseError(format!(
                    "Invalid SUBACK return code in payload: {}",
                    code
                )));
            }
        }

        Ok(ParseOk::Packet(
            MqttPacket::SubAck3(MqttSubAck::new(message_id, return_codes)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suback_serialization() {
        let suback = MqttSubAck::new(123, vec![0x00, 0x01, 0x02, 0x80]);
        let bytes = suback.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0x90, // Packet type
                6,    // Remaining length (2 for msg id + 4 for codes)
                0x00, 0x7B, // Message ID
                0x00, 0x01, 0x02, 0x80, // Return codes
            ]
        );
    }

    #[test]
    fn test_suback_deserialization() {
        let bytes = vec![0x90, 0x05, 0x00, 0x0A, 0x00, 0x01, 0x80];
        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck3(suback), consumed) => {
                assert_eq!(consumed, 7);
                assert_eq!(suback.message_id, 10);
                assert_eq!(suback.return_codes, vec![0x00, 0x01, 0x80]);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_roundtrip() {
        let original = MqttSubAck::new(99, vec![0x02, 0x80, 0x01, 0x00]);
        let bytes = original.to_bytes().unwrap();
        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_invalid_flags() {
        let bytes = vec![0x91, 0x03, 0x00, 0x01, 0x00];
        match MqttSubAck::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_suback_invalid_return_code_serialize() {
        let suback = MqttSubAck::new(1, vec![0x03]); // 0x03 is not a valid code
        assert!(suback.to_bytes().is_err());
    }

    #[test]
    fn test_suback_invalid_return_code_deserialize() {
        let bytes = vec![0x90, 0x03, 0x00, 0x01, 0x03];
        match MqttSubAck::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("Invalid SUBACK return code") => {}
            other => panic!(
                "Expected ParseError with return code message, got {:?}",
                other
            ),
        }
    }
}
