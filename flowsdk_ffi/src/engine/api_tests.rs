use super::*;
use flowsdk::mqtt_serde::{control_packet::MqttPacket, mqttv5::common::properties::Property};

fn options(version: u8) -> MqttConnectOptionsFFI {
    MqttOptionsFFI {
        client_id: "ffi-contract-test".into(),
        mqtt_version: version,
        clean_start: true,
        keep_alive: 30,
        username: None,
        password: None,
        reconnect_base_delay_ms: 1000,
        reconnect_max_delay_ms: 30000,
        max_reconnect_attempts: 0,
    }
    .into()
}

fn connected(engine: &mut MqttEngine, version: u8) {
    engine.connect().unwrap();
    engine.take_outgoing();
    engine.handle_incoming(if version == 5 {
        &[0x20, 3, 0, 0, 0]
    } else {
        &[0x20, 2, 0, 0]
    });
    assert!(engine.is_connected());
    engine.take_events();
}

#[test]
fn every_property_preserves_its_type_and_value_across_the_ffi() {
    let values = vec![
        Property::PayloadFormatIndicator(1),
        Property::MessageExpiryInterval(u32::MAX),
        Property::ContentType("application/octet-stream".into()),
        Property::ResponseTopic("reply".into()),
        Property::CorrelationData(vec![0, 255]),
        Property::SubscriptionIdentifier(268_435_455),
        Property::SessionExpiryInterval(u32::MAX),
        Property::AssignedClientIdentifier("assigned".into()),
        Property::ServerKeepAlive(u16::MAX),
        Property::AuthenticationMethod("method".into()),
        Property::AuthenticationData(vec![0, 255]),
        Property::RequestProblemInformation(0),
        Property::WillDelayInterval(u32::MAX),
        Property::RequestResponseInformation(1),
        Property::ResponseInformation("response".into()),
        Property::ServerReference("server".into()),
        Property::ReasonString("reason".into()),
        Property::ReceiveMaximum(u16::MAX),
        Property::TopicAliasMaximum(u16::MAX),
        Property::TopicAlias(1),
        Property::MaximumQoS(1),
        Property::RetainAvailable(0),
        Property::UserProperty("repeat".into(), "one".into()),
        Property::UserProperty("repeat".into(), "two".into()),
        Property::MaximumPacketSize(u32::MAX),
        Property::WildcardSubscriptionAvailable(1),
        Property::SubscriptionIdentifierAvailable(0),
        Property::SharedSubscriptionAvailable(1),
    ];
    let ffi: Vec<_> = values
        .clone()
        .into_iter()
        .map(MqttPropertyFFI::from)
        .collect();
    assert_eq!(
        ffi.into_iter().map(Property::from).collect::<Vec<_>>(),
        values
    );
}

#[test]
fn property_validation_rejects_invalid_numeric_and_binary_boundaries() {
    use MqttPropertyFFI::*;
    for invalid in [
        RequestProblemInformation { value: 2 },
        RequestResponseInformation { value: 2 },
        MaximumQoS { value: 2 },
        RetainAvailable { value: 2 },
        WildcardSubscriptionAvailable { value: 2 },
        SubscriptionIdentifierAvailable { value: 2 },
        SharedSubscriptionAvailable { value: 2 },
        SubscriptionIdentifier { value: 0 },
        SubscriptionIdentifier { value: 268_435_456 },
        ReceiveMaximum { value: 0 },
        MaximumPacketSize { value: 0 },
        AuthenticationData {
            value: vec![0; 65536],
        },
        UserProperty {
            key: "key".into(),
            value: "nul\0".into(),
        },
    ] {
        assert!(
            properties::validate(vec![invalid.clone()], 5, |_| true).is_err(),
            "{invalid:?}"
        );
    }
    for valid in [
        SubscriptionIdentifier { value: 268_435_455 },
        AuthenticationData {
            value: vec![0; 65535],
        },
    ] {
        assert!(properties::validate(vec![valid], 5, |_| true).is_ok());
    }
}

