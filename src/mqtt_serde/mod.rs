// SPDX-License-Identifier: MPL-2.0

pub mod base_data;
pub mod control_packet;
pub mod mqttv3;
pub mod mqttv5;
pub mod parser;

use crate::mqtt_serde::base_data::{BinaryData, TwoByteInteger, Utf8String, VariableByteInteger};
use crate::mqtt_serde::parser::ParseError;
//re export
pub use crate::mqtt_serde::parser::stream::MqttStream;

// MQTT 5.0 Spec, 1.5.4
pub(crate) fn encode_binary_data(data: &[u8]) -> Result<Vec<u8>, ParseError> {
    if data.len() > u16::MAX as usize {
        return Err(ParseError::StringTooLong);
    }
    Ok(BinaryData::encode(data))
}

pub(crate) fn encode_utf8_string(s: &str) -> Result<Vec<u8>, ParseError> {
    if s.len() > u16::MAX as usize {
        return Err(ParseError::StringTooLong);
    }

    // MQTT 5.0 Protocol Compliance: Strict UTF-8 validation
    #[cfg(feature = "strict-protocol-compliance")]
    validate_mqtt_utf8_string(s)?;

    Ok(Utf8String::encode(s))
}

#[cfg(feature = "strict-protocol-compliance")]
fn validate_mqtt_utf8_string(s: &str) -> Result<(), ParseError> {
    // MQTT 5.0 Spec: UTF-8 strings MUST NOT contain these characters:
    // - U+0000 (null character)
    // - U+D800 to U+DFFF (UTF-16 surrogate pairs)
    // - BOM (Byte Order Mark) at the beginning

    for (i, ch) in s.char_indices() {
        let code_point = ch as u32;

        match code_point {
            // Check for null character
            0x0000 => {
                return Err(ParseError::ParseError(
                    "UTF-8 string contains null character (U+0000)".to_string(),
                ));
            }
            // Check for UTF-16 surrogate pairs
            0xD800..=0xDFFF => {
                return Err(ParseError::ParseError(format!(
                    "UTF-8 string contains surrogate character (U+{:04X})",
                    code_point
                )));
            }
            // Check for BOM at the beginning
            0xFEFF if i == 0 => {
                return Err(ParseError::ParseError(
                    "UTF-8 string starts with BOM (U+FEFF)".to_string(),
                ));
            }
            _ => {} // Valid character
        }
    }

    Ok(())
}

/// Validates MQTT topic filter syntax according to MQTT 5.0 specification
#[cfg(feature = "strict-protocol-compliance")]
pub(crate) fn validate_topic_filter(topic_filter: &str) -> Result<(), ParseError> {
    if topic_filter.is_empty() {
        return Err(ParseError::ParseError(
            "Topic filter cannot be empty".to_string(),
        ));
    }

    // Check maximum length (65535 bytes for UTF-8 strings)
    if topic_filter.len() > 65535 {
        return Err(ParseError::StringTooLong);
    }

    // Split into levels for wildcard validation
    let levels: Vec<&str> = topic_filter.split('/').collect();

    for (i, level) in levels.iter().enumerate() {
        // Multi-level wildcard (#) validation
        if level.contains('#') {
            // # must be the only character in the level
            if *level != "#" {
                return Err(ParseError::ParseError(
                    "Multi-level wildcard (#) must be the only character in topic level"
                        .to_string(),
                ));
            }
            // # must be the last level
            if i != levels.len() - 1 {
                return Err(ParseError::ParseError(
                    "Multi-level wildcard (#) must be the last level in topic filter".to_string(),
                ));
            }
        }

        // Single-level wildcard (+) validation
        if level.contains('+') {
            // + must be the only character in the level
            if *level != "+" {
                return Err(ParseError::ParseError(
                    "Single-level wildcard (+) must be the only character in topic level"
                        .to_string(),
                ));
            }
        }
    }

    Ok(())
}

