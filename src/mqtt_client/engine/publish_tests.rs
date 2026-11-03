// SPDX-License-Identifier: Apache-2.0

use super::test_support::connect;
use super::*;
#[cfg(feature = "shared-payloads")]
use crate::mqtt_client::payload_tests::tracked_payload;

use crate::mqtt_serde::parser::ParseOk;

fn packets(mut bytes: &[u8], version: u8) -> Vec<MqttPacket> {
    let mut result = Vec::new();
    while !bytes.is_empty() {
        let ParseOk::Packet(packet, consumed) =
            MqttPacket::from_bytes_with_version(bytes, version).unwrap()
        else {
            panic!("incomplete packet");
        };
        result.push(packet);
        bytes = &bytes[consumed..];
    }
    result
}

fn publication(packet: &MqttPacket) -> (&str, &[u8]) {
    match packet {
        MqttPacket::Publish3(packet) => (&packet.topic_name, &packet.payload),
        MqttPacket::Publish5(packet) => (&packet.topic_name, &packet.payload),
        packet => panic!("expected PUBLISH, got {packet:?}"),
    }
}

#[test]
fn publish_moves_owned_topic_and_payload_into_inflight() {
    for version in [4, 5] {
        for qos in [1, 2] {
            for encoded in [false, true] {
                let mut engine =
                    MqttEngine::new(MqttClientOptions::builder().mqtt_version(version).build());
                connect(&mut engine, vec![]);
                let command = PublishCommand::simple("owned/topic", vec![42; 64], qos, false);
                let topic_ptr = command.topic_name.as_ptr();
                let payload_ptr = command.payload.as_ptr();
                let id = if encoded {
                    engine.publish_encoded(command, Some(7)).unwrap().0.unwrap()
                } else {
                    engine.publish(command).unwrap().unwrap()
                };
                let entry = engine.inflight_queue.get(id).unwrap();
                let (topic, payload) = publication(&entry.packet);
                assert_eq!(topic.as_ptr(), topic_ptr);
                assert_eq!(payload.as_ptr(), payload_ptr);
                assert_eq!(entry.stream, encoded.then_some(7));
            }
        }
    }
}

#[test]
fn validated_encoding_and_single_output_keep_their_allocation() {
    let mut engine = MqttEngine::new(MqttClientOptions::default());
    connect(&mut engine, vec![]);
    let packet = MqttPacket::Publish5(
        PublishCommand::simple("one", vec![9; 64], 0, false).into_mqtt_publish(),
    );
    let encoded = packet.to_bytes().unwrap();
    let pointer = encoded.as_ptr();
    let expected = encoded.clone();
    engine.state.queued_bytes = encoded.len();
    engine.priority_queue.enqueue(128, packet);
    engine.process_queue_with_encoding(Some(encoded));
    assert_eq!(engine.outgoing_buffer.front().unwrap().as_ptr(), pointer);
    assert_eq!(engine.state.queued_bytes, 0);
    let output = engine.take_outgoing();
    assert_eq!(output.as_ptr(), pointer);
    assert_eq!(output, expected);
}

#[test]
fn deferred_queue_preserves_priority_and_drains_after_existing_output() {
    for version in [4, 5] {
        for next_priority in [128, 255] {
            let mut engine = MqttEngine::new(
                MqttClientOptions::builder()
                    .mqtt_version(version)
                    .max_outgoing_packet_count(2)
                    .build(),
            );
            connect(&mut engine, vec![]);
            for value in [1, 2] {
                engine
                    .publish(PublishCommand::simple("ready", vec![value], 0, false))
                    .unwrap();
            }
            let deferred = PublishCommand::simple("queued", vec![3; 64], 0, false);
            let payload_ptr = deferred.payload.as_ptr();
            engine.publish(deferred).unwrap();
            let (_, first) = engine.priority_queue.peek().unwrap();
            assert_eq!(publication(first).1.as_ptr(), payload_ptr);
            engine
                .publish(PublishCommand::with_priority(
                    "queued",
                    vec![4; 64],
                    0,
                    false,
                    next_priority,
                ))
                .unwrap();
            let output = packets(&engine.take_outgoing(), version);
            let values: Vec<_> = output
                .iter()
                .map(|packet| publication(packet).1[0])
                .collect();
            assert_eq!(
                values,
                if next_priority == 128 {
                    vec![1, 2, 3, 4]
                } else {
                    vec![1, 2, 4, 3]
                }
            );
            assert_eq!(engine.state.queued_bytes, 0);
            assert!(engine.priority_queue.is_empty());
            assert!(!engine.has_pending_output());
        }
    }
}

