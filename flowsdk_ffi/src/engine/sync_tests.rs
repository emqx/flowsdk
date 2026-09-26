// SPDX-License-Identifier: MPL-2.0
use super::*;
use flowsdk::mqtt_serde::{control_packet::MqttPacket, parser::ParseOk};

fn options() -> MqttConnectOptionsFFI {
    let mut opts: MqttConnectOptionsFFI = MqttOptionsFFI {
        client_id: "ffi-durable".into(),
        clean_start: false,
        keep_alive: 0,
        ..Default::default()
    }
    .into();
    opts.properties
        .push(MqttPropertyFFI::SessionExpiryInterval { value: 3600 });
    opts
}

fn runtime() -> MqttRuntimeOptionsFFI {
    MqttRuntimeOptionsFFI {
        peer: Some("tcp://broker:1883".into()),
        ..Default::default()
    }
}

fn engine() -> MqttEngineFFI {
    MqttEngineFFI::new_with_runtime_options(options(), runtime()).unwrap()
}

fn connect(engine: &MqttEngineFFI, present: bool) {
    engine.connect_checked().unwrap();
    assert_eq!(engine.take_outgoing()[0], 0x10);
    engine.handle_incoming(vec![0x20, 3, u8::from(present), 0, 0]);
    assert!(engine.is_connected());
    engine.take_events();
}

