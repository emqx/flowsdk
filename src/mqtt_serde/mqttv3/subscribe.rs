use serde::{Deserialize, Serialize};

use crate::mqtt_serde::base_data::{TwoByteInteger, Utf8String};
use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{
    packet_type, parse_remaining_length, parse_utf8_string, ParseError, ParseOk,
};

/// Represents a subscription to a single topic in a SUBSCRIBE packet.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct SubscriptionTopic {
    pub topic_filter: String,
    pub qos: u8,
}

/// Represents the SUBSCRIBE packet in MQTT v3.1.1.
///
/// The SUBSCRIBE packet is sent from the Client to the Server to create one or more Subscriptions.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttSubscribe {
    /// The Packet Identifier is used to correlate the SUBSCRIBE packet with a SUBACK packet.
    pub message_id: u16,
    /// The list of subscriptions for the session.
    pub subscriptions: Vec<SubscriptionTopic>,
}

impl MqttSubscribe {
    /// Creates a new `MqttSubscribe` packet.
    pub fn new(message_id: u16, subscriptions: Vec<SubscriptionTopic>) -> Self {
        Self {
            message_id,
            subscriptions,
        }
    }
}

impl MqttControlPacket for MqttSubscribe {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::SUBSCRIBE as u8
    }

    fn flags(&self) -> u8 {
        // For SUBSCRIBE, bits 3,2,1,0 MUST be 0,0,1,0
        0x02
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        Ok(TwoByteInteger::encode(self.message_id).to_vec())
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        let mut payload = Vec::new();
        for sub in &self.subscriptions {
            payload.extend(Utf8String::encode(&sub.topic_filter));
            // QoS must be 0, 1, or 2.
            if sub.qos > 2 {
                return Err(ParseError::ParseError("Invalid QoS level".to_string()));
            }
            payload.push(sub.qos);
        }
        Ok(payload)
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::SUBSCRIBE as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // For SUBSCRIBE, bits 3,2,1,0 of fixed header MUST be 0,0,1,0.
        let flags = buffer[0] & 0x0F;
        if flags != 0x02 {
            return Err(ParseError::ParseError(
                "SUBSCRIBE packet has invalid fixed header flags".to_string(),
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
                "SUBSCRIBE packet must have a 2-byte message identifier".to_string(),
            ));
        }
        let message_id = u16::from_be_bytes([buffer[offset], buffer[offset + 1]]);
        offset += 2;

        // Payload: Subscriptions
        let mut subscriptions = Vec::new();
        let payload_end = total_len;
        while offset < payload_end {
            let (topic_filter, consumed) = parse_utf8_string(&buffer[offset..payload_end])?;
            offset += consumed;

            if offset >= payload_end {
                return Err(ParseError::ParseError(
                    "SUBSCRIBE payload is missing QoS byte".to_string(),
                ));
            }
            let qos = buffer[offset];
            if qos > 2 {
                return Err(ParseError::ParseError(
                    "Invalid QoS level in payload".to_string(),
                ));
            }
            offset += 1;

            subscriptions.push(SubscriptionTopic { topic_filter, qos });
        }

        if offset != total_len {
            return Err(ParseError::InternalError(
                "Failed to consume entire SUBSCRIBE packet".to_string(),
            ));
        }

        Ok(ParseOk::Packet(
            MqttPacket::Subscribe3(MqttSubscribe::new(message_id, subscriptions)),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscribe_serialization_single() {
        let subscribe = MqttSubscribe::new(
            1,
            vec![SubscriptionTopic {
                topic_filter: "a/b".to_string(),
                qos: 1,
            }],
        );
        let bytes = subscribe.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0x82, // Packet type and flags
                6,    // Remaining length (2 msg id + 3 topic + 1 qos)
                0x00, 0x01, // Message ID
                0x00, 0x03, b'a', b'/', b'b', // Topic
                0x01, // QoS
            ]
        );
    }

    #[test]
    fn test_subscribe_deserialization_multiple() {
        let bytes = vec![
            0x82, 11, 0x00, 0x0A, 0x00, 0x03, b'a', b'/', b'b', 0x01, 0x00, 0x01, b'c', 0x02,
        ];
        match MqttSubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe3(subscribe), consumed) => {
                assert_eq!(consumed, 13);
                assert_eq!(subscribe.message_id, 10);
                assert_eq!(subscribe.subscriptions.len(), 2);
                assert_eq!(subscribe.subscriptions[0].topic_filter, "a/b");
                assert_eq!(subscribe.subscriptions[0].qos, 1);
                assert_eq!(subscribe.subscriptions[1].topic_filter, "c");
                assert_eq!(subscribe.subscriptions[1].qos, 2);
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_subscribe_roundtrip() {
        let original = MqttSubscribe::new(
            123,
            vec![
                SubscriptionTopic {
                    topic_filter: "test/topic".to_string(),
                    qos: 0,
                },
                SubscriptionTopic {
                    topic_filter: "another/topic".to_string(),
                    qos: 2,
                },
            ],
        );
        let bytes = original.to_bytes().unwrap();
        match MqttSubscribe::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_subscribe_invalid_flags() {
        let bytes = vec![0x80, 0x04, 0x00, 0x01, 0x00, 0x00]; // Invalid flags (0)
        match MqttSubscribe::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {}
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_subscribe_invalid_qos_serialize() {
        let subscribe = MqttSubscribe::new(
            1,
            vec![SubscriptionTopic {
                topic_filter: "a/b".to_string(),
                qos: 3,
            }],
        );
        assert!(subscribe.to_bytes().is_err());
    }

    #[test]
    fn test_subscribe_invalid_qos_deserialize() {
        let bytes = vec![0x82, 0x06, 0x00, 0x01, 0x00, 0x03, b'a', b'/', b'b', 0x03];
        match MqttSubscribe::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("Invalid QoS level") => {}
            _ => panic!("Expected ParseError with QoS message"),
        }
    }

    #[test]
    fn test_subscribe_payload_missing_qos() {
        let bytes = vec![0x82, 0x05, 0x00, 0x01, 0x00, 0x03, b'a', b'/', b'b']; // Missing QoS byte
        match MqttSubscribe::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("missing QoS byte") => {}
            other => panic!(
                "Expected ParseError with missing QoS message, got {:?}",
                other
            ),
        }
    }
}
