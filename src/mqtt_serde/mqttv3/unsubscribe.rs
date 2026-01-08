// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::base_data::{TwoByteInteger, Utf8String};
use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{
    packet_type, parse_remaining_length, parse_utf8_string, ParseError, ParseOk,
};

/// Represents the UNSUBSCRIBE packet in MQTT v3.1.1.
///
/// The UNSUBSCRIBE packet is sent by the Client to the Server to unsubscribe from named topics.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttUnsubscribe {
    /// The Packet Identifier is used to correlate the UNSUBSCRIBE packet with an UNSUBACK packet.
    pub message_id: u16,
    /// The list of topic filters to which the Client wants to unsubscribe.
    pub topic_filters: Vec<String>,
}

impl MqttUnsubscribe {
    /// Creates a new `MqttUnsubscribe` packet.
    pub fn new(message_id: u16, topic_filters: Vec<String>) -> Self {
        Self {
            message_id,
            topic_filters,
        }
    }
}

impl MqttControlPacket for MqttUnsubscribe {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::UNSUBSCRIBE as u8
    }

    fn flags(&self) -> u8 {
        // For UNSUBSCRIBE, bits 3,2,1,0 MUST be 0,0,1,0
        0x02
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(TwoByteInteger::encode(self.message_id).to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        let mut payload = Vec::new();
        for topic in &self.topic_filters {
            payload.extend(Utf8String::encode(topic));
        }
        Ok(payload)
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::UNSUBSCRIBE as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // For UNSUBSCRIBE, bits 3,2,1,0 of fixed header MUST be 0,0,1,0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x02 {
            return Err(ParseError::ParseError(
                "UNSUBSCRIBE packet has invalid fixed header flags".to_string(),
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
                "UNSUBSCRIBE packet must have a 2-byte message identifier".to_string(),
            ));
        }
        let message_id = u16::from_be_bytes([buffer[offset], buffer[offset + 1]]);
        offset += 2;

        // Payload: Topic Filters
        let mut topic_filters = Vec::new();
        let payload_end = total_len;
        while offset < payload_end {
            let (topic, consumed) = parse_utf8_string(&buffer[offset..payload_end])?;
            topic_filters.push(topic);
            offset += consumed;
        }

        if offset != total_len {
            return Err(ParseError::InternalError(
                "Failed to consume entire UNSUBSCRIBE packet".to_string(),
            ));
        }

        Ok(ParseOk::Packet(
            MqttPacket::Unsubscribe3(MqttUnsubscribe::new(message_id, topic_filters)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsubscribe_new() {
        let unsubscribe = MqttUnsubscribe::new(123, vec!["a/b".to_string()]);
        assert_eq!(unsubscribe.message_id, 123);
        assert_eq!(unsubscribe.topic_filters, vec!["a/b".to_string()]);
    }

    #[test]
    fn test_unsubscribe_serialization_single_topic() {
        let unsubscribe = MqttUnsubscribe::new(10, vec!["test/topic".to_string()]);
        let bytes = unsubscribe.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0xA2, // Packet type and flags
                14,   // Remaining length (2 for msg id + 12 for topic)
                0x00, 0x0A, // Message ID
                0x00, 0x0A, // Topic length
                b't', b'e', b's', b't', b'/', b't', b'o', b'p', b'i', b'c', // Topic
            ]
        );
    }

    #[test]
    fn test_unsubscribe_serialization_multiple_topics() {
        let unsubscribe = MqttUnsubscribe::new(11, vec!["a".to_string(), "b".to_string()]);
        let bytes = unsubscribe.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0xA2, // Packet type and flags
                8,    // Remaining length (2 for msg id + 3 for 'a' + 3 for 'b')
                0x00, 0x0B, // Message ID
                0x00, 0x01, b'a', // Topic 1
                0x00, 0x01, b'b', // Topic 2
            ]
        );
    }

    #[test]
    fn test_unsubscribe_deserialization() {
        let bytes = vec![
            0xA2, 14, 0x00, 0x0A, 0x00, 0x0A, b't', b'e', b's', b't', b'/', b't', b'o', b'p', b'i',
            b'c',
        ];
        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe3(unsubscribe), consumed) => {
                assert_eq!(consumed, 16);
                assert_eq!(unsubscribe.message_id, 10);
                assert_eq!(unsubscribe.topic_filters, vec!["test/topic".to_string()]);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_roundtrip() {
        let original = MqttUnsubscribe::new(99, vec!["a/b".to_string(), "c/d/e".to_string()]);
        let bytes = original.to_bytes().unwrap();
        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscribe_invalid_flags() {
        let bytes = vec![0xA0, 0x03, 0x00, 0x01, 0x00]; // Invalid flags (0)
        match MqttUnsubscribe::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_unsubscribe_no_payload() {
        let bytes = vec![0xA2, 0x02, 0x00, 0x01]; // No payload
        match MqttUnsubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe3(unsubscribe), _) => {
                assert_eq!(unsubscribe.message_id, 1);
                assert!(unsubscribe.topic_filters.is_empty());
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }
}
