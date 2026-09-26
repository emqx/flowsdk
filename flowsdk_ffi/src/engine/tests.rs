// SPDX-License-Identifier: MPL-2.0
use super::*;
use flowsdk::mqtt_client::engine::QuicZeroRttStatus;
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::parser::ParseOk;
use std::time::Duration;

fn authenticated_options(version: u8) -> MqttOptionsFFI {
    MqttOptionsFFI {
        client_id: "ffi-auth-test".to_string(),
        mqtt_version: version,
        clean_start: true,
        keep_alive: 30,
        username: Some("test-user".to_string()),
        password: Some("test-password".to_string()),
        reconnect_base_delay_ms: 1000,
        reconnect_max_delay_ms: 30000,
        max_reconnect_attempts: 0,
    }
}

#[test]
fn full_message_mapping_keeps_originating_stream() {
    let message = flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish::new_with_prop(
        1,
        "test".into(),
        Some(7),
        vec![],
        false,
        false,
        vec![],
    );
    let events = map_events(vec![
        MqttEvent::PublishReceived {
            packet_id: Some(7),
            stream: Some(12),
        },
        MqttEvent::MessageReceived(message),
    ]);
    assert!(matches!(
        &events[0],
        MqttEventFFI::PublishReceived {
            stream_id: Some(12),
            ..
        }
    ));
    assert!(
        matches!(&events[1], MqttEventFFI::MessageReceived(message) if message.stream_id == Some(12))
    );
}

#[test]
fn connect_encodes_ffi_credentials() {
    for version in [3, 4, 5] {
        let engine = MqttEngineFFI::new_with_opts(authenticated_options(version)).unwrap();
        engine.connect();
        let bytes = engine.take_outgoing();
        let (username, password) = match MqttPacket::from_bytes_with_version(&bytes, version)
            .unwrap()
        {
            ParseOk::Packet(MqttPacket::Connect3(packet), _) => (packet.username, packet.password),
            ParseOk::Packet(MqttPacket::Connect5(packet), _) => (packet.username, packet.password),
            packet => panic!("Expected CONNECT, got {packet:?}"),
        };
        assert_eq!(username.as_deref(), Some("test-user"));
        assert_eq!(password.as_deref(), Some(b"test-password".as_slice()));
    }
}

