use serde::{Deserialize, Serialize};

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::{
    encode_properities_hdr, parse_properties_hdr, Property,
};
use crate::mqtt_serde::parser::{packet_type, parse_remaining_length, ParseError, ParseOk};

/// Represents the AUTH packet in MQTT v5.0.
///
/// The AUTH packet is sent from Client to Server or Server to Client as part of
/// an authentication exchange. It contains authentication-related data and properties.
/// The AUTH packet is only used when the CONNECT packet contains an Authentication Method.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MqttAuth {
    pub reason_code: u8,
    pub properties: Vec<Property>,
}

impl MqttAuth {
    /// Creates a new `MqttAuth` packet.
    ///
    /// # Arguments
    /// * `reason_code` - The reason for the authentication challenge/response
    /// * `properties` - Authentication properties (typically includes AuthenticationMethod and/or AuthenticationData)
    pub fn new(reason_code: u8, properties: Vec<Property>) -> Self {
        Self {
            reason_code,
            properties,
        }
    }

    /// Creates an AUTH packet for successful authentication (reason code 0x00).
    pub fn new_success() -> Self {
        Self::new(0x00, Vec::new())
    }

    /// Creates an AUTH packet for continue authentication (reason code 0x18).
    /// This is used during multi-step authentication processes.
    pub fn new_continue_authentication() -> Self {
        Self::new(0x18, Vec::new())
    }

    /// Creates an AUTH packet for re-authentication (reason code 0x19).
    /// This is used to refresh authentication credentials.
    pub fn new_re_authenticate() -> Self {
        Self::new(0x19, Vec::new())
    }

    /// Creates an AUTH packet with authentication method and data.
    ///
    /// # Arguments
    /// * `reason_code` - The reason code for the authentication packet
    /// * `auth_method` - The authentication method (e.g., "SCRAM-SHA-1")
    /// * `auth_data` - The authentication data
    pub fn new_with_auth_data(reason_code: u8, auth_method: String, auth_data: Vec<u8>) -> Self {
        let properties = vec![
            Property::AuthenticationMethod(auth_method),
            Property::AuthenticationData(auth_data),
        ];
        Self::new(reason_code, properties)
    }

    /// Creates an AUTH packet with only authentication method.
    ///
    /// # Arguments
    /// * `reason_code` - The reason code for the authentication packet
    /// * `auth_method` - The authentication method (e.g., "SCRAM-SHA-1")
    pub fn new_with_method(reason_code: u8, auth_method: String) -> Self {
        let properties = vec![Property::AuthenticationMethod(auth_method)];
        Self::new(reason_code, properties)
    }

    /// Creates an AUTH packet for continuing authentication with data.
    ///
    /// # Arguments
    /// * `auth_method` - The authentication method
    /// * `auth_data` - The authentication challenge/response data
    pub fn new_continue_with_data(auth_method: String, auth_data: Vec<u8>) -> Self {
        Self::new_with_auth_data(0x18, auth_method, auth_data)
    }

    /// Creates an AUTH packet for re-authentication with new credentials.
    ///
    /// # Arguments
    /// * `auth_method` - The authentication method
    /// * `auth_data` - The new authentication data
    pub fn new_re_auth_with_data(auth_method: String, auth_data: Vec<u8>) -> Self {
        Self::new_with_auth_data(0x19, auth_method, auth_data)
    }
}

impl Default for MqttAuth {
    fn default() -> Self {
        Self::new_success()
    }
}

impl MqttControlPacket for MqttAuth {
    fn control_packet_type(&self) -> u8 {
        ControlPacketType::AUTH as u8
    }

    fn variable_header(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        // MQTT 5.0: 3.15.2 AUTH Variable Header
        // Always include the reason code
        bytes.push(self.reason_code);

        // MQTT 5.0: 3.15.2.1 AUTH Properties
        bytes.extend(encode_properities_hdr(&self.properties)?);

        Ok(bytes)
    }

    fn payload(&self) -> Result<Vec<u8>, ParseError> {
        // MQTT 5.0: 3.15.3 AUTH Payload
        // The AUTH packet has no Payload.
        Ok(Vec::new())
    }

    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type = packet_type(buffer)?;
        if packet_type != ControlPacketType::AUTH as u8 {
            return Err(ParseError::InvalidPacketType);
        }

        let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
        let mut offset: usize = 1 + vbi_len;
        let total_len = offset + size;

        if total_len > buffer.len() {
            return Ok(ParseOk::Continue(total_len - buffer.len(), 0));
        }

        // MQTT 5.0: 3.15.1 Reserved bits in the Fixed Header
        // Bits 3,2,1 and 0 of the Fixed Header in the AUTH packet are reserved and MUST be set to 0,0,0,0.
        #[cfg(feature = "strict-protocol-compliance")]
        {
            let flags = buffer[0] & 0x0F;
            if flags != 0x00 {
                return Err(ParseError::ParseError(
                    "AUTH packet has invalid fixed header flags".to_string(),
                ));
            }
        }

