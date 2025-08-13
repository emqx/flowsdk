use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{encode_properities_hdr, Property};
use crate::mqtt_serde::parser;
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, parse_topic_name, ParseError, ParseOk,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPublish {
    pub topic_name: String,
    pub qos: u8,
    pub dup: bool,
    pub retain: bool,
    pub packet_id: Option<u16>,
    pub payload: Vec<u8>,
    pub properties: Vec<Property>,
}

impl MqttPublish {
    pub fn new(
        qos: u8,
        topic_name: String,
        packet_id: Option<u16>,
        payload: Vec<u8>,
        retain: bool,
        dup: bool,
    ) -> Self {
        MqttPublish {
            topic_name,
            packet_id,
            payload,
            qos,
            dup,
            retain,
            properties: Vec::new(),
        }
    }

    pub fn properties(&self) -> Vec<u8> {
        //@TODO: implement properties
        Vec::new()
    }

    fn qos(flags: u8) -> u8 {
        (flags & 0x06) >> 1
    }
}

impl MqttControlPacket for MqttPublish {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBLISH as u8
    }

    fn flags(&self) -> u8 {
        let mut val: u8 = self.qos << 1;
        if self.dup {
            val |= 0x08;
        }
        if self.retain {
            val |= 0x01;
        }
        return val;
    }

    fn variable_header(&self) -> Result<Vec<u8>, parser::ParseError> {
        let mut bytes = Vec::new();
        // MQTT 5.0: 3.3.2.1 Topic Name
        bytes.extend(crate::mqtt_serde::encode_utf8_string(&self.topic_name)?);

        // MQTT 5.0: 3.3.2.2 Packet ID
        // If QoS > 0, we need a packet identifier
        if self.qos > 0 {
            if let Some(packet_id) = self.packet_id {
                bytes.extend(packet_id.to_be_bytes());
            } else {
                // @TODO need a parse error for that
                return Err(ParseError::ParseError(
                    "QoS > 0 requires a packet identifier".to_string(),
                ));
            }
        }
        // MQTT 5.0: 3.3.2.3 Publish Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);
        Ok(bytes)
    }

    fn payload(&self) -> Result<Vec<u8>, parser::ParseError> {
        Ok(self.payload.clone())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let first_byte = *buffer.get(0).ok_or(ParseError::BufferTooShort)?;
        let packet_type = packet_type(buffer)?;

        // MQTT 5.0: 3.3.1 PUBLISH Fixed Header
        if packet_type != ControlPacketType::PUBLISH as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        // MQTT 5.0: 3.3.1.1 DUP flag
        let dup = (first_byte & 0x8) != 0;

        // MQTT 5.0: 3.3.1.2 QoS
        let qos = Self::qos(first_byte);
        
        // MQTT 5.0 Protocol Compliance: QoS validation
        #[cfg(feature = "strict-protocol-compliance")]
        if qos == 3 {
            return Err(ParseError::ParseError(
                "PUBLISH QoS bits must not be set to 11 (invalid QoS 3)".to_string(),
            ));
        }

        // MQTT 5.0: 3.3.1.3 Retain
        let retain = (first_byte & 0x1) != 0;
        
        // MQTT 5.0 Protocol Compliance: DUP flag validation for QoS 0
        #[cfg(feature = "strict-protocol-compliance")]
        if qos == 0 && dup {
            return Err(ParseError::ParseError(
                "PUBLISH DUP flag must be 0 for QoS 0 messages".to_string(),
            ));
        }

        // MQTT 5.0: 3.3.1.4 Remaining Length
        let (msgsize, consumed) = parse_remaining_length(&buffer[1..])?;

        let mut offset = 1 + consumed;
        let packet_size = msgsize + offset;

        if packet_size > buffer.len() {
            return Ok(ParseOk::Continue(packet_size - buffer.len(), 0));
        }

        // MQTT 5.0: 3.3.2 PUBLISH VARIABLE HEADER
        // MQTT 5.0: 3.3.2.1 Topic Name
        let (topic_name, consumed) =
            match parse_topic_name(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)? {
                ParseOk::TopicName(name, consumed) => (name, consumed),
                _ => return Err(ParseError::ParseError("Expected TopicName".to_string())),
            };
        offset += consumed;
        
        // MQTT 5.0 Protocol Compliance: Topic name must not contain wildcards
        #[cfg(feature = "strict-protocol-compliance")]
        if topic_name.contains('#') || topic_name.contains('+') {
            return Err(ParseError::ParseError(
                "PUBLISH Topic Name must not contain wildcard characters (# or +)".to_string(),
            ));
        }

        // MQTT 5.0: 3.3.2.2 Packet ID
        let packet_id_opt = if qos > 0 {
            let (packet_id, consumed) =
                parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
            offset += consumed;
            Some(packet_id)
        } else {
            None
        };

        // MQTT 5.0: 3.3.2.3 Publish Properties
        let (properties, consumed) =
            crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
                buffer.get(offset..).ok_or(ParseError::BufferTooShort)?,
            )?;
        offset += consumed;

        // MQTT 5.0: 3.3.3 PUBLISH Payload
        let payload_len = packet_size - offset;
        let payload = buffer
            .get(offset..offset + payload_len)
            .ok_or(ParseError::BufferTooShort)?
            .to_vec();
        offset += payload_len;

        if offset != packet_size {
            return Err(ParseError::InvalidLength);
        }

        let publish = MqttPublish {
            topic_name,
            qos: qos,
            dup: dup,
            retain: retain,
            packet_id: packet_id_opt,
            payload,
            properties,
        };

        Ok(ParseOk::Packet(MqttPacket::Publish(publish), offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex;

    #[test]
    fn test_parse_publish() {
        let pub_msg = "308c020009756e646566696e65640061616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161";
        // convert buffer from hex string to bytes
        let buffer = hex::decode(pub_msg).unwrap();
        println!("buffer: {:?}", buffer);
        match MqttPublish::from_bytes(&buffer).unwrap() {
            ParseOk::Packet(MqttPacket::Publish(publish), consumed) => {
                assert_eq!(consumed, 1 + 2 + 268);
                assert_eq!(publish.topic_name, "undefined");
                assert_eq!(publish.packet_id, None);
                assert_eq!(publish.payload, vec![0x61; 256]);
            }
            _ => panic!("Invalid packet"),
        }
    }

    #[test]
    fn test_parse_publish_qos1() {
        let pub_msg = "3287020002616100020061616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161";
        // convert buffer from hex string to bytes
        let buffer = hex::decode(pub_msg).unwrap();
        println!("buffer: {:?}", buffer);
        match MqttPublish::from_bytes(&buffer).unwrap() {
            ParseOk::Packet(MqttPacket::Publish(publish), consumed) => {
                assert_eq!(consumed, 1 + 2 + 263);
                assert_eq!(publish.qos, 1);
                assert_eq!(publish.topic_name, "aa");
                assert_eq!(publish.packet_id, Some(2));
                assert_eq!(publish.payload, vec![0x61; 256]);
            }
            _ => panic!("Invalid packet"),
        }
    }

    #[test]
    fn test_parse_publish_qos2() {
        let pub_msg = "3487020002616100020061616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161";
        // convert buffer from hex string to bytes
        let buffer = hex::decode(pub_msg).unwrap();
        println!("buffer: {:?}", buffer);
        match MqttPublish::from_bytes(&buffer).unwrap() {
            ParseOk::Packet(MqttPacket::Publish(publish), consumed) => {
                assert_eq!(consumed, 1 + 2 + 263);
                assert_eq!(publish.qos, 2);
                assert_eq!(publish.topic_name, "aa");
                assert_eq!(publish.packet_id, Some(2));
                assert_eq!(publish.payload, vec![0x61; 256]);
            }
            _ => panic!("Invalid packet"),
        }
    }

    #[test]
    fn test_to_bytes_qos2() {
        let publish = MqttPublish::new(2, "aa".to_string(), Some(2), vec![0x61; 256], false, false);
        let expected = "3487020002616100020061616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161";
        assert_eq!(publish.to_bytes().unwrap(), hex::decode(expected).unwrap());
    }
    #[test]
    fn test_to_bytes_qos0() {
        let publish = MqttPublish::new(
            0,
            "undefined".to_string(),
            Some(2),
            vec![0x61; 256],
            false,
            false,
        );
        let expected = "308c020009756e646566696e65640061616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161";
        assert_eq!(publish.to_bytes().unwrap(), hex::decode(expected).unwrap());
    }

    #[test]
    fn test_pub1() {
        let packet_bytes: [u8; 9] = [0x30, 0x07, 0x00, 0x02, 0x61, 0x61, 0x00, 0x61, 0x61];
        let result = MqttPublish::from_bytes(&packet_bytes);
        match result {
            Ok(ParseOk::Packet(packet, consumed)) => {
                assert_eq!(consumed, 9);
                match packet {
                    MqttPacket::Publish(publish) => {
                        assert_eq!(publish.topic_name, "aa");
                        assert_eq!(publish.qos, 0);
                        assert_eq!(publish.packet_id, None);
                        assert_eq!(publish.properties, Vec::new());
                        assert_eq!(publish.payload, b"aa".to_vec());
                    }
                    _ => panic!("Invalid packet"),
                }
            }
            _ => panic!("Invalid result"),
        }
    }

    #[test]
    fn test_publish_qos0_dup1_is_invalid() {
        // [MQTT-3.3.1-2]
        let publish = MqttPublish::new(0, "topic".to_string(), None, vec![], false, true);
        let bytes = publish.to_bytes().unwrap();

        let result = MqttPublish::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_publish_invalid_qos3() {
        // [MQTT-3.3.1-4]
        let publish = MqttPublish::new(2, "topic".to_string(), Some(1234), vec![], false, false);
        let mut bytes = publish.to_bytes().unwrap();

        // Manually set QoS to 3 in the fixed header (byte 0)
        bytes[0] |= 0x06;

        let result = MqttPublish::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_publish_topic_with_wildcard_is_invalid() {
        // [MQTT-3.3.2-2]
        let publish = MqttPublish::new(1, "topic/+".to_string(), Some(1), vec![], false, false);
        let bytes = publish.to_bytes().unwrap();
        let result = MqttPublish::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));

        let publish = MqttPublish::new(1, "topic/#".to_string(), Some(1), vec![], false, false);
        let bytes = publish.to_bytes().unwrap();
        let result = MqttPublish::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }
}
