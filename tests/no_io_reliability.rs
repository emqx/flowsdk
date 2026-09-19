// SPDX-License-Identifier: MPL-2.0
use flowsdk::mqtt_client::{
    MqttClientError, MqttClientOptions, MqttEvent, NoIoMqttClient, OperationKind,
    OperationTimeouts, PublishCommand, SubscribeCommand,
};
use flowsdk::mqtt_serde::{
    control_packet::MqttPacket,
    mqttv5::{common::properties::Property, connackv5::MqttConnAck, publishv5::MqttPublish},
    parser::ParseOk,
};
use std::time::{Duration, Instant};

fn connack(present: bool, properties: Vec<Property>) -> Vec<u8> {
    MqttPacket::ConnAck5(MqttConnAck::new(present, 0, Some(properties)))
        .to_bytes()
        .unwrap()
}

#[test]
fn session_present_without_local_session_state_disconnects() {
    let mut client = NoIoMqttClient::new(
        MqttClientOptions::builder()
            .clean_start(false)
            .session_expiry_interval(3600)
            .build(),
    );
    client.connect().unwrap();
    client.take_outgoing();
    // Allocating an identifier for new work does not establish a resumable session.
    client
        .publish(PublishCommand::simple("queued", vec![1], 1, false))
        .unwrap();
    let events = client.handle_incoming(&connack(true, vec![]));
    assert!(!client.is_connected());
    assert!(events.iter().any(|event| matches!(
        event,
        MqttEvent::Error(MqttClientError::ProtocolViolation { .. })
    )));
    assert!(events
        .iter()
        .any(|event| matches!(event, MqttEvent::Disconnected(_))));
    assert!(!events
        .iter()
        .any(|event| matches!(event, MqttEvent::Connected(_))));
    assert!(client.take_outgoing().is_empty());
}

#[test]
fn mqtt3_session_present_without_local_state_disconnects() {
    for version in [3, 4] {
        let mut client = NoIoMqttClient::new(
            MqttClientOptions::builder()
                .mqtt_version(version)
                .clean_start(false)
                .build(),
        );
        client.connect().unwrap();
        client.take_outgoing();
        let events = client.handle_incoming(&[0x20, 2, 1, 0]);
        assert!(!client.is_connected());
        assert!(events
            .iter()
            .any(|event| matches!(event, MqttEvent::Disconnected(_))));
    }
}

#[test]
fn an_established_session_can_resume_without_outstanding_messages() {
    let mut client = connected(
        MqttClientOptions::builder()
            .clean_start(false)
            .session_expiry_interval(3600)
            .build(),
        vec![],
    );
    client.handle_connection_lost();
    client.connect().unwrap();
    client.take_outgoing();
    let events = client.handle_incoming(&connack(true, vec![]));
    assert!(client.is_connected());
    assert!(events
        .iter()
        .any(|event| matches!(event, MqttEvent::Connected(result) if result.session_present)));
}

#[test]
fn unsuccessful_handshakes_do_not_establish_session_state() {
    for refused in [false, true] {
        let mut client =
            NoIoMqttClient::new(MqttClientOptions::builder().clean_start(false).build());
        client.connect().unwrap();
        client.take_outgoing();
        if refused {
            client.handle_incoming(&[0x20, 3, 0, 0x87, 0]);
        }
        client.handle_connection_lost();
        client.connect().unwrap();
        client.take_outgoing();
        let events = client.handle_incoming(&connack(true, vec![]));
        assert!(!client.is_connected());
        assert!(events
            .iter()
            .any(|event| matches!(event, MqttEvent::Disconnected(_))));
    }
}

