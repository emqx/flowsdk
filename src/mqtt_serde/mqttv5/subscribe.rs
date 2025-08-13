use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::encode_utf8_string;
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, parse_utf8_string, ParseError, ParseOk,
};

/// Represents a topic subscription with its options in MQTT v5.0.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct TopicSubscription {
    pub topic_filter: String,
    pub qos: u8,                   // QoS level (0, 1, or 2)
    pub no_local: bool,            // No Local option
    pub retain_as_published: bool, // Retain As Published option
    pub retain_handling: u8,       // Retain Handling option (0, 1, or 2)
}

impl TopicSubscription {
    /// Creates a new topic subscription with the specified options.
    pub fn new(
        topic_filter: String,
        qos: u8,
        no_local: bool,
        retain_as_published: bool,
        retain_handling: u8,
    ) -> Self {
        Self {
            topic_filter,
            qos,
            no_local,
            retain_as_published,
            retain_handling,
        }
    }

    /// Creates a simple subscription with just QoS (other options default to false/0).
    pub fn new_simple(topic_filter: String, qos: u8) -> Self {
        Self::new(topic_filter, qos, false, false, 0)
    }

    /// Encodes the subscription options byte according to MQTT 5.0 spec.
    /// Bit layout:
    /// - Bits 0-1: Maximum QoS
    /// - Bit 2: No Local flag
    /// - Bit 3: Retain As Published flag  
    /// - Bits 4-5: Retain Handling option
    /// - Bits 6-7: Reserved (must be 0)
    fn encode_subscription_options(&self) -> u8 {
        let mut options = 0u8;

        // Bits 0-1: Maximum QoS
        options |= self.qos & 0x03;

        // Bit 2: No Local flag
        if self.no_local {
            options |= 0x04;
        }

        // Bit 3: Retain As Published flag
        if self.retain_as_published {
            options |= 0x08;
        }

        // Bits 4-5: Retain Handling option
        options |= (self.retain_handling & 0x03) << 4;

        // Bits 6-7: Reserved (already 0)

        options
    }

    /// Decodes the subscription options byte according to MQTT 5.0 spec.
    fn decode_subscription_options(options: u8) -> (u8, bool, bool, u8) {
        let qos = options & 0x03;
        let no_local = (options & 0x04) != 0;
        let retain_as_published = (options & 0x08) != 0;
        let retain_handling = (options >> 4) & 0x03;

        (qos, no_local, retain_as_published, retain_handling)
    }

    /// Encodes this topic subscription to bytes for inclusion in SUBSCRIBE payload.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // Topic Filter (UTF-8 String)
        bytes.extend(encode_utf8_string(&self.topic_filter)?);

        // Subscription Options
        bytes.push(self.encode_subscription_options());

        Ok(bytes)
    }

    /// Parses a topic subscription from bytes.
    pub fn from_bytes(buffer: &[u8]) -> Result<(TopicSubscription, usize), ParseError> {
        let mut offset = 0;

        // Parse Topic Filter (UTF-8 String)
        let (topic_filter, consumed) = parse_utf8_string(&buffer[offset..])?;
        offset += consumed;
        
        // MQTT 5.0 Protocol Compliance: Topic filter validation
        #[cfg(feature = "strict-protocol-compliance")]
        {
            crate::mqtt_serde::validate_topic_filter(&topic_filter)?;
            crate::mqtt_serde::validate_shared_subscription(&topic_filter)?;
        }

        // Parse Subscription Options
        let options = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
        offset += 1;

        // MQTT 5.0 Protocol Compliance: Reserved bits must be 0
        #[cfg(feature = "strict-protocol-compliance")]
        if options & 0xC0 != 0 {
            return Err(ParseError::ParseError(
                "SUBSCRIBE Subscription Options reserved bits must be 0".to_string(),
            ));
        }

        let (qos, no_local, retain_as_published, retain_handling) =
            Self::decode_subscription_options(options);
            
        // MQTT 5.0 Protocol Compliance: No Local flag validation for Shared Subscriptions
        #[cfg(feature = "strict-protocol-compliance")]
        if no_local && topic_filter.starts_with("$share/") {
            return Err(ParseError::ParseError(
                "SUBSCRIBE No Local flag must not be set on Shared Subscriptions".to_string(),
            ));
        }

        let subscription = TopicSubscription::new(
            topic_filter,
            qos,
            no_local,
            retain_as_published,
            retain_handling,
        );

        Ok((subscription, offset))
    }
}

