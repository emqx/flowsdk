use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, ParseError, ParseOk,
};

/// Represents the PUBREC packet in MQTT v5.0.
/// PUBREC is the first acknowledgment of a QoS 2 PUBLISH packet flow.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubRec {
    pub packet_id: u16,
    pub reason_code: u8,
    pub properties: Vec<Property>,
}

impl MqttPubRec {
    /// Creates a new `MqttPubRec` packet.
    ///
    /// # Arguments
    /// * `packet_id` - The packet identifier from the PUBLISH packet
    /// * `reason_code` - The reason code (0x00 = Success, others indicate error)
    /// * `properties` - Optional properties
    pub fn new(packet_id: u16, reason_code: u8, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            reason_code,
            properties,
        }
    }

    /// Creates a successful PUBREC with no properties (minimal packet)
    pub fn new_success(packet_id: u16) -> Self {
        Self::new(packet_id, 0x00, Vec::new())
    }

    /// Creates a PUBREC with an error reason code
    pub fn new_error(packet_id: u16, reason_code: u8, properties: Vec<Property>) -> Self {
        Self::new(packet_id, reason_code, properties)
    }
}

impl MqttControlPacket for MqttPubRec {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBREC as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.5.2 PUBREC Variable Header
        // Packet Identifier - always present
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.5.2.1 PUBREC Reason Code
        // The Reason Code and Properties are only present if the Remaining Length is greater than 2.
        // For a successful acknowledgment with no properties, we send a minimal packet.
        if self.reason_code == 0x00 && self.properties.is_empty() {
            return Ok(bytes);
        }

        // Include reason code if not default success or if properties are present
        bytes.push(self.reason_code);

        // MQTT 5.0: 3.5.2.2 PUBREC Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.5.3 PUBREC Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBREC packets have no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBREC as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.5.2 PUBREC Variable Header
        // Packet Identifier - always present
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        // MQTT 5.0: 3.5.2.1 PUBREC Reason Code
        // Defaults to 0x00 (Success) if not present (when remaining length is exactly 2).
        let reason_code = if size > 2 {
            let code = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
            offset += 1;
            code
        } else {
            0x00 // Default to Success
        };

        // MQTT 5.0: 3.5.2.2 PUBREC Properties
        let (properties, consumed) = if offset < total_len {
            parse_properties_hdr(
                buffer
                    .get(offset..total_len)
                    .ok_or(ParseError::BufferTooShort)?,
            )?
        } else {
            (vec![], 0)
        };
        offset += consumed;

        if offset != total_len {
            return Err(ParseError::InternalError(format!(
                "Inconsistent offset {} != total: {}",
                offset, total_len
            )));
        }

        let pubrec = MqttPubRec {
            packet_id,
            reason_code,
            properties,
        };

        Ok(ParseOk::Packet(MqttPacket::PubRec(pubrec), offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pubrec_minimal_success() {
        // Test minimal PUBREC (success, no properties)
        let pubrec = MqttPubRec::new_success(0x1234);
        let bytes = pubrec.to_bytes().unwrap();

        // Expected: [0x50, 0x02, 0x12, 0x34]
        // 0x50 = PUBREC packet type (5 << 4)
        // 0x02 = remaining length (2 bytes for packet ID only)
        // 0x12, 0x34 = packet ID in big-endian
        let expected = vec![0x50, 0x02, 0x12, 0x34];
        assert_eq!(bytes, expected);

        // Test round-trip parsing
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec(parsed_pubrec), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(parsed_pubrec.packet_id, 0x1234);
                assert_eq!(parsed_pubrec.reason_code, 0x00);
                assert!(parsed_pubrec.properties.is_empty());
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_with_reason_code() {
        // Test PUBREC with reason code but no properties
        let pubrec = MqttPubRec::new(0x5678, 0x80, Vec::new()); // 0x80 = Unspecified error
        let bytes = pubrec.to_bytes().unwrap();

        // Expected: [0x50, 0x04, 0x56, 0x78, 0x80, 0x00]
        // 0x50 = PUBREC packet type
        // 0x04 = remaining length (2 bytes packet ID + 1 byte reason code + 1 byte properties length)
        // 0x56, 0x78 = packet ID
        // 0x80 = reason code (Unspecified error)
        // 0x00 = properties length (empty)
        let expected = vec![0x50, 0x04, 0x56, 0x78, 0x80, 0x00];
        assert_eq!(bytes, expected);

        // Test parsing
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec(parsed_pubrec), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(parsed_pubrec.packet_id, 0x5678);
                assert_eq!(parsed_pubrec.reason_code, 0x80);
                assert!(parsed_pubrec.properties.is_empty());
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_with_properties() {
        // Test PUBREC with properties
        let properties = vec![
            Property::ReasonString("Test error".to_string()),
            Property::UserProperty("key".to_string(), "value".to_string()),
        ];
        let pubrec = MqttPubRec::new(0xABCD, 0x83, properties); // 0x83 = Implementation specific error
        let bytes = pubrec.to_bytes().unwrap();

        // Test that it can be parsed back
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec(parsed_pubrec), _) => {
                assert_eq!(parsed_pubrec.packet_id, 0xABCD);
                assert_eq!(parsed_pubrec.reason_code, 0x83);
                assert_eq!(parsed_pubrec.properties.len(), 2);
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_parsing_minimal() {
        // Test parsing minimal PUBREC packet (2 bytes remaining length)
        let bytes = vec![0x50, 0x02, 0x00, 0x01]; // packet ID = 1

        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec(pubrec), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubrec.packet_id, 1);
                assert_eq!(pubrec.reason_code, 0x00); // Default success
                assert!(pubrec.properties.is_empty());
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_error_conditions() {
        // Test various PUBREC reason codes
        let error_codes = vec![
            0x10, // No matching subscribers
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x91, // Quota exceeded
            0x97, // Payload format invalid
        ];

        for &error_code in &error_codes {
            let pubrec = MqttPubRec::new_error(0x1000, error_code, Vec::new());
            let bytes = pubrec.to_bytes().unwrap();

            match MqttPubRec::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::PubRec(parsed), _) => {
                    assert_eq!(parsed.packet_id, 0x1000);
                    assert_eq!(parsed.reason_code, error_code);
                }
                _ => panic!("Expected PUBREC packet for error code {:#x}", error_code),
            }
        }
    }

    #[test]
    fn test_parse_pubrec() {
        let buffer: [u8; 6] = [0x50, 0x04, 0x00, 0x02, 0x10, 0x00];
        let result = MqttPubRec::from_bytes(&buffer);
        assert!(result.is_ok());
        let (parsed, _) = match result.unwrap() {
            ParseOk::Packet(packet, _) => (packet, 0),
            _ => panic!("Invalid packet"),
        };
        assert_eq!(
            parsed,
            MqttPacket::PubRec(MqttPubRec::new(2_u16, 0x10, vec![]))
        );
    }
}