#[test]
fn sessionless_client_rejects_resumption_after_discarding_state() {
    let mut client = connected(
        MqttClientOptions::builder()
            .clean_start(false)
            .sessionless(true)
            .build(),
        vec![],
    );
    client.handle_connection_lost();
    client.connect().unwrap();
    client.take_outgoing();
    let events = client.handle_incoming(&connack(true, vec![]));
    assert!(!client.is_connected());
    assert!(events
        .iter()
        .any(|event| matches!(event, MqttEvent::Disconnected(_))));
}
fn connected(options: MqttClientOptions, properties: Vec<Property>) -> NoIoMqttClient {
    let mut client = NoIoMqttClient::new(options);
    client.connect().unwrap();
    client.take_outgoing();
    client.handle_incoming(&connack(false, properties));
    assert!(client.is_connected());
    client
}
fn incoming(qos: u8, id: u16, dup: bool) -> Vec<u8> {
    MqttPacket::Publish5(MqttPublish::new(
        qos,
        "commands".into(),
        Some(id),
        b"run".to_vec(),
        false,
        dup,
    ))
    .to_bytes()
    .unwrap()
}
fn packets(bytes: &[u8]) -> Vec<MqttPacket> {
    let mut rest = bytes;
    let mut result = Vec::new();
    while !rest.is_empty() {
        match MqttPacket::from_bytes_with_version(rest, 5).unwrap() {
            ParseOk::Packet(p, n) => {
                result.push(p);
                rest = &rest[n..];
            }
            other => panic!("incomplete output: {other:?}"),
        }
    }
    result
}
fn drain(client: &mut NoIoMqttClient) -> Vec<MqttPacket> {
    let mut result = Vec::new();
    while client.has_pending_output() {
        result.extend(packets(&client.take_outgoing()));
    }
    result
}

#[test]
fn dedicated_connect_properties_are_encoded_and_conflicts_rejected() {
    let mut client = NoIoMqttClient::new(
        MqttClientOptions::builder()
            .session_expiry_interval(3600)
            .maximum_packet_size(1024)
            .incoming_receive_maximum(4)
            .request_response_information(true)
            .request_problem_information(false)
            .connect_properties(vec![Property::SessionExpiryInterval(3600)])
            .build(),
    );
    client.connect().unwrap();
    let MqttPacket::Connect5(p) = drain(&mut client).remove(0) else {
        panic!()
    };
    assert_eq!(p.properties.len(), 5);
    assert!(p
        .properties
        .contains(&Property::SessionExpiryInterval(3600)));
    assert!(p.properties.contains(&Property::ReceiveMaximum(4)));
    for properties in [
        vec![Property::SessionExpiryInterval(2)],
        vec![Property::ReceiveMaximum(0)],
        vec![Property::TopicAlias(1)],
    ] {
        let mut client = NoIoMqttClient::new(
            MqttClientOptions::builder()
                .session_expiry_interval(1)
                .connect_properties(properties)
                .build(),
        );
        assert!(client.connect().is_err());
        assert!(client.take_outgoing().is_empty());
    }
}

#[test]
fn reconnect_discards_old_transport_bytes_and_partial_input() {
    let mut client = connected(MqttClientOptions::default(), vec![]);
    client
        .publish(PublishCommand::simple("t", vec![1], 1, false))
        .unwrap();
    client.handle_incoming(&[0x30]);
    client.handle_connection_lost();
    client.connect().unwrap();
    assert!(matches!(
        drain(&mut client).as_slice(),
        [MqttPacket::Connect5(_)]
    ));
    client.handle_incoming(&connack(true, vec![]));
    assert!(client.is_connected());
    assert!(matches!(drain(&mut client).as_slice(), [MqttPacket::Publish5(p)] if p.dup));
}

#[test]
fn zero_pubrel_identifier_fails_without_panicking() {
    for version in [3, 4, 5] {
        let mut client =
            NoIoMqttClient::new(MqttClientOptions::builder().mqtt_version(version).build());
        client.connect().unwrap();
        client.take_outgoing();
        client.handle_incoming(if version == 5 {
            &[0x20, 3, 0, 0, 0]
        } else {
            &[0x20, 2, 0, 0]
        });
        let events = client.handle_incoming(&[0x62, 2, 0, 0]);
        assert!(events.iter().any(|e| matches!(e, MqttEvent::Error(_))));
        assert!(!client.is_connected());
        assert!(client.take_outgoing().is_empty());
    }
}