/// Represents the SUBSCRIBE packet in MQTT v5.0.
/// SUBSCRIBE is used by clients to subscribe to one or more topics.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttSubscribe {
    pub packet_id: u16,
    pub subscriptions: Vec<TopicSubscription>,
    pub properties: Vec<Property>,
}

impl MqttSubscribe {
    /// Creates a new `MqttSubscribe` packet.
    ///
    /// # Arguments
    /// * `packet_id` - The packet identifier (must be > 0)
    /// * `subscriptions` - List of topic subscriptions
    /// * `properties` - Optional properties
    pub fn new(
        packet_id: u16,
        subscriptions: Vec<TopicSubscription>,
        properties: Vec<Property>,
    ) -> Self {
        Self {
            packet_id,
            subscriptions,
            properties,
        }
    }

    /// Creates a simple SUBSCRIBE packet with basic topic subscriptions.
    pub fn new_simple(packet_id: u16, subscriptions: Vec<TopicSubscription>) -> Self {
        Self::new(packet_id, subscriptions, Vec::new())
    }

    /// Adds a topic subscription to this SUBSCRIBE packet.
    pub fn add_subscription(&mut self, subscription: TopicSubscription) {
        self.subscriptions.push(subscription);
    }
}

impl MqttControlPacket for MqttSubscribe {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::SUBSCRIBE as u8
    }

    // MQTT 5.0: 3.8.1.1
    fn flags(&self) -> u8 {
        0x02 // Fixed header flags: bit 1 = 1, others = 0 (reserved)
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.8.2 SUBSCRIBE Variable Header
        // Packet Identifier - always present and must be > 0
        #[cfg(feature = "strict-protocol-compliance")]
        if self.packet_id == 0 {
            return Err(ParseError::ParseError(
                "SUBSCRIBE packet_id must be > 0".to_string(),
            ));
        }
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.8.2.1 SUBSCRIBE Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.8.3 SUBSCRIBE Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // The payload must contain at least one topic filter
        #[cfg(feature = "strict-protocol-compliance")]
        if self.subscriptions.is_empty() {
            return Err(ParseError::ParseError(
                "SUBSCRIBE payload must contain at least one subscription".to_string(),
            ));
        }

        // Encode each subscription
        for subscription in &self.subscriptions {
            bytes.extend(subscription.to_bytes()?);
        }

        Ok(bytes)
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::SUBSCRIBE as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // MQTT 5.0: 3.8.1.1 Fixed Header Flags validation
        #[cfg(feature = "strict-protocol-compliance")]
        {
            let flags = buffer[0] & 0x0F;
            if flags != 0x02 {
                return Err(ParseError::ParseError(
                    "SUBSCRIBE fixed header flags must be 0010".to_string(),
                ));
            }
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.8.2 SUBSCRIBE Variable Header
        // Packet Identifier - always present and must be > 0
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        #[cfg(feature = "strict-protocol-compliance")]
        if packet_id == 0 {
            return Err(ParseError::ParseError(
                "SUBSCRIBE packet_id must be > 0".to_string(),
            ));
        }

        // MQTT 5.0: 3.8.2.1 SUBSCRIBE Properties
        let (properties, consumed) = parse_properties_hdr(
            buffer
                .get(offset..total_len)
                .ok_or(ParseError::BufferTooShort)?,
        )?;
        offset += consumed;

        // MQTT 5.0: 3.8.3 SUBSCRIBE Payload
        let mut subscriptions = Vec::new();

        while offset < total_len {
            let (subscription, consumed) = TopicSubscription::from_bytes(&buffer[offset..])?;
            subscriptions.push(subscription);
            offset += consumed;
        }

        if subscriptions.is_empty() {
            #[cfg(feature = "strict-protocol-compliance")]
            return Err(ParseError::ParseError(
                "SUBSCRIBE payload must contain at least one subscription".to_string(),
            ));
        }

        if offset != total_len {
            return Err(ParseError::InternalError(format!(
                "Inconsistent offset {} != total: {}",
                offset, total_len
            )));
        }

        let subscribe = MqttSubscribe {
            packet_id,
            subscriptions,
            properties,
        };

        Ok(ParseOk::Packet(MqttPacket::Subscribe(subscribe), offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_subscription_simple() {
        // Test simple subscription with QoS 1
        let subscription = TopicSubscription::new_simple("test/topic".to_string(), 1);
        assert_eq!(subscription.topic_filter, "test/topic");
        assert_eq!(subscription.qos, 1);
        assert!(!subscription.no_local);
        assert!(!subscription.retain_as_published);
        assert_eq!(subscription.retain_handling, 0);

        // Test encoding/decoding
        let bytes = subscription.to_bytes().unwrap();
        let (decoded, consumed) = TopicSubscription::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, subscription);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn test_topic_subscription_options() {
        // Test subscription with all options
        let subscription = TopicSubscription::new(
            "advanced/topic".to_string(),
            2,    // QoS 2
            true, // No Local
            true, // Retain As Published
            1,    // Retain Handling = 1
        );

        let bytes = subscription.to_bytes().unwrap();
        let (decoded, _) = TopicSubscription::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, subscription);
    }

    #[test]
    fn test_subscription_options_encoding() {
        let subscription = TopicSubscription::new(
            "test".to_string(),
            2,     // QoS 2 (bits 0-1 = 10)
            true,  // No Local (bit 2 = 1)
            false, // Retain As Published (bit 3 = 0)
            2,     // Retain Handling = 2 (bits 4-5 = 10)
        );

        let options = subscription.encode_subscription_options();
        // Expected: 00100110 = 0x26
        // Bits 0-1: 10 (QoS 2)
        // Bit 2: 1 (No Local)
        // Bit 3: 0 (Not Retain As Published)
        // Bits 4-5: 10 (Retain Handling 2)
        // Bits 6-7: 00 (Reserved)
        assert_eq!(options, 0x26);

        let (qos, no_local, retain_as_published, retain_handling) =
            TopicSubscription::decode_subscription_options(options);
        assert_eq!(qos, 2);
        assert!(no_local);
        assert!(!retain_as_published);
        assert_eq!(retain_handling, 2);
    }

    #[test]
    fn test_subscribe_minimal() {
        // Test minimal SUBSCRIBE packet
        let subscriptions = vec![TopicSubscription::new_simple("test/topic".to_string(), 1)];
        let subscribe = MqttSubscribe::new_simple(1234, subscriptions);

        let bytes = subscribe.to_bytes().unwrap();

        // Verify packet structure
        assert_eq!(bytes[0], 0x82); // SUBSCRIBE (8 << 4) | 0x02 flags

        // Test round-trip parsing
        match MqttSubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe(parsed_subscribe), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_subscribe.packet_id, 1234);
                assert_eq!(parsed_subscribe.subscriptions.len(), 1);
                assert_eq!(parsed_subscribe.subscriptions[0].topic_filter, "test/topic");
                assert_eq!(parsed_subscribe.subscriptions[0].qos, 1);
                assert!(parsed_subscribe.properties.is_empty());
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_subscribe_multiple_topics() {
        // Test SUBSCRIBE with multiple topic subscriptions
        let subscriptions = vec![
            TopicSubscription::new_simple("home/temperature".to_string(), 0),
            TopicSubscription::new_simple("home/humidity".to_string(), 1),
            TopicSubscription::new("office/+/status".to_string(), 2, true, false, 1),
        ];
        let subscribe = MqttSubscribe::new_simple(5678, subscriptions);

        let bytes = subscribe.to_bytes().unwrap();

        match MqttSubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe(parsed_subscribe), _) => {
                assert_eq!(parsed_subscribe.packet_id, 5678);
                assert_eq!(parsed_subscribe.subscriptions.len(), 3);

                // Check first subscription
                assert_eq!(
                    parsed_subscribe.subscriptions[0].topic_filter,
                    "home/temperature"
                );
                assert_eq!(parsed_subscribe.subscriptions[0].qos, 0);

                // Check second subscription
                assert_eq!(
                    parsed_subscribe.subscriptions[1].topic_filter,
                    "home/humidity"
                );
                assert_eq!(parsed_subscribe.subscriptions[1].qos, 1);

                // Check third subscription with options
                assert_eq!(
                    parsed_subscribe.subscriptions[2].topic_filter,
                    "office/+/status"
                );
                assert_eq!(parsed_subscribe.subscriptions[2].qos, 2);
                assert!(parsed_subscribe.subscriptions[2].no_local);
                assert!(!parsed_subscribe.subscriptions[2].retain_as_published);
                assert_eq!(parsed_subscribe.subscriptions[2].retain_handling, 1);
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_subscribe_with_properties() {
        // Test SUBSCRIBE with properties
        let subscriptions = vec![TopicSubscription::new_simple("sensor/data".to_string(), 1)];
        let properties = vec![
            Property::SubscriptionIdentifier(42),
            Property::UserProperty("client".to_string(), "sensor_client_1".to_string()),
        ];
        let subscribe = MqttSubscribe::new(9999, subscriptions, properties);

        let bytes = subscribe.to_bytes().unwrap();

        match MqttSubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe(parsed_subscribe), _) => {
                assert_eq!(parsed_subscribe.packet_id, 9999);
                assert_eq!(parsed_subscribe.subscriptions.len(), 1);
                assert_eq!(parsed_subscribe.properties.len(), 2);

                // Verify subscription
                assert_eq!(
                    parsed_subscribe.subscriptions[0].topic_filter,
                    "sensor/data"
                );
                assert_eq!(parsed_subscribe.subscriptions[0].qos, 1);
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_subscribe_error_conditions() {
        // Test SUBSCRIBE with packet_id = 0 (invalid)
        let subscriptions = vec![TopicSubscription::new_simple("test/topic".to_string(), 1)];
        let subscribe = MqttSubscribe::new_simple(0, subscriptions);

        assert!(subscribe.to_bytes().is_err());

        // Test SUBSCRIBE with no subscriptions (invalid)
        let subscribe = MqttSubscribe::new_simple(1, Vec::new());
        assert!(subscribe.to_bytes().is_err());
    }

    #[test]
    fn test_subscribe_wildcard_topics() {
        // Test SUBSCRIBE with wildcard topic filters
        let subscriptions = vec![
            TopicSubscription::new_simple("home/+/temperature".to_string(), 1),
            TopicSubscription::new_simple("sensors/#".to_string(), 0),
            TopicSubscription::new_simple("+/+/status".to_string(), 2),
        ];
        let subscribe = MqttSubscribe::new_simple(1111, subscriptions);

        let bytes = subscribe.to_bytes().unwrap();

        match MqttSubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe(parsed_subscribe), _) => {
                assert_eq!(parsed_subscribe.subscriptions.len(), 3);
                assert_eq!(
                    parsed_subscribe.subscriptions[0].topic_filter,
                    "home/+/temperature"
                );
                assert_eq!(parsed_subscribe.subscriptions[1].topic_filter, "sensors/#");
                assert_eq!(parsed_subscribe.subscriptions[2].topic_filter, "+/+/status");
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_subscribe_fixed_header_flags() {
        // Test that SUBSCRIBE packets have correct fixed header flags (0x02)
        let subscriptions = vec![TopicSubscription::new_simple("test".to_string(), 0)];
        let subscribe = MqttSubscribe::new_simple(1, subscriptions);

        let bytes = subscribe.to_bytes().unwrap();

        // First byte should be 0x82 (SUBSCRIBE type 8 << 4 | flags 0x02)
        assert_eq!(bytes[0], 0x82);

        // Test that parsing validates the flags
        let mut invalid_bytes = bytes.clone();
        invalid_bytes[0] = 0x80; // Wrong flags (should be 0x02)

        assert!(MqttSubscribe::from_bytes(&invalid_bytes).is_err());
    }

    #[test]
    fn test_qos_values() {
        // Test all valid QoS values
        for qos in 0..=2 {
            let subscription = TopicSubscription::new_simple(format!("test/qos{}", qos), qos);
            let subscriptions = vec![subscription];
            let subscribe = MqttSubscribe::new_simple(1, subscriptions);

            let bytes = subscribe.to_bytes().unwrap();

            match MqttSubscribe::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::Subscribe(parsed_subscribe), _) => {
                    assert_eq!(parsed_subscribe.subscriptions[0].qos, qos);
                }
                _ => panic!("Expected SUBSCRIBE packet for QoS {}", qos),
            }
        }
    }

    #[test]
    fn test_subscribe_roundtrip_comprehensive() {
        // Comprehensive roundtrip test with all features
        let subscriptions = vec![
            TopicSubscription::new("simple/topic".to_string(), 0, false, false, 0),
            TopicSubscription::new("advanced/topic".to_string(), 2, true, true, 2),
            TopicSubscription::new("wildcard/+/topic".to_string(), 1, false, true, 1),
        ];
        let properties = vec![
            Property::SubscriptionIdentifier(12345),
            Property::UserProperty("client_type".to_string(), "test_client".to_string()),
            Property::ReasonString("Test subscription".to_string()),
        ];
        let original_subscribe = MqttSubscribe::new(0x7FFF, subscriptions, properties);

        let bytes = original_subscribe.to_bytes().unwrap();

        match MqttSubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe(parsed_subscribe), consumed) => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(parsed_subscribe.packet_id, original_subscribe.packet_id);
                assert_eq!(
                    parsed_subscribe.subscriptions.len(),
                    original_subscribe.subscriptions.len()
                );
                assert_eq!(
                    parsed_subscribe.properties.len(),
                    original_subscribe.properties.len()
                );

                // Verify each subscription is identical
                for (original, parsed) in original_subscribe
                    .subscriptions
                    .iter()
                    .zip(parsed_subscribe.subscriptions.iter())
                {
                    assert_eq!(original, parsed);
                }
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_parse_subscribe() {
        let packet_bytes: [u8; 16] = [
            0x82, 0x0e, 0x00, 0x02, 0x00, 0x00, 0x08, 0x74, 0x65, 0x73, 0x74, 0x2f, 0x31, 0x2f,
            0x61, 0x00,
        ];

        let result = MqttSubscribe::from_bytes(&packet_bytes);
        match result {
            Ok(ParseOk::Packet(packet, consumed)) => {
                assert_eq!(consumed, packet_bytes.len());
                match packet {
                    MqttPacket::Subscribe(subscribe) => {
                        assert_eq!(subscribe.packet_id, 2);
                        assert_eq!(subscribe.subscriptions.len(), 1);
                        assert_eq!(subscribe.subscriptions[0].topic_filter, "test/1/a");
                        assert_eq!(subscribe.subscriptions[0].qos, 0);
                    }
                    _ => panic!("Expected Subscribe"),
                }
            }
            Ok(ParseOk::Continue(hint, _parsed)) => {
                panic!("Expected ParseResult::Continue, got {:?}", hint)
            }
            Err(e) => panic!("Error: {:?}", e),

            _ => panic!("Expected ParseOk::Packet"),
        }
    }

    #[test]
    fn test_subscribe_invalid_subscription_options_reserved_bits() {
        // [MQTT-3.8.3-5]
        let sub = MqttSubscribe::new_simple(1, vec![TopicSubscription::new_simple("a/b".to_string(), 1)]);
        let mut bytes = sub.to_bytes().unwrap();

        // In this simple case, the options byte is the last byte.
        let last_byte_index = bytes.len() - 1;
        bytes[last_byte_index] |= 0b11000000; // set bits 6 and 7

        let result = MqttSubscribe::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_subscribe_no_local_on_shared_subscription_is_invalid() {
        // [MQTT-3.8.3-4]
        let sub = MqttSubscribe::new_simple(1, vec![TopicSubscription::new("$share/g/t".to_string(), 1, true, false, 0)]);
        let bytes = sub.to_bytes().unwrap();
        let result = MqttSubscribe::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_subscribe_invalid_topic_filter_wildcard_not_last() {
        // [MQTT-4.7.1-1]
        let sub = MqttSubscribe::new_simple(1, vec![TopicSubscription::new_simple("a/#/b".to_string(), 1)]);
        let bytes = sub.to_bytes().unwrap();
        let result = MqttSubscribe::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_subscribe_invalid_topic_filter_wildcard_not_level() {
        // [MQTT-4.7.1-2]
        let sub = MqttSubscribe::new_simple(1, vec![TopicSubscription::new_simple("a/b+".to_string(), 1)]);
        let bytes = sub.to_bytes().unwrap();
        let result = MqttSubscribe::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_subscribe_empty_topic_filter_is_invalid() {
        // [MQTT-4.7.3-1]
        let sub = MqttSubscribe::new_simple(1, vec![TopicSubscription::new_simple("".to_string(), 1)]);
        let bytes = sub.to_bytes().unwrap();
        let result = MqttSubscribe::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_subscribe_invalid_shared_subscription_topic() {
        // [MQTT-4.8.2-1] & [MQTT-4.8.2-2]
        let topics = vec![
            "$share//t",   // empty share name
            "$share/g/",   // empty filter
            "$share/g+/t", // wildcard in share name
            "$share/g#/t", // wildcard in share name
        ];
        for topic in topics {
            let sub = MqttSubscribe::new_simple(1, vec![TopicSubscription::new_simple(topic.to_string(), 1)]);
            let bytes = sub.to_bytes().unwrap();
            let result = MqttSubscribe::from_bytes(&bytes);
            assert!(matches!(result, Err(ParseError::ParseError(_))));
        }
    }
}
