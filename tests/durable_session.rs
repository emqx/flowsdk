// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "durable-session")]

#[path = "../examples/support/session_store.rs"]
mod session_store;

use flowsdk::mqtt_client::{
    ClientSessionState, ClientSessionStore, MqttClientError, MqttClientOptions, MqttEvent,
    NoIoMqttClient, PublishCommand, SubscribeCommand,
};
use flowsdk::mqtt_serde::{
    control_packet::MqttPacket,
    mqttv5::{common::properties::Property, connackv5::MqttConnAck, publishv5::MqttPublish},
    parser::ParseOk,
};
use session_store::FileSessionStore;

fn options(version: u8) -> MqttClientOptions {
    MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("durable-test")
        .mqtt_version(version)
        .clean_start(false)
        .session_expiry_interval(3600)
        .auto_ack(false)
        .keep_alive(0)
        .reconnect(false)
        .build()
}

fn connack(
    client: &mut NoIoMqttClient,
    present: bool,
    properties: Vec<Property>,
) -> Vec<MqttEvent> {
    let bytes = if client.options().mqtt_version == 5 {
        MqttPacket::ConnAck5(MqttConnAck::new(present, 0, Some(properties)))
            .to_bytes()
            .unwrap()
    } else {
        vec![0x20, 2, u8::from(present), 0]
    };
    let events = client.handle_incoming(&bytes);
    assert!(client.is_connected(), "{events:?}");
    events
}

fn drain(client: &mut NoIoMqttClient) -> Vec<MqttPacket> {
    let bytes = client.take_outgoing();
    let mut rest = bytes.as_slice();
    let mut packets = Vec::new();
    while !rest.is_empty() {
        let ParseOk::Packet(packet, n) =
            MqttPacket::from_bytes_with_version(rest, client.options().mqtt_version).unwrap()
        else {
            panic!("Incomplete packet");
        };
        packets.push(packet);
        rest = &rest[n..];
    }
    packets
}

fn connected(version: u8) -> NoIoMqttClient {
    let mut client = NoIoMqttClient::new(options(version));
    client.connect().unwrap();
    drain(&mut client);
    connack(&mut client, false, vec![]);
    client
}

fn round_trip(client: NoIoMqttClient) -> NoIoMqttClient {
    let version = client.options().mqtt_version;
    let serialized = serde_json::to_vec(&client.snapshot_session().unwrap()).unwrap();
    drop(client);
    let state: ClientSessionState = serde_json::from_slice(&serialized).unwrap();
    let mut restored = NoIoMqttClient::new(options(version));
    restored.restore_session_state(state).unwrap();
    assert!(!restored.is_connected());
    assert!(drain(&mut restored).is_empty());
    restored
}

fn reconnect(
    client: &mut NoIoMqttClient,
    present: bool,
    properties: Vec<Property>,
) -> Vec<MqttEvent> {
    client.connect().unwrap();
    assert!(matches!(
        drain(client).as_slice(),
        [MqttPacket::Connect5(_) | MqttPacket::Connect3(_)]
    ));
    connack(client, present, properties)
}

fn ack(kind: u8, id: u16) -> Vec<u8> {
    vec![kind, 2, (id >> 8) as u8, id as u8]
}

fn assert_publish(packet: &MqttPacket, id: u16, qos: u8, dup: bool) {
    match packet {
        MqttPacket::Publish5(p) => assert_eq!((p.packet_id, p.qos, p.dup), (Some(id), qos, dup)),
        MqttPacket::Publish3(p) => assert_eq!((p.message_id, p.qos, p.dup), (Some(id), qos, dup)),
        other => panic!("Expected PUBLISH: {other:?}"),
    }
}

