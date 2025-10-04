use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::encode_utf8_string;
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::mqttv5::will::Will;
use crate::mqtt_serde::parser::{self, parse_binary_data};
use crate::mqtt_serde::parser::{
    packet_type, parse_remaining_length, parse_utf8_string, ParseError, ParseOk,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
//add extra fields for serde, packet_type, properties, etc.
pub struct MqttConnect {
    pub protocol_name: String,
    pub protocol_version: u8,
    pub keep_alive: u16,
    pub client_id: String,
    pub will: Option<Will>,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    pub properties: Vec<Property>,
    pub clean_start: bool,
}

impl MqttConnect {
    pub fn new(
        client_id: String,
        username: Option<String>,
        password: Option<Vec<u8>>,
        will: Option<Will>,
        keep_alive: u16,
        clean_start: bool,
        properties: Vec<Property>,
    ) -> Self {
        MqttConnect {
            protocol_name: "MQTT".to_string(),
            protocol_version: 5,
            keep_alive,
            client_id,
            will,
            username,
            password,
            properties,
            clean_start,
        }
    }

    pub fn is_clean_start(&self) -> bool {
        self.clean_start
    }

    pub fn is_keep_alive(&self) -> bool {
        self.keep_alive > 0
    }

    fn connect_flags(&self) -> u8 {
        let mut flags: u8 = 0;
        if self.username.is_some() {
            flags |= 0x80;
        }
        if self.password.is_some() {
            flags |= 0x40;
        }
        if let Some(will) = &self.will {
            flags |= 0x04; // Will Flag
            flags |= will.will_qos << 3;
            if will.will_retain {
                flags |= 0x20;
            }
        }
        if self.is_clean_start() {
            flags |= 0x02;
        }
        flags
    }
}

impl MqttControlPacket for MqttConnect {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::CONNECT as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.1.2.1 protocol name
        bytes.extend(encode_utf8_string(&self.protocol_name)?);
        // MQTT 5.0: 3.1.2.2 protocol level
        bytes.push(self.protocol_version);

        // MQTT 5.0: 3.1.2.3 connect flags
        bytes.push(self.connect_flags());
        bytes.extend(self.keep_alive.to_be_bytes());

        // MQTT 5.0: 3.1.2.11 properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    // MQTT 5.0: 3.1.3 Connect Payload
    fn payload(&self) -> Result<Vec<u8>, parser::ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.1.3.1 Client Identifier
        bytes.extend(crate::mqtt_serde::encode_utf8_string(&self.client_id)?);

        // MQTT 5.0: 3.1.3.2 Will Properties
        if let Some(will) = &self.will {
            let mut will_properties = vec![];

            // MQTT 5.0: 3.1.3.2.2 - Only add if present
            if let Some(will_delay_interval) = will.properties.will_delay_interval {
                will_properties.push(Property::WillDelayInterval(will_delay_interval));
            }

            // MQTT 5.0: 3.1.3.2.3 - Only add if present
            if let Some(payload_format_indicator) = will.properties.payload_format_indicator {
                will_properties.push(Property::PayloadFormatIndicator(payload_format_indicator));
            }

            if let Some(message_expiry_interval) = &will.properties.message_expiry_interval {
                // MQTT 5.0: 3.1.3.2.4
                will_properties.push(Property::MessageExpiryInterval(*message_expiry_interval));
            }
            if let Some(content_type) = &will.properties.content_type {
                // MQTT 5.0: 3.1.3.2.5
                will_properties.push(Property::ContentType(content_type.clone()));
            }
            if let Some(response_topic) = &will.properties.response_topic {
                // MQTT 5.0: 3.1.3.2.6
                will_properties.push(Property::ResponseTopic(response_topic.clone()));
            }
            if let Some(correlation_data) = &will.properties.correlation_data {
                // MQTT 5.0: 3.1.3.2.7
                will_properties.push(Property::CorrelationData(correlation_data.clone()));
            }
            // MQTT 5.0: 3.1.3.2.8
            will_properties.extend(will.properties.user_properties.clone());
            bytes.extend(encode_properities_hdr(&will_properties)?);
            bytes.extend(crate::mqtt_serde::encode_utf8_string(&will.will_topic)?);
            bytes.extend(crate::mqtt_serde::encode_binary_data(&will.will_message)?);
        }

        // MQTT 5.0: 3.1.3.5 Username
        if let Some(username) = &self.username {
            bytes.extend(crate::mqtt_serde::encode_utf8_string(username)?);
        }
        // MQTT 5.0: 3.1.3.6 Password
        if let Some(password) = &self.password {
            bytes.extend(crate::mqtt_serde::encode_binary_data(password)?);
        }
        Ok(bytes)
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::CONNECT as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;

        let total_len = offset + size;

        if total_len > buffer.len() {
            let hint = total_len - buffer.len();
            return Ok(ParseOk::Continue(hint, 0));
        }

        // MQTT 5.0: 3.1.2 Var hdrs
        // MQTT 5.0: 3.1.2.1 protocol name
        let (protocol_name, consumed) =
            parse_utf8_string(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        if protocol_name != "MQTT" {
            return Err(ParseError::UnSuppProtoVsn);
        }
        offset += consumed;

        // MQTT 5.0: 3.1.2.2 protocol version
        let protocol_version = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
        offset += 1;

        // MQTT 5.0: 3.1.2.3 connect flags, contains
        // - MQTT 5.0: 3.1.2.4 clean start bit 1
        // - MQTT 5.0: 3.1.2.5 will flag   bit 2
        // - MQTT 5.0: 3.1.2.6 will QoS    bit 3-4
        // - MQTT 5.0: 3.1.2.7 will Retain bit 5
        // - MQTT 5.0: 3.1.2.8 user name   bit 6
        // - MQTT 5.0: 3.1.2.9 password    bit 7
        let connect_flags = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
        offset += 1;

        // [MQTT-3.1.2-3] The Server MUST validate that the reserved flag in the CONNECT packet is set to 0
        #[cfg(feature = "strict-protocol-compliance")]
        if connect_flags & 0x01 != 0 {
            return Err(ParseError::ParseError(
                "CONNECT packet reserved flag is not 0".to_string(),
            ));
        }

        let will_flag = (connect_flags & 0x04) != 0;
        let will_qos = (connect_flags >> 3) & 0x03;
        let will_retain = (connect_flags & 0x20) != 0;

        if !will_flag {
            // [MQTT-3.1.2-11] If the Will Flag is set to 0, then the Will QoS MUST be set to 0
            #[cfg(feature = "strict-protocol-compliance")]
            if will_qos != 0 {
                return Err(ParseError::ParseError(
                    "Will QoS must be 0 if Will Flag is 0".to_string(),
                ));
            }
            // [MQTT-3.1.2-13] If the Will Flag is set to 0, then Will Retain MUST be set to 0
            #[cfg(feature = "strict-protocol-compliance")]
            if will_retain {
                return Err(ParseError::ParseError(
                    "Will Retain must be 0 if Will Flag is 0".to_string(),
                ));
            }
        } else {
            // [MQTT-3.1.2-12] A value of 3 (0x03) is a Malformed Packet.
            #[cfg(feature = "strict-protocol-compliance")]
            if will_qos == 3 {
                return Err(ParseError::ParseError("Will QoS cannot be 3".to_string()));
            }
        }

        // MQTT 5.0: 3.1.2.10 keep alive
        let slice = buffer
            .get(offset..offset + 2)
            .ok_or(ParseError::BufferTooShort)?;
        let keep_alive = u16::from_be_bytes(slice.try_into().unwrap());
        offset += 2;

        // MQTT 5.0: 3.1.2.11 Properties
        let (properties, consumed) =
            parse_properties_hdr(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        // MQTT 5.0: 3.1.3 Payload
        // MQTT 5.0: 3.1.3.1 CLientID
        let (client_id, consumed) =
            parse_utf8_string(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        // MQTT 5.0: 3.1.3.2 Will Properties
        // MQTT 5.0: 3.1.3.3 Will Topic
        // MQTT 5.0: 3.1.3.4 Will Payload
        let will = if will_flag {
            // Will is present
            let (will, consumed) = Will::from_bytes(&buffer[offset..], connect_flags)?;
            offset += consumed;
            if will.will_topic.is_empty() || will.will_message.is_empty() {
                return Err(ParseError::ParseError(
                    "Will topic or message cannot be empty".to_string(),
                ));
            }
            Some(will)
        } else {
            // No Will
            None
        };

        // MQTT 5.0: 3.1.3.5 Username
        let username = if connect_flags & 0x80 != 0 {
            let (username, consumed) =
                parse_utf8_string(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
            offset += consumed;
            if username.is_empty() {
                return Err(ParseError::ParseError(
                    "Username cannot be empty".to_string(),
                ));
            }
            Some(username.to_string())
        } else {
            None
        };
        // MQTT 5.0: 3.1.3.6 Password
        let password = if connect_flags & 0x40 != 0 {
            let (password, consumed) =
                parse_binary_data(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
            offset += consumed;
            if password.is_empty() {
                return Err(ParseError::ParseError(
                    "Password cannot be empty".to_string(),
                ));
            }
            Some(password)
        } else {
            None
        };

        let packet = MqttPacket::Connect5(MqttConnect {
            protocol_name: protocol_name.to_string(),
            protocol_version,
            keep_alive,
            client_id: client_id.to_string(),
            will,
            username,
            password,
            properties,
            clean_start: connect_flags & 0x02 != 0,
        });

        if offset != total_len {
            return Err(ParseError::InternalError(format!(
                "Inconsistent offset {} != total: {}",
                offset, total_len
            )));
        }

        Ok(ParseOk::Packet(packet, offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_imcomplete_connect() {
        let buffer = vec![
            0x10, 0x0d, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, 0x00, 0x00, 0x00, 0x3c,
        ];
        match MqttConnect::from_bytes(&buffer).unwrap() {
            ParseOk::Continue(hint, consumed) => {
                assert_eq!(hint, 2);
                assert_eq!(consumed, 0);
            }
            _ => panic!("Unexpected result"),
        }
    }

    #[test]
    fn test_parse_connect2() {
        let packet_bytes = vec![
            0x10, 0x22, 0x00, 0x04, 0x4d, 0x51, 0x54, 0x54, 0x05, 0x02, 0x01, 0x2c, 0x05, 0x11,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x71, 0x75, 0x69, 0x63, 0x5f, 0x62, 0x65, 0x6e,
            0x63, 0x68, 0x5f, 0x70, 0x75, 0x62, 0x5f, 0x31,
        ];
        if let ParseOk::Packet(packet, consumed) = MqttConnect::from_bytes(&packet_bytes).unwrap() {
            assert_eq!(consumed, 36);
            let MqttPacket::Connect5(connect) = packet else {
                panic!("Invalid packet type");
            };
            assert_eq!(connect.protocol_name, "MQTT");
            assert_eq!(connect.protocol_version, 5);
            assert_eq!(connect.keep_alive, 300);
            assert_eq!(connect.client_id, "quic_bench_pub_1".to_string());
        } else {
            panic!("Unexpected Continue");
        }
    }

    #[test]
    fn test_parse_connect3() {
        let packet_bytes = vec![
            0x10, 0x10, 0x0, 0x4, 0x4d, 0x51, 0x54, 0x54, 0x5, 0x2, 0x0, 0x3c, 0x3, 0x21, 0x0,
            0x14, 0x0, 0x0,
        ];

        if let ParseOk::Packet(packet, consumed) = MqttConnect::from_bytes(&packet_bytes).unwrap() {
            assert_eq!(consumed, 18);
            let MqttPacket::Connect5(connect) = packet else {
                panic!("Invalid packet type");
            };
            assert_eq!(connect.protocol_name, "MQTT");
            assert_eq!(connect.protocol_version, 5);
            assert_eq!(connect.keep_alive, 60);
            assert_eq!(connect.client_id, "");
        } else {
            panic!("Unexpected Continue");
        }
    }
    #[test]
    fn test_serde_json() {
        let connect = MqttConnect::new(
            "test_client".to_string(),
            Some("user".to_string()),
            Some(b"password".to_vec()),
            None,
            0,
            true,
            vec![
                Property::SessionExpiryInterval(3600),
                Property::PayloadFormatIndicator(1),
            ],
        );

        let serialized = serde_json::to_string(&connect).unwrap();
        //println!("Serialized MqttConnect: {}", serialized);
        let deserialized: MqttConnect = serde_json::from_str(&serialized).unwrap();

        assert_eq!(connect, deserialized);
    }

    #[test]
    fn test_serde_bytes() {
        let connect = MqttConnect::new(
            "test_client".to_string(),
            Some("user".to_string()),
            Some(b"password".to_vec()),
            None,
            0,
            true,
            vec![
                Property::SessionExpiryInterval(3600),
                Property::PayloadFormatIndicator(1),
            ],
        );

        let serialized = serde_json::to_string(&connect).unwrap();
        println!("Serialized MqttConnect: {}", serialized);
        let deserialized: MqttConnect = serde_json::from_str(&serialized).unwrap();

        assert_eq!(connect, deserialized);
    }

    #[test]
    fn test_will() {
        let will = Will::new("will_topic".to_string(), b"will_message".to_vec(), 1, true);
        let connect = MqttConnect::new(
            "test_client".to_string(),
            Some("user".to_string()),
            Some(b"password".to_vec()),
            Some(will),
            15,
            true,
            vec![
                Property::SessionExpiryInterval(3600),
                Property::PayloadFormatIndicator(1),
            ],
        );

        let data = connect.to_bytes().unwrap();
        println!("Serialized MqttConnect with Will: {:?}", hex::encode(&data));
        let ParseOk::Packet(connect1, _) = MqttConnect::from_bytes(&data).unwrap() else {
            panic!("Expected packet");
        };
        assert_eq!(connect1, MqttPacket::Connect5(connect));
    }

    #[test]
    fn test_connect_invalid_reserved_flag() {
        // [MQTT-3.1.2-3]
        let connect = MqttConnect::new("c".to_string(), None, None, None, 60, true, vec![]);
        let mut bytes = connect.to_bytes().unwrap();
        // Connect flags are at byte 9, after 1b type, 1b len, 6b proto name, 1b proto ver
        bytes[9] |= 0x01; // Set reserved bit 0

        let result = MqttConnect::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_connect_invalid_will_qos_if_no_will() {
        // [MQTT-3.1.2-11]
        let connect = MqttConnect::new("c".to_string(), None, None, None, 60, true, vec![]);
        let mut bytes = connect.to_bytes().unwrap();
        bytes[9] |= 0x08; // Set Will QoS to 1, but Will Flag is 0

        let result = MqttConnect::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_connect_invalid_will_retain_if_no_will() {
        // [MQTT-3.1.2-13]
        let connect = MqttConnect::new("c".to_string(), None, None, None, 60, true, vec![]);
        let mut bytes = connect.to_bytes().unwrap();
        bytes[9] |= 0x20; // Set Will Retain to 1, but Will Flag is 0

        let result = MqttConnect::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_connect_invalid_will_qos_3() {
        // [MQTT-3.1.2-12]
        let will = Will::new("topic".to_string(), vec![1, 2, 3], 3, false); // QoS 3 is invalid
        let connect = MqttConnect::new("c".to_string(), None, None, Some(will), 60, true, vec![]);

        let mut bytes = connect.to_bytes().unwrap();
        bytes[9] |= 0x18; // Manually set Will QoS to 3, just in case connect_flags changes

        let result = MqttConnect::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }

    #[test]
    fn test_invalid_client_id() {
        let connect = MqttConnect::new(
            "client\0id".to_string(),
            Some("user".to_string()),
            Some(b"password".to_vec()),
            None,
            0,
            true,
            vec![],
        );
        let result = connect.to_bytes();
        println!("Result: {:?}", result);
        assert_eq!(result, Err(ParseError::ParseError("UTF-8 string contains null character (U+0000)".to_string())));
    }
}