#[test]
fn connect_validates_credentials_and_preserves_all_will_fields() {
    let mut opts = options(5);
    opts.properties = vec![
        MqttPropertyFFI::MaximumPacketSize { value: 4096 },
        MqttPropertyFFI::RequestResponseInformation { value: 1 },
        MqttPropertyFFI::RequestProblemInformation { value: 0 },
        MqttPropertyFFI::AuthenticationMethod {
            value: "method".into(),
        },
        MqttPropertyFFI::AuthenticationData {
            value: vec![0, 255],
        },
    ];
    opts.will = Some(MqttWillFFI {
        topic: "status".into(),
        payload: vec![0, 255],
        qos: 2,
        retain: true,
        properties: vec![
            MqttPropertyFFI::WillDelayInterval { value: 10 },
            MqttPropertyFFI::PayloadFormatIndicator { value: 1 },
            MqttPropertyFFI::MessageExpiryInterval { value: 20 },
            MqttPropertyFFI::ContentType {
                value: "binary".into(),
            },
            MqttPropertyFFI::ResponseTopic {
                value: "reply".into(),
            },
            MqttPropertyFFI::CorrelationData {
                value: vec![0, 255],
            },
            MqttPropertyFFI::UserProperty {
                key: "key".into(),
                value: "value".into(),
            },
        ],
    });
    let core = opts.clone().into_core().unwrap();
    assert_eq!(core.maximum_packet_size, Some(4096));
    assert_eq!(core.request_response_information, Some(true));
    assert_eq!(core.request_problem_information, Some(false));
    let will = core.will.unwrap();
    assert_eq!(will.will_message, [0, 255]);
    assert_eq!(will.properties.will_delay_interval, Some(10));
    assert_eq!(will.properties.payload_format_indicator, Some(1));
    assert_eq!(will.properties.message_expiry_interval, Some(20));
    assert_eq!(will.properties.content_type.as_deref(), Some("binary"));
    assert_eq!(will.properties.response_topic.as_deref(), Some("reply"));
    assert_eq!(will.properties.correlation_data, Some(vec![0, 255]));
    assert_eq!(
        will.properties.user_properties,
        vec![Property::UserProperty("key".into(), "value".into())]
    );
    let mut bad = opts.clone();
    bad.options.username = Some("invalid\0".into());
    assert!(bad.into_core().is_err());
    let mut bad = opts.clone();
    bad.options.password = Some("password".into());
    bad.binary_password = Some(vec![]);
    assert!(bad.into_core().is_err());
    let mut bad = opts.clone();
    bad.binary_password = Some(vec![0; 65536]);
    assert!(bad.into_core().is_err());
    let mut bad = opts.clone();
    bad.will.as_mut().unwrap().qos = 3;
    assert!(bad.into_core().is_err());
    opts.will.as_mut().unwrap().payload = vec![0; 65536];
    assert!(opts.into_core().is_err());
}

#[test]
fn manual_acknowledgements_validate_reason_codes_and_encode_each_version() {
    for version in [3, 5] {
        let mut opts = options(version);
        opts.engine_options = Some(MqttEngineOptionsFFI {
            auto_ack: Some(false),
            ..Default::default()
        });
        let engine = MqttEngineFFI::new_with_options(opts).unwrap();
        connected(&mut engine.engine.lock().unwrap(), version);
        for (kind, header) in [
            (MqttAcknowledgementFFI::PubAck, 0x40),
            (MqttAcknowledgementFFI::PubRec, 0x50),
            (MqttAcknowledgementFFI::PubComp, 0x70),
        ] {
            engine.acknowledge(kind, 42, 0, vec![], None).unwrap();
            let bytes = engine.take_outgoing();
            assert_eq!(bytes[0], header);
            assert_eq!(&bytes[2..4], &[0, 42]);
            assert!(MqttPacket::from_bytes_with_version(&bytes, version).is_ok());
            for (pid, rc, props) in [
                (0, 0, vec![]),
                (1, 0xff, vec![]),
                (
                    1,
                    0,
                    vec![MqttPropertyFFI::ContentType {
                        value: "invalid".into(),
                    }],
                ),
            ] {
                assert!(engine.acknowledge(kind, pid, rc, props, None).is_err());
                assert!(engine.take_outgoing().is_empty());
            }
            assert!(engine.acknowledge(kind, 42, 0, vec![], Some(0)).is_err());
            if version != 5 {
                assert!(engine.acknowledge(kind, 42, 0x80, vec![], None).is_err());
            }
        }
    }
    let engine = MqttEngineFFI::new(None, 5).unwrap();
    assert!(engine
        .acknowledge(MqttAcknowledgementFFI::PubAck, 1, 0, vec![], None)
        .is_err());
}