fn publish(engine: &MqttEngineFFI, qos: u8) -> u16 {
    engine
        .publish_with_options(
            "test".into(),
            vec![0, 255],
            MqttPublishOptionsFFI {
                qos,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap()
}

fn ack(header: u8, id: u16) -> Vec<u8> {
    vec![header, 2, (id >> 8) as u8, id as u8]
}

#[test]
fn checked_connect_rejects_repetition_without_discarding_output_or_events() {
    let client = engine();
    client.connect_checked().unwrap();
    assert!(client.connect_checked().is_err());
    assert!(client.has_pending_output());
    assert_eq!(client.take_outgoing()[0], 0x10);
    client.handle_incoming(vec![0x20, 3, 0, 0, 0]);
    assert!(client.connect_checked().is_err());
    assert!(client.is_connected());
    assert!(matches!(
        client.take_events().as_slice(),
        [MqttEventFFI::Connected(_)]
    ));
}

#[test]
fn runtime_limits_deadlines_and_property_merging() {
    let mut opts = options();
    opts.properties
        .extend(vec![MqttPropertyFFI::ReceiveMaximum { value: 7 }; 2]);
    let config = MqttRuntimeOptionsFFI {
        incoming_receive_maximum: Some(7),
        max_incoming_packet_size: Some(4096),
        max_incoming_buffer_bytes: Some(8192),
        max_outgoing_buffer_bytes: Some(4096),
        operation_timeouts: Some(mqtt_cloud_timeouts()),
        reconnect: Some(true),
        ..runtime()
    };
    let client = MqttEngineFFI::new_with_runtime_options(opts.clone(), config.clone()).unwrap();
    {
        let inner = client.engine.lock().unwrap();
        let core = inner.options();
        assert_eq!(
            core.operation_timeouts.connect,
            Some(Duration::from_secs(30))
        );
        assert_eq!(core.max_incoming_buffer_bytes, Some(8192));
        assert_eq!(core.max_outgoing_buffer_bytes, Some(4096));
        assert_eq!(core.max_incoming_packet_size, Some(4096));
        assert_eq!(core.incoming_receive_maximum, Some(7));
        assert_ne!(core.receive_maximum, 7);
    }
    client.connect_checked().unwrap();
    let ParseOk::Packet(MqttPacket::Connect5(packet), _) =
        MqttPacket::from_bytes_with_version(&client.take_outgoing(), 5).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        packet
            .properties
            .iter()
            .filter(|p| matches!(
                p,
                flowsdk::mqtt_serde::mqttv5::common::properties::Property::ReceiveMaximum(7)
            ))
            .count(),
        1
    );
    let bad = MqttEngineFFI::new_with_runtime_options(
        opts.clone(),
        MqttRuntimeOptionsFFI {
            incoming_receive_maximum: Some(8),
            ..config
        },
    )
    .unwrap();
    assert!(bad.connect_checked().is_err());
    for runtime in [
        MqttRuntimeOptionsFFI {
            incoming_receive_maximum: Some(0),
            ..Default::default()
        },
        MqttRuntimeOptionsFFI {
            max_outgoing_buffer_bytes: Some(u64::MAX),
            ..Default::default()
        },
        MqttRuntimeOptionsFFI {
            peer: Some("".into()),
            ..Default::default()
        },
    ] {
        assert!(MqttEngineFFI::new_with_runtime_options(opts.clone(), runtime).is_err());
    }
}

#[test]
fn operation_timeout_preserves_packet_id_and_late_ack_without_closing() {
    let config = MqttRuntimeOptionsFFI {
        operation_timeouts: Some(MqttOperationTimeoutsFFI {
            publish_ms: Some(10),
            ..Default::default()
        }),
        ..runtime()
    };
    let client = MqttEngineFFI::new_with_runtime_options(options(), config).unwrap();
    connect(&client, false);
    let id = publish(&client, 1);
    client.take_outgoing();
    client.handle_tick(client.elapsed_ms() + 100);
    assert!(client.is_connected());
    assert!(
        matches!(client.take_events().as_slice(), [MqttEventFFI::OperationFailed {
        operation: MqttOperationKindFFI::Publish, packet_id: Some(pid), kind: MqttFailureKindFFI::Timeout, timeout_ms: Some(10), ..
    }] if *pid == id)
    );
    assert_ne!(publish(&client, 1), id);
    client.handle_incoming(ack(0x40, id));
    assert!(client
        .take_events()
        .iter()
        .any(|e| matches!(e, MqttEventFFI::Published(p) if p.packet_id == Some(id))));
}

#[test]
fn connect_deadline_and_reconnect_events_are_structured_and_cancelable() {
    let client = MqttEngineFFI::new_with_runtime_options(
        options(),
        MqttRuntimeOptionsFFI {
            operation_timeouts: Some(MqttOperationTimeoutsFFI {
                connect_ms: Some(0),
                ..Default::default()
            }),
            ..runtime()
        },
    )
    .unwrap();
    client.connect_checked().unwrap();
    client.handle_tick(client.elapsed_ms() + 1);
    assert!(client.take_events().iter().any(|e| matches!(
        e,
        MqttEventFFI::OperationFailed {
            operation: MqttOperationKindFFI::Connect,
            packet_id: None,
            kind: MqttFailureKindFFI::Timeout,
            ..
        }
    )));
    client.set_reconnect(true);
    client.schedule_reconnect(client.elapsed_ms()).unwrap();
    client.handle_tick(client.elapsed_ms());
    client.set_reconnect(false);
    assert!(!client.take_events().iter().any(|e| matches!(
        e,
        MqttEventFFI::ReconnectScheduled { .. } | MqttEventFFI::ReconnectNeeded
    )));
}

#[cfg(feature = "durable-session")]
#[test]
fn disk_restart_replays_each_outgoing_qos_stage_with_same_id() {
    for (qos, pubrel) in [(1, false), (2, false), (2, true)] {
        let old = engine();
        connect(&old, false);
        let id = publish(&old, qos);
        old.take_outgoing();
        if pubrel {
            old.handle_incoming(ack(0x50, id));
            old.take_outgoing();
        }
        let saved = old.snapshot_session().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.json");
        std::fs::write(&path, saved).unwrap();
        drop(old);
        let client = engine();
        client
            .restore_session_state(std::fs::read(path).unwrap())
            .unwrap();
        connect(&client, true);
        let output = client.take_outgoing();
        if pubrel {
            assert_eq!(output, ack(0x62, id));
        } else {
            let ParseOk::Packet(MqttPacket::Publish5(packet), _) =
                MqttPacket::from_bytes_with_version(&output, 5).unwrap()
            else {
                panic!()
            };
            assert_eq!(packet.packet_id, Some(id));
            assert!(packet.dup);
            assert_eq!(packet.payload, [0, 255]);
        }
    }
}

#[cfg(feature = "durable-session")]
#[test]
fn restore_errors_are_atomic_and_require_fresh_explicit_identity() {
    let old = engine();
    connect(&old, false);
    publish(&old, 1);
    old.take_outgoing();
    let saved = old.snapshot_session().unwrap();
    let metadata = inspect_session_state(saved.clone()).unwrap();
    assert_eq!(metadata.peer, "tcp://broker:1883");
    assert!(metadata.has_session);
    let client = engine();
    for field in ["peer", "client_id", "mqtt_version", "version", "outbound"] {
        let mut corrupt: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        match field {
            "peer" | "client_id" => corrupt[field] = "different".into(),
            "outbound" => {
                let duplicate = corrupt[field][0].clone();
                corrupt[field].as_array_mut().unwrap().push(duplicate);
            }
            _ => corrupt[field] = 99.into(),
        }
        assert!(client
            .restore_session_state(serde_json::to_vec(&corrupt).unwrap())
            .is_err());
        assert!(client.take_events().is_empty());
    }
    assert!(client
        .restore_session_state(b"secret bad JSON".to_vec())
        .unwrap_err()
        .to_string()
        .contains("Malformed"));
    client.restore_session_state(saved.clone()).unwrap();
    assert!(client.restore_session_state(saved.clone()).is_err());
    connect(&client, true);
    assert!(!client.take_outgoing().is_empty());
    assert!(old.restore_session_state(saved.clone()).is_err());
    let implicit = MqttEngineFFI::new_with_options(options()).unwrap();
    assert!(implicit.snapshot_session().is_err());
    assert!(implicit.restore_session_state(saved.clone()).is_err());
    let mut clean = options();
    clean.options.clean_start = true;
    assert!(MqttEngineFFI::new_with_runtime_options(clean, runtime())
        .unwrap()
        .restore_session_state(saved.clone())
        .is_err());
    let small = MqttEngineFFI::new_with_runtime_options(
        options(),
        MqttRuntimeOptionsFFI {
            max_outgoing_buffer_bytes: Some(1),
            ..runtime()
        },
    )
    .unwrap();
    assert!(small.restore_session_state(saved).is_err());
}

#[cfg(feature = "durable-session")]
#[test]
fn broker_session_loss_reports_saved_operations_individually() {
    let old = engine();
    connect(&old, false);
    let id = publish(&old, 1);
    old.take_outgoing();
    let saved = old.snapshot_session().unwrap();
    let client = engine();
    client.restore_session_state(saved).unwrap();
    client.connect_checked().unwrap();
    client.take_outgoing();
    client.handle_incoming(vec![0x20, 3, 0, 0, 0]);
    assert!(client.is_connected());
    assert!(client.take_events().iter().any(|e| matches!(e, MqttEventFFI::OperationFailed {
        operation: MqttOperationKindFFI::Publish, packet_id: Some(pid), kind: MqttFailureKindFFI::SessionExpired, ..
    } if *pid == id)));
}

#[cfg(feature = "tls")]
#[cfg(feature = "durable-session")]
#[test]
fn tls_checkpoint_delegation_rejects_restore_after_handshake_output() {
    let tls = MqttTlsOptionsFFI {
        insecure_skip_verify: true,
        ..Default::default()
    };
    let old = engine();
    connect(&old, false);
    publish(&old, 2);
    old.take_outgoing();
    let saved = old.snapshot_session().unwrap();
    let client = TlsMqttEngineFFI::new_with_runtime_options(
        options(),
        runtime(),
        tls.clone(),
        "broker".into(),
    )
    .unwrap();
    client.restore_session_state(saved.clone()).unwrap();
    assert_eq!(
        inspect_session_state(client.snapshot_session().unwrap())
            .unwrap()
            .client_id,
        "ffi-durable"
    );
    let started =
        TlsMqttEngineFFI::new_with_runtime_options(options(), runtime(), tls, "broker".into())
            .unwrap();
    started.handle_tick(0);
    started.take_socket_data();
    assert!(started.restore_session_state(saved).is_err());
    client.connect_checked().unwrap();
    assert!(client.connect_checked().is_err());
}

#[cfg(feature = "quic")]
#[cfg(feature = "durable-session")]
#[test]
fn quic_checkpoint_and_startup_control_api_guards() {
    let old = engine();
    connect(&old, false);
    publish(&old, 2);
    old.take_outgoing();
    let saved = old.snapshot_session().unwrap();
    let client = QuicMqttEngineFFI::new_with_runtime_options(options(), runtime()).unwrap();
    client.restore_session_state(saved.clone()).unwrap();
    assert!(
        inspect_session_state(client.snapshot_session().unwrap())
            .unwrap()
            .has_session
    );
    assert!(client
        .subscribe_on_control(MqttSubscribeOptionsFFI {
            subscriptions: vec![MqttSubscriptionFFI {
                topic_filter: "test".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .is_err());
    assert!(client
        .unsubscribe_on_control(MqttUnsubscribeOptionsFFI {
            topics: vec!["test".into()],
            ..Default::default()
        })
        .is_err());
    let started = QuicMqttEngineFFI::new_with_runtime_options(options(), runtime()).unwrap();
    let tls = MqttTlsOptionsFFI {
        insecure_skip_verify: true,
        ..Default::default()
    };
    started
        .connect("127.0.0.1:14567".into(), "broker".into(), tls.clone(), 0)
        .unwrap();
    assert!(started.restore_session_state(saved).is_err());
    assert!(started
        .connect("127.0.0.1:14567".into(), "broker".into(), tls.clone(), 0)
        .is_err());
    assert!(started
        .connect_with_zero_rtt(
            "127.0.0.1:14567".into(),
            "broker".into(),
            tls,
            QuicZeroRttOptionsFFI {
                session_cache_size: 16,
                replay_on_reject: true
            },
            0
        )
        .is_err());
    started.handle_tick(1);
    assert!(!started.take_outgoing_datagrams().is_empty());
}

#[cfg(feature = "durable-session")]
#[test]
fn incoming_qos2_stages_survive_binding_round_trip_without_redelivery() {
    for stage in 0..3 {
        let mut options = options();
        options.engine_options = Some(MqttEngineOptionsFFI {
            auto_ack: Some(false),
            ..Default::default()
        });
        let old = MqttEngineFFI::new_with_runtime_options(options.clone(), runtime()).unwrap();
        connect(&old, false);
        let original = vec![0x34, 7, 0, 1, b't', 0, 71, 0, 9];
        assert!(old
            .handle_incoming(original.clone())
            .iter()
            .any(|e| matches!(e, MqttEventFFI::MessageReceived(_))));
        if stage >= 1 {
            old.acknowledge(MqttAcknowledgementFFI::PubRec, 71, 0, vec![], None)
                .unwrap();
            old.take_outgoing();
        }
        if stage == 2 {
            old.handle_incoming(ack(0x62, 71));
        }
        let saved = old.snapshot_session().unwrap();
        drop(old);
        let client = MqttEngineFFI::new_with_runtime_options(options, runtime()).unwrap();
        client.restore_session_state(saved).unwrap();
        connect(&client, true);
        if stage < 2 {
            let mut dup = original.clone();
            dup[0] |= 8;
            assert!(!client
                .handle_incoming(dup)
                .iter()
                .any(|e| matches!(e, MqttEventFFI::MessageReceived(_))));
            if stage == 0 {
                client
                    .acknowledge(MqttAcknowledgementFFI::PubRec, 71, 0, vec![], None)
                    .unwrap();
            }
            assert!(!client.take_outgoing().is_empty());
        }
        assert!(client
            .handle_incoming(ack(0x62, 71))
            .iter()
            .any(|e| matches!(e, MqttEventFFI::PubRelReceived { packet_id: 71, .. })));
        client
            .acknowledge(MqttAcknowledgementFFI::PubComp, 71, 0, vec![], None)
            .unwrap();
        assert!(!client.take_outgoing().is_empty());
        assert!(client
            .handle_incoming(original)
            .iter()
            .any(|e| matches!(e, MqttEventFFI::MessageReceived(_))));
    }
}

#[cfg(feature = "durable-session")]
#[test]
fn interrupted_connect_preserves_queued_work_and_assigned_identity() {
    let mut opts = options();
    opts.options.client_id.clear();
    let old = MqttEngineFFI::new_with_runtime_options(opts, runtime()).unwrap();
    old.connect_checked().unwrap();
    old.take_outgoing();
    // Broker assigns "id" and an established persistent session.
    old.handle_incoming(vec![0x20, 8, 0, 0, 5, 0x12, 0, 2, b'i', b'd']);
    assert!(old.is_connected());
    let id = publish(&old, 1);
    old.take_outgoing();
    old.handle_connection_lost();
    let queued = publish(&old, 1);
    old.connect_checked().unwrap();
    old.take_outgoing();
    let saved = old.snapshot_session().unwrap();
    let info = inspect_session_state(saved.clone()).unwrap();
    assert_eq!(info.client_id, "id");
    let mut opts = options();
    opts.options.client_id = info.client_id;
    let client = MqttEngineFFI::new_with_runtime_options(opts, runtime()).unwrap();
    client.restore_session_state(saved).unwrap();
    client.connect_checked().unwrap();
    client.take_outgoing();
    // Reduced replay quota: one packet at a time.
    client.handle_incoming(vec![0x20, 6, 1, 0, 3, 0x21, 0, 1]);
    for expected in [id, queued] {
        let wire = client.take_outgoing();
        let ParseOk::Packet(MqttPacket::Publish5(packet), len) =
            MqttPacket::from_bytes_with_version(&wire, 5).unwrap()
        else {
            panic!()
        };
        assert_eq!(len, wire.len());
        assert_eq!(packet.packet_id, Some(expected));
        client.handle_incoming(ack(0x40, expected));
    }
}
