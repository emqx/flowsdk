use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, ParseError, ParseOk,
};

/// Represents the UNSUBACK packet in MQTT v5.0.
/// UNSUBACK is sent by the server to acknowledge an UNSUBSCRIBE packet.
/// It contains reason codes for each topic filter in the UNSUBSCRIBE packet.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttUnsubAck {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<Property>,
}

impl MqttUnsubAck {
    /// Creates a new `MqttUnsubAck` packet.
    ///
    /// # Arguments
    /// * `packet_id` - The packet identifier from the corresponding UNSUBSCRIBE packet
    /// * `reason_codes` - Reason codes for each topic unsubscription (must match UNSUBSCRIBE count)
    /// * `properties` - Optional properties
    pub fn new(packet_id: u16, reason_codes: Vec<u8>, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            reason_codes,
            properties,
        }
    }

    /// Creates a simple UNSUBACK with success codes for all unsubscriptions.
    pub fn new_success(packet_id: u16, unsubscription_count: usize) -> Self {
        let reason_codes = vec![0x00; unsubscription_count]; // All success
        Self::new(packet_id, reason_codes, Vec::new())
    }

    /// Creates an UNSUBACK with specific reason codes but no properties.
    pub fn new_simple(packet_id: u16, reason_codes: Vec<u8>) -> Self {
        Self::new(packet_id, reason_codes, Vec::new())
    }

    /// Creates an UNSUBACK with mixed success and failure reason codes.
    pub fn new_mixed(packet_id: u16, reason_codes: Vec<u8>, properties: Vec<Property>) -> Self {
        Self::new(packet_id, reason_codes, properties)
    }

    /// Creates an UNSUBACK with all unsubscriptions failed due to same error.
    pub fn new_all_failed(packet_id: u16, count: usize, reason_code: u8) -> Self {
        let reason_codes = vec![reason_code; count];
        Self::new(packet_id, reason_codes, Vec::new())
    }
}