#[test]
fn deferred_publication_is_revalidated_after_reconnect() {
    for property in [
        Property::MaximumPacketSize(8),
        Property::MaximumQoS(0),
        Property::RetainAvailable(0),
    ] {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .max_outgoing_packet_count(1)
                .build(),
        );
        connect(&mut engine, vec![]);
        engine
            .publish(PublishCommand::simple("full", vec![], 0, false))
            .unwrap();
        let id = engine
            .publish(PublishCommand::simple("later", vec![7; 32], 1, true))
            .unwrap()
            .unwrap();
        assert_eq!(engine.priority_queue.len(), 1);
        assert!(engine.state.queued_bytes > 0);
        engine.handle_connection_lost();
        engine.take_events();
        let events = connect(&mut engine, vec![property.clone()]);
        assert!(events.iter().any(|event| matches!(event, MqttEvent::OperationFailed { operation: OperationKind::Publish, packet_id: Some(failed), .. } if *failed == id)), "{property:?}: {events:?}");
        assert!(engine.take_outgoing().is_empty());
        assert_eq!(engine.state.queued_bytes, 0);
        assert!(!engine.state.reserved.contains(&id));
    }
}

#[test]
fn deferred_alias_encoding_is_rebuilt_for_new_transport() {
    let mut engine = MqttEngine::new(
        MqttClientOptions::builder()
            .max_outgoing_packet_count(1)
            .build(),
    );
    connect(&mut engine, vec![Property::TopicAliasMaximum(1)]);
    engine
        .publish(PublishCommand::simple("full", vec![], 0, false))
        .unwrap();
    let mut command = PublishCommand::simple("canonical/topic", vec![5; 64], 0, false);
    let user_properties = vec![
        Property::UserProperty("key".into(), "first".into()),
        Property::UserProperty("key".into(), "second".into()),
    ];
    command.properties = user_properties.clone();
    command.properties.push(Property::TopicAlias(1));
    engine.publish(command).unwrap();
    engine.handle_connection_lost();
    connect(&mut engine, vec![]);
    let output = packets(&engine.take_outgoing(), 5);
    let [MqttPacket::Publish5(packet)] = output.as_slice() else {
        panic!("{output:?}")
    };
    assert_eq!(packet.topic_name, "canonical/topic");
    assert_eq!(packet.payload, vec![5; 64]);
    assert_eq!(packet.properties, user_properties);
    assert_eq!(engine.state.queued_bytes, 0);
}

#[test]
fn inflight_quota_defers_output_until_acknowledged() {
    let mut engine = MqttEngine::new(MqttClientOptions::default());
    connect(&mut engine, vec![Property::ReceiveMaximum(1)]);
    let first = engine
        .publish(PublishCommand::simple("one", vec![1; 64], 1, false))
        .unwrap()
        .unwrap();
    engine.take_outgoing();
    let command = PublishCommand::simple("two", vec![2; 64], 1, false);
    let payload_ptr = command.payload.as_ptr();
    let second = engine.publish(command).unwrap().unwrap();
    assert!(engine.take_outgoing().is_empty());
    let (_, packet) = engine.priority_queue.peek().unwrap();
    assert_eq!(publication(packet).1.as_ptr(), payload_ptr);
    engine.handle_incoming(&[0x40, 2, (first >> 8) as u8, first as u8]);
    let output = packets(&engine.take_outgoing(), 5);
    assert!(
        matches!(output.as_slice(), [MqttPacket::Publish5(packet)] if packet.packet_id == Some(second))
    );
    assert_eq!(
        publication(&engine.inflight_queue.get(second).unwrap().packet)
            .1
            .as_ptr(),
        payload_ptr
    );
    assert_eq!(engine.state.queued_bytes, 0);
}