#[test]
fn qos2_delivers_once_and_resumes_receive_state() {
    for auto in [true, false] {
        let mut client = connected(MqttClientOptions::builder().auto_ack(auto).build(), vec![]);
        let events = client.handle_incoming(&incoming(2, 7, false));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, MqttEvent::MessageReceived(_)))
                .count(),
            1
        );
        if !auto {
            client.pubrec(7, 0, vec![]).unwrap();
        }
        drain(&mut client);
        client.handle_connection_lost();
        client.connect().unwrap();
        drain(&mut client);
        client.handle_incoming(&connack(true, vec![]));
        let events = client.handle_incoming(&incoming(2, 7, true));
        assert!(!events
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        assert!(matches!(
            drain(&mut client).as_slice(),
            [MqttPacket::PubRec5(_)]
        ));
        client.handle_incoming(&[0x62, 2, 0, 7]);
        if !auto {
            client.pubcomp(7, 0, vec![]).unwrap();
        }
        assert!(matches!(
            drain(&mut client).as_slice(),
            [MqttPacket::PubComp5(_)]
        ));
        assert!(client
            .handle_incoming(&incoming(2, 7, false))
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
    }
}

#[test]
fn packet_ids_skip_outstanding_and_reject_caller_collisions() {
    let mut client = connected(MqttClientOptions::builder().keep_alive(0).build(), vec![]);
    let first = client
        .publish(PublishCommand::simple("t", vec![], 1, false))
        .unwrap()
        .unwrap();
    drain(&mut client);
    for _ in 0..65534 {
        let id = client.subscribe(SubscribeCommand::single("t", 0)).unwrap();
        drain(&mut client);
        client.handle_incoming(&[0x90, 4, (id >> 8) as u8, id as u8, 0, 0]);
    }
    let next = client
        .publish(PublishCommand::simple("t", vec![], 1, false))
        .unwrap()
        .unwrap();
    assert_ne!(first, next);
    let mut command = PublishCommand::simple("t", vec![], 1, false);
    command.packet_id = Some(first);
    assert!(matches!(
        client.publish(command),
        Err(MqttClientError::InvalidPacketId { .. })
    ));
}

#[test]
fn broker_limits_are_enforced() {
    let mut client = connected(
        MqttClientOptions::default(),
        vec![
            Property::ServerKeepAlive(2),
            Property::MaximumPacketSize(32),
            Property::MaximumQoS(0),
            Property::RetainAvailable(0),
            Property::WildcardSubscriptionAvailable(0),
        ],
    );
    assert!(client.next_tick_at().unwrap() <= Instant::now() + Duration::from_secs(2));
    assert!(client
        .publish(PublishCommand::simple("t", vec![0; 32], 0, false))
        .is_err());
    assert!(client
        .publish(PublishCommand::simple("t", vec![], 1, false))
        .is_err());
    assert!(client
        .publish(PublishCommand::simple("t", vec![], 0, true))
        .is_err());
    assert!(client
        .subscribe(SubscribeCommand::single("t/#", 0))
        .is_err());
    assert!(!client.has_pending_output());
}

#[test]
fn output_pressure_never_loses_automatic_ack() {
    let mut client = connected(
        MqttClientOptions::builder()
            .max_outgoing_packet_count(1)
            .build(),
        vec![],
    );
    client
        .publish(PublishCommand::simple("t", vec![], 0, false))
        .unwrap();
    client.handle_incoming(&incoming(1, 9, false));
    let output = drain(&mut client);
    assert!(output
        .iter()
        .any(|p| matches!(p, MqttPacket::PubAck5(a) if a.packet_id == 9)));
}

#[test]
fn manual_ack_is_transactional_and_checks_stage() {
    let mut client = connected(
        MqttClientOptions::builder()
            .auto_ack(false)
            .max_outgoing_packet_count(1)
            .build(),
        vec![],
    );
    client.handle_incoming(&incoming(1, 9, false));
    client
        .publish(PublishCommand::simple("t", vec![], 0, false))
        .unwrap();
    assert!(client.puback(9, 0, vec![]).is_err());
    drain(&mut client);
    client.puback(9, 0, vec![]).unwrap();
    assert!(client.puback(9, 0, vec![]).is_err());
    drain(&mut client);
    client.handle_incoming(&incoming(2, 10, false));
    assert!(client.pubcomp(10, 0, vec![]).is_err());
    client.pubrec(10, 0, vec![]).unwrap();
    assert!(client.pubcomp(10, 0, vec![]).is_err());
}

