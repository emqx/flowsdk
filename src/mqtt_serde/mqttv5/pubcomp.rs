// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv5::packet_id_ack::v5_packet_id_ack;

v5_packet_id_ack!(MqttPubComp, PUBCOMP, 0x00, "PUBCOMP", PubComp5, false);

#[cfg(test)]
mod tests {
    use super::MqttPubComp;
    use crate::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::mqttv5::common::properties::Property;
    use crate::mqtt_serde::parser::ParseOk;
    #[test]
    fn test_pubcomp_minimal_success() {
        // Test minimal PUBCOMP (success, no properties)
        let pubcomp = MqttPubComp::new_success(0x1234);
        let bytes = pubcomp.to_bytes().unwrap();

        // Expected: [0x70, 0x02, 0x12, 0x34]
        // 0x70 = PUBCOMP packet type (7 << 4)
        // 0x02 = remaining length (2 bytes for packet ID only)
        // 0x12, 0x34 = packet ID in big-endian
        let expected = vec![0x70, 0x02, 0x12, 0x34];
        assert_eq!(bytes, expected);

        // Test round-trip parsing
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp5(parsed_pubcomp), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(parsed_pubcomp.packet_id, 0x1234);
                assert_eq!(parsed_pubcomp.reason_code, 0x00);
                assert!(parsed_pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_with_reason_code() {
        // Test PUBCOMP with reason code but no properties
        let pubcomp = MqttPubComp::new(0x5678, 0x92, Vec::new()); // 0x92 = Packet Identifier not found
        let bytes = pubcomp.to_bytes().unwrap();

        // Expected: [0x70, 0x04, 0x56, 0x78, 0x92, 0x00]
        // 0x70 = PUBCOMP packet type
        // 0x04 = remaining length (2 bytes packet ID + 1 byte reason code + 1 byte properties length)
        // 0x56, 0x78 = packet ID
        // 0x92 = reason code (Packet Identifier not found)
        // 0x00 = properties length (empty)
        let expected = vec![0x70, 0x04, 0x56, 0x78, 0x92, 0x00];
        assert_eq!(bytes, expected);

        // Test parsing
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp5(parsed_pubcomp), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(parsed_pubcomp.packet_id, 0x5678);
                assert_eq!(parsed_pubcomp.reason_code, 0x92);
                assert!(parsed_pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_with_properties() {
        // Test PUBCOMP with properties
        let properties = vec![
            Property::ReasonString("QoS 2 flow completed".to_string()),
            Property::UserProperty("flow".to_string(), "qos2".to_string()),
        ];
        let pubcomp = MqttPubComp::new(0xABCD, 0x00, properties); // Success with properties
        let bytes = pubcomp.to_bytes().unwrap();

        // Test that it can be parsed back
        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp5(parsed_pubcomp), _) => {
                assert_eq!(parsed_pubcomp.packet_id, 0xABCD);
                assert_eq!(parsed_pubcomp.reason_code, 0x00);
                assert_eq!(parsed_pubcomp.properties.len(), 2);
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_parsing_minimal() {
        // Test parsing minimal PUBCOMP packet (2 bytes remaining length)
        let bytes = vec![0x70, 0x02, 0x00, 0x01]; // packet ID = 1

        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp5(pubcomp), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubcomp.packet_id, 1);
                assert_eq!(pubcomp.reason_code, 0x00); // Default success
                assert!(pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_pubcomp_error_conditions() {
        // Test various PUBCOMP reason codes
        let error_codes = vec![
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x92, // Packet Identifier not found
        ];

        for &error_code in &error_codes {
            let pubcomp = MqttPubComp::new_error(0x3000, error_code, Vec::new());
            let bytes = pubcomp.to_bytes().unwrap();

            match MqttPubComp::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::PubComp5(parsed), _) => {
                    assert_eq!(parsed.packet_id, 0x3000);
                    assert_eq!(parsed.reason_code, error_code);
                }
                _ => panic!("Expected PUBCOMP packet for error code {:#x}", error_code),
            }
        }
    }

    #[test]
    fn test_parse_pubcomp() {
        // Test parsing a PUBCOMP packet
        let buffer: [u8; 6] = [0x70, 0x04, 0x00, 0x05, 0x00, 0x00];
        let result = MqttPubComp::from_bytes(&buffer);
        assert!(result.is_ok());

        match result.unwrap() {
            ParseOk::Packet(MqttPacket::PubComp5(pubcomp), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(pubcomp.packet_id, 5);
                assert_eq!(pubcomp.reason_code, 0x00); // Success
                assert!(pubcomp.properties.is_empty());
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_qos2_flow_completion() {
        // Test creating PUBCOMP as part of QoS 2 flow completion
        // This would typically be in response to a PUBREL
        let packet_id = 0x1A2B;

        // Successful completion
        let pubcomp_success = MqttPubComp::new_success(packet_id);
        assert_eq!(pubcomp_success.packet_id, packet_id);
        assert_eq!(pubcomp_success.reason_code, 0x00);

        // Error during completion
        let pubcomp_error = MqttPubComp::new_error(
            packet_id,
            0x92, // Packet Identifier not found
            vec![Property::ReasonString("PUBREL not found".to_string())],
        );
        assert_eq!(pubcomp_error.packet_id, packet_id);
        assert_eq!(pubcomp_error.reason_code, 0x92);
        assert_eq!(pubcomp_error.properties.len(), 1);
    }

    #[test]
    fn test_pubcomp_roundtrip_with_properties() {
        // Test complete roundtrip with properties
        let original_pubcomp = MqttPubComp::new(
            0xDEAD,
            0x83,
            vec![
                Property::ReasonString("Processing completed with warnings".to_string()),
                Property::UserProperty("timestamp".to_string(), "2025-01-01T12:00:00Z".to_string()),
            ],
        );

        let bytes = original_pubcomp.to_bytes().unwrap();

        match MqttPubComp::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubComp5(parsed_pubcomp), _) => {
                assert_eq!(parsed_pubcomp.packet_id, original_pubcomp.packet_id);
                assert_eq!(parsed_pubcomp.reason_code, original_pubcomp.reason_code);
                assert_eq!(
                    parsed_pubcomp.properties.len(),
                    original_pubcomp.properties.len()
                );
            }
            _ => panic!("Expected PUBCOMP packet"),
        }
    }

    #[test]
    fn test_parse_pubcomp_2() {
        let packet = vec![0x70, 0x03, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00];
        let result = MqttPubComp::from_bytes(&packet);
        match result {
            Ok(ParseOk::Packet(MqttPacket::PubComp5(pubcomp), _consumed)) => {
                assert_eq!(pubcomp.reason_code, 0);
                assert_eq!(pubcomp.properties, vec![]);
            }
            _ => panic!("Invalid result: {:?}", result),
        }
    }

    #[test]
    fn test_parse_pubcomp_with_properties() {
        let packet_bytes: [u8; 6] = [0x70, 0x04, 0x00, 0x02, 0x00, 0x00];

        let result = MqttPubComp::from_bytes(&packet_bytes);
        match result {
            Ok(ParseOk::Packet(MqttPacket::PubComp5(pubcomp), _consumed)) => {
                assert_eq!(pubcomp.packet_id, 2);
                assert_eq!(pubcomp.reason_code, 0);
                assert_eq!(pubcomp.properties, vec![]);
            }
            _ => panic!("Invalid result: {:?}", result),
        }
    }
}
