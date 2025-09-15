use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the DISCONNECT packet in MQTT v5.0.
///
/// The DISCONNECT packet is sent from the Client to the Server to indicate that
/// the Client is disconnecting cleanly. It can also be sent from the Server to
/// the Client to indicate that the Server is disconnecting.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttDisconnect {
    pub reason_code: u8,
    pub properties: Vec<Property>,
}

impl MqttDisconnect {
    /// Creates a new `MqttDisconnect` packet.
    ///
    /// # Arguments
    /// * `reason_code` - The reason for disconnection
    /// * `properties` - Optional properties
    pub fn new(reason_code: u8, properties: Vec<Property>) -> Self {
        Self {
            reason_code,
            properties,
        }
    }

    /// Creates a new DISCONNECT packet with normal disconnection (reason code 0x00).
    pub fn new_normal() -> Self {
        Self::new(0x00, Vec::new())
    }

    /// Creates a simple DISCONNECT with a specific reason code but no properties.
    pub fn new_simple(reason_code: u8) -> Self {
        Self::new(reason_code, Vec::new())
    }

    /// Creates a DISCONNECT packet with disconnect with Will Message (reason code 0x04).
    pub fn new_with_will_message() -> Self {
        Self::new(0x04, Vec::new())
    }

    /// Creates a DISCONNECT packet for unspecified error (reason code 0x80).
    pub fn new_unspecified_error() -> Self {
        Self::new(0x80, Vec::new())
    }

    /// Creates a DISCONNECT packet for malformed packet error (reason code 0x81).
    pub fn new_malformed_packet() -> Self {
        Self::new(0x81, Vec::new())
    }

    /// Creates a DISCONNECT packet for protocol error (reason code 0x82).
    pub fn new_protocol_error() -> Self {
        Self::new(0x82, Vec::new())
    }

    /// Creates a DISCONNECT packet for implementation specific error (reason code 0x83).
    pub fn new_implementation_specific_error() -> Self {
        Self::new(0x83, Vec::new())
    }

    /// Creates a DISCONNECT packet for not authorized (reason code 0x87).
    pub fn new_not_authorized() -> Self {
        Self::new(0x87, Vec::new())
    }

    /// Creates a DISCONNECT packet for server busy (reason code 0x89).
    pub fn new_server_busy() -> Self {
        Self::new(0x89, Vec::new())
    }

    /// Creates a DISCONNECT packet for server shutting down (reason code 0x8B).
    pub fn new_server_shutting_down() -> Self {
        Self::new(0x8B, Vec::new())
    }

    /// Creates a DISCONNECT packet for keep alive timeout (reason code 0x8D).
    pub fn new_keep_alive_timeout() -> Self {
        Self::new(0x8D, Vec::new())
    }

    /// Creates a DISCONNECT packet for session taken over (reason code 0x8E).
    pub fn new_session_taken_over() -> Self {
        Self::new(0x8E, Vec::new())
    }

    /// Creates a DISCONNECT packet for topic filter invalid (reason code 0x8F).
    pub fn new_topic_filter_invalid() -> Self {
        Self::new(0x8F, Vec::new())
    }

    /// Creates a DISCONNECT packet for topic name invalid (reason code 0x90).
    pub fn new_topic_name_invalid() -> Self {
        Self::new(0x90, Vec::new())
    }

    /// Creates a DISCONNECT packet for receive maximum exceeded (reason code 0x93).
    pub fn new_receive_maximum_exceeded() -> Self {
        Self::new(0x93, Vec::new())
    }

    /// Creates a DISCONNECT packet for topic alias invalid (reason code 0x94).
    pub fn new_topic_alias_invalid() -> Self {
        Self::new(0x94, Vec::new())
    }

    /// Creates a DISCONNECT packet for packet too large (reason code 0x95).
    pub fn new_packet_too_large() -> Self {
        Self::new(0x95, Vec::new())
    }

    /// Creates a DISCONNECT packet for message rate too high (reason code 0x96).
    pub fn new_message_rate_too_high() -> Self {
        Self::new(0x96, Vec::new())
    }

    /// Creates a DISCONNECT packet for quota exceeded (reason code 0x97).
    pub fn new_quota_exceeded() -> Self {
        Self::new(0x97, Vec::new())
    }

    /// Creates a DISCONNECT packet for administrative action (reason code 0x98).
    pub fn new_administrative_action() -> Self {
        Self::new(0x98, Vec::new())
    }

    /// Creates a DISCONNECT packet for payload format invalid (reason code 0x99).
    pub fn new_payload_format_invalid() -> Self {
        Self::new(0x99, Vec::new())
    }
}

impl Default for MqttDisconnect {
    fn default() -> Self {
        Self::new_normal()
    }
}