/// Validates shared subscription syntax according to MQTT 5.0 specification
#[cfg(feature = "strict-protocol-compliance")]
pub(crate) fn validate_shared_subscription(topic_filter: &str) -> Result<(), ParseError> {
    if topic_filter.starts_with("$share/") {
        // Extract share name and topic filter
        let parts: Vec<&str> = topic_filter.splitn(3, '/').collect();
        if parts.len() < 3 {
            return Err(ParseError::ParseError(
                "Invalid shared subscription format: must be $share/ShareName/TopicFilter"
                    .to_string(),
            ));
        }

        let share_name = parts[1];
        let actual_topic_filter = parts[2];

        if share_name.is_empty() {
            return Err(ParseError::ParseError(
                "Shared subscription ShareName cannot be empty".to_string(),
            ));
        }

        if actual_topic_filter.is_empty() {
            return Err(ParseError::ParseError(
                "Shared subscription TopicFilter cannot be empty".to_string(),
            ));
        }

        // Validate the actual topic filter part
        validate_topic_filter(actual_topic_filter)?;
    }

    Ok(())
}

// MQTT 5.0 Spec, 1.5.5
fn encode_variable_length(len: usize) -> Vec<u8> {
    VariableByteInteger::encode(len as u32)
}

// for property
fn decode_binary_data(buffer: &[u8]) -> Result<(Vec<u8>, usize), ParseError> {
    BinaryData::decode(buffer)
}

pub fn topic_name(buffer: &[u8]) -> Result<(String, usize), ParseError> {
    Utf8String::decode(buffer)
}

pub fn packet_id(buffer: &[u8]) -> Result<(u16, usize), ParseError> {
    TwoByteInteger::decode(buffer)
}

#[cfg(test)]
mod protocol_compliance_tests {
    use super::*;

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_utf8_string_validation_null_character() {
        let test_string = "hello\u{0000}world";
        let result = encode_utf8_string(test_string);
        assert!(result.is_err());
        if let Err(ParseError::ParseError(msg)) = result {
            assert!(msg.contains("null character"));
        }
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_utf8_string_validation_bom() {
        let test_string = "\u{FEFF}hello";
        let result = encode_utf8_string(test_string);
        assert!(result.is_err());
        if let Err(ParseError::ParseError(msg)) = result {
            assert!(msg.contains("BOM"));
        }
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_utf8_string_validation_valid() {
        let test_string = "hello_world_123_åäö";
        let result = encode_utf8_string(test_string);
        assert!(result.is_ok());
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_topic_filter_validation_empty() {
        let result = validate_topic_filter("");
        assert!(result.is_err());
        if let Err(ParseError::ParseError(msg)) = result {
            assert!(msg.contains("empty"));
        }
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_topic_filter_validation_invalid_multilevel_wildcard() {
        let result = validate_topic_filter("home/temperature/#/extra");
        assert!(result.is_err());
        if let Err(ParseError::ParseError(msg)) = result {
            assert!(msg.contains("last level"));
        }
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_topic_filter_validation_invalid_multilevel_wildcard_mixed() {
        let result = validate_topic_filter("home/#extra");
        assert!(result.is_err());
        if let Err(ParseError::ParseError(msg)) = result {
            assert!(msg.contains("only character"));
        }
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_topic_filter_validation_invalid_single_level_wildcard() {
        let result = validate_topic_filter("home/+extra");
        assert!(result.is_err());
        if let Err(ParseError::ParseError(msg)) = result {
            assert!(msg.contains("only character"));
        }
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_topic_filter_validation_valid() {
        assert!(validate_topic_filter("home/temperature").is_ok());
        assert!(validate_topic_filter("home/+/temperature").is_ok());
        assert!(validate_topic_filter("home/#").is_ok());
        assert!(validate_topic_filter("+/temperature").is_ok());
        assert!(validate_topic_filter("#").is_ok());
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_shared_subscription_validation_valid() {
        assert!(validate_shared_subscription("$share/group1/home/temperature").is_ok());
        assert!(validate_shared_subscription("$share/mygroup/home/+/sensor").is_ok());
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_shared_subscription_validation_invalid() {
        // Missing share name
        let result = validate_shared_subscription("$share//home/temperature");
        assert!(result.is_err());

        // Missing topic filter
        let result = validate_shared_subscription("$share/group1/");
        assert!(result.is_err());

        // Incomplete format
        let result = validate_shared_subscription("$share/group1");
        assert!(result.is_err());
    }

    #[test]
    fn test_utf8_string_validation_without_strict_compliance() {
        // Without strict compliance, these should pass
        let test_string = "normal_string";
        let result = encode_utf8_string(test_string);
        assert!(result.is_ok());
    }
}
