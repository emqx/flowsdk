// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::mqttv5::packet_id_ack::v5_packet_id_ack;

v5_packet_id_ack!(MqttPubAck, PUBACK, 0x00, "PUBACK", PubAck5, false);

#[cfg(test)]
mod tests {
    use super::MqttPubAck;
    use crate::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::mqttv5::common::properties::Property;
    use crate::mqtt_serde::parser::{ParseError, ParseOk};
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
