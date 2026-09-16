use flowsdk::mqtt_serde::{
    control_packet::MqttPacket, mqttv3, mqttv5, mqttv5::common::properties::Property,
    parser::ParseOk,
};
use mqtt_grpc_proxy::{
    convert_mqtt_to_stream_message, convert_stream_message_to_mqtt_packet,
    mqtt_unified_pb::{self, MessageDirection},
    mqttv3pb, mqttv5pb, MqttConnectPacket,
};

fn assert_round_trip(packet: MqttPacket) {
    for direction in [
        MessageDirection::ClientToBroker,
        MessageDirection::BrokerToClient,
    ] {
        let message = convert_mqtt_to_stream_message(&packet, 42, direction).unwrap();
        assert_eq!(message.sequence_id, 42);
        assert_eq!(message.direction, direction as i32);
        let restored = convert_stream_message_to_mqtt_packet(message).unwrap();
        assert_eq!(
            restored.to_bytes().unwrap(),
            packet.to_bytes().unwrap(),
            "{packet:?}"
        );
    }
}

#[test]
fn all_packet_types_round_trip_between_mqtt_and_protobuf() {
    for version in [3, 5] {
        let mut packets = vec![
            vec![0x20, 2, 1, 0],
            vec![0x3b, 7, 0, 1, b't', 0, 42, 0, 255],
            vec![0x40, 2, 0, 42],
            vec![0x50, 2, 0, 42],
            vec![0x62, 2, 0, 42],
            vec![0x70, 2, 0, 42],
            vec![0x82, 6, 0, 42, 0, 1, b't', 2],
            vec![0x90, 4, 0, 42, 1, 0x80],
            vec![0xa2, 5, 0, 42, 0, 1, b't'],
            vec![0xb0, 2, 0, 42],
            vec![0xc0, 0],
            vec![0xd0, 0],
            vec![0xe0, 0],
        ];
        if version == 5 {
            for (index, property_offset) in [(0, 4), (1, 7), (6, 4), (7, 4), (8, 4), (9, 4)] {
                packets[index].insert(property_offset, 0);
                packets[index][1] += 1;
            }
            packets[9].push(0x11);
            packets[9][1] += 1;
            packets.push(vec![0xf0, 0]);
        }
        for bytes in packets {
            let ParseOk::Packet(packet, _) = MqttPacket::from_bytes_with_version(&bytes, version)
                .unwrap_or_else(|error| panic!("MQTT {version} fixture {bytes:?}: {error:?}"))
            else {
                panic!("Incomplete fixture: {bytes:?}")
            };
            assert_round_trip(packet);
        }
    }
}

#[test]
fn connect_round_trip_keeps_credentials_and_will_properties() {
    for include_will in [false, true] {
        let mut connect = mqttv3::connect::MqttConnect::new("v3-client".into(), 30, false);
        connect.username = Some("username".into());
        connect.password = Some(vec![0, 255]);
        if include_will {
            connect.will = Some(mqttv3::connect::Will {
                topic: "status".into(),
                message: vec![0, 255],
                qos: 2,
                retain: true,
            });
        }
        let wrapper = MqttConnectPacket::V3(connect.clone());
        assert_eq!(wrapper.client_id(), "v3-client");
        assert_eq!(wrapper.version(), 3);
        assert_round_trip(MqttPacket::Connect3(connect));

        let mut will = mqttv5::will::Will::new("status".into(), vec![0, 255], 2, true);
        will.properties = mqttv5::will::WillProperties {
            will_delay_interval: Some(10),
            payload_format_indicator: Some(0),
            message_expiry_interval: Some(20),
            content_type: Some("binary".into()),
            response_topic: Some("reply".into()),
            correlation_data: Some(vec![0, 255]),
            user_properties: vec![
                Property::UserProperty("key".into(), "one".into()),
                Property::UserProperty("key".into(), "two".into()),
            ],
        };
        let connect = mqttv5::connect::MqttConnect::new(
            "v5-client".into(),
            Some("username".into()),
            Some(vec![0, 255]),
            include_will.then_some(will),
            30,
            false,
            vec![
                Property::SessionExpiryInterval(60),
                Property::ReceiveMaximum(10),
            ],
        );
        let wrapper = MqttConnectPacket::V5(connect.clone());
        assert_eq!(wrapper.client_id(), "v5-client");
        assert_eq!(wrapper.version(), 5);
        assert_round_trip(MqttPacket::Connect5(connect));
    }
}

#[test]
fn properties_preserve_widths_binary_values_and_repeated_keys() {
    let properties = vec![
        Property::PayloadFormatIndicator(0),
        Property::PayloadFormatIndicator(1),
        Property::MessageExpiryInterval(u32::MAX),
        Property::ContentType("binary".into()),
        Property::ResponseTopic("reply".into()),
        Property::CorrelationData(vec![0, 255]),
        Property::SubscriptionIdentifier(268_435_455),
        Property::SessionExpiryInterval(u32::MAX),
        Property::AssignedClientIdentifier("client".into()),
        Property::ServerKeepAlive(u16::MAX),
        Property::AuthenticationMethod("method".into()),
        Property::AuthenticationData(vec![0, 255]),
        Property::RequestProblemInformation(0),
        Property::RequestProblemInformation(1),
        Property::WillDelayInterval(u32::MAX),
        Property::RequestResponseInformation(0),
        Property::RequestResponseInformation(1),
        Property::ResponseInformation("info".into()),
        Property::ServerReference("server".into()),
        Property::ReasonString("reason".into()),
        Property::ReceiveMaximum(u16::MAX),
        Property::TopicAliasMaximum(u16::MAX),
        Property::TopicAlias(1),
        Property::MaximumQoS(1),
        Property::RetainAvailable(0),
        Property::RetainAvailable(1),
        Property::UserProperty("key".into(), "one".into()),
        Property::UserProperty("key".into(), "two".into()),
        Property::MaximumPacketSize(u32::MAX),
        Property::WildcardSubscriptionAvailable(0),
        Property::WildcardSubscriptionAvailable(1),
        Property::SubscriptionIdentifierAvailable(0),
        Property::SubscriptionIdentifierAvailable(1),
        Property::SharedSubscriptionAvailable(0),
        Property::SharedSubscriptionAvailable(1),
    ];
    let converted: Vec<mqttv5pb::Property> = properties
        .iter()
        .cloned()
        .map(|p| p.try_into().unwrap())
        .collect();
    let restored: Vec<Property> = converted
        .into_iter()
        .map(|p| p.try_into().unwrap())
        .collect();
    assert_eq!(restored, properties);
    assert!(Property::try_from(mqttv5pb::Property {
        property_type: None
    })
    .is_err());
}

#[test]
fn empty_and_control_messages_have_no_mqtt_packet() {
    use mqtt_unified_pb::mqtt_stream_message::Payload;
    for payload in [
        None,
        Some(Payload::MqttV3Packet(mqttv3pb::MqttPacket { packet: None })),
        Some(Payload::MqttV5Packet(mqttv5pb::MqttPacket { packet: None })),
        Some(Payload::SessionControl(mqtt_unified_pb::SessionControl {
            control_type: 0,
            client_id: "client".into(),
        })),
    ] {
        assert!(
            convert_stream_message_to_mqtt_packet(mqtt_unified_pb::MqttStreamMessage {
                sequence_id: 0,
                direction: 0,
                payload,
            })
            .is_none()
        );
    }
}
