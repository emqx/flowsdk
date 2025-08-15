use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, ParseError, ParseOk,
};

/// Represents the PUBREL packet in MQTT v5.0.
/// PUBREL is the second acknowledgment in a QoS 2 PUBLISH packet flow.
/// It is sent in response to a PUBREC packet.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubRel {
    pub packet_id: u16,
    pub reason_code: u8,
    pub properties: Vec<Property>,
}

impl MqttPubRel {
    /// Creates a new `MqttPubRel` packet.
    ///
    /// # Arguments
    /// * `packet_id` - The packet identifier from the corresponding PUBREC packet
    /// * `reason_code` - The reason code (0x00 = Success, others indicate error)
    /// * `properties` - Optional properties
    pub fn new(packet_id: u16, reason_code: u8, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            reason_code,
            properties,
        }
    }

    /// Creates a successful PUBREL with no properties (minimal packet)
    pub fn new_success(packet_id: u16) -> Self {
        Self::new(packet_id, 0x00, Vec::new())
    }

    /// Creates a PUBREL with an error reason code
    pub fn new_error(packet_id: u16, reason_code: u8, properties: Vec<Property>) -> Self {
        Self::new(packet_id, reason_code, properties)
    }
}

impl MqttControlPacket for MqttPubRel {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBREL as u8
    }

    /// PUBREL is the only control packet that has fixed header flags set to 0010 (bit 1 = 1)
    /// This is required by MQTT 5.0 specification section 3.6.1.1
    fn flags(&self) -> u8 {
        0x02 // Fixed header flags: bit 1 = 1, others = 0
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.6.2 PUBREL Variable Header
        // Packet Identifier - always present
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.6.2.1 PUBREL Reason Code
        // The Reason Code and Properties are only present if the Remaining Length is greater than 2.
        // For a successful acknowledgment with no properties, we send a minimal packet.
        if self.reason_code == 0x00 && self.properties.is_empty() {
            return Ok(bytes);
        }

        // Include reason code if not default success or if properties are present
        bytes.push(self.reason_code);

        // MQTT 5.0: 3.6.2.2 PUBREL Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.6.3 PUBREL Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBREL packets have no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let first_byte = *buffer.first().ok_or(ParseError::BufferTooShort)?;
        let packet_type = packet_type(buffer)?;

        if packet_type != ControlPacketType::PUBREL as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // MQTT 5.0: 3.6.1.1 - PUBREL Fixed Header flags must be 0010
        #[cfg(feature = "strict-protocol-compliance")]
        {
            let flags = first_byte & 0x0F;
            if flags != 0x02 {
                return Err(ParseError::ParseError(format!(
                    "Invalid PUBREL flags: expected 0x02, got 0x{:02x}",
                    flags
                )));
            }
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.6.2 PUBREL Variable Header
        // Packet Identifier - always present
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        // MQTT 5.0: 3.6.2.1 PUBREL Reason Code
        // Defaults to 0x00 (Success) if not present (when remaining length is exactly 2).
        let reason_code = if size > 2 {
            let code = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
            offset += 1;
            code
        } else {
            0x00 // Default to Success
        };

        // MQTT 5.0: 3.6.2.2 PUBREL Properties
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

        let pubrel = MqttPubRel {
            packet_id,
            reason_code,
            properties,
        };

        Ok(ParseOk::Packet(MqttPacket::PubRel(pubrel), offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pubrel_minimal_success() {
        // Test minimal PUBREL (success, no properties)
        let pubrel = MqttPubRel::new_success(0x1234);
        let bytes = pubrel.to_bytes().unwrap();

        // Expected: [0x62, 0x02, 0x12, 0x34]
        // 0x62 = PUBREL packet type (6 << 4) | flags (0x02)
        // 0x02 = remaining length (2 bytes for packet ID only)
        // 0x12, 0x34 = packet ID in big-endian
        let expected = vec![0x62, 0x02, 0x12, 0x34];
        assert_eq!(bytes, expected);

        // Test round-trip parsing
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel(parsed_pubrel), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(parsed_pubrel.packet_id, 0x1234);
                assert_eq!(parsed_pubrel.reason_code, 0x00);
                assert!(parsed_pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_with_reason_code() {
        // Test PUBREL with reason code but no properties
        let pubrel = MqttPubRel::new(0x5678, 0x92, Vec::new()); // 0x92 = Packet Identifier not found
        let bytes = pubrel.to_bytes().unwrap();

        // Expected: [0x62, 0x04, 0x56, 0x78, 0x92, 0x00]
        // 0x62 = PUBREL packet type with flags
        // 0x04 = remaining length (2 bytes packet ID + 1 byte reason code + 1 byte properties length)
        // 0x56, 0x78 = packet ID
        // 0x92 = reason code (Packet Identifier not found)
        // 0x00 = properties length (empty)
        let expected = vec![0x62, 0x04, 0x56, 0x78, 0x92, 0x00];
        assert_eq!(bytes, expected);

        // Test parsing
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel(parsed_pubrel), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(parsed_pubrel.packet_id, 0x5678);
                assert_eq!(parsed_pubrel.reason_code, 0x92);
                assert!(parsed_pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_with_properties() {
        // Test PUBREL with properties
        let properties = vec![
            Property::ReasonString("Packet processing error".to_string()),
            Property::UserProperty("context".to_string(), "test".to_string()),
        ];
        let pubrel = MqttPubRel::new(0xABCD, 0x80, properties); // 0x80 = Unspecified error
        let bytes = pubrel.to_bytes().unwrap();

        // Test that it can be parsed back
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel(parsed_pubrel), _) => {
                assert_eq!(parsed_pubrel.packet_id, 0xABCD);
                assert_eq!(parsed_pubrel.reason_code, 0x80);
                assert_eq!(parsed_pubrel.properties.len(), 2);
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_parsing_minimal() {
        // Test parsing minimal PUBREL packet (2 bytes remaining length)
        let bytes = vec![0x62, 0x02, 0x00, 0x01]; // packet ID = 1

        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel(pubrel), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubrel.packet_id, 1);
                assert_eq!(pubrel.reason_code, 0x00); // Default success
                assert!(pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_flags_validation() {
        // Test that PUBREL with incorrect flags is rejected
        let bytes_wrong_flags = vec![0x60, 0x02, 0x00, 0x01]; // 0x60 = wrong flags (should be 0x62)

        match MqttPubRel::from_bytes(&bytes_wrong_flags) {
            Err(ParseError::ParseError(msg)) => {
                assert!(msg.contains("Invalid PUBREL flags"));
            }
            _ => panic!("Expected parse error for wrong flags"),
        }
    }

    #[test]
    fn test_pubrel_error_conditions() {
        // Test various PUBREL reason codes
        let error_codes = vec![
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x92, // Packet Identifier not found
        ];

        for &error_code in &error_codes {
            let pubrel = MqttPubRel::new_error(0x2000, error_code, Vec::new());
            let bytes = pubrel.to_bytes().unwrap();

            match MqttPubRel::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::PubRel(parsed), _) => {
                    assert_eq!(parsed.packet_id, 0x2000);
                    assert_eq!(parsed.reason_code, error_code);
                }
                _ => panic!("Expected PUBREL packet for error code {:#x}", error_code),
            }
        }
    }

    #[test]
    fn test_pubrel_flags() {
        // Test that PUBREL has the correct flags (0x02)
        let pubrel = MqttPubRel::new_success(0x1000);
        assert_eq!(pubrel.flags(), 0x02);
    }

    #[test]
    fn test_parse_pubrel_extended() {
        // Test parsing a PUBREL with reason code and properties
        let buffer: [u8; 6] = [0x62, 0x04, 0x00, 0x10, 0x92, 0x00];
        let result = MqttPubRel::from_bytes(&buffer);
        assert!(result.is_ok());

        match result.unwrap() {
            ParseOk::Packet(MqttPacket::PubRel(pubrel), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(pubrel.packet_id, 16);
                assert_eq!(pubrel.reason_code, 0x92); // Packet Identifier not found
                assert!(pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_parse_pubrel() {
        let packet_bytes: [u8; 6] = [0x62, 0x04, 0x00, 0x03, 0x00, 0x00];

        let result = MqttPubRel::from_bytes(&packet_bytes);
        match result {
            Ok(ParseOk::Packet(packet, consumed)) => {
                assert_eq!(consumed, 6);
                assert_eq!(packet, MqttPacket::PubRel(MqttPubRel::new(3, 0, vec![])));
            }
            _ => panic!("Invalid result: {:?}", result),
        }
    }
}