        // MQTT 5.0: 3.15.2 AUTH Variable Header
        // The variable header MUST contain a reason code
        #[cfg(feature = "strict-protocol-compliance")]
        if size == 0 {
            return Err(ParseError::ParseError(
                "AUTH packet must contain a reason code".to_string(),
            ));
        }

        // Parse reason code
        if offset >= total_len {
            return Err(ParseError::BufferTooShort);
        }
        let reason_code = buffer[offset];
        offset += 1;

        // Parse properties
        let (properties, consumed) = parse_properties_hdr(
            buffer
                .get(offset..total_len)
                .ok_or(ParseError::BufferTooShort)?,
        )?;
        offset += consumed;

        if offset != total_len {
            return Err(ParseError::InternalError(format!(
                "Inconsistent offset {} != total: {}",
                offset, total_len
            )));
        }

        let auth = MqttAuth::new(reason_code, properties);
        Ok(ParseOk::Packet(MqttPacket::Auth(auth), total_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_creation() {
        let auth = MqttAuth::new(0x00, Vec::new());
        assert_eq!(auth.control_packet_type(), ControlPacketType::AUTH as u8);
        assert_eq!(auth.reason_code, 0x00);
        assert!(auth.properties.is_empty());
    }

    #[test]
    fn test_auth_default() {
        let auth = MqttAuth::default();
        assert_eq!(auth.reason_code, 0x00);
        assert!(auth.properties.is_empty());
    }

    #[test]
    fn test_auth_success() {
        let auth = MqttAuth::new_success();
        assert_eq!(auth.reason_code, 0x00);
        assert!(auth.properties.is_empty());
    }

    #[test]
    fn test_auth_convenience_methods() {
        assert_eq!(MqttAuth::new_continue_authentication().reason_code, 0x18);
        assert_eq!(MqttAuth::new_re_authenticate().reason_code, 0x19);
    }

    #[test]
    fn test_auth_with_authentication_data() {
        let auth_method = "SCRAM-SHA-1".to_string();
        let auth_data = vec![0x01, 0x02, 0x03, 0x04];
        let auth = MqttAuth::new_with_auth_data(0x18, auth_method.clone(), auth_data.clone());

        assert_eq!(auth.reason_code, 0x18);
        assert_eq!(auth.properties.len(), 2);

        // Check that the properties contain the expected authentication data
        let has_method = auth
            .properties
            .iter()
            .any(|p| matches!(p, Property::AuthenticationMethod(m) if m == &auth_method));
        let has_data = auth
            .properties
            .iter()
            .any(|p| matches!(p, Property::AuthenticationData(d) if d == &auth_data));

        assert!(
            has_method,
            "AUTH packet should contain authentication method"
        );
        assert!(has_data, "AUTH packet should contain authentication data");
    }

    #[test]
    fn test_auth_with_method_only() {
        let auth_method = "SCRAM-SHA-256".to_string();
        let auth = MqttAuth::new_with_method(0x00, auth_method.clone());

        assert_eq!(auth.reason_code, 0x00);
        assert_eq!(auth.properties.len(), 1);

        let has_method = auth
            .properties
            .iter()
            .any(|p| matches!(p, Property::AuthenticationMethod(m) if m == &auth_method));
        assert!(
            has_method,
            "AUTH packet should contain authentication method"
        );
    }

    #[test]
    fn test_auth_continue_with_data() {
        let auth_method = "OAuth".to_string();
        let auth_data = vec![0xAA, 0xBB, 0xCC];
        let auth = MqttAuth::new_continue_with_data(auth_method.clone(), auth_data.clone());

        assert_eq!(auth.reason_code, 0x18);
        assert_eq!(auth.properties.len(), 2);
    }

    #[test]
    fn test_auth_re_auth_with_data() {
        let auth_method = "JWT".to_string();
        let auth_data = vec![0xFF, 0xEE, 0xDD];
        let auth = MqttAuth::new_re_auth_with_data(auth_method.clone(), auth_data.clone());

        assert_eq!(auth.reason_code, 0x19);
        assert_eq!(auth.properties.len(), 2);
    }

    #[test]
    fn test_auth_serialization() {
        let auth = MqttAuth::new_success();
        let bytes = auth.to_bytes().unwrap();

        // Expected: 0xF0 (packet type 15, flags 0) + 0x02 (remaining length 2) + 0x00 (reason code) + 0x00 (no properties)
        assert_eq!(bytes, vec![0xF0, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn test_auth_with_properties_serialization() {
        let auth_method = "PLAIN".to_string();
        let auth = MqttAuth::new_with_method(0x18, auth_method);
        let bytes = auth.to_bytes().unwrap();

        // Should contain packet type, remaining length, reason code, properties length, and property data
        assert!(
            bytes.len() > 4,
            "AUTH packet with properties should be longer than minimal packet"
        );
        assert_eq!(bytes[0], 0xF0); // Packet type and flags
        assert_eq!(bytes[2], 0x18); // Reason code
    }

    #[test]
    fn test_auth_deserialization() {
        let bytes = vec![0xF0, 0x02, 0x00, 0x00]; // AUTH with reason code 0x00, no properties

        match MqttAuth::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(auth), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(auth.reason_code, 0x00);
                assert!(auth.properties.is_empty());
            }
            _ => panic!("Expected AUTH packet"),
        }
    }

    #[test]
    fn test_auth_continue_deserialization() {
        let bytes = vec![0xF0, 0x02, 0x18, 0x00]; // AUTH with reason code 0x18, no properties

        match MqttAuth::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(auth), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(auth.reason_code, 0x18);
                assert!(auth.properties.is_empty());
            }
            _ => panic!("Expected AUTH packet"),
        }
    }

    #[test]
    fn test_auth_roundtrip() {
        let original = MqttAuth::new_continue_authentication();
        let bytes = original.to_bytes().unwrap();

        match MqttAuth::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected AUTH packet"),
        }
    }

