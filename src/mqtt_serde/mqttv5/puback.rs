// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{
    packet_type, parse_packet_id, parse_remaining_length, ParseError, ParseOk,
};

/// Represents the PUBACK packet in MQTT v5.0.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttPubAck {
    pub packet_id: u16,
    pub reason_code: u8,
    pub properties: Vec<Property>,
}

impl MqttPubAck {
    /// Creates a new `MqttPubAck` packet.
    pub fn new(packet_id: u16, reason_code: u8, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            reason_code,
            properties,
        }
    }
}

impl MqttControlPacket for MqttPubAck {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::PUBACK as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();
        // Packet Identifier is always present.
        bytes.extend_from_slice(&self.packet_id.to_be_bytes());

        // MQTT 5.0: 3.4.2.1, the Reason Code and Properties are only
        // present if the Remaining Length is greater than 2.
        // For a successful ack with no properties, we send a minimal packet.
        if self.reason_code == 0x00 && self.properties.is_empty() {
            return Ok(bytes);
        }

        bytes.push(self.reason_code);

        // MQTT 5.0: 3.4.2.2 Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);
        Ok(bytes)
    }

    // MQTT 5.0: 3.4.3 PUBACK Payload
    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // PUBACK packets have no payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::PUBACK as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // Variable Header
        // MQTT 5.0: 3.4.2 Packet Identifier
        let (packet_id, consumed) =
            parse_packet_id(buffer.get(offset..).ok_or(ParseError::BufferTooShort)?)?;
        offset += consumed;

        // MQTT 5.0: 3.4.2.1 PUBACK Reason Code
        // Defaults to 0x00 (Success) if not present.
        let reason_code = if size > 2 {
            let code = *buffer.get(offset).ok_or(ParseError::BufferTooShort)?;
            offset += 1;
            code
        } else {
            0x00
        };

        // MQTT 5.0: 3.4.2.2 Properties
        let (properties, consumed) = if offset < total_len {
            parse_properties_hdr(
                buffer
                    .get(offset..total_len)
                    .ok_or(ParseError::BufferTooShort)?,
            )?
        } else {
            (vec![], 0)
        };
        offset += consumed;

        let packet = MqttPacket::PubAck5(MqttPubAck {
            packet_id,
            reason_code,
            properties,
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
    use crate::mqtt_serde::control_packet::MqttPacket;
    use hex;

    #[test]
    fn test_puback_serde_roundtrip_success() {
        // Test case 1: Simple success case with no properties
        let puback_original = MqttPubAck::new(42, 0x00, vec![]);
        let bytes = puback_original.to_bytes().unwrap();

        // Expected: 0x40 (PUBACK), 0x02 (Remaining Length), 0x00, 0x2A (Packet ID 42)
        assert_eq!(bytes, vec![0x40, 0x02, 0x00, 0x2A]);

        let ParseOk::Packet(MqttPacket::PubAck5(puback_deserialized), consumed) =
            MqttPubAck::from_bytes(&bytes).unwrap()
        else {
            panic!("Failed to deserialize");
        };

        assert_eq!(puback_original, puback_deserialized);
        assert_eq!(consumed, 4);
    }

    #[test]
    fn test_puback_serde_roundtrip_with_properties() {
        // Test case 2: Failure case with a Reason String property
        let properties = vec![Property::ReasonString("Not authorized".to_string())];
        let puback_original = MqttPubAck::new(100, 0x87, properties);
        let bytes = puback_original.to_bytes().unwrap();

        println!("Serialized PUBACK with properties: {}", hex::encode(&bytes));

        let ParseOk::Packet(MqttPacket::PubAck5(puback_deserialized), consumed) =
            MqttPubAck::from_bytes(&bytes).unwrap()
        else {
            panic!("Failed to deserialize");
        };

        assert_eq!(puback_original, puback_deserialized);
        assert!(consumed > 4); // Should be longer than the minimal packet
    }

    #[test]
    fn test_parse_minimal_puback() {
        // A broker might send a PUBACK with just Packet ID for success.
        let bytes = vec![0x40, 0x02, 0x00, 0x2C]; // PUBACK for Packet ID 44
        let ParseOk::Packet(MqttPacket::PubAck5(puback_deserialized), _consumed) =
            MqttPubAck::from_bytes(&bytes).unwrap()
        else {
            panic!("Failed to deserialize");
        };

        assert_eq!(puback_deserialized.packet_id, 44);
        assert_eq!(puback_deserialized.reason_code, 0x00); // Should default to success
        assert!(puback_deserialized.properties.is_empty());
    }

    #[test]
    fn test_puback() {
        let bytes = hex::decode("400400021000").unwrap();
        let bytes_len = bytes.len();

        let res: Result<ParseOk, ParseError> = MqttPubAck::from_bytes(&bytes);
        match res {
            Ok(ParseOk::Packet(MqttPacket::PubAck5(puback), consumed)) => {
                assert_eq!(puback.packet_id, 2);
                assert_eq!(puback.reason_code, 0x10);
                assert_eq!(puback.properties, vec![]);
                assert_eq!(consumed, bytes_len);
            }
            _ => panic!("Expected PubAck"),
        }
    }
}
