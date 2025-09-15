/// Integration test to verify the complete subscription workflow
#[cfg(test)]
mod tests {
    use crate::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
    use crate::mqtt_serde::mqttv5::auth::MqttAuth;
    use crate::mqtt_serde::mqttv5::common::properties::Property;
    use crate::mqtt_serde::mqttv5::disconnect::MqttDisconnect;
    use crate::mqtt_serde::mqttv5::pingreq::MqttPingReq;
    use crate::mqtt_serde::mqttv5::pingresp::MqttPingResp;
    use crate::mqtt_serde::mqttv5::suback::MqttSubAck;
    use crate::mqtt_serde::mqttv5::subscribe::{MqttSubscribe, TopicSubscription};
    use crate::mqtt_serde::mqttv5::unsuback::MqttUnsubAck;
    use crate::mqtt_serde::mqttv5::unsubscribe::MqttUnsubscribe;
    use crate::mqtt_serde::parser::ParseOk;

    #[test]
    fn test_subscription_workflow_integration() {
        // Step 1: Create a SUBSCRIBE packet
        let subscriptions = vec![
            TopicSubscription::new("home/temperature".to_string(), 1, false, false, 0),
            TopicSubscription::new("home/humidity".to_string(), 2, true, false, 1),
        ];
        let subscribe = MqttSubscribe::new_simple(1234, subscriptions);

        // Step 2: Serialize SUBSCRIBE packet
        let subscribe_bytes = subscribe.to_bytes().unwrap();

        // Step 3: Parse SUBSCRIBE packet
        match MqttPacket::from_bytes_v5(&subscribe_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Subscribe5(parsed_subscribe), _) => {
                assert_eq!(parsed_subscribe.packet_id, 1234);
                assert_eq!(parsed_subscribe.subscriptions.len(), 2);

                // Step 4: Create SUBACK response
                let reason_codes = vec![0x01, 0x02]; // Maximum QoS 1 and 2 granted
                let suback = MqttSubAck::new_simple(parsed_subscribe.packet_id, reason_codes);

                // Step 5: Serialize SUBACK packet
                let suback_bytes = suback.to_bytes().unwrap();

                // Step 6: Parse SUBACK packet
                match MqttPacket::from_bytes_v5(&suback_bytes).unwrap() {
                    ParseOk::Packet(MqttPacket::SubAck5(parsed_suback), _) => {
                        assert_eq!(parsed_suback.packet_id, 1234);
                        assert_eq!(parsed_suback.reason_codes, vec![0x01, 0x02]);
                    }
                    _ => panic!("Expected SUBACK packet"),
                }
            }
            _ => panic!("Expected SUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_unsubscription_workflow_integration() {
        // Step 1: Create an UNSUBSCRIBE packet
        let topic_filters = vec!["home/temperature".to_string(), "home/humidity".to_string()];
        let unsubscribe = MqttUnsubscribe::new_simple(5678, topic_filters);

        // Step 2: Serialize UNSUBSCRIBE packet
        let unsubscribe_bytes = unsubscribe.to_bytes().unwrap();

        // Step 3: Parse UNSUBSCRIBE packet
        match MqttPacket::from_bytes_v5(&unsubscribe_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Unsubscribe5(parsed_unsubscribe), _) => {
                assert_eq!(parsed_unsubscribe.packet_id, 5678);
                assert_eq!(parsed_unsubscribe.topic_filters.len(), 2);

                // Step 4: Create UNSUBACK response
                let reason_codes = vec![0x00, 0x00]; // Success for both topics
                let unsuback = MqttUnsubAck::new_simple(parsed_unsubscribe.packet_id, reason_codes);

                // Step 5: Serialize UNSUBACK packet
                let unsuback_bytes = unsuback.to_bytes().unwrap();

                // Step 6: Parse UNSUBACK packet
                match MqttPacket::from_bytes_v5(&unsuback_bytes).unwrap() {
                    ParseOk::Packet(MqttPacket::UnsubAck5(parsed_unsuback), _) => {
                        assert_eq!(parsed_unsuback.packet_id, 5678);
                        assert_eq!(parsed_unsuback.reason_codes, vec![0x00, 0x00]);
                    }
                    _ => panic!("Expected UNSUBACK packet"),
                }
            }
            _ => panic!("Expected UNSUBSCRIBE packet"),
        }
    }