#[test]
fn deadlines_are_opt_in_and_late_ack_still_completes() {
    let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    client.connect().unwrap();
    assert!(client.next_tick_at().is_none());
    let mut client = NoIoMqttClient::new(
        MqttClientOptions::builder()
            .operation_timeouts(OperationTimeouts::cloud())
            .build(),
    );
    client.connect().unwrap();
    let at = client.next_tick_at().unwrap();
    assert!(client.handle_tick(at).iter().any(|e| matches!(
        e,
        MqttEvent::OperationFailed {
            operation: OperationKind::Connect,
            ..
        }
    )));
    let mut client = connected(
        MqttClientOptions::builder()
            .keep_alive(0)
            .operation_timeouts(OperationTimeouts::cloud())
            .build(),
        vec![],
    );
    let id = client
        .publish(PublishCommand::simple("t", vec![], 1, false))
        .unwrap()
        .unwrap();
    drain(&mut client);
    let at = client.next_tick_at().unwrap();
    assert!(client.handle_tick(at).iter().any(
        |e| matches!(e, MqttEvent::OperationFailed { packet_id: Some(pid), .. } if *pid == id)
    ));
    assert!(client.handle_tick(at + Duration::from_secs(1)).is_empty());
    assert!(client
        .handle_incoming(&[0x40, 2, (id >> 8) as u8, id as u8])
        .iter()
        .any(|e| matches!(e, MqttEvent::Published(_))));
}

#[test]
fn retry_is_scheduled_once_and_intentional_disconnect_cancels_it() {
    let mut client = connected(MqttClientOptions::builder().reconnect(true).build(), vec![]);
    client.handle_connection_lost();
    let deadline = client.next_tick_at().unwrap();
    client.schedule_reconnect(Instant::now());
    assert_eq!(client.next_tick_at(), Some(deadline));
    assert!(!client
        .take_events()
        .iter()
        .any(|e| matches!(e, MqttEvent::ReconnectNeeded)));
    assert!(client
        .handle_tick(deadline)
        .iter()
        .any(|e| matches!(e, MqttEvent::ReconnectNeeded)));
    client.schedule_reconnect(deadline);
    client.disconnect().unwrap();
    assert!(client.next_tick_at().is_none());
}

#[test]
fn oversized_frame_is_rejected_from_its_header() {
    let mut client = connected(
        MqttClientOptions::builder()
            .max_incoming_packet_size(32)
            .build(),
        vec![],
    );
    let events = client.handle_incoming(&[0x30, 127]);
    assert!(!client.is_connected());
    assert!(events.iter().any(|e| matches!(e, MqttEvent::Error(_))));
}

#[test]
fn qos2_send_quota_is_held_until_pubcomp() {
    let mut client = connected(
        MqttClientOptions::default(),
        vec![Property::ReceiveMaximum(1)],
    );
    let id = client
        .publish(PublishCommand::simple("t", vec![], 2, false))
        .unwrap()
        .unwrap();
    drain(&mut client);
    client
        .publish(PublishCommand::simple("t", vec![], 1, false))
        .unwrap();
    client.handle_incoming(&[0x50, 2, (id >> 8) as u8, id as u8]);
    assert!(matches!(
        drain(&mut client).as_slice(),
        [MqttPacket::PubRel5(_)]
    ));
    client.handle_incoming(&[0x70, 2, (id >> 8) as u8, id as u8]);
    assert!(matches!(
        drain(&mut client).as_slice(),
        [MqttPacket::Publish5(_)]
    ));
}

#[test]
fn failed_subscribe_does_not_reserve_packet_id() {
    let mut client = connected(
        MqttClientOptions::builder()
            .max_outgoing_packet_count(1)
            .build(),
        vec![],
    );
    client
        .publish(PublishCommand::simple("t", vec![], 0, false))
        .unwrap();
    let mut command = SubscribeCommand::single("t", 0);
    command.packet_id = Some(99);
    assert!(client.subscribe(command.clone()).is_err());
    drain(&mut client);
    assert_eq!(client.subscribe(command).unwrap(), 99);
}