impl MqttControlPacket for MqttUnsubAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::UNSUBACK as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.11.2 UNSUBACK Variable Header
        // Packet Identifier - always present and must be > 0
        #[cfg(feature = "strict-protocol-compliance")]
        if self.packet_id == 0 {
            return Err(ParseError::ParseError(
                "UNSUBACK packet_id must be > 0".to_string(),
            ));
        }
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.11.2.1 UNSUBACK Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.11.3 UNSUBACK Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // The payload must contain at least one reason code
        #[cfg(feature = "strict-protocol-compliance")]
        if self.reason_codes.is_empty() {
            return Err(ParseError::ParseError(
                "UNSUBACK payload must contain at least one reason code".to_string(),
            ));
        }

        // Each reason code is a single byte
        Ok(self.reason_codes.clone())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::UNSUBACK as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.11.2 UNSUBACK Variable Header
        // Packet Identifier - always present and must be > 0
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        #[cfg(feature = "strict-protocol-compliance")]
        if packet_id == 0 {
            return Err(ParseError::ParseError(
                "UNSUBACK packet_id must be > 0".to_string(),
            ));
        }

        // MQTT 5.0: 3.11.2.1 UNSUBACK Properties
        let (properties, consumed) = parse_properties_hdr(
            buffer
                .get(offset..total_len)
                .ok_or(ParseError::BufferTooShort)?,
        )?;
        offset += consumed;

        // MQTT 5.0: 3.11.3 UNSUBACK Payload
        // Remaining bytes are reason codes (one byte each)
        let reason_codes = buffer
            .get(offset..total_len)
            .ok_or(ParseError::BufferTooShort)?
            .to_vec();

        if reason_codes.is_empty() {
            #[cfg(feature = "strict-protocol-compliance")]
            return Err(ParseError::ParseError(
                "UNSUBACK payload must contain at least one reason code".to_string(),
            ));
        }

        let unsuback = MqttUnsubAck {
            packet_id,
            reason_codes,
            properties,
        };

        Ok(ParseOk::Packet(MqttPacket::UnsubAck5(unsuback), total_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsuback_success() {
        // Test UNSUBACK with all successful unsubscriptions
        let unsuback = MqttUnsubAck::new_success(1234, 3);
        assert_eq!(unsuback.packet_id, 1234);
        assert_eq!(unsuback.reason_codes, vec![0x00, 0x00, 0x00]);
        assert!(unsuback.properties.is_empty());

        let bytes = unsuback.to_bytes().unwrap();

        // Check packet structure
        assert_eq!(bytes[0], 0xB0); // UNSUBACK packet type (11 << 4)

        // Test round-trip parsing
        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_unsuback.packet_id, 1234);
                assert_eq!(parsed_unsuback.reason_codes, vec![0x00, 0x00, 0x00]);
                assert!(parsed_unsuback.properties.is_empty());
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_mixed_results() {
        // Test UNSUBACK with mixed success and failure codes
        let reason_codes = vec![
            0x00, // Success
            0x11, // No subscription existed
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x87, // Not authorized
            0x8F, // Topic Filter invalid
            0x91, // Packet Identifier in use
        ];
        let unsuback = MqttUnsubAck::new_simple(5678, reason_codes.clone());

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.packet_id, 5678);
                assert_eq!(parsed_unsuback.reason_codes, reason_codes);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_with_properties() {
        // Test UNSUBACK with properties
        let reason_codes = vec![0x00, 0x11]; // Success and No subscription existed
        let properties = vec![
            Property::ReasonString("Unsubscriptions processed".to_string()),
            Property::UserProperty("server".to_string(), "mqtt_broker_v5".to_string()),
        ];
        let unsuback = MqttUnsubAck::new(9999, reason_codes.clone(), properties);

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.packet_id, 9999);
                assert_eq!(parsed_unsuback.reason_codes, reason_codes);
                assert_eq!(parsed_unsuback.properties.len(), 2);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_single_unsubscription() {
        // Test UNSUBACK for single unsubscription
        let unsuback = MqttUnsubAck::new_simple(42, vec![0x00]); // Success

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.packet_id, 42);
                assert_eq!(parsed_unsuback.reason_codes, vec![0x00]);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_all_failed() {
        // Test UNSUBACK with all unsubscriptions failed
        let unsuback = MqttUnsubAck::new_all_failed(1111, 5, 0x87); // All Not authorized

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.packet_id, 1111);
                assert_eq!(parsed_unsuback.reason_codes, vec![0x87; 5]);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_error_conditions() {
        // Test UNSUBACK with packet_id = 0 (invalid)
        let unsuback = MqttUnsubAck::new_simple(0, vec![0x00]);
        assert!(unsuback.to_bytes().is_err());

        // Test UNSUBACK with no reason codes (invalid)
        let unsuback = MqttUnsubAck::new_simple(1, Vec::new());
        assert!(unsuback.to_bytes().is_err());
    }

    #[test]
    fn test_unsuback_all_success_codes() {
        // Test UNSUBACK with all successful unsubscriptions (various counts)
        for count in 1..=10 {
            let unsuback = MqttUnsubAck::new_success(2000 + count as u16, count);

            let bytes = unsuback.to_bytes().unwrap();

            match MqttUnsubAck::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                    assert_eq!(parsed_unsuback.reason_codes.len(), count);
                    assert!(parsed_unsuback
                        .reason_codes
                        .iter()
                        .all(|&code| code == 0x00));
                }
                _ => panic!("Expected UNSUBACK packet for count {}", count),
            }
        }
    }

    #[test]
    fn test_unsuback_no_subscription_existed() {
        // Test UNSUBACK where no subscriptions existed for the topics
        let reason_codes = vec![
            0x11, // No subscription existed
            0x11, // No subscription existed
            0x11, // No subscription existed
        ];
        let unsuback = MqttUnsubAck::new_simple(2222, reason_codes.clone());

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.reason_codes, reason_codes);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_all_error_codes() {
        // Test UNSUBACK with various error codes
        let reason_codes = vec![
            0x00, // Success
            0x11, // No subscription existed
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x87, // Not authorized
            0x8F, // Topic Filter invalid
            0x91, // Packet Identifier in use
        ];
        let unsuback = MqttUnsubAck::new_simple(3333, reason_codes.clone());

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.reason_codes, reason_codes);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_roundtrip_comprehensive() {
        // Comprehensive roundtrip test
        let reason_codes = vec![0x00, 0x11, 0x80, 0x87, 0x8F];
        let properties = vec![
            Property::ReasonString("Mixed unsubscription results".to_string()),
            Property::UserProperty("timestamp".to_string(), "2025-01-01T12:00:00Z".to_string()),
        ];
        let original_unsuback = MqttUnsubAck::new(0x7FFF, reason_codes.clone(), properties);

        let bytes = original_unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_unsuback.packet_id, original_unsuback.packet_id);
                assert_eq!(parsed_unsuback.reason_codes, original_unsuback.reason_codes);
                assert_eq!(
                    parsed_unsuback.properties.len(),
                    original_unsuback.properties.len()
                );
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_large_unsubscription_list() {
        // Test UNSUBACK with many unsubscriptions
        let reason_codes: Vec<u8> = (0..100)
            .map(|i| if i % 5 == 0 { 0x11 } else { 0x00 })
            .collect();
        let unsuback = MqttUnsubAck::new_simple(4444, reason_codes.clone());

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.packet_id, 4444);
                assert_eq!(parsed_unsuback.reason_codes.len(), 100);
                assert_eq!(parsed_unsuback.reason_codes, reason_codes);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_authorization_failure() {
        // Test UNSUBACK with authorization failures
        let reason_codes = vec![
            0x00, // Success
            0x87, // Not authorized
            0x87, // Not authorized
            0x00, // Success
        ];
        let properties = vec![Property::ReasonString(
            "Some topics require higher privileges".to_string(),
        )];
        let unsuback = MqttUnsubAck::new(5555, reason_codes.clone(), properties);

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.reason_codes, reason_codes);
                assert_eq!(parsed_unsuback.properties.len(), 1);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_unsuback_topic_filter_invalid() {
        // Test UNSUBACK with invalid topic filter errors
        let reason_codes = vec![
            0x00, // Success
            0x8F, // Topic Filter invalid
            0x00, // Success
            0x8F, // Topic Filter invalid
        ];
        let unsuback = MqttUnsubAck::new_simple(6666, reason_codes.clone());

        let bytes = unsuback.to_bytes().unwrap();

        match MqttUnsubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                assert_eq!(parsed_unsuback.reason_codes, reason_codes);
            }
            _ => panic!("Expected UNSUBACK packet"),
        }
    }

    #[test]
    fn test_parse_unsuback_2() {
        let packet = vec![0xb0, 0x04, 0x72, 0x0c, 0x00, 0x00];
        let result = MqttUnsubAck::from_bytes(&packet);
        match result {
            Ok(ParseOk::Packet(packet, consumed)) => {
                assert_eq!(consumed, 6);
                match packet {
                    MqttPacket::UnsubAck5(unsuback) => {
                        assert_eq!(unsuback.packet_id, 29196);
                        assert_eq!(unsuback.reason_codes, vec![0x00]);
                    }
                    _ => panic!("Invalid packet"),
                }
            }
            Ok(c) => panic!("Error parsing packet: {:?}", c),
            Err(e) => panic!("Error parsing packet: {:?}", e),
        }
    }
}