#[cfg(feature = "tls")]
#[test]
fn tls_manual_ack_and_parser_controls_reach_the_mqtt_engine() {
    let mut opts = options(5);
    opts.engine_options = Some(MqttEngineOptionsFFI {
        auto_ack: Some(false),
        ..Default::default()
    });
    let engine = TlsMqttEngineFFI::new_with_options(
        opts,
        MqttTlsOptionsFFI {
            insecure_skip_verify: true,
            ..Default::default()
        },
        "localhost".into(),
    )
    .unwrap();
    connected(engine.engine.lock().unwrap().engine_mut(), 5);
    engine
        .acknowledge(MqttAcknowledgementFFI::PubAck, 42, 0, vec![], None)
        .unwrap();
    assert_eq!(
        engine.engine.lock().unwrap().engine_mut().take_outgoing()[0],
        0x40
    );
    assert!(engine
        .acknowledge(MqttAcknowledgementFFI::PubRec, 42, 0, vec![], Some(4))
        .is_err());
    for level in [
        MqttParseLevelFFI::HeadersParsed,
        MqttParseLevelFFI::TypeOnly,
        MqttParseLevelFFI::Full,
    ] {
        engine.set_parse_level(level).unwrap();
        assert_eq!(
            engine.engine.lock().unwrap().engine().parse_level(),
            level.into()
        );
    }
}

#[cfg(feature = "quic")]
#[test]
fn quic_controls_reject_missing_connections_and_invalid_options() {
    let mut opts = options(5);
    opts.engine_options = Some(MqttEngineOptionsFFI {
        auto_ack: Some(false),
        ..Default::default()
    });
    let engine = QuicMqttEngineFFI::new_with_options(opts).unwrap();
    assert!(engine
        .publish_on(4, "topic".into(), vec![1], MqttPublishOptionsFFI::default())
        .is_err());
    assert!(engine
        .publish_on(
            4,
            "topic".into(),
            vec![1],
            MqttPublishOptionsFFI {
                priority: Some(42),
                ..Default::default()
            }
        )
        .is_err());
    assert!(engine
        .subscribe_on(
            4,
            MqttSubscribeOptionsFFI {
                subscriptions: vec![MqttSubscriptionFFI {
                    topic_filter: "topic".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }
        )
        .is_err());
    assert!(engine
        .unsubscribe_on(
            4,
            MqttUnsubscribeOptionsFFI {
                topics: vec!["topic".into()],
                ..Default::default()
            }
        )
        .is_err());
    assert!(engine.set_stream_priority(4, 42).is_err());
    engine.notify_local_address_changed().unwrap();
    assert!(!engine.is_connected());
    assert!(engine
        .acknowledge(MqttAcknowledgementFFI::PubAck, 42, 0, vec![], None)
        .is_err());
    assert!(engine
        .acknowledge(MqttAcknowledgementFFI::PubAck, 42, 0, vec![], Some(4))
        .is_err());
    assert!(engine.set_parse_level(MqttParseLevelFFI::TypeOnly).is_err());
    assert!(engine
        .connect_with_zero_rtt(
            "127.0.0.1:14567".into(),
            "localhost".into(),
            MqttTlsOptionsFFI::default(),
            QuicZeroRttOptionsFFI {
                session_cache_size: 0,
                replay_on_reject: false
            },
            0
        )
        .is_err());
    assert!(engine.close_transport(0, b"shutdown".to_vec()).is_err());
    assert_eq!(engine.data_stream_count(), 0);
}
