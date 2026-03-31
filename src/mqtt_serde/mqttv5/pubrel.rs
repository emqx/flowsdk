// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv5::packet_id_ack::v5_packet_id_ack;

v5_packet_id_ack!(MqttPubRel, PUBREL, 0x02, "PUBREL", PubRel5, true);

#[cfg(test)]
mod tests {
    use super::MqttPubRel;
    use crate::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::mqttv5::common::properties::Property;
    use crate::mqtt_serde::parser::{ParseError, ParseOk};
    #[test]
    fn test_pubrel_minimal_success() {
        // Test minimal PUBREL (success, no properties)
        let pubrel = MqttPubRel::new_success(0x1234);
        let bytes = pubrel.to_bytes().unwrap();

        // Expected: [0x62, 0x02, 0x12, 0x34]
        // 0x62 = PUBREL packet type (6 << 4) | flags (0x02)
        // 0x02 = remaining length (2 bytes for packet ID only)
        // 0x12, 0x34 = packet ID in big-endian
        let expected = vec![0x62, 0x02, 0x12, 0x34];
        assert_eq!(bytes, expected);

        // Test round-trip parsing
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel5(parsed_pubrel), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(parsed_pubrel.packet_id, 0x1234);
                assert_eq!(parsed_pubrel.reason_code, 0x00);
                assert!(parsed_pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_with_reason_code() {
        // Test PUBREL with reason code but no properties
        let pubrel = MqttPubRel::new(0x5678, 0x92, Vec::new()); // 0x92 = Packet Identifier not found
        let bytes = pubrel.to_bytes().unwrap();

        // Expected: [0x62, 0x04, 0x56, 0x78, 0x92, 0x00]
        // 0x62 = PUBREL packet type with flags
        // 0x04 = remaining length (2 bytes packet ID + 1 byte reason code + 1 byte properties length)
        // 0x56, 0x78 = packet ID
        // 0x92 = reason code (Packet Identifier not found)
        // 0x00 = properties length (empty)
        let expected = vec![0x62, 0x04, 0x56, 0x78, 0x92, 0x00];
        assert_eq!(bytes, expected);

        // Test parsing
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel5(parsed_pubrel), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(parsed_pubrel.packet_id, 0x5678);
                assert_eq!(parsed_pubrel.reason_code, 0x92);
                assert!(parsed_pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_with_properties() {
        // Test PUBREL with properties
        let properties = vec![
            Property::ReasonString("Packet processing error".to_string()),
            Property::UserProperty("context".to_string(), "test".to_string()),
        ];
        let pubrel = MqttPubRel::new(0xABCD, 0x80, properties); // 0x80 = Unspecified error
        let bytes = pubrel.to_bytes().unwrap();

        // Test that it can be parsed back
        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel5(parsed_pubrel), _) => {
                assert_eq!(parsed_pubrel.packet_id, 0xABCD);
                assert_eq!(parsed_pubrel.reason_code, 0x80);
                assert_eq!(parsed_pubrel.properties.len(), 2);
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_pubrel_parsing_minimal() {
        // Test parsing minimal PUBREL packet (2 bytes remaining length)
        let bytes = vec![0x62, 0x02, 0x00, 0x01]; // packet ID = 1

        match MqttPubRel::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRel5(pubrel), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubrel.packet_id, 1);
                assert_eq!(pubrel.reason_code, 0x00); // Default success
                assert!(pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[cfg(feature = "strict-protocol-compliance")]
    #[test]
    fn test_pubrel_flags_validation() {
        // Test that PUBREL with incorrect flags is rejected
        let bytes_wrong_flags = vec![0x60, 0x02, 0x00, 0x01]; // 0x60 = wrong flags (should be 0x62)

        match MqttPubRel::from_bytes(&bytes_wrong_flags) {
            Err(ParseError::ParseError(msg)) => {
                assert!(msg.contains("Invalid PUBREL flags"));
            }
            _ => panic!("Expected parse error for wrong flags"),
        }
    }

    #[test]
    fn test_pubrel_error_conditions() {
        // Test various PUBREL reason codes
        let error_codes = vec![
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x92, // Packet Identifier not found
        ];

        for &error_code in &error_codes {
            let pubrel = MqttPubRel::new_error(0x2000, error_code, Vec::new());
            let bytes = pubrel.to_bytes().unwrap();

            match MqttPubRel::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::PubRel5(parsed), _) => {
                    assert_eq!(parsed.packet_id, 0x2000);
                    assert_eq!(parsed.reason_code, error_code);
                }
                _ => panic!("Expected PUBREL packet for error code {:#x}", error_code),
            }
        }
    }

    #[test]
    fn test_pubrel_flags() {
        // Test that PUBREL has the correct flags (0x02)
        let pubrel = MqttPubRel::new_success(0x1000);
        assert_eq!(pubrel.flags(), 0x02);
    }

    #[test]
    fn test_parse_pubrel_extended() {
        // Test parsing a PUBREL with reason code and properties
        let buffer: [u8; 6] = [0x62, 0x04, 0x00, 0x10, 0x92, 0x00];
        let result = MqttPubRel::from_bytes(&buffer);
        assert!(result.is_ok());

        match result.unwrap() {
            ParseOk::Packet(MqttPacket::PubRel5(pubrel), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(pubrel.packet_id, 16);
                assert_eq!(pubrel.reason_code, 0x92); // Packet Identifier not found
                assert!(pubrel.properties.is_empty());
            }
            _ => panic!("Expected PUBREL packet"),
        }
    }

    #[test]
    fn test_parse_pubrel() {
        let packet_bytes: [u8; 6] = [0x62, 0x04, 0x00, 0x03, 0x00, 0x00];

        let result = MqttPubRel::from_bytes(&packet_bytes);
        match result {
            Ok(ParseOk::Packet(packet, consumed)) => {
                assert_eq!(consumed, 6);
                assert_eq!(packet, MqttPacket::PubRel5(MqttPubRel::new(3, 0, vec![])));
            }
            _ => panic!("Invalid result: {:?}", result),
        }
    }
}