#[test]
fn aliases_resolve_and_are_reset_on_reconnect() {
    let mut client = connected(
        MqttClientOptions::builder()
            .connect_properties(vec![Property::TopicAliasMaximum(2)])
            .build(),
        vec![Property::TopicAliasMaximum(2)],
    );
    let p = |topic: &str, alias| {
        MqttPacket::Publish5(MqttPublish::new_with_prop(
            0,
            topic.into(),
            None,
            vec![1],
            false,
            false,
            vec![Property::TopicAlias(alias)],
        ))
        .to_bytes()
        .unwrap()
    };
    client.handle_incoming(&p("commands", 1));
    assert!(client.handle_incoming(&p("", 1)).iter().any(
        |e| matches!(e, MqttEvent::MessageReceived(message) if message.topic_name == "commands")
    ));
    let mut command = PublishCommand::simple("data", vec![], 1, false);
    command.properties = vec![Property::TopicAlias(1)];
    client.publish(command).unwrap();
    drain(&mut client);
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![]));
    assert!(
        matches!(drain(&mut client).as_slice(), [MqttPacket::Publish5(p)] if p.topic_name == "data" && !p.properties.iter().any(|v| matches!(v, Property::TopicAlias(_))))
    );
    assert!(client
        .handle_incoming(&p("", 1))
        .iter()
        .any(|e| matches!(e, MqttEvent::Error(_))));
}

#[test]
fn resumed_publications_obey_a_reduced_send_quota() {
    let mut client = connected(MqttClientOptions::default(), vec![]);
    for _ in 0..3 {
        client
            .publish(PublishCommand::simple("t", vec![], 1, false))
            .unwrap();
    }
    drain(&mut client);
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![Property::ReceiveMaximum(1)]));
    for _ in 0..3 {
        let output = drain(&mut client);
        let [MqttPacket::Publish5(p)] = output.as_slice() else {
            panic!("{output:?}")
        };
        let id = p.packet_id.unwrap();
        client.handle_incoming(&[0x40, 2, (id >> 8) as u8, id as u8]);
    }
    assert!(drain(&mut client).is_empty());
}

#[test]
fn pubrel_replay_ignores_reduced_receive_maximum() {
    for output_capacity in [1, 10] {
        let mut client = connected(
            MqttClientOptions::builder()
                .max_outgoing_packet_count(output_capacity)
                .build(),
            vec![Property::ReceiveMaximum(2)],
        );
        let mut ids = Vec::new();
        for _ in 0..2 {
            let id = client
                .publish(PublishCommand::simple("t", vec![], 2, false))
                .unwrap()
                .unwrap();
            drain(&mut client);
            client.handle_incoming(&[0x50, 2, (id >> 8) as u8, id as u8]);
            assert!(matches!(drain(&mut client).as_slice(),
                [MqttPacket::PubRel5(rel)] if rel.packet_id == id));
            ids.push(id);
        }
        client.handle_connection_lost();
        client.connect().unwrap();
        drain(&mut client);
        client.handle_incoming(&connack(true, vec![Property::ReceiveMaximum(1)]));

        let replayed = drain(&mut client);
        let expected: Vec<_> = ids
            .iter()
            .map(|&id| {
                MqttPacket::PubRel5(
                    flowsdk::mqtt_serde::mqttv5::pubrelv5::MqttPubRel::new_success(id),
                )
            })
            .collect();
        assert_eq!(replayed, expected, "output capacity {output_capacity}");
        // Both exchanges must remain registered until their own PUBCOMP arrives.
        for id in ids {
            let events = client.handle_incoming(&[0x70, 2, (id >> 8) as u8, id as u8]);
            assert!(events.iter().any(|event| matches!(event,
                MqttEvent::Published(result) if result.packet_id == Some(id) && result.qos == 2)));
            assert!(client.is_connected());
        }
        assert!(drain(&mut client).is_empty());
    }
}

