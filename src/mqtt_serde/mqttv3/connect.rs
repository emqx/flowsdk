use serde::{Deserialize, Serialize};

use crate::mqtt_serde::base_data::{TwoByteInteger, Utf8String};
use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::parser::{
    packet_type, parse_remaining_length, parse_utf8_string, ParseError, ParseOk,
};

/// Represents the Will message and its properties.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Will {
    pub retain: bool,
    pub qos: u8,
    pub topic: String,
    pub message: Vec<u8>,
}

/// Represents the CONNECT packet in MQTT v3.1.1.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttConnect {
    pub clean_session: bool,
    pub keep_alive: u16,
    pub client_id: String,
    pub will: Option<Will>,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
}

impl MqttConnect {
    pub fn new(client_id: String, keep_alive: u16, clean_session: bool) -> Self {
        Self {
            clean_session,
            keep_alive,
            client_id,
            will: None,
            username: None,
            password: None,
        }
    }
}

impl MqttControlPacket for MqttConnect {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::CONNECT as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut vh = Vec::new();
        // Protocol Name
        vh.extend(Utf8String::encode("MQTT"));
        // Protocol Version
        vh.push(4);

        // Connect Flags
        let mut flags = 0u8;
        if self.clean_session {
            flags |= 0x02;
        }
        if let Some(will) = &self.will {
            flags |= 0x04; // Will Flag
            flags |= will.qos << 3;
            if will.retain {
                flags |= 0x20;
            }
        }
        if self.username.is_some() {
            flags |= 0x80;
        }
        if self.password.is_some() {
            flags |= 0x40;
        }
        vh.push(flags);

        // Keep Alive
        vh.extend_from_slice(&TwoByteInteger::encode(self.keep_alive));
        Ok(vh)
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        let mut payload = Vec::new();
        payload.extend(Utf8String::encode(&self.client_id));

        if let Some(will) = &self.will {
            payload.extend(Utf8String::encode(&will.topic));
            // Will message is a byte array, needs length prefix
            payload.extend_from_slice(&TwoByteInteger::encode(will.message.len() as u16));
            payload.extend_from_slice(&will.message);
        }

        if let Some(username) = &self.username {
            payload.extend(Utf8String::encode(username));
        }

        if let Some(password) = &self.password {
            // Password is a byte array, needs length prefix
            payload.extend_from_slice(&TwoByteInteger::encode(password.len() as u16));
            payload.extend_from_slice(password);
        }

        Ok(payload)
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::CONNECT as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Variable Header
        let (proto_name, consumed) = parse_utf8_string(&buffer[offset..])?;
        offset += consumed;
        if proto_name != "MQTT" {
            return Err(ParseError::ParseError("Invalid protocol name".to_string()));
        }

        if offset >= buffer.len() {
            return Err(ParseError::BufferTooShort);
        }
        let version = buffer[offset];
        offset += 1;
        if version != 4 {
            return Err(ParseError::ParseError(
                "Invalid protocol version".to_string(),
            ));
        }

        if offset >= buffer.len() {
            return Err(ParseError::BufferTooShort);
        }
        let flags = buffer[offset];
        offset += 1;
        let clean_session = (flags & 0x02) > 0;
        let will_flag = (flags & 0x04) > 0;
        let will_qos = (flags & 0x18) >> 3;
        let will_retain = (flags & 0x20) > 0;
        let username_flag = (flags & 0x80) > 0;
        let password_flag = (flags & 0x40) > 0;

        if (flags & 0x01) != 0 {
            return Err(ParseError::ParseError(
                "CONNECT reserved flag bit is not 0".to_string(),
            ));
        }
        if password_flag && !username_flag {
            return Err(ParseError::ParseError(
                "Password flag requires username flag".to_string(),
            ));
        }

        let (keep_alive, _) = TwoByteInteger::decode(&buffer[offset..])?;
        offset += 2;

        // Payload
        let (client_id, consumed) = parse_utf8_string(&buffer[offset..])?;
        offset += consumed;

        let will = if will_flag {
            let (topic, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            let (msg_len, _) = TwoByteInteger::decode(&buffer[offset..])?;
            let msg_len = msg_len as usize;
            offset += 2;
            if offset + msg_len > buffer.len() {
                return Err(ParseError::BufferTooShort);
            }
            let message = buffer[offset..offset + msg_len].to_vec();
            offset += msg_len;
            Some(Will {
                retain: will_retain,
                qos: will_qos,
                topic,
                message,
            })
        } else {
            None
        };

        let username = if username_flag {
            let (u, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Some(u)
        } else {
            None
        };

        let password = if password_flag {
            let (pass_len, _) = TwoByteInteger::decode(&buffer[offset..])?;
            let pass_len = pass_len as usize;
            offset += 2;
            if offset + pass_len > buffer.len() {
                return Err(ParseError::BufferTooShort);
            }
            let p = buffer[offset..offset + pass_len].to_vec();
            offset += pass_len;
            Some(p)
        } else {
            None
        };

        if offset != total_len {
            return Err(ParseError::InternalError(
                "Failed to consume entire CONNECT packet".to_string(),
            ));
        }

        Ok(ParseOk::Packet(
            MqttPacket::Connect3(MqttConnect {
                clean_session,
                keep_alive,
                client_id,
                will,
                username,
                password,
            }),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_minimal_roundtrip() {
        let original = MqttConnect::new("test-client".to_string(), 60, true);
        let bytes = original.to_bytes().unwrap();
        match MqttConnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Connect3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected CONNECT packet"),
        }
    }

    #[test]
    fn test_connect_full_roundtrip() {
        let mut original = MqttConnect::new("test-client-full".to_string(), 30, false);
        original.will = Some(Will {
            retain: true,
            qos: 2,
            topic: "will/topic".to_string(),
            message: b"last will".to_vec(),
        });
        original.username = Some("user".to_string());
        original.password = Some(b"pass".to_vec());

        let bytes = original.to_bytes().unwrap();
        match MqttConnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Connect3(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected CONNECT packet"),
        }
    }

    #[test]
    fn test_connect_password_without_username_is_error() {
        let mut conn = MqttConnect::new("client".to_string(), 60, true);
        conn.password = Some(b"pass".to_vec());
        // This should fail during serialization in a real scenario, but here we test the parser
        let bytes = vec![
            0x10, 24, // type, len
            0x00, 0x04, b'M', b'Q', b'T', b'T', // proto name
            0x04, // version
            0x42, // flags: password, no username, clean session
            0x00, 0x3C, // keep alive
            0x00, 0x06, b'c', b'l', b'i', b'e', b'n', b't', // client id
            0x00, 0x04, b'p', b'a', b's', b's', // password
        ];
        assert!(MqttConnect::from_bytes(&bytes).is_err());
    }
}