#[test]
fn response_failure_discards_transient_encoding_before_reconnect() {
    let mut engine = MqttEngine::new(
        MqttClientOptions::builder()
            .max_outgoing_buffer_bytes(256)
            .build(),
    );
    connect(&mut engine, vec![Property::TopicAliasMaximum(1)]);
    // An oversized pending protocol response fails the transport while the
    // publish queue is trying to progress, normalizing its topic aliases.
    engine
        .state
        .responses
        .push_back(MqttPacket::PubAck5(MqttPubAck::new(
            1,
            0,
            vec![Property::ReasonString("x".repeat(256))],
        )));
    let mut command = PublishCommand::simple("canonical", vec![7; 16], 0, false);
    command.properties.push(Property::TopicAlias(1));
    engine.publish(command).unwrap();
    assert!(!engine.is_connected());
    assert!(engine.take_outgoing().is_empty());
    let (_, packet) = engine.priority_queue.peek().unwrap();
    assert!(matches!(packet, MqttPacket::Publish5(packet) if packet.properties.is_empty()));
    assert_eq!(engine.state.queued_bytes, packet.to_bytes().unwrap().len());
    connect(&mut engine, vec![]);
    let output = packets(&engine.take_outgoing(), 5);
    assert!(
        matches!(output.as_slice(), [MqttPacket::Publish5(packet)] if packet.topic_name == "canonical" && packet.properties.is_empty())
    );
    assert_eq!(engine.state.queued_bytes, 0);
}

#[cfg(feature = "shared-payloads")]
fn ack(kind: u8, id: u16) -> [u8; 4] {
    [kind, 2, (id >> 8) as u8, id as u8]
}

#[cfg(feature = "shared-payloads")]
fn complete(engine: &mut MqttEngine, qos: u8, id: u16) {
    let mut events = engine.handle_incoming(&ack(if qos == 1 { 0x40 } else { 0x50 }, id));
    if qos == 2 {
        assert_eq!(engine.take_outgoing(), ack(0x62, id));
        events = engine.handle_incoming(&ack(0x70, id));
    }
    assert!(events.iter().any(|event| matches!(event, MqttEvent::Published(result) if result.packet_id == Some(id) && result.is_success())));
}

#[cfg(feature = "shared-payloads")]
#[test]
fn shared_payload_retained_through_queue_inflight_and_released_after_ack() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let mut engine = MqttEngine::new(
                MqttClientOptions::builder()
                    .mqtt_version(version)
                    .max_outgoing_packet_count(1)
                    .build(),
            );
            connect(&mut engine, vec![]);
            engine
                .publish(PublishCommand::simple("block", vec![0], 0, false))
                .unwrap();
            let (backing, weak) = tracked_payload(65536);
            let application = backing.slice(17..4113);
            drop(backing);
            let ptr = application.as_ptr();
            let id = engine
                .publish(PublishCommand::from_bytes(
                    "shared",
                    application.clone(),
                    qos,
                    false,
                ))
                .unwrap();
            let (_, queued) = engine.priority_queue.peek().unwrap();
            assert_eq!(publication(queued).1.as_ptr(), ptr);
            assert_eq!(engine.state.queued_bytes, queued.to_bytes().unwrap().len());
            let wire = packets(&engine.take_outgoing(), version);
            assert_eq!(publication(&wire[1]).1, &[42; 4096]);
            assert_eq!(engine.state.queued_bytes, 0);
            if let Some(id) = id {
                assert_eq!(
                    publication(&engine.inflight_queue.get(id).unwrap().packet)
                        .1
                        .as_ptr(),
                    ptr
                );
                drop(application);
                assert!(weak.upgrade().is_some());
                complete(&mut engine, qos, id);
                assert!(engine.inflight_queue.get(id).is_none());
                assert!(!engine.state.reserved.contains(&id));
            } else {
                drop(application);
            }
            // Parsed wire output owns different storage and cannot mask a leak.
            assert!(weak.upgrade().is_none(), "version={version} qos={qos}");
        }
    }
}

