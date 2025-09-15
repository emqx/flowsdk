use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::encode_utf8_string;
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, parse_utf8_string, ParseError, ParseOk,
};

/// Represents the UNSUBSCRIBE packet in MQTT v5.0.
/// UNSUBSCRIBE is used by clients to unsubscribe from one or more topics.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttUnsubscribe {
    pub packet_id: u16,
    pub topic_filters: Vec<String>,
    pub properties: Vec<Property>,
}

impl MqttUnsubscribe {
    /// Creates a new `MqttUnsubscribe` packet.
    ///
    /// # Arguments
    /// * `packet_id` - The packet identifier (must be > 0)
    /// * `topic_filters` - List of topic filters to unsubscribe from
    /// * `properties` - Optional properties
    pub fn new(packet_id: u16, topic_filters: Vec<String>, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            topic_filters,
            properties,
        }
    }

    /// Creates a simple UNSUBSCRIBE packet with basic topic filters.
    pub fn new_simple(packet_id: u16, topic_filters: Vec<String>) -> Self {
        Self::new(packet_id, topic_filters, Vec::new())
    }

    /// Adds a topic filter to this UNSUBSCRIBE packet.
    pub fn add_topic_filter(&mut self, topic_filter: String) {
        self.topic_filters.push(topic_filter);
    }

    /// Creates an UNSUBSCRIBE packet for a single topic.
    pub fn new_single(packet_id: u16, topic_filter: String) -> Self {
        Self::new_simple(packet_id, vec![topic_filter])
    }
}

