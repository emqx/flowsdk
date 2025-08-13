use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, ParseError, ParseOk,
};

/// Represents the PUBCOMP packet in MQTT v5.0.
/// PUBCOMP is the final acknowledgment in a QoS 2 PUBLISH packet flow.
/// It is sent in response to a PUBREL packet.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubComp {
    pub packet_id: u16,
    pub reason_code: u8,
    pub properties: Vec<Property>,
}

impl MqttPubComp {
    /// Creates a new `MqttPubComp` packet.
    ///
    /// # Arguments
    /// * `packet_id` - The packet identifier from the corresponding PUBREL packet
    /// * `reason_code` - The reason code (0x00 = Success, others indicate error)
    /// * `properties` - Optional properties
    pub fn new(packet_id: u16, reason_code: u8, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            reason_code,
            properties,
        }
    }

    /// Creates a successful PUBCOMP with no properties (minimal packet)
    pub fn new_success(packet_id: u16) -> Self {
        Self::new(packet_id, 0x00, Vec::new())
    }

    /// Creates a PUBCOMP with an error reason code
    pub fn new_error(packet_id: u16, reason_code: u8, properties: Vec<Property>) -> Self {
        Self::new(packet_id, reason_code, properties)
    }
}

impl MqttControlPacket for MqttPubComp {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBCOMP as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.7.2 PUBCOMP Variable Header
        // Packet Identifier - always present
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.7.2.1 PUBCOMP Reason Code
        // The Reason Code and Properties are only present if the Remaining Length is greater than 2.
        // For a successful acknowledgment with no properties, we send a minimal packet.
        if self.reason_code == 0x00 && self.properties.is_empty() {
            return Ok(bytes);
        }

        // Include reason code if not default success or if properties are present
        bytes.push(self.reason_code);

        // MQTT 5.0: 3.7.2.2 PUBCOMP Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.7.3 PUBCOMP Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBCOMP packets have no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBCOMP as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.7.2 PUBCOMP Variable Header
        // Packet Identifier - always present
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        // MQTT 5.0: 3.7.2.1 PUBCOMP Reason Code
        // Defaults to 0x00 (Success) if not present (when remaining length is exactly 2).
        let reason_code = if size > 2 {
            let code = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
            offset += 1;
            code
        } else {
            0x00 // Default to Success
        };

        // MQTT 5.0: 3.7.2.2 PUBCOMP Properties
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

        let pubcomp = MqttPubComp {
            packet_id,
            reason_code,
            properties,
        };