#[test]
fn pubrel_replay_is_not_blocked_by_earlier_publish_replay() {
    let mut client = connected(MqttClientOptions::default(), vec![]);
    let mut publishes = Vec::new();
    let mut pubrels = Vec::new();
    for qos in [1, 1, 2, 2] {
        let id = client
            .publish(PublishCommand::simple("t", vec![], qos, false))
            .unwrap()
            .unwrap();
        drain(&mut client);
        if qos == 2 {
            client.handle_incoming(&[0x50, 2, (id >> 8) as u8, id as u8]);
            drain(&mut client);
            pubrels.push(id);
        } else {
            publishes.push(id);
        }
    }
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![Property::ReceiveMaximum(1)]));
    let replayed = drain(&mut client);
    let replayed_pubrels: Vec<_> = replayed
        .iter()
        .filter_map(|packet| match packet {
            MqttPacket::PubRel5(rel) => Some(rel.packet_id),
            _ => None,
        })
        .collect();
    assert_eq!(replayed_pubrels, pubrels);
    let replayed_publishes: Vec<_> = replayed
        .iter()
        .filter_map(|packet| match packet {
            MqttPacket::Publish5(p) => Some((p.packet_id.unwrap(), p.dup)),
            _ => None,
        })
        .collect();
    assert_eq!(replayed_publishes, [(publishes[0], true)]);
    let id = publishes[0];
    client.handle_incoming(&[0x40, 2, (id >> 8) as u8, id as u8]);
    assert!(matches!(drain(&mut client).as_slice(),
        [MqttPacket::Publish5(p)] if p.packet_id == Some(publishes[1]) && p.dup));
}

#[test]
fn pubrel_replay_does_not_consume_the_new_connections_publish_quota() {
    let mut client = connected(MqttClientOptions::default(), vec![]);
    let id = client
        .publish(PublishCommand::simple("old", vec![], 2, false))
        .unwrap()
        .unwrap();
    drain(&mut client);
    client.handle_incoming(&[0x50, 2, (id >> 8) as u8, id as u8]);
    drain(&mut client);
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![Property::ReceiveMaximum(1)]));
    assert!(matches!(drain(&mut client).as_slice(),
        [MqttPacket::PubRel5(rel)] if rel.packet_id == id));
    let first = client
        .publish(PublishCommand::simple("new", vec![], 1, false))
        .unwrap()
        .unwrap();
    let second = client
        .publish(PublishCommand::simple("new", vec![], 1, false))
        .unwrap()
        .unwrap();
    assert!(matches!(drain(&mut client).as_slice(),
        [MqttPacket::Publish5(p)] if p.packet_id == Some(first)));
    // PUBCOMP replenishes the new connection's quota, capped by Receive Maximum.
    client.handle_incoming(&[0x70, 2, (id >> 8) as u8, id as u8]);
    assert!(matches!(drain(&mut client).as_slice(),
        [MqttPacket::Publish5(p)] if p.packet_id == Some(second)));
    assert!(client.is_connected());
}

#[test]
fn queued_publications_leave_capacity_for_session_replay() {
    let mut client = connected(
        MqttClientOptions::builder()
            .max_outgoing_buffer_bytes(128)
            .build(),
        vec![Property::ReceiveMaximum(1)],
    );
    let first = client
        .publish(PublishCommand::simple("t", vec![1; 80], 1, false))
        .unwrap()
        .unwrap();
    drain(&mut client);
    client.handle_connection_lost();
    assert!(matches!(
        client.publish(PublishCommand::simple("t", vec![2; 40], 1, false)),
        Err(MqttClientError::BufferFull { .. })
    ));
    client
        .publish(PublishCommand::simple("t", vec![2; 20], 1, false))
        .unwrap();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![Property::ReceiveMaximum(1)]));
    assert!(
        matches!(drain(&mut client).as_slice(), [MqttPacket::Publish5(p)] if p.payload == vec![1; 80] && p.dup)
    );
    client.handle_incoming(&[0x40, 2, (first >> 8) as u8, first as u8]);
    assert!(
        matches!(drain(&mut client).as_slice(), [MqttPacket::Publish5(p)] if p.payload == vec![2; 20])
    );
}