impl MqttControlPacket for MqttDisconnect {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::DISCONNECT as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.14.2.1 Disconnect Reason Code
        // If the Remaining Length is 0, the Reason Code and Properties are omitted
        // and the Disconnect Reason Code is 0x00 (Normal disconnection).
        if self.reason_code == 0x00 && self.properties.is_empty() {
            return Ok(bytes);
        }

        bytes.push(self.reason_code);

        // MQTT 5.0: 3.14.2.2 Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);
        Ok(bytes)
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT 5.0: 3.14.3 DISCONNECT Payload
        // The DISCONNECT packet has no Payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::DISCONNECT as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.14.1 Reserved bits in the Fixed Header
        // Bits 3,2,1 and 0 of the Fixed Header in the DISCONNECT packet are reserved and MUST be set to 0,0,0,0.
        #[cfg(feature = "strict-protocol-compliance")]
        {
            let flags = buffer[0] & 0x0F;
            if flags != 0x00 {
                return Err(ParseError::ParseError(
                    "DISCONNECT packet has invalid fixed header flags".to_string(),
                ));
            }
        }

        // Default values for minimal packet
        let mut reason_code = 0x00;
        let mut properties = Vec::new();

        // MQTT 5.0: 3.14.2 Variable Header
        if size > 0 {
            // Parse reason code
            if offset >= total_len {
                return Err(ParseError::BufferTooShort);
            }
            reason_code = buffer[offset];
            offset += 1;

            // Parse properties if there's more data
            if offset < total_len {
                let (parsed_properties, consumed) = parse_properties_hdr(
                    buffer
                        .get(offset..total_len)
                        .ok_or(ParseError::BufferTooShort)?,
                )?;
                properties = parsed_properties;
                offset += consumed;
            }
        }

        if offset != total_len {
            return Err(ParseError::InternalError(format!(
                "Inconsistent offset {} != total: {}",
                offset, total_len
            )));
        }