        Ok(ParseOk::Packet(MqttPacket::PubComp(pubcomp), offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pubcomp_minimal_success() {
        // Test minimal PUBCOMP (success, no properties)
        let pubcomp = MqttPubComp::new_success(0x1234);
        let bytes = pubcomp.to_bytes().unwrap();

        // Expected: [0x70, 0x02, 0x12, 0x34]
        // 0x70 = PUBCOMP packet type (7 << 4)
        // 0x02 = remaining length (2 bytes for packet ID only)
        // 0x12, 0x34 = packet ID in big-endian
        let expected = vec![0x70, 0x02, 0x12, 0x34];
        assert_eq!(bytes, expected);

        // Test round-trip parsing
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp(parsed_pubcomp), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(parsed_pubcomp.packet_id, 0x1234);
                assert_eq!(parsed_pubcomp.reason_code, 0x00);
                assert!(parsed_pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_with_reason_code() {
        // Test PUBCOMP with reason code but no properties
        let pubcomp = MqttPubComp::new(0x5678, 0x92, Vec::new()); // 0x92 = Packet Identifier not found
        let bytes = pubcomp.to_bytes().unwrap();

        // Expected: [0x70, 0x04, 0x56, 0x78, 0x92, 0x00]
        // 0x70 = PUBCOMP packet type
        // 0x04 = remaining length (2 bytes packet ID + 1 byte reason code + 1 byte properties length)
        // 0x56, 0x78 = packet ID
        // 0x92 = reason code (Packet Identifier not found)
        // 0x00 = properties length (empty)
        let expected = vec![0x70, 0x04, 0x56, 0x78, 0x92, 0x00];
        assert_eq!(bytes, expected);

        // Test parsing
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp(parsed_pubcomp), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(parsed_pubcomp.packet_id, 0x5678);
                assert_eq!(parsed_pubcomp.reason_code, 0x92);
                assert!(parsed_pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_with_properties() {
        // Test PUBCOMP with properties
        let properties = vec![
            Property::ReasonString("QoS 2 flow completed".to_string()),
            Property::UserProperty("flow".to_string(), "qos2".to_string()),
        ];
        let pubcomp = MqttPubComp::new(0xABCD, 0x00, properties); // Success with properties
        let bytes = pubcomp.to_bytes().unwrap();

        // Test that it can be parsed back
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp(parsed_pubcomp), _) => {
                assert_eq!(parsed_pubcomp.packet_id, 0xABCD);
                assert_eq!(parsed_pubcomp.reason_code, 0x00);
                assert_eq!(parsed_pubcomp.properties.len(), 2);
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_parsing_minimal() {
        // Test parsing minimal PUBCOMP packet (2 bytes remaining length)
        let bytes = vec![0x70, 0x02, 0x00, 0x01]; // packet ID = 1

        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp(pubcomp), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubcomp.packet_id, 1);
                assert_eq!(pubcomp.reason_code, 0x00); // Default success
                assert!(pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_error_conditions() {
        // Test various PUBCOMP reason codes
        let error_codes = vec![
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x92, // Packet Identifier not found
        ];

        for &error_code in &error_codes {
            let pubcomp = MqttPubComp::new_error(0x3000, error_code, Vec::new());
            let bytes = pubcomp.to_bytes().unwrap();

            match MqttPubComp::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::PubComp(parsed), _) => {
                    assert_eq!(parsed.packet_id, 0x3000);
                    assert_eq!(parsed.reason_code, error_code);
                }
                _ => panic!("Expected PUBCOMP packet for error code {:#x}", error_code),
            }
        }
    }

    #[test]
    fn test_parse_pubcomp() {
        // Test parsing a PUBCOMP packet
        let buffer: [u8; 6] = [0x70, 0x04, 0x00, 0x05, 0x00, 0x00];
        let result = MqttPubComp::from_bytes(&buffer);
        assert!(result.is_ok());

        match result.unwrap() {
            ParseOk::Packet(MqttPacket::PubComp(pubcomp), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(pubcomp.packet_id, 5);
                assert_eq!(pubcomp.reason_code, 0x00); // Success
                assert!(pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_qos2_flow_completion() {
        // Test creating PUBCOMP as part of QoS 2 flow completion
        // This would typically be in response to a PUBREL
        let packet_id = 0x1A2B;

        // Successful completion
        let pubcomp_success = MqttPubComp::new_success(packet_id);
        assert_eq!(pubcomp_success.packet_id, packet_id);
        assert_eq!(pubcomp_success.reason_code, 0x00);

        // Error during completion
        let pubcomp_error = MqttPubComp::new_error(
            packet_id,
            0x92, // Packet Identifier not found
            vec![Property::ReasonString("PUBREL not found".to_string())],
        );
        assert_eq!(pubcomp_error.packet_id, packet_id);
        assert_eq!(pubcomp_error.reason_code, 0x92);
        assert_eq!(pubcomp_error.properties.len(), 1);
    }

    #[test]
    fn test_pubcomp_roundtrip_with_properties() {
        // Test complete roundtrip with properties
        let original_pubcomp = MqttPubComp::new(
            0xDEAD,
            0x83,
            vec![
                Property::ReasonString("Processing completed with warnings".to_string()),
                Property::UserProperty("timestamp".to_string(), "2025-01-01T12:00:00Z".to_string()),
            ],
        );

        let bytes = original_pubcomp.to_bytes().unwrap();

        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp(parsed_pubcomp), _) => {
                assert_eq!(parsed_pubcomp.packet_id, original_pubcomp.packet_id);
                assert_eq!(parsed_pubcomp.reason_code, original_pubcomp.reason_code);
                assert_eq!(
                    parsed_pubcomp.properties.len(),
                    original_pubcomp.properties.len()
                );
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_parse_pubcomp_2() {
        let packet = vec![0x70, 0x03, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00];
        let result = MqttPubComp::from_bytes(&packet);
        match result {
            Ok(ParseOk::Packet(MqttPacket::PubComp(pubcomp), _consumed)) => {
                assert_eq!(pubcomp.reason_code, 0);
                assert_eq!(pubcomp.properties, vec![]);
            }
            _ => panic!("Invalid result: {:?}", result),
        }
    }

    #[test]
    fn test_parse_pubcomp_with_properties() {
        let packet_bytes: [u8; 6] = [0x70, 0x04, 0x00, 0x02, 0x00, 0x00];

        let result = MqttPubComp::from_bytes(&packet_bytes);
        match result {
            Ok(ParseOk::Packet(MqttPacket::PubComp(pubcomp), _consumed)) => {
                assert_eq!(pubcomp.packet_id, 2);
                assert_eq!(pubcomp.reason_code, 0);
                assert_eq!(pubcomp.properties, vec![]);
            }
            _ => panic!("Invalid result: {:?}", result),
        }
    }
}