    #[test]
    fn test_auth_roundtrip_with_properties() {
        let original = MqttAuth::new_with_method(0x19, "SASL".to_string());
        let bytes = original.to_bytes().unwrap();

        match MqttAuth::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(parsed), _) => {
                assert_eq!(original, parsed);
            }
            _ => panic!("Expected AUTH packet"),
        }
    }

    #[test]
    fn test_auth_invalid_flags() {
        let bytes = vec![0xF1, 0x02, 0x00, 0x00]; // AUTH with invalid flags (should be 0x00)

        match MqttAuth::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("invalid fixed header flags") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with flags message"),
        }
    }

    #[test]
    fn test_auth_incomplete_packet() {
        let bytes = vec![0xF0]; // Incomplete AUTH (missing remaining length)

        match MqttAuth::from_bytes(&bytes) {
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
    fn test_auth_wrong_packet_type() {
        let bytes = vec![0xE0, 0x02, 0x00, 0x00]; // Wrong packet type (14), not AUTH (15)

        match MqttAuth::from_bytes(&bytes) {
            Err(ParseError::InvalidPacketType) => {
                // Expected error
            }
            _ => panic!("Expected InvalidPacketType error"),
        }
    }

    #[test]
    fn test_auth_missing_reason_code() {
        let bytes = vec![0xF0, 0x00]; // AUTH with remaining length 0 (no reason code)

        match MqttAuth::from_bytes(&bytes) {
            Err(ParseError::ParseError(msg)) if msg.contains("must contain a reason code") => {
                // Expected error
            }
            _ => panic!("Expected ParseError with reason code message"),
        }
    }

    #[test]
    fn test_auth_empty_payload() {
        let auth = MqttAuth::new_success();
        assert!(auth.payload().unwrap().is_empty());
    }

    #[test]
    fn test_auth_all_reason_codes() {
        // Test common AUTH reason codes
        let test_cases = vec![
            (0x00, "Success"),
            (0x18, "Continue authentication"),
            (0x19, "Re-authenticate"),
        ];

        for (reason_code, _description) in test_cases {
            let auth = MqttAuth::new(reason_code, Vec::new());
            let bytes = auth.to_bytes().unwrap();

            match MqttAuth::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::Auth(parsed), _) => {
                    assert_eq!(parsed.reason_code, reason_code);
                }
                _ => panic!("Failed to parse AUTH with reason code {:#04X}", reason_code),
            }
        }
    }

    #[test]
    fn test_auth_with_complex_properties() {
        let auth_method = "SCRAM-SHA-256".to_string();
        let auth_data = b"client-first-message".to_vec();
        let reason_string = "Authentication challenge".to_string();

        let properties = vec![
            Property::AuthenticationMethod(auth_method.clone()),
            Property::AuthenticationData(auth_data.clone()),
            Property::ReasonString(reason_string.clone()),
        ];

        let auth = MqttAuth::new(0x18, properties.clone());
        let bytes = auth.to_bytes().unwrap();

        match MqttAuth::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(parsed), _) => {
                assert_eq!(parsed.reason_code, 0x18);
                assert_eq!(parsed.properties, properties);
            }
            _ => panic!("Expected AUTH packet"),
        }
    }

    #[test]
    fn test_auth_variable_header() {
        let auth = MqttAuth::new(0x19, vec![Property::ReasonString("Test".to_string())]);
        let var_header = auth.variable_header().unwrap();

        // Should start with reason code
        assert_eq!(var_header[0], 0x19);

        // Should contain property length and property data
        assert!(var_header.len() > 1);
    }
}
