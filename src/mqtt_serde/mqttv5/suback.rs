// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, ParseError, ParseOk,
};

/// Represents the SUBACK packet in MQTT v5.0.
/// SUBACK is sent by the server to acknowledge a SUBSCRIBE packet.
/// It contains reason codes for each topic filter in the SUBSCRIBE packet.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttSubAck {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<Property>,
}

impl MqttSubAck {
    /// Creates a new `MqttSubAck` packet.
    ///
    /// # Arguments
    /// * `packet_id` - The packet identifier from the corresponding SUBSCRIBE packet
    /// * `reason_codes` - Reason codes for each topic subscription (must match SUBSCRIBE count)
    /// * `properties` - Optional properties
    pub fn new(packet_id: u16, reason_codes: Vec<u8>, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            reason_codes,
            properties,
        }
    }

    /// Creates a simple SUBACK with success codes for all subscriptions.
    pub fn new_success(packet_id: u16, subscription_count: usize) -> Self {
        let reason_codes = vec![0x00; subscription_count]; // All success
        Self::new(packet_id, reason_codes, Vec::new())
    }

    /// Creates a SUBACK with specific reason codes but no properties.
    pub fn new_simple(packet_id: u16, reason_codes: Vec<u8>) -> Self {
        Self::new(packet_id, reason_codes, Vec::new())
    }

    /// Creates a SUBACK with all subscriptions granted at the requested QoS levels.
    pub fn new_granted(packet_id: u16, qos_levels: Vec<u8>) -> Self {
        // QoS levels 0, 1, 2 are also valid reason codes for granted subscriptions
        Self::new(packet_id, qos_levels, Vec::new())
    }

    /// Creates a SUBACK with mixed success and failure reason codes.
    pub fn new_mixed(packet_id: u16, reason_codes: Vec<u8>, properties: Vec<Property>) -> Self {
        Self::new(packet_id, reason_codes, properties)
    }
}