#[test]
fn publish_options_reach_the_wire() {
    for version in [3, 5] {
        let engine = MqttEngineFFI::new_with_opts(authenticated_options(version)).unwrap();
        engine.connect();
        engine.take_outgoing();
        engine.handle_incoming(if version == 5 {
            vec![0x20, 3, 0, 0, 0]
        } else {
            vec![0x20, 2, 0, 0]
        });
        let properties = if version == 5 {
            vec![
                MqttPropertyFFI::UserProperty {
                    key: "source".into(),
                    value: "one".into(),
                },
                MqttPropertyFFI::UserProperty {
                    key: "source".into(),
                    value: "two".into(),
                },
                MqttPropertyFFI::CorrelationData {
                    value: vec![0, 255],
                },
            ]
        } else {
            vec![]
        };
        engine
            .publish_with_options(
                "test/topic".into(),
                b"data".to_vec(),
                MqttPublishOptionsFFI {
                    retain: true,
                    properties: properties.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
        let bytes = engine.take_outgoing();
        match MqttPacket::from_bytes_with_version(&bytes, version).unwrap() {
            ParseOk::Packet(MqttPacket::Publish5(packet), _) => {
                assert!(packet.retain);
                assert_eq!(
                    packet.properties,
                    properties.into_iter().map(Into::into).collect::<Vec<_>>()
                );
            }
            ParseOk::Packet(MqttPacket::Publish3(packet), _) => assert!(packet.retain),
            other => panic!("Expected PUBLISH, got {other:?}"),
        }
    }
}

#[cfg(feature = "quic")]
#[test]
fn quic_constructor_preserves_mqtt_credentials() {
    for version in [3, 4, 5] {
        let engine = QuicMqttEngineFFI::new(authenticated_options(version)).unwrap();
        let inner = engine.engine.lock().unwrap();
        let options = inner.engine().options();
        assert_eq!(options.username.as_deref(), Some("test-user"));
        assert_eq!(
            options.password.as_deref(),
            Some(b"test-password".as_slice())
        );
    }
}

#[test]
fn connect_options_encode_will_properties_and_binary_password() {
    for version in [3, 5] {
        let mut opts = authenticated_options(version);
        opts.password = None;
        let properties = if version == 5 {
            vec![
                MqttPropertyFFI::SessionExpiryInterval { value: 60 },
                MqttPropertyFFI::ReceiveMaximum { value: 10 },
            ]
        } else {
            vec![]
        };
        let engine = MqttEngineFFI::new_with_options(MqttConnectOptionsFFI {
            options: opts,
            properties: properties.clone(),
            engine_options: None,
            binary_password: Some(vec![0, 255]),
            will: Some(MqttWillFFI {
                topic: "status".into(),
                payload: vec![1, 255],
                qos: 1,
                retain: true,
                properties: if version == 5 {
                    vec![MqttPropertyFFI::WillDelayInterval { value: 30 }]
                } else {
                    vec![]
                },
            }),
        })
        .unwrap();
        engine.connect();
        match MqttPacket::from_bytes_with_version(&engine.take_outgoing(), version).unwrap() {
            ParseOk::Packet(MqttPacket::Connect5(packet), _) => {
                assert_eq!(packet.password, Some(vec![0, 255]));
                assert_eq!(
                    packet.properties,
                    properties.into_iter().map(Into::into).collect::<Vec<_>>()
                );
                let will = packet.will.unwrap();
                assert_eq!(will.will_message, vec![1, 255]);
                assert_eq!(will.properties.will_delay_interval, Some(30));
            }
            ParseOk::Packet(MqttPacket::Connect3(packet), _) => {
                assert_eq!(packet.password, Some(vec![0, 255]));
                assert_eq!(packet.will.unwrap().message, vec![1, 255]);
            }
            packet => panic!("Expected CONNECT, got {packet:?}"),
        }
    }
    assert!(MqttEngineFFI::new(None, 0).is_err());
    let mut options: MqttConnectOptionsFFI = authenticated_options(3).into();
    options.properties = vec![MqttPropertyFFI::SessionExpiryInterval { value: 60 }];
    assert!(MqttEngineFFI::new_with_options(options).is_err());
}

#[cfg(all(feature = "tls", feature = "quic"))]
#[test]
fn invalid_transport_configuration_returns_errors() {
    let tls_opts = MqttTlsOptionsFFI {
        insecure_skip_verify: true,
        ..Default::default()
    };
    assert!(TlsMqttEngineFFI::new(
        authenticated_options(5),
        tls_opts.clone(),
        "invalid server name".into(),
    )
    .is_err());
    let quic = QuicMqttEngineFFI::new(authenticated_options(5)).unwrap();
    assert!(matches!(
        quic.connect("not an address".into(), "localhost".into(), tls_opts, 0),
        Err(MqttErrorFFI::InvalidArgument { .. })
    ));
}

#[test]
fn zero_rtt_status_event_is_not_reported_as_ffi_error() {
    let event = MqttEvent::ZeroRttStatusChanged {
        status: QuicZeroRttStatus::Attempted,
    };

    assert!(matches!(
        map_event(event),
        Some(MqttEventFFI::ZeroRttStatusChanged {
            status: QuicZeroRttStatusFFI::Attempted
        })
    ));
}

#[test]
fn transport_closed_event_is_not_reported_as_ffi_error() {
    let event = MqttEvent::TransportClosed {
        reason: "connection closed".to_string(),
        by_peer: true,
        error_code: Some(0),
    };

    assert!(matches!(
        map_event(event),
        Some(MqttEventFFI::TransportClosed {
            by_peer: true,
            error_code: Some(0),
            ..
        })
    ));
}

#[test]
fn maps_stream_and_reconnect_events() {
    assert!(matches!(
        map_event(MqttEvent::StreamClosed {
            stream_id: 7,
            reason: "recv_finished".to_string(),
            by_peer: true,
        }),
        Some(MqttEventFFI::StreamClosed {
            stream_id: 7,
            by_peer: true,
            ..
        })
    ));
    assert!(matches!(
        map_event(MqttEvent::StreamReset {
            stream_id: 8,
            error_code: 42,
        }),
        Some(MqttEventFFI::StreamReset {
            stream_id: 8,
            error_code: 42,
        })
    ));
    assert!(matches!(
        map_event(MqttEvent::StreamStopped {
            stream_id: 9,
            error_code: 43,
        }),
        Some(MqttEventFFI::StreamStopped {
            stream_id: 9,
            error_code: 43,
        })
    ));
    assert!(matches!(
        map_event(MqttEvent::ReconnectNeeded),
        Some(MqttEventFFI::ReconnectNeeded)
    ));
    assert!(matches!(
        map_event(MqttEvent::ReconnectScheduled {
            attempt: 3,
            delay: Duration::from_millis(250),
        }),
        Some(MqttEventFFI::ReconnectScheduled {
            attempt: 3,
            delay_ms: 250,
        })
    ));
}