#[cfg(feature = "shared-payloads")]
#[test]
fn shared_payload_encoded_publish_keeps_app_handle_and_releases_sdk_handle() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let mut engine =
                MqttEngine::new(MqttClientOptions::builder().mqtt_version(version).build());
            connect(&mut engine, vec![]);
            let application = bytes::Bytes::from(vec![42; 4096]);
            let (id, wire) = engine
                .publish_encoded(
                    PublishCommand::from_bytes("shared", application.clone(), qos, false),
                    None,
                )
                .unwrap();
            assert_eq!(
                publication(&packets(&wire, version)[0]).1,
                application.as_ref()
            );
            if let Some(id) = id {
                assert!(!application.is_unique());
                assert_eq!(
                    publication(&engine.inflight_queue.get(id).unwrap().packet)
                        .1
                        .as_ptr(),
                    application.as_ptr()
                );
                complete(&mut engine, qos, id);
            }
            assert!(application.is_unique());
            assert_eq!(application.as_ref(), &[42; 4096]);
        }
    }
}

#[cfg(feature = "shared-payloads")]
#[test]
fn shared_payload_reconnect_replay_waiting_for_quota_keeps_original_storage() {
    for version in [4, 5] {
        for qos in [1, 2] {
            let mut engine = MqttEngine::new(
                MqttClientOptions::builder()
                    .mqtt_version(version)
                    .clean_start(false)
                    .receive_maximum(2)
                    .build(),
            );
            connect(&mut engine, vec![]);
            let first = engine
                .publish(PublishCommand::simple("first", vec![1], 1, false))
                .unwrap()
                .unwrap();
            let (application, weak) = tracked_payload(4096);
            let ptr = application.as_ptr();
            let second = engine
                .publish(PublishCommand::from_bytes(
                    "second",
                    application.clone(),
                    qos,
                    false,
                ))
                .unwrap()
                .unwrap();
            engine.take_outgoing();
            engine.handle_connection_lost();
            engine.connect().unwrap();
            engine.take_outgoing();
            assert_eq!(
                publication(
                    &engine
                        .previous_inflight
                        .iter()
                        .find(|(id, _)| *id == second)
                        .unwrap()
                        .1
                )
                .1
                .as_ptr(),
                ptr
            );
            engine.inflight_queue.update_receive_maximum(1);
            let events = engine.handle_incoming(if version == 5 {
                &[0x20, 3, 1, 0, 0]
            } else {
                &[0x20, 2, 1, 0]
            });
            assert!(events.iter().any(
                |event| matches!(event, MqttEvent::Connected(result) if result.session_present)
            ));
            assert_eq!(engine.session_replay.len(), 1);
            assert_eq!(
                publication(&engine.session_replay.front().unwrap().1)
                    .1
                    .as_ptr(),
                ptr
            );
            engine.take_outgoing();
            complete(&mut engine, 1, first);
            let replay = packets(&engine.take_outgoing(), version);
            assert_eq!(publication(&replay[0]).1, application.as_ref());
            assert!(engine.session_replay.is_empty());
            assert_eq!(
                publication(&engine.inflight_queue.get(second).unwrap().packet)
                    .1
                    .as_ptr(),
                ptr
            );
            drop(application);
            assert!(weak.upgrade().is_some());
            complete(&mut engine, qos, second);
            assert!(weak.upgrade().is_none());
        }
    }
}

#[cfg(feature = "shared-payloads")]
#[test]
fn shared_payload_rejection_and_engine_drop_release_storage() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let mut engine = MqttEngine::new(
                MqttClientOptions::builder()
                    .mqtt_version(version)
                    .max_outgoing_packet_count(1)
                    .build(),
            );
            connect(&mut engine, vec![]);
            let (payload, weak) = tracked_payload(4096);
            assert!(engine
                .publish(PublishCommand::from_bytes("invalid/#", payload, qos, false))
                .is_err());
            assert!(weak.upgrade().is_none());
            let (payload, first) = tracked_payload(4096);
            engine
                .publish(PublishCommand::from_bytes("first", payload, qos, false))
                .unwrap();
            let (payload, queued) = tracked_payload(4096);
            engine
                .publish(PublishCommand::from_bytes("queued", payload, qos, false))
                .unwrap();
            let (payload, rejected) = tracked_payload(4096);
            assert!(matches!(
                engine.publish(PublishCommand::from_bytes("rejected", payload, qos, false)),
                Err(MqttClientError::BufferFull { .. })
            ));
            assert!(rejected.upgrade().is_none());
            assert!(queued.upgrade().is_some());
            drop(engine);
            assert!(first.upgrade().is_none());
            assert!(queued.upgrade().is_none());
        }
    }
}