impl MqttControlPacket for MqttSubAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::SUBACK as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.9.2 SUBACK Variable Header
        // Packet Identifier - always present and must be > 0
        #[cfg(feature = "strict-protocol-compliance")]
        if self.packet_id == 0 {
            return Err(ParseError::ParseError(
                "SUBACK packet_id must be > 0".to_string(),
            ));
        }
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.9.2.1 SUBACK Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.9.3 SUBACK Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // The payload must contain at least one reason code
        #[cfg(feature = "strict-protocol-compliance")]
        if self.reason_codes.is_empty() {
            return Err(ParseError::ParseError(
                "SUBACK payload must contain at least one reason code".to_string(),
            ));
        }

        // Each reason code is a single byte
        Ok(self.reason_codes.clone())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::SUBACK as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.9.2 SUBACK Variable Header
        // Packet Identifier - always present and must be > 0
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        #[cfg(feature = "strict-protocol-compliance")]
        if packet_id == 0 {
            return Err(ParseError::ParseError(
                "SUBACK packet_id must be > 0".to_string(),
            ));
        }

        // MQTT 5.0: 3.9.2.1 SUBACK Properties
        let (properties, consumed) = parse_properties_hdr(
            buffer
                .get(offset..total_len)
                .ok_or(ParseError::BufferTooShort)?,
        )?;
        offset += consumed;

        // MQTT 5.0: 3.9.3 SUBACK Payload
        // Remaining bytes are reason codes (one byte each)
        let reason_codes = buffer
            .get(offset..total_len)
            .ok_or(ParseError::BufferTooShort)?
            .to_vec();

        if reason_codes.is_empty() {
            #[cfg(feature = "strict-protocol-compliance")]
            return Err(ParseError::ParseError(
                "SUBACK payload must contain at least one reason code".to_string(),
            ));
        }

        let suback = MqttSubAck {
            packet_id,
            reason_codes,
            properties,
        };

        Ok(ParseOk::Packet(MqttPacket::SubAck5(suback), total_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suback_success() {
        // Test SUBACK with all successful subscriptions
        let suback = MqttSubAck::new_success(1234, 3);
        assert_eq!(suback.packet_id, 1234);
        assert_eq!(suback.reason_codes, vec![0x00, 0x00, 0x00]);
        assert!(suback.properties.is_empty());

        let bytes = suback.to_bytes().unwrap();

        // Test round-trip parsing
        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_suback.packet_id, 1234);
                assert_eq!(parsed_suback.reason_codes, vec![0x00, 0x00, 0x00]);
                assert!(parsed_suback.properties.is_empty());
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_granted_qos() {
        // Test SUBACK with granted QoS levels (0, 1, 2)
        let qos_levels = vec![0x00, 0x01, 0x02]; // QoS 0, 1, 2 granted
        let suback = MqttSubAck::new_granted(5678, qos_levels.clone());

        let bytes = suback.to_bytes().unwrap();

        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), _) => {
                assert_eq!(parsed_suback.packet_id, 5678);
                assert_eq!(parsed_suback.reason_codes, qos_levels);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_mixed_results() {
        // Test SUBACK with mixed success and failure codes
        let reason_codes = vec![
            0x00, // Maximum QoS 0 granted
            0x01, // Maximum QoS 1 granted
            0x02, // Maximum QoS 2 granted
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x8F, // Topic Filter invalid
            0x91, // Packet Identifier in use
            0x97, // Quota exceeded
        ];
        let suback = MqttSubAck::new_simple(9999, reason_codes.clone());

        let bytes = suback.to_bytes().unwrap();

        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), _) => {
                assert_eq!(parsed_suback.packet_id, 9999);
                assert_eq!(parsed_suback.reason_codes, reason_codes);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_with_properties() {
        // Test SUBACK with properties
        let reason_codes = vec![0x01, 0x02]; // QoS 1 and 2 granted
        let properties = vec![
            Property::ReasonString("Subscriptions processed".to_string()),
            Property::UserProperty("server".to_string(), "mqtt_broker_v5".to_string()),
        ];
        let suback = MqttSubAck::new(1111, reason_codes.clone(), properties);

        let bytes = suback.to_bytes().unwrap();

        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), _) => {
                assert_eq!(parsed_suback.packet_id, 1111);
                assert_eq!(parsed_suback.reason_codes, reason_codes);
                assert_eq!(parsed_suback.properties.len(), 2);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_single_subscription() {
        // Test SUBACK for single subscription
        let suback = MqttSubAck::new_simple(42, vec![0x02]); // QoS 2 granted

        let bytes = suback.to_bytes().unwrap();

        // Check packet structure
        assert_eq!(bytes[0], 0x90); // SUBACK packet type (9 << 4)

        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), _) => {
                assert_eq!(parsed_suback.packet_id, 42);
                assert_eq!(parsed_suback.reason_codes, vec![0x02]);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_error_conditions() {
        // Test SUBACK with packet_id = 0 (invalid)
        let suback = MqttSubAck::new_simple(0, vec![0x00]);
        assert!(suback.to_bytes().is_err());

        // Test SUBACK with no reason codes (invalid)
        let suback = MqttSubAck::new_simple(1, Vec::new());
        assert!(suback.to_bytes().is_err());
    }

    #[test]
    fn test_suback_all_error_codes() {
        // Test SUBACK with all subscription failures
        let reason_codes = vec![
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x87, // Not authorized
            0x8F, // Topic Filter invalid
            0x91, // Packet Identifier in use
            0x97, // Quota exceeded
            0x9E, // Shared Subscriptions not supported
            0xA1, // Subscription Identifiers not supported
            0xA2, // Wildcard Subscriptions not supported
        ];
        let suback = MqttSubAck::new_simple(2222, reason_codes.clone());

        let bytes = suback.to_bytes().unwrap();

        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), _) => {
                assert_eq!(parsed_suback.reason_codes, reason_codes);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_roundtrip_comprehensive() {
        // Comprehensive roundtrip test
        let reason_codes = vec![0x00, 0x01, 0x02, 0x80, 0x8F];
        let properties = vec![
            Property::ReasonString("Mixed subscription results".to_string()),
            Property::UserProperty("timestamp".to_string(), "2025-01-01T12:00:00Z".to_string()),
        ];
        let original_suback = MqttSubAck::new(0x7FFF, reason_codes.clone(), properties);

        let bytes = original_suback.to_bytes().unwrap();

        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_suback.packet_id, original_suback.packet_id);
                assert_eq!(parsed_suback.reason_codes, original_suback.reason_codes);
                assert_eq!(
                    parsed_suback.properties.len(),
                    original_suback.properties.len()
                );
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_suback_large_subscription_list() {
        // Test SUBACK with many subscriptions
        let reason_codes: Vec<u8> = (0..100)
            .map(|i| if i % 3 == 0 { 0x80 } else { (i % 3) as u8 })
            .collect();
        let suback = MqttSubAck::new_simple(3333, reason_codes.clone());

        let bytes = suback.to_bytes().unwrap();

        match MqttSubAck::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), _) => {
                assert_eq!(parsed_suback.packet_id, 3333);
                assert_eq!(parsed_suback.reason_codes.len(), 100);
                assert_eq!(parsed_suback.reason_codes, reason_codes);
            }
            _ => panic!("Expected SUBACK packet"),
        }
    }

    #[test]
    fn test_parse_suback() {
        let packet_bytes: [u8; 6] = [0x90, 0x04, 0xa9, 0x8f, 0x00, 0x00];
        let result = MqttSubAck::from_bytes(packet_bytes.as_ref());
        match result {
            Ok(ParseOk::Packet(packet, consumed)) => {
                assert_eq!(consumed, 6);
                match packet {
                    MqttPacket::SubAck5(suback) => {
                        assert_eq!(suback.packet_id, 43407);
                        assert_eq!(suback.reason_codes, vec![0]);
                    }
                    _ => panic!("Invalid packet"),
                }
            }
            _ => panic!("Invalid result"),
        }
    }
}