#[test]
fn file_store_lifecycle_survives_client_and_store_recreation() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = connected(5);
    let mut store = FileSessionStore::new(directory.path()).unwrap();
    // The backend is usable through a trait object as well as generically.
    let backend: &mut dyn ClientSessionStore<Error = std::io::Error> = &mut store;
    assert!(backend.resume("test").unwrap().is_none());
    let initial = client.snapshot_session().unwrap();
    assert_eq!(
        backend.update("test", &initial).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    backend.create("test", &initial).unwrap();
    assert_eq!(
        backend.create("test", &initial).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    let id = client
        .publish(PublishCommand::simple("topic", vec![7], 1, false))
        .unwrap()
        .unwrap();
    backend
        .update("test", &client.snapshot_session().unwrap())
        .unwrap();
    drop(client);
    drop(store);

    let mut store = FileSessionStore::new(directory.path()).unwrap();
    let state = store.resume("test").unwrap().unwrap();
    assert!(store.resume("test").unwrap().is_some()); // Loading is not destructive.
    let mut client = NoIoMqttClient::new(options(5));
    client.restore_session_state(state).unwrap();
    reconnect(&mut client, true, vec![]);
    let output = drain(&mut client);
    assert_eq!(output.len(), 1);
    assert_publish(&output[0], id, 1, true);
    client.handle_incoming(&ack(0x40, id));
    store
        .update("test", &client.snapshot_session().unwrap())
        .unwrap();
    drop(client);
    let mut client = NoIoMqttClient::new(options(5));
    client
        .restore_session_state(store.resume("test").unwrap().unwrap())
        .unwrap();
    reconnect(&mut client, true, vec![]);
    assert!(drain(&mut client).is_empty());
    store.delete("test").unwrap();
    store.delete("test").unwrap();
    assert!(store.resume("test").unwrap().is_none());
}

#[test]
fn corruption_is_a_storage_error_not_a_missing_session() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory
            .path()
            .join(format!("{}.json", hex::encode("test"))),
        b"truncated",
    )
    .unwrap();
    let mut store = FileSessionStore::new(directory.path()).unwrap();
    assert!(store.resume("test").is_err());
}

#[test]
fn outbound_qos_stages_survive_restart_and_finish_with_original_ids() {
    for version in [3, 4, 5] {
        for stage in [1, 2, 3] {
            let qos = if stage == 1 { 1 } else { 2 };
            let mut client = connected(version);
            let id = client
                .publish(PublishCommand::simple("topic", vec![1], qos, false))
                .unwrap()
                .unwrap();
            drain(&mut client);
            if stage == 3 {
                client.handle_incoming(&ack(0x50, id));
                drain(&mut client);
            }
            let mut client = round_trip(client);
            reconnect(&mut client, true, vec![]);
            let replay = drain(&mut client);
            assert_eq!(replay.len(), 1);
            if stage == 3 {
                assert!(
                    matches!(&replay[0], MqttPacket::PubRel5(p) if p.packet_id == id)
                        || matches!(&replay[0], MqttPacket::PubRel3(p) if p.message_id == id)
                );
            } else {
                assert_publish(&replay[0], id, qos, true);
            }
            let mut collision = PublishCommand::simple("collision", vec![], 1, false);
            collision.packet_id = Some(id);
            assert!(matches!(
                client.publish(collision),
                Err(MqttClientError::InvalidPacketId { .. })
            ));
            let next = client
                .publish(PublishCommand::simple("next", vec![], 1, false))
                .unwrap()
                .unwrap();
            assert_eq!(next, id + 1);
            if stage == 2 {
                client.handle_incoming(&ack(0x50, id));
            }
            let events = client.handle_incoming(&ack(if qos == 1 { 0x40 } else { 0x70 }, id));
            assert!(events
                .iter()
                .any(|e| matches!(e, MqttEvent::Published(p) if p.packet_id == Some(id))));
        }
    }
}