#[test]
fn failed_connect_preserves_publications_waiting_for_replay() {
    let mut client = connected(
        MqttClientOptions::builder()
            .max_outgoing_packet_count(1)
            .build(),
        vec![],
    );
    for value in [1, 2] {
        client
            .publish(PublishCommand::simple("t", vec![value], 1, false))
            .unwrap();
        drain(&mut client);
    }
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![Property::ReceiveMaximum(1)]));
    assert_eq!(drain(&mut client).len(), 1);
    client.handle_connection_lost();
    client.ping().unwrap();
    assert!(matches!(
        client.connect(),
        Err(MqttClientError::BufferFull { .. })
    ));
    drain(&mut client);
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![]));
    let replayed = drain(&mut client);
    assert_eq!(replayed.len(), 2);
    assert!(replayed
        .iter()
        .all(|p| matches!(p, MqttPacket::Publish5(p) if p.dup)));
}

#[test]
fn expired_session_reports_failures_and_keeps_accepted_unsent_publications() {
    let mut client = connected(
        MqttClientOptions::default(),
        vec![Property::ReceiveMaximum(1)],
    );
    let first = client
        .publish(PublishCommand::simple("t", vec![1], 1, false))
        .unwrap();
    drain(&mut client);
    client
        .publish(PublishCommand::simple("t", vec![2], 1, false))
        .unwrap();
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    let events = client.handle_incoming(&connack(false, vec![]));
    assert!(events
        .iter()
        .any(|e| matches!(e, MqttEvent::OperationFailed { packet_id, .. } if *packet_id == first)));
    assert!(matches!(drain(&mut client).as_slice(), [MqttPacket::Publish5(p)] if p.payload == [2]));
}

#[test]
fn queued_publications_leave_capacity_for_receive_acknowledgements() {
    let mut client = connected(
        MqttClientOptions::builder()
            .max_outgoing_buffer_bytes(128)
            .build(),
        vec![Property::ReceiveMaximum(1)],
    );
    client
        .publish(PublishCommand::simple("t", vec![], 1, false))
        .unwrap();
    drain(&mut client);
    client
        .publish(PublishCommand::simple("t", vec![0; 90], 1, false))
        .unwrap();
    client.handle_incoming(&incoming(1, 42, false));
    assert!(matches!(
        drain(&mut client).as_slice(),
        [MqttPacket::PubAck5(_)]
    ));
}

#[test]
fn receive_maximum_and_malformed_connack_are_rejected() {
    let mut client = connected(
        MqttClientOptions::builder()
            .auto_ack(false)
            .incoming_receive_maximum(1)
            .build(),
        vec![],
    );
    client.handle_incoming(&incoming(1, 1, false));
    assert!(client
        .handle_incoming(&incoming(1, 2, false))
        .iter()
        .any(|e| matches!(e, MqttEvent::Error(_))));
    // Raw packets keep this endpoint test independent of encoder validation.
    for bytes in [
        vec![0x20, 6, 0, 0, 3, 0x21, 0, 0],
        vec![0x20, 9, 0, 0, 6, 0x13, 0, 1, 0x13, 0, 2],
        vec![0x20, 5, 0, 0, 2, 0x25, 2],
    ] {
        let mut client = NoIoMqttClient::new(MqttClientOptions::default());
        client.connect().unwrap();
        drain(&mut client);
        assert!(client
            .handle_incoming(&bytes)
            .iter()
            .any(|e| matches!(e, MqttEvent::Error(_))));
        assert!(!client.is_connected());
    }
}

#[test]
fn qos1_manual_receive_can_be_redelivered_after_reconnect() {
    let mut client = connected(MqttClientOptions::builder().auto_ack(false).build(), vec![]);
    client.handle_incoming(&incoming(1, 1, false));
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(true, vec![]));
    assert!(client
        .handle_incoming(&incoming(1, 1, true))
        .iter()
        .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
    client.puback(1, 0, vec![]).unwrap();
}