        let disconnect = MqttDisconnect::new(reason_code, properties);
        Ok(ParseOk::Packet(
            MqttPacket::Disconnect5(disconnect),
            total_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disconnect_creation() {
        let disconnect = MqttDisconnect::new(0x00, Vec::new());
        assert_eq!(
            disconnect.control_packet_type(),
            ControlPacketType::DISCONNECT as u8
        );
        assert_eq!(disconnect.reason_code, 0x00);
        assert!(disconnect.properties.is_empty());
    }

    #[test]
    fn test_disconnect_default() {
        let disconnect = MqttDisconnect::default();
        assert_eq!(disconnect.reason_code, 0x00);
        assert!(disconnect.properties.is_empty());
    }

    #[test]
    fn test_disconnect_normal() {
        let disconnect = MqttDisconnect::new_normal();
        assert_eq!(disconnect.reason_code, 0x00);
        assert!(disconnect.properties.is_empty());
    }

    #[test]
    fn test_disconnect_convenience_methods() {
        assert_eq!(MqttDisconnect::new_with_will_message().reason_code, 0x04);
        assert_eq!(MqttDisconnect::new_unspecified_error().reason_code, 0x80);
        assert_eq!(MqttDisconnect::new_malformed_packet().reason_code, 0x81);
        assert_eq!(MqttDisconnect::new_protocol_error().reason_code, 0x82);
        assert_eq!(
            MqttDisconnect::new_implementation_specific_error().reason_code,
            0x83
        );
        assert_eq!(MqttDisconnect::new_not_authorized().reason_code, 0x87);
        assert_eq!(MqttDisconnect::new_server_busy().reason_code, 0x89);
        assert_eq!(MqttDisconnect::new_server_shutting_down().reason_code, 0x8B);
        assert_eq!(MqttDisconnect::new_keep_alive_timeout().reason_code, 0x8D);
        assert_eq!(MqttDisconnect::new_session_taken_over().reason_code, 0x8E);
        assert_eq!(MqttDisconnect::new_topic_filter_invalid().reason_code, 0x8F);
        assert_eq!(MqttDisconnect::new_topic_name_invalid().reason_code, 0x90);
        assert_eq!(
            MqttDisconnect::new_receive_maximum_exceeded().reason_code,
            0x93
        );
        assert_eq!(MqttDisconnect::new_topic_alias_invalid().reason_code, 0x94);
        assert_eq!(MqttDisconnect::new_packet_too_large().reason_code, 0x95);
        assert_eq!(
            MqttDisconnect::new_message_rate_too_high().reason_code,
            0x96
        );
        assert_eq!(MqttDisconnect::new_quota_exceeded().reason_code, 0x97);
        assert_eq!(
            MqttDisconnect::new_administrative_action().reason_code,
            0x98
        );
        assert_eq!(
            MqttDisconnect::new_payload_format_invalid().reason_code,
            0x99
        );
    }

    #[test]
    fn test_disconnect_minimal_serialization() {
        let disconnect = MqttDisconnect::new_normal();
        let bytes = disconnect.to_bytes().unwrap();

        // Expected: Fixed header only: 0xE0 (packet type 14, flags 0) + 0x00 (remaining length 0)
        assert_eq!(bytes, vec![0xE0, 0x00]);
    }

    #[test]
    fn test_disconnect_with_reason_code_serialization() {
        let disconnect = MqttDisconnect::new_simple(0x81);
        let bytes = disconnect.to_bytes().unwrap();

        // Expected: 0xE0 (packet type 14, flags 0) + 0x02 (remaining length 2) + 0x81 (reason code) + 0x00 (no properties)
        assert_eq!(bytes, vec![0xE0, 0x02, 0x81, 0x00]);
    }

    #[test]
    fn test_disconnect_minimal_deserialization() {
        let bytes = vec![0xE0, 0x00]; // DISCONNECT with remaining length 0

        match MqttDisconnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect5(disconnect), consumed) => {
                assert_eq!(consumed, 2);
                assert_eq!(disconnect.reason_code, 0x00);
                assert!(disconnect.properties.is_empty());
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_disconnect_with_reason_code_deserialization() {
        let bytes = vec![0xE0, 0x02, 0x81, 0x00]; // DISCONNECT with reason code 0x81, no properties

        match MqttDisconnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect5(disconnect), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(disconnect.reason_code, 0x81);
                assert!(disconnect.properties.is_empty());
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_disconnect_roundtrip_normal() {
        let original = MqttDisconnect::new_normal();
        let bytes = original.to_bytes().unwrap();

        match MqttDisconnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect5(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_disconnect_roundtrip_with_reason_code() {
        let original = MqttDisconnect::new_simple(0x82);
        let bytes = original.to_bytes().unwrap();

        match MqttDisconnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect5(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_disconnect_invalid_flags() {
        let bytes = vec![0xE1, 0x00]; // DISCONNECT with invalid flags (should be 0x00)

        match MqttDisconnect::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_disconnect_incomplete_packet() {
        let bytes = vec![0xE0]; // Incomplete DISCONNECT (missing remaining length)

        match MqttDisconnect::from_bytes(&bytes) {
            Ok(ParseOk::Continue(needed, _)) => {
                assert_eq!(needed, 1); // Need 1 more byte for remaining length
            }
            Err(ParseError::BufferTooShort) => {
                // This is also acceptable for incomplete packets
            }
            other => panic!(
                "Expected Continue parse result or BufferTooShort, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_disconnect_wrong_packet_type() {
        let bytes = vec![0xF0, 0x00]; // Wrong packet type (15), not DISCONNECT (14)

        match MqttDisconnect::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {
                // Expected error
            }
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_disconnect_empty_payload() {
        let disconnect = MqttDisconnect::new_normal();

        assert!(disconnect.payload().unwrap().is_empty());
    }

    #[test]
    fn test_disconnect_all_reason_codes() {
        // Test that all defined reason codes work
        let test_cases = vec![
            (0x00, "Normal disconnection"),
            (0x04, "Disconnect with Will Message"),
            (0x80, "Unspecified error"),
            (0x81, "Malformed Packet"),
            (0x82, "Protocol Error"),
            (0x83, "Implementation specific error"),
            (0x87, "Not authorized"),
            (0x89, "Server busy"),
            (0x8B, "Server shutting down"),
            (0x8D, "Keep Alive timeout"),
            (0x8E, "Session taken over"),
            (0x8F, "Topic Filter invalid"),
            (0x90, "Topic Name invalid"),
            (0x93, "Receive Maximum exceeded"),
            (0x94, "Topic Alias invalid"),
            (0x95, "Packet too large"),
            (0x96, "Message rate too high"),
            (0x97, "Quota exceeded"),
            (0x98, "Administrative action"),
            (0x99, "Payload format invalid"),
        ];

        for (reason_code, _description) in test_cases {
            let disconnect = MqttDisconnect::new_simple(reason_code);
            let bytes = disconnect.to_bytes().unwrap();

            match MqttDisconnect::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::Disconnect5(parsed), _) => {
                    assert_eq!(parsed.reason_code, reason_code);
                }
                _ => panic!(
                    "Failed to parse DISCONNECT with reason code {:#04X}",
                    reason_code
                ),
            }
        }
    }

    #[test]
    fn test_disconnect_with_properties() {
        let properties = vec![
            Property::SessionExpiryInterval(3600), // Session expires in 1 hour
            Property::ReasonString("Server maintenance".to_string()),
        ];
        let disconnect = MqttDisconnect::new(0x8B, properties.clone());
        let bytes = disconnect.to_bytes().unwrap();

        match MqttDisconnect::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect5(parsed), _) => {
                assert_eq!(parsed.reason_code, 0x8B);
                assert_eq!(parsed.properties, properties);
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }
}
