// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv5::packet_id_ack::v5_packet_id_ack;

v5_packet_id_ack!(MqttPubRec, PUBREC, 0x00, "PUBREC", PubRec5, false);

#[cfg(test)]
mod tests {
    use super::MqttPubRec;
    use crate::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::mqttv5::common::properties::Property;
    use crate::mqtt_serde::parser::ParseOk;
    #[test]
    fn test_pubrec_minimal_success() {
        // Test minimal PUBREC (success, no properties)
        let pubrec = MqttPubRec::new_success(0x1234);
        let bytes = pubrec.to_bytes().unwrap();

        // Expected: [0x50, 0x02, 0x12, 0x34]
        // 0x50 = PUBREC packet type (5 << 4)
        // 0x02 = remaining length (2 bytes for packet ID only)
        // 0x12, 0x34 = packet ID in big-endian
        let expected = vec![0x50, 0x02, 0x12, 0x34];
        assert_eq!(bytes, expected);

        // Test round-trip parsing
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec5(parsed_pubrec), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(parsed_pubrec.packet_id, 0x1234);
                assert_eq!(parsed_pubrec.reason_code, 0x00);
                assert!(parsed_pubrec.properties.is_empty());
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_with_reason_code() {
        // Test PUBREC with reason code but no properties
        let pubrec = MqttPubRec::new(0x5678, 0x80, Vec::new()); // 0x80 = Unspecified error
        let bytes = pubrec.to_bytes().unwrap();

        // Expected: [0x50, 0x04, 0x56, 0x78, 0x80, 0x00]
        // 0x50 = PUBREC packet type
        // 0x04 = remaining length (2 bytes packet ID + 1 byte reason code + 1 byte properties length)
        // 0x56, 0x78 = packet ID
        // 0x80 = reason code (Unspecified error)
        // 0x00 = properties length (empty)
        let expected = vec![0x50, 0x04, 0x56, 0x78, 0x80, 0x00];
        assert_eq!(bytes, expected);

        // Test parsing
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec5(parsed_pubrec), consumed) => {
                assert_eq!(consumed, 6);
                assert_eq!(parsed_pubrec.packet_id, 0x5678);
                assert_eq!(parsed_pubrec.reason_code, 0x80);
                assert!(parsed_pubrec.properties.is_empty());
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_with_properties() {
        // Test PUBREC with properties
        let properties = vec![
            Property::ReasonString("Test error".to_string()),
            Property::UserProperty("key".to_string(), "value".to_string()),
        ];
        let pubrec = MqttPubRec::new(0xABCD, 0x83, properties); // 0x83 = Implementation specific error
        let bytes = pubrec.to_bytes().unwrap();

        // Test that it can be parsed back
        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec5(parsed_pubrec), _) => {
                assert_eq!(parsed_pubrec.packet_id, 0xABCD);
                assert_eq!(parsed_pubrec.reason_code, 0x83);
                assert_eq!(parsed_pubrec.properties.len(), 2);
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_parsing_minimal() {
        // Test parsing minimal PUBREC packet (2 bytes remaining length)
        let bytes = vec![0x50, 0x02, 0x00, 0x01]; // packet ID = 1

        match MqttPubRec::from_bytes(&bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PubRec5(pubrec), consumed) => {
                assert_eq!(consumed, 4);
                assert_eq!(pubrec.packet_id, 1);
                assert_eq!(pubrec.reason_code, 0x00); // Default success
                assert!(pubrec.properties.is_empty());
            }
            _ => panic!("Expected PUBREC packet"),
        }
    }

    #[test]
    fn test_pubrec_error_conditions() {
        // Test various PUBREC reason codes
        let error_codes = vec![
            0x10, // No matching subscribers
            0x80, // Unspecified error
            0x83, // Implementation specific error
            0x91, // Quota exceeded
            0x97, // Payload format invalid
        ];

        for &error_code in &error_codes {
            let pubrec = MqttPubRec::new_error(0x1000, error_code, Vec::new());
            let bytes = pubrec.to_bytes().unwrap();

            match MqttPubRec::from_bytes(&bytes).unwrap() {
                ParseOk::Packet(MqttPacket::PubRec5(parsed), _) => {
                    assert_eq!(parsed.packet_id, 0x1000);
                    assert_eq!(parsed.reason_code, error_code);
                }
                _ => panic!("Expected PUBREC packet for error code {:#x}", error_code),
            }
        }
    }

    #[test]
    fn test_parse_pubrec() {
        let buffer: [u8; 6] = [0x50, 0x04, 0x00, 0x02, 0x10, 0x00];
        let result = MqttPubRec::from_bytes(&buffer);
        assert!(result.is_ok());
        let (parsed, _) = match result.unwrap() {
            ParseOk::Packet(packet, _) => (packet, 0),
            _ => panic!("Invalid packet"),
        };
        assert_eq!(
            parsed,
            MqttPacket::PubRec5(MqttPubRec::new(2_u16, 0x10, vec![]))
        );
    }
}
