use serde::{Deserialize, Serialize};

use crate::mqtt_serde::base_data::{TwoByteInteger, Utf8String};
use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{
    packet_type, parse_remaining_length, parse_utf8_string, ParseError, ParseOk,
};

/// Represents the PUBLISH packet in MQTT v3.1.1.
///
/// A PUBLISH packet is sent from a Client to a Server or from a Server to a Client
/// to transport an application message.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPublish {
    pub dup: bool,
    pub qos: u8,
    pub retain: bool,
    pub topic_name: String,
    /// Packet Identifier is only present for QoS levels 1 and 2.
    pub message_id: Option<u16>,
    pub payload: Vec<u8>,
}

impl MqttPublish {
    /// Creates a new `MqttPublish` packet.
    pub fn new(
        topic_name: String,
        qos: u8,
        payload: Vec<u8>,
        message_id: Option<u16>,
        retain: bool,
        dup: bool,
    ) -> Self {
        Self {
            dup,
            qos,
            retain,
            topic_name,
            message_id,
            payload,
        }
    }
}

impl MqttControlPacket for MqttPublish {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBLISH as u8
    }

    fn flags(&self) -> u8 {
        ((self.dup as u8) << 3) | (self.qos << 1) | (self.retain as u8)
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut vh = Utf8String::encode(&self.topic_name);
        if self.qos > 0 {
            let msg_id = self.message_id.ok_or_else(|| {
                ParseError::ParseError("Message ID is required for QoS > 0".to_string())
            })?;
            vh.extend_from_slice(&TwoByteInteger::encode(msg_id));
        }
        Ok(vh)
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        Ok(self.payload.clone())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBLISH as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let flags = buffer[0] & 0x0F;
        let dup = (flags & 0x08) > 0;
        let qos = (flags & 0x06) >> 1;
        let retain = (flags & 0x01) > 0;

        if qos > 2 {
            return Err(ParseError::ParseError("Invalid QoS level".to_string()));
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Variable Header
        let (topic_name, consumed) = parse_utf8_string(&buffer[offset..total_len])?;
        offset += consumed;

        let message_id = if qos > 0 {
            if offset + 2 > total_len {
                return Err(ParseError::ParseError(
                    "Missing message ID for QoS > 0".to_string(),
                ));
            }
            let id = u16::from_be_bytes([buffer[offset], buffer[offset + 1]]);
            offset += 2;
            Some(id)
        } else {
            None
        };

        // Payload
        let payload = buffer[offset..total_len].to_vec();

        Ok(ParseOk::Packet(
            MqttPacket::Publish3(MqttPublish {
                dup,
                qos,
                retain,
                topic_name,
                message_id,
                payload,
            }),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_publish_qos0_serialization() {
        let publish = MqttPublish::new("a/b".to_string(), 0, vec![1, 2, 3], None, false, false);
        let bytes = publish.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0x30, // Type + flags
                8,    // Remaining length (3 topic + 5 payload)
                0x00, 0x03, b'a', b'/', b'b', // Topic
                1, 2, 3, // Payload
            ]
        );
    }

    #[test]
    fn test_publish_qos1_serialization() {
        let publish = MqttPublish::new("a/b".to_string(), 1, vec![1, 2, 3], Some(123), true, true);
        let bytes = publish.to_bytes().unwrap();
        assert_eq!(
            bytes,
            vec![
                0x3B, // Type + DUP, QoS1, RETAIN
                10,   // Remaining length (3 topic + 2 msg id + 5 payload)
                0x00, 0x03, b'a', b'/', b'b', // Topic
                0x00, 0x7B, // Message ID
                1, 2, 3, // Payload
            ]
        );
    }

    #[test]
    fn test_publish_qos0_deserialization() {
        let bytes = vec![0x30, 8, 0x00, 0x03, b'a', b'/', b'b', 1, 2, 3];
        match MqttPublish::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Publish3(p), len) => {
                assert_eq!(len, 10);
                assert!(!p.dup);
                assert_eq!(p.qos, 0);
                assert!(!p.retain);
                assert_eq!(p.topic_name, "a/b");
                assert_eq!(p.message_id, None);
                assert_eq!(p.payload, vec![1, 2, 3]);
            }
            _ => panic!("Deserialization failed"),
        }
    }

    #[test]
    fn test_publish_qos1_deserialization() {
        let bytes = vec![0x3B, 10, 0x00, 0x03, b'a', b'/', b'b', 0x00, 0x7B, 1, 2, 3];
        match MqttPublish::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Publish3(p), len) => {
                assert_eq!(len, 12);
                assert!(p.dup);
                assert_eq!(p.qos, 1);
                assert!(p.retain);
                assert_eq!(p.topic_name, "a/b");
                assert_eq!(p.message_id, Some(123));
                assert_eq!(p.payload, vec![1, 2, 3]);
            }
            _ => panic!("Deserialization failed"),
        }
    }

    #[test]
    fn test_publish_roundtrip_qos2() {
        let original = MqttPublish::new(
            "qos/2/topic".to_string(),
            2,
            b"hello qos 2".to_vec(),
            Some(54321),
            false,
            true,
        );
        let bytes = original.to_bytes().unwrap();
        match MqttPublish::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Publish3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected PUBLISH packet"),
        }
    }

    #[test]
    fn test_publish_invalid_qos() {
        let bytes = vec![0x3E, 0x01, 0x00]; // QoS 3 is invalid
        assert!(MqttPublish::from_bytes(&bytes).is_err());
    }

    #[test]
    fn test_publish_missing_message_id() {
        // QoS 1 but no message ID
        let bytes = vec![0x32, 5, 0x00, 0x03, b'a', b'/', b'b'];
        assert!(MqttPublish::from_bytes(&bytes).is_err());
    }
}