impl MqttControlPacket for MqttUnsubscribe {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::UNSUBSCRIBE as u8
    }

    /// UNSUBSCRIBE packets require fixed header flags to be 0010 (bit 1 = 1)
    // MQTT 5.0: 3.10.1
    fn flags(&self) -> u8 {
        0x02 // Fixed header flags: bit 1 = 1, others = 0 (reserved)
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.10.2 UNSUBSCRIBE Variable Header
        // Packet Identifier - always present and must be > 0
        #[cfg(feature = "strict-protocol-compliance")]
        if self.packet_id == 0 {
            return Err(ParseError::ParseError(
                "UNSUBSCRIBE packet_id must be > 0".to_string(),
            ));
        }
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.10.2.1 UNSUBSCRIBE Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.10.3 UNSUBSCRIBE Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // The payload must contain at least one topic filter
        #[cfg(feature = "strict-protocol-compliance")]
        if self.topic_filters.is_empty() {
            return Err(ParseError::ParseError(
                "UNSUBSCRIBE payload must contain at least one topic filter".to_string(),
            ));
        }

        // Encode each topic filter as UTF-8 string
        for topic_filter in &self.topic_filters {
            bytes.extend(encode_utf8_string(topic_filter)?);
        }

        Ok(bytes)
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::UNSUBSCRIBE as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // MQTT 5.0: 3.10.1.1 Fixed Header Flags validation
        #[cfg(feature = "strict-protocol-compliance")]
        {
            let flags = buffer[0] & 0x0F;
            if flags != 0x02 {
                return Err(ParseError::ParseError(
                    "UNSUBSCRIBE fixed header flags must be 0010".to_string(),
                ));
            }
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.10.2 UNSUBSCRIBE Variable Header
        // Packet Identifier - always present and must be > 0
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        #[cfg(feature = "strict-protocol-compliance")]
        if packet_id == 0 {
            return Err(ParseError::ParseError(
                "UNSUBSCRIBE packet_id must be > 0".to_string(),
            ));
        }

        // MQTT 5.0: 3.10.2.1 UNSUBSCRIBE Properties
        let (properties, consumed) = parse_properties_hdr(
            buffer
                .get(offset..total_len)
                .ok_or(ParseError::BufferTooShort)?,
        )?;
        offset += consumed;

        // MQTT 5.0: 3.10.3 UNSUBSCRIBE Payload
        let mut topic_filters = Vec::new();

        while offset < total_len {
            let (topic_filter, consumed) = parse_utf8_string(&buffer[offset..])?;
            topic_filters.push(topic_filter);
            offset += consumed;
        }

        if topic_filters.is_empty() {
            #[cfg(feature = "strict-protocol-compliance")]
            return Err(ParseError::ParseError(
                "UNSUBSCRIBE payload must contain at least one topic filter".to_string(),
            ));
        }

        if offset != total_len {
            return Err(ParseError::InternalError(format!(
                "Inconsistent offset {} != total: {}",
                offset, total_len
            )));
        }

        let unsubscribe = MqttUnsubscribe {
            packet_id,
            topic_filters,
            properties,
        };

        Ok(ParseOk::Packet(
            MqttPacket::Unsubscribe5(unsubscribe),
            offset,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsubscribe_single_topic() {
        // Test UNSUBSCRIBE with single topic
        let unsubscribe = MqttUnsubscribe::new_single(1234, "test/topic".to_string());
        assert_eq!(unsubscribe.packet_id, 1234);
        assert_eq!(unsubscribe.topic_filters, vec!["test/topic"]);
        assert!(unsubscribe.properties.is_empty());

        let bytes = unsubscribe.to_bytes().unwrap();

        // Verify packet structure
        assert_eq!(bytes[0], 0xA2); // UNSUBSCRIBE (10 << 4) | 0x02 flags

        // Test round-trip parsing
        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_unsubscribe.packet_id, 1234);
                assert_eq!(parsed_unsubscribe.topic_filters, vec!["test/topic"]);
                assert!(parsed_unsubscribe.properties.is_empty());
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_multiple_topics() {
        // Test UNSUBSCRIBE with multiple topics
        let topic_filters = vec![
            "home/temperature".to_string(),
            "home/humidity".to_string(),
            "office/+/status".to_string(),
            "sensors/#".to_string(),
        ];
        let unsubscribe = MqttUnsubscribe::new_simple(5678, topic_filters.clone());

        let bytes = unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.packet_id, 5678);
                assert_eq!(parsed_unsubscribe.topic_filters, topic_filters);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_with_properties() {
        // Test UNSUBSCRIBE with properties
        let topic_filters = vec!["sensor/data".to_string()];
        let properties = vec![
            Property::UserProperty("client".to_string(), "sensor_client_1".to_string()),
            Property::ReasonString("Unsubscribing from sensors".to_string()),
        ];
        let unsubscribe = MqttUnsubscribe::new(9999, topic_filters.clone(), properties);

        let bytes = unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.packet_id, 9999);
                assert_eq!(parsed_unsubscribe.topic_filters, topic_filters);
                assert_eq!(parsed_unsubscribe.properties.len(), 2);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_wildcard_topics() {
        // Test UNSUBSCRIBE with wildcard topic filters
        let topic_filters = vec![
            "home/+/temperature".to_string(),
            "sensors/#".to_string(),
            "+/+/status".to_string(),
            "data/+/sensor/+/reading".to_string(),
        ];
        let unsubscribe = MqttUnsubscribe::new_simple(1111, topic_filters.clone());

        let bytes = unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.topic_filters, topic_filters);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_error_conditions() {
        // Test UNSUBSCRIBE with packet_id = 0 (invalid)
        let unsubscribe = MqttUnsubscribe::new_single(0, "test/topic".to_string());
        assert!(unsubscribe.to_bytes().is_err());

        // Test UNSUBSCRIBE with no topic filters (invalid)
        let unsubscribe = MqttUnsubscribe::new_simple(1, Vec::new());
        assert!(unsubscribe.to_bytes().is_err());
    }

    #[test]
    fn test_unsubscribe_fixed_header_flags() {
        // Test that UNSUBSCRIBE packets have correct fixed header flags (0x02)
        let unsubscribe = MqttUnsubscribe::new_single(1, "test".to_string());

        let bytes = unsubscribe.to_bytes().unwrap();

        // First byte should be 0xA2 (UNSUBSCRIBE type 10 << 4 | flags 0x02)
        assert_eq!(bytes[0], 0xA2);

        // Test that parsing validates the flags
        let mut invalid_bytes = bytes.clone();
        invalid_bytes[0] = 0xA0; // Wrong flags (should be 0x02)

        assert!(MqttUnsubscribe::from_bytes(&invalid_bytes).is_err());
    }

    #[test]
    fn test_unsubscribe_add_topic_filter() {
        // Test adding topic filters dynamically
        let mut unsubscribe = MqttUnsubscribe::new_simple(2222, vec!["initial/topic".to_string()]);

        unsubscribe.add_topic_filter("additional/topic".to_string());
        unsubscribe.add_topic_filter("another/+/topic".to_string());

        assert_eq!(unsubscribe.topic_filters.len(), 3);
        assert_eq!(unsubscribe.topic_filters[0], "initial/topic");
        assert_eq!(unsubscribe.topic_filters[1], "additional/topic");
        assert_eq!(unsubscribe.topic_filters[2], "another/+/topic");

        let bytes = unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.topic_filters, unsubscribe.topic_filters);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_long_topic_names() {
        // Test UNSUBSCRIBE with long topic names
        let long_topic = "a".repeat(1000); // Very long topic name
        let topic_filters = vec![
            long_topic.clone(),
            "short".to_string(),
            format!("{}/suffix", long_topic),
        ];
        let unsubscribe = MqttUnsubscribe::new_simple(3333, topic_filters.clone());

        let bytes = unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.topic_filters, topic_filters);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_roundtrip_comprehensive() {
        // Comprehensive roundtrip test with all features
        let topic_filters = vec![
            "simple/topic".to_string(),
            "wildcard/+/topic".to_string(),
            "multi/level/#".to_string(),
            "complex/+/path/+/ending".to_string(),
        ];
        let properties = vec![
            Property::UserProperty("client_type".to_string(), "test_client".to_string()),
            Property::ReasonString("Comprehensive unsubscribe test".to_string()),
        ];
        let original_unsubscribe = MqttUnsubscribe::new(0x7FFF, topic_filters.clone(), properties);

        let bytes = original_unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_unsubscribe.packet_id, original_unsubscribe.packet_id);
                assert_eq!(
                    parsed_unsubscribe.topic_filters,
                    original_unsubscribe.topic_filters
                );
                assert_eq!(
                    parsed_unsubscribe.properties.len(),
                    original_unsubscribe.properties.len()
                );
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_empty_topic_filter() {
        // Test UNSUBSCRIBE with empty topic filter (should work - server will handle validation)
        let topic_filters = vec!["".to_string()];
        let unsubscribe = MqttUnsubscribe::new_simple(4444, topic_filters.clone());

        let bytes = unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.topic_filters, topic_filters);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_special_characters() {
        // Test UNSUBSCRIBE with special characters in topic names
        let topic_filters = vec![
            "topic/with/unicode/🌡️".to_string(),
            "topic-with-dashes".to_string(),
            "topic_with_underscores".to_string(),
            "topic.with.dots".to_string(),
            "topic with spaces".to_string(), // Note: spaces are allowed in MQTT topics
        ];
        let unsubscribe = MqttUnsubscribe::new_simple(5555, topic_filters.clone());

        let bytes = unsubscribe.to_bytes().unwrap();

        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.topic_filters, topic_filters);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_parse_unsubscribe_2() {
        let packet = vec![
            0xa2, 0x10, 0x72, 0x0c, 0x00, 0x00, 0x0b, 0x74, 0x65, 0x73, 0x74, 0x74, 0x6f, 0x70,
            0x69, 0x63, 0x2f, 0x23,
        ];

        let result = MqttUnsubscribe::from_bytes(&packet);
        //assert!(result.is_ok());
        let (parsed_packet, offset) = match result.unwrap() {
            ParseOk::Packet(packet, offset) => (packet, offset),
            other => panic!("Expected ParseResult::Packet, got {:?}", other),
        };
        assert_eq!(offset, packet.len());
        assert_eq!(
            parsed_packet,
            MqttPacket::Unsubscribe5(MqttUnsubscribe::new(
                29196,
                vec!["testtopic/#".to_string()],
                vec![]
            ))
        );
    }
}