    #[test]
    fn test_all_new_packet_types_through_control_system() {
        // Test that all new packet types can be created, serialized, and parsed through the control packet system

        // SUBSCRIBE
        let subscribe = MqttSubscribe::new_simple(
            1,
            vec![TopicSubscription::new_simple("test".to_string(), 1)],
        );
        let subscribe_bytes = subscribe.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&subscribe_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::Subscribe5(_), _)
        ));

        // SUBACK
        let suback = MqttSubAck::new_simple(2, vec![0x01]);
        let suback_bytes = suback.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&suback_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::SubAck5(_), _)
        ));

        // UNSUBSCRIBE
        let unsubscribe = MqttUnsubscribe::new_simple(3, vec!["test".to_string()]);
        let unsubscribe_bytes = unsubscribe.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&unsubscribe_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::Unsubscribe5(_), _)
        ));

        // UNSUBACK
        let unsuback = MqttUnsubAck::new_simple(4, vec![0x00]);
        let unsuback_bytes = unsuback.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&unsuback_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::UnsubAck5(_), _)
        ));

        // PINGREQ
        let pingreq = MqttPingReq::new();
        let pingreq_bytes = pingreq.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&pingreq_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::PingReq5(_), _)
        ));

        // PINGRESP
        let pingresp = MqttPingResp::new();
        let pingresp_bytes = pingresp.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&pingresp_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::PingResp5(_), _)
        ));

        // DISCONNECT
        let disconnect = MqttDisconnect::new_normal();
        let disconnect_bytes = disconnect.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&disconnect_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::Disconnect5(_), _)
        ));

        // AUTH
        let auth = MqttAuth::new_continue_authentication();
        let auth_bytes = auth.to_bytes().unwrap();
        assert!(matches!(
            MqttPacket::from_bytes_v5(&auth_bytes).unwrap(),
            ParseOk::Packet(MqttPacket::Auth(_), _)
        ));
    }

    #[test]
    fn test_ping_workflow_integration() {
        // Step 1: Client sends PINGREQ
        let pingreq = MqttPingReq::new();
        let pingreq_bytes = pingreq.to_bytes().unwrap();

        // Step 2: Parse PINGREQ packet
        match MqttPacket::from_bytes_v5(&pingreq_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::PingReq5(_parsed_pingreq), _) => {
                // Step 3: Server responds with PINGRESP
                let pingresp = MqttPingResp::new();
                let pingresp_bytes = pingresp.to_bytes().unwrap();

                // Step 4: Parse PINGRESP packet
                match MqttPacket::from_bytes_v5(&pingresp_bytes).unwrap() {
                    ParseOk::Packet(MqttPacket::PingResp5(_parsed_pingresp), _) => {
                        // Successful ping workflow
                        assert_eq!(pingreq_bytes, vec![0xC0, 0x00]);
                        assert_eq!(pingresp_bytes, vec![0xD0, 0x00]);
                    }
                    _ => panic!("Expected PINGRESP packet"),
                }
            }
            _ => panic!("Expected PINGREQ packet"),
        }
    }

    #[test]
    fn test_disconnect_workflow_integration() {
        // Test different types of DISCONNECT scenarios

        // Normal disconnection
        let disconnect_normal = MqttDisconnect::new_normal();
        let disconnect_bytes = disconnect_normal.to_bytes().unwrap();

        match MqttPacket::from_bytes_v5(&disconnect_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect5(parsed_disconnect), _) => {
                assert_eq!(parsed_disconnect.reason_code, 0x00);
                assert!(parsed_disconnect.properties.is_empty());
                assert_eq!(disconnect_bytes, vec![0xE0, 0x00]); // Minimal packet
            }
            _ => panic!("Expected DISCONNECT packet"),
        }

        // Error disconnection
        let disconnect_error = MqttDisconnect::new_protocol_error();
        let disconnect_error_bytes = disconnect_error.to_bytes().unwrap();

        match MqttPacket::from_bytes_v5(&disconnect_error_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Disconnect5(parsed_disconnect), _) => {
                assert_eq!(parsed_disconnect.reason_code, 0x82);
                assert!(parsed_disconnect.properties.is_empty());
                assert_eq!(disconnect_error_bytes, vec![0xE0, 0x02, 0x82, 0x00]);
                // With reason code
            }
            _ => panic!("Expected DISCONNECT packet"),
        }
    }

    #[test]
    fn test_auth_workflow_integration() {
        // Test complete authentication workflow

        // Step 1: Client initiates authentication challenge
        let auth_method = "SCRAM-SHA-256".to_string();
        let client_first_data = b"client-first-message".to_vec();
        let auth_continue =
            MqttAuth::new_continue_with_data(auth_method.clone(), client_first_data.clone());
        let auth_bytes = auth_continue.to_bytes().unwrap();

        // Step 2: Parse AUTH packet through control system
        match MqttPacket::from_bytes_v5(&auth_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(parsed_auth), _) => {
                assert_eq!(parsed_auth.reason_code, 0x18); // Continue authentication
                assert_eq!(parsed_auth.properties.len(), 2);

                // Check authentication method and data are preserved
                let has_method = parsed_auth
                    .properties
                    .iter()
                    .any(|p| matches!(p, Property::AuthenticationMethod(m) if m == &auth_method));
                let has_data = parsed_auth.properties.iter().any(
                    |p| matches!(p, Property::AuthenticationData(d) if d == &client_first_data),
                );

                assert!(
                    has_method,
                    "AUTH packet should contain authentication method"
                );
                assert!(has_data, "AUTH packet should contain authentication data");
            }
            _ => panic!("Expected AUTH packet"),
        }

        // Step 3: Server responds with challenge
        let server_challenge = b"server-challenge-data".to_vec();
        let server_auth =
            MqttAuth::new_continue_with_data(auth_method.clone(), server_challenge.clone());
        let server_auth_bytes = server_auth.to_bytes().unwrap();

        // Step 4: Parse server's AUTH response
        match MqttPacket::from_bytes_v5(&server_auth_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(parsed_server_auth), _) => {
                assert_eq!(parsed_server_auth.reason_code, 0x18);

                let has_challenge = parsed_server_auth.properties.iter().any(
                    |p| matches!(p, Property::AuthenticationData(d) if d == &server_challenge),
                );
                assert!(has_challenge, "Server AUTH should contain challenge data");
            }
            _ => panic!("Expected AUTH packet from server"),
        }

        // Step 5: Final authentication success
        let auth_success = MqttAuth::new_success();
        let success_bytes = auth_success.to_bytes().unwrap();

        match MqttPacket::from_bytes_v5(&success_bytes).unwrap() {
            ParseOk::Packet(MqttPacket::Auth(parsed_success), _) => {
                assert_eq!(parsed_success.reason_code, 0x00); // Success
                assert_eq!(success_bytes, vec![0xF0, 0x02, 0x00, 0x00]); // Minimal success AUTH
            }
            _ => panic!("Expected AUTH success packet"),
        }
    }
}