#[test]
fn incoming_publish_handles_every_fragment_boundary() {
    let bytes = incoming(1, 10, false);
    for split in 1..bytes.len() {
        let mut client = connected(MqttClientOptions::default(), vec![]);
        assert!(client.handle_incoming(&bytes[..split]).is_empty());
        assert!(client
            .handle_incoming(&bytes[split..])
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        assert!(matches!(
            drain(&mut client).as_slice(),
            [MqttPacket::PubAck5(_)]
        ));
    }
}

#[test]
fn subscribe_and_unsubscribe_deadlines_preserve_ids_until_ack() {
    use flowsdk::mqtt_client::UnsubscribeCommand;
    let mut client = connected(
        MqttClientOptions::builder()
            .keep_alive(0)
            .operation_timeouts(OperationTimeouts::cloud())
            .build(),
        vec![],
    );
    let sub = client.subscribe(SubscribeCommand::single("t", 0)).unwrap();
    let unsub = client
        .unsubscribe(UnsubscribeCommand::from_topics(vec!["other".into()]))
        .unwrap();
    drain(&mut client);
    let events = client.handle_tick(Instant::now() + Duration::from_secs(20));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, MqttEvent::OperationFailed { .. }))
            .count(),
        2
    );
    let mut command = SubscribeCommand::single("t", 0);
    command.packet_id = Some(sub);
    assert!(client.subscribe(command.clone()).is_err());
    client.handle_incoming(&[0x90, 4, (sub >> 8) as u8, sub as u8, 0, 0]);
    assert!(client.subscribe(command).is_ok());
    assert!(client
        .handle_incoming(&[0xb0, 4, (unsub >> 8) as u8, unsub as u8, 0, 0])
        .iter()
        .any(|e| matches!(e, MqttEvent::Unsubscribed(_))));
}

#[test]
fn mqtt3_qos2_suppresses_duplicates_at_every_fragment_boundary() {
    for version in [3, 4] {
        let mut client =
            NoIoMqttClient::new(MqttClientOptions::builder().mqtt_version(version).build());
        client.connect().unwrap();
        client.take_outgoing();
        client.handle_incoming(&[0x20, 2, 0, 0]);
        let packet =
            MqttPacket::Publish3(flowsdk::mqtt_serde::mqttv3::publishv3::MqttPublish::new(
                "commands".into(),
                2,
                vec![0; 256],
                Some(7),
                false,
                false,
            ));
        let bytes = packet.to_bytes().unwrap();
        for byte in &bytes[..bytes.len() - 1] {
            assert!(client.handle_incoming(&[*byte]).is_empty());
        }
        assert!(client
            .handle_incoming(&bytes[bytes.len() - 1..])
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        client.take_outgoing();
        let mut duplicate = bytes;
        duplicate[0] |= 8;
        assert!(!client
            .handle_incoming(&duplicate)
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
        assert_eq!(client.take_outgoing(), [0x50, 2, 0, 7]);
    }
}

#[test]
fn reconnect_does_not_reuse_aliases_in_unsent_publications() {
    let mut client = connected(
        MqttClientOptions::default(),
        vec![Property::TopicAliasMaximum(1), Property::ReceiveMaximum(1)],
    );
    let mut command = PublishCommand::simple("data", vec![1], 1, false);
    command.properties.push(Property::TopicAlias(1));
    client.publish(command.clone()).unwrap();
    drain(&mut client);
    command.payload = vec![2];
    client.publish(command).unwrap();
    client.handle_connection_lost();
    client.connect().unwrap();
    drain(&mut client);
    client.handle_incoming(&connack(false, vec![]));
    assert!(
        matches!(drain(&mut client).as_slice(), [MqttPacket::Publish5(p)] if p.payload == [2] && p.properties.is_empty())
    );
}

#[test]
fn packet_id_exhaustion_recovers_after_acknowledgement() {
    let mut client = connected(MqttClientOptions::builder().keep_alive(0).build(), vec![]);
    for _ in 0..u16::MAX {
        client
            .publish(PublishCommand::simple("t", vec![], 1, false))
            .unwrap();
        client.take_outgoing();
    }
    assert!(matches!(
        client.publish(PublishCommand::simple("t", vec![], 1, false)),
        Err(MqttClientError::PacketIdExhausted)
    ));
    client.handle_incoming(&[0x40, 2, 0, 1]);
    assert_eq!(
        client
            .publish(PublishCommand::simple("t", vec![], 1, false))
            .unwrap(),
        Some(1)
    );
}