#[cfg(feature = "shared-payloads")]
#[test]
fn shared_payload_static_and_unique_slices_keep_pointer_and_wire_contents() {
    static PAYLOAD: [u8; 64] = [42; 64];
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            for sliced in [false, true] {
                let payload = if sliced {
                    let backing = bytes::Bytes::from(vec![42; 65536]);
                    let slice = backing.slice(17..81);
                    drop(backing);
                    assert!(slice.is_unique());
                    slice
                } else {
                    bytes::Bytes::from_static(&PAYLOAD)
                };
                let ptr = payload.as_ptr();
                let command = PublishCommand::from_bytes("t", payload, qos, false);
                assert_eq!(command.payload.as_ptr(), ptr);
                let mut engine =
                    MqttEngine::new(MqttClientOptions::builder().mqtt_version(version).build());
                connect(&mut engine, vec![]);
                let id = engine.publish(command).unwrap();
                let output = packets(&engine.take_outgoing(), version);
                assert_eq!(publication(&output[0]).1, &PAYLOAD);
                if let Some(id) = id {
                    assert_eq!(
                        publication(&engine.inflight_queue.get(id).unwrap().packet)
                            .1
                            .as_ptr(),
                        ptr
                    );
                    complete(&mut engine, qos, id);
                }
            }
        }
    }
}

#[cfg(feature = "shared-payloads")]
#[test]
fn shared_payload_negative_ack_releases_storage() {
    for qos in [1, 2] {
        let mut engine = MqttEngine::new(MqttClientOptions::default());
        connect(&mut engine, vec![]);
        let (payload, weak) = tracked_payload(4096);
        let id = engine
            .publish(PublishCommand::from_bytes("t", payload, qos, false))
            .unwrap()
            .unwrap();
        engine.take_outgoing();
        let events = engine.handle_incoming(&[
            if qos == 1 { 0x40 } else { 0x50 },
            4,
            (id >> 8) as u8,
            id as u8,
            0x80,
            0,
        ]);
        assert!(events
            .iter()
            .any(|event| matches!(event, MqttEvent::Published(result) if !result.is_success())));
        assert!(weak.upgrade().is_none());
        assert!(!engine.state.reserved.contains(&id));
    }
}

#[cfg(feature = "durable-session")]
#[cfg(feature = "shared-payloads")]
#[test]
fn shared_payload_checkpoint_shares_storage_and_keeps_v1_schema() {
    for version in [4, 5] {
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .mqtt_version(version)
                .client_id("checkpoint")
                .clean_start(false)
                .build(),
        );
        connect(&mut engine, vec![]);
        let (payload, weak) = tracked_payload(4096);
        let ptr = payload.as_ptr();
        let id = engine
            .publish(PublishCommand::from_bytes("shared", payload, 1, false))
            .unwrap()
            .unwrap();
        engine.take_outgoing();
        let snapshot = engine.snapshot_session().unwrap();
        assert_eq!(publication(&snapshot.outbound[0].1).1.as_ptr(), ptr);
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["version"], 1);
        let restored = serde_json::from_value(json).unwrap();
        let mut restored_engine = MqttEngine::new(
            MqttClientOptions::builder()
                .mqtt_version(version)
                .client_id("checkpoint")
                .clean_start(false)
                .build(),
        );
        restored_engine.restore_session_state(restored).unwrap();
        assert_eq!(
            publication(&restored_engine.previous_inflight[0].1).1,
            &[42; 4096]
        );
        complete(&mut engine, 1, id);
        assert!(weak.upgrade().is_some());
        drop(snapshot);
        assert!(weak.upgrade().is_none());
    }
}
