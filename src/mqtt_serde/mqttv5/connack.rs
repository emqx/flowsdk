use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser;
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttConnAck {
    pub session_present: bool,
    pub reason_code: u8,
    pub properties: Option<Vec<Property>>,
}

impl MqttConnAck {
    pub fn new(session_present: bool, reason_code: u8, properties: Option<Vec<Property>>) -> Self {
        MqttConnAck {
            session_present: session_present,
            reason_code: reason_code,
            properties: properties,
        }
    }
}

impl MqttControlPacket for MqttConnAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::CONNACK as u8
    }

    // MQTT 5.0: 3.2.2, Variable header
    fn variable_header(&self) -> Result<Vec<u8>, parser::ParseError> {
        let mut bytes = Vec::new();
        // MQTT 5.0: 3.2.2.1, Session Present Flag
        bytes.push(if self.session_present { 1 } else { 0 });
        // MQTT 5.0: 3.2.2.2, Connect Reason Code
        bytes.push(self.reason_code);
        // MQTT 5.0: 3.2.2.3, CONNACK Properties
        if let Some(properties) = &self.properties {
            bytes.extend(encode_properities_hdr(&properties)?);
        }
        Ok(bytes)
    }

    // MQTT 5.0: 3.2.3, CONNACK Payload
    fn payload(&self) -> Result<Vec<u8>, parser::ParseError> {
        // CONNACK has no payload
        Ok(Vec::new())
    }

    fn from_bytes(bytes: &[u8]) -> Result<ParseOk, ParseError> {
        parse_connack(bytes)
    }
}

pub fn parse_connack(buffer: &[u8]) -> Result<ParseOk, ParseError> {
    let packet_type = packet_type(&buffer)?;
    if packet_type != ControlPacketType::CONNACK as u8 {
        return Err(ParseError::InvalidPacketType);
    }

    let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;

    let mut offset = 1 + vbi_len;
    let total_len = offset + size;

    if total_len > buffer.len() {
        return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
    }

    // MQTT 5.0 3.2.2.1 Connack Flag
    let session_present_byte = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
    let session_present: bool = (session_present_byte & 0x01) == 1u8;

    // MQTT 5.0 Protocol Compliance: Reserved bits must be 0
    #[cfg(feature = "strict-protocol-compliance")]
    if session_present_byte & 0xFE != 0 {
        return Err(ParseError::ParseError(
            "CONNACK Connect Acknowledge Flags reserved bits must be 0".to_string(),
        ));
    }

    // MQTT 5.0: 3.2.2.2 Connect Reason Code
    let reason_code = *buffer.get(offset + 1).ok_or(ParseError::BufferTooShort)?;
    offset += 2; // move past the flags and reason code

    // MQTT 5.0: 3.2.2.3 Connack props
    let (properties, consumed) =
        parse_properties_hdr(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
    offset += consumed;
    let properties = if properties.is_empty() {
        None
    } else {
        Some(properties)
    };

    // MQTT 5.0: 3.2.3 Connack Payload, NO payload
    if offset != total_len {
        return Err(ParseError::InvalidLength);
    }

    Ok(ParseOk::Packet(
        MqttPacket::ConnAck(MqttConnAck::new(session_present, reason_code, properties)),
        offset,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_serde::control_packet::MqttPacket;
    use crate::mqtt_serde::parser::ParseOk;

    #[test]
    fn test_parse_connack() {
        let packet_bytes: [u8; 24] = [
            0x20, 0x16, 0x00, 0x00, 0x13, 0x2a, 0x01, 0x29, 0x01, 0x28, 0x01, 0x27, 0x00, 0x10,
            0x00, 0x00, 0x25, 0x01, 0x22, 0xff, 0xff, 0x21, 0x00, 0x20,
        ];

        let result = MqttConnAck::from_bytes(&packet_bytes);
        match result {
            Ok(ParseOk::Packet(packet, consumed)) => {
                assert_eq!(consumed, 24);
                match packet {
                    MqttPacket::ConnAck(connack) => {
                        assert_eq!(connack.session_present, false);
                        assert_eq!(connack.reason_code, 0);
                    }
                    _ => panic!("Invalid packet"),
                }
            }
            _ => panic!("Invalid result"),
        }
    }

    #[test]
    fn test_connack_invalid_reserved_flags() {
        // [MQTT-3.2.2-1]
        let connack = MqttConnAck::new(false, 0, None);
        let mut bytes = connack.to_bytes().unwrap();

        // A simple CONNACK has a remaining length of 2, so the var header starts at index 2.
        // The first byte of the var header is the Connect Acknowledge Flags.
        bytes[2] |= 0x02; // Set reserved bit 1 to 1

        let result = MqttConnAck::from_bytes(&bytes);
        assert!(matches!(result, Err(ParseError::ParseError(_))));
    }
}