#[test]
fn incoming_qos2_stages_suppress_redelivery_after_restart() {
    for version in [4, 5] {
        for stage in [0, 1, 2] {
            let mut client = connected(version);
            let incoming = if version == 5 {
                MqttPacket::Publish5(MqttPublish::new(
                    2,
                    "topic".into(),
                    Some(71),
                    vec![1],
                    false,
                    false,
                ))
            } else {
                MqttPacket::Publish3(flowsdk::mqtt_serde::mqttv3::publishv3::MqttPublish::new(
                    "topic".into(),
                    2,
                    vec![1],
                    Some(71),
                    false,
                    false,
                ))
            };
            assert!(client
                .handle_incoming(&incoming.to_bytes().unwrap())
                .iter()
                .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
            if stage >= 1 {
                client.pubrec(71, 0, vec![]).unwrap();
                drain(&mut client);
            }
            if stage == 2 {
                client.handle_incoming(&ack(0x62, 71));
            }
            let mut client = round_trip(client);
            reconnect(&mut client, true, vec![]);
            if stage < 2 {
                let mut incoming = incoming.clone();
                incoming.set_dup(true);
                let events = client.handle_incoming(&incoming.to_bytes().unwrap());
                assert!(!events
                    .iter()
                    .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
                if stage == 0 {
                    client.pubrec(71, 0, vec![]).unwrap();
                }
                assert_eq!(drain(&mut client).len(), 1);
            }
            let events = client.handle_incoming(&ack(0x62, 71));
            assert!(events
                .iter()
                .any(|e| matches!(e, MqttEvent::PubRelReceived { packet_id: 71, .. })));
            client.pubcomp(71, 0, vec![]).unwrap();
            assert_eq!(drain(&mut client).len(), 1);
            assert!(client
                .handle_incoming(&incoming.to_bytes().unwrap())
                .iter()
                .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        }
    }
}

#[test]
fn restart_preserves_unsent_priorities_and_replay_waiting_for_quota() {
    let mut client = connected(5);
    let first = client
        .publish(PublishCommand::simple("first", vec![], 1, false))
        .unwrap()
        .unwrap();
    let second = client
        .publish(PublishCommand::simple("second", vec![], 1, false))
        .unwrap()
        .unwrap();
    drain(&mut client);
    client.handle_connection_lost();
    let low = client
        .publish(PublishCommand::with_priority("low", vec![], 1, false, 1))
        .unwrap()
        .unwrap();
    let high = client
        .publish(PublishCommand::with_priority("high", vec![], 1, false, 9))
        .unwrap()
        .unwrap();
    let mut client = round_trip(client);
    reconnect(&mut client, true, vec![Property::ReceiveMaximum(1)]);
    let output = drain(&mut client);
    assert_eq!(output.len(), 1);
    assert_publish(&output[0], first, 1, true);
    // Restart while second is waiting in the replay queue.
    let mut client = round_trip(client);
    // Interrupt CONNECT as well; pending replay must survive both checkpoints.
    client.connect().unwrap();
    drain(&mut client);
    let mut client = round_trip(client);
    reconnect(&mut client, true, vec![Property::ReceiveMaximum(1)]);
    for (id, dup) in [(first, true), (second, true), (high, false), (low, false)] {
        let output = drain(&mut client);
        assert_eq!(output.len(), 1);
        assert_publish(&output[0], id, 1, dup);
        client.handle_incoming(&ack(0x40, id));
    }
    assert!(drain(&mut client).is_empty());
}

#[test]
fn broker_session_loss_uses_existing_mqtt_version_recovery_rules() {
    for version in [4, 5] {
        let mut client = connected(version);
        let id = client
            .publish(PublishCommand::simple("topic", vec![], 1, false))
            .unwrap()
            .unwrap();
        let mut client = round_trip(client);
        let events = reconnect(&mut client, false, vec![]);
        let output = drain(&mut client);
        if version == 5 {
            assert!(output.is_empty());
            assert!(events.iter().any(|e| matches!(e, MqttEvent::OperationFailed { packet_id: Some(pid), error: MqttClientError::SessionExpired, .. } if *pid == id)));
        } else {
            assert_eq!(output.len(), 1);
            assert_publish(&output[0], id, 1, true);
        }
    }
}

#[test]
fn subscription_operations_are_failed_instead_of_replayed_after_restart() {
    let mut client = connected(5);
    let id = client
        .subscribe(SubscribeCommand::single("topic", 1))
        .unwrap();
    let mut client = round_trip(client);
    let events = reconnect(&mut client, true, vec![]);
    assert!(drain(&mut client).is_empty());
    assert!(events.iter().any(
        |e| matches!(e, MqttEvent::OperationFailed { packet_id: Some(pid), .. } if *pid == id)
    ));
}

#[test]
fn snapshots_strip_connection_aliases_and_keep_assigned_identity() {
    let mut client = NoIoMqttClient::new(options(5));
    client.connect().unwrap();
    drain(&mut client);
    connack(
        &mut client,
        false,
        vec![
            Property::AssignedClientIdentifier("assigned".into()),
            Property::TopicAliasMaximum(1),
        ],
    );
    let mut command = PublishCommand::simple("topic", vec![], 1, false);
    command.properties = vec![Property::TopicAlias(1)];
    let id = client.publish(command).unwrap().unwrap();
    let saved = client.snapshot_session().unwrap();
    assert_eq!(saved.client_id(), "assigned");
    assert_eq!(saved.peer(), "localhost:1883");
    assert_eq!(saved.mqtt_version(), 5);
    assert_eq!(saved.session_expiry_interval(), 3600);
    let mut opts = options(5);
    opts.client_id = saved.client_id().into();
    let mut restored = NoIoMqttClient::new(opts);
    restored.restore_session_state(saved).unwrap();
    reconnect(&mut restored, true, vec![]);
    let output = drain(&mut restored);
    assert_publish(&output[0], id, 1, true);
    assert!(
        matches!(&output[0], MqttPacket::Publish5(p) if p.topic_name == "topic" && p.properties.is_empty())
    );
}

#[test]
fn invalid_checkpoints_are_rejected_without_mutating_the_new_engine() {
    let mut client = connected(5);
    client
        .publish(PublishCommand::simple("topic", vec![], 1, false))
        .unwrap();
    let valid = client.snapshot_session().unwrap();
    let json = serde_json::to_value(&valid).unwrap();
    let mut invalid = Vec::new();
    for (field, value) in [
        ("version", serde_json::json!(999)),
        ("peer", serde_json::json!("other")),
        ("client_id", serde_json::json!("other")),
        ("mqtt_version", serde_json::json!(4)),
    ] {
        let mut changed = json.clone();
        changed[field] = value;
        invalid.push(changed);
    }
    let mut duplicate = json.clone();
    let packet = duplicate["outbound"][0].clone();
    duplicate["outbound"].as_array_mut().unwrap().push(packet);
    invalid.push(duplicate);
    let mut zero = json.clone();
    zero["outbound"][0][0] = serde_json::json!(0);
    invalid.push(zero);
    let mut malformed = json;
    malformed["outbound"][0][1]["topic_name"] = serde_json::json!("");
    invalid.push(malformed);
    for invalid in invalid {
        let mut client = NoIoMqttClient::new(options(5));
        assert!(client
            .restore_session_state(serde_json::from_value(invalid).unwrap())
            .is_err());
        assert!(!client.snapshot_session().unwrap().has_session());
        client.restore_session_state(valid.clone()).unwrap();
        assert!(client.restore_session_state(valid.clone()).is_err());
    }
    for (clean_start, sessionless) in [(true, false), (false, true)] {
        let mut opts = options(5);
        opts.clean_start = clean_start;
        opts.sessionless = sessionless;
        assert!(NoIoMqttClient::new(opts)
            .restore_session_state(valid.clone())
            .is_err());
    }
}

#[test]
fn restored_queue_must_fit_new_limits_without_evicting_messages() {
    let mut client = connected(5);
    client.handle_connection_lost();
    for _ in 0..2 {
        client
            .publish(PublishCommand::simple("topic", vec![0; 100], 1, false))
            .unwrap();
    }
    let saved = client.snapshot_session().unwrap();
    for byte_limit in [false, true] {
        let mut opts = options(5);
        if byte_limit {
            opts.max_outgoing_buffer_bytes = Some(100);
        } else {
            opts.max_outgoing_packet_count = 1;
        }
        let mut client = NoIoMqttClient::new(opts);
        assert!(client.restore_session_state(saved.clone()).is_err());
        assert!(!client.snapshot_session().unwrap().has_session());
    }
}

#[test]
fn empty_established_session_is_distinct_from_a_never_connected_checkpoint() {
    let mut resumed = round_trip(connected(5));
    reconnect(&mut resumed, true, vec![]);
    assert!(drain(&mut resumed).is_empty());
    let mut fresh = round_trip(NoIoMqttClient::new(options(5)));
    fresh.connect().unwrap();
    drain(&mut fresh);
    let events = fresh.handle_incoming(&[0x20, 3, 1, 0, 0]);
    assert!(!fresh.is_connected());
    assert!(events.iter().any(|e| matches!(
        e,
        MqttEvent::Error(MqttClientError::ProtocolViolation { .. })
    )));
}
