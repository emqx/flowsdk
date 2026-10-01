// SPDX-License-Identifier: MPL-2.0
use core::time::Duration;
use flowsdk::{
    mqtt_client::{
        inflight::InflightQueue, MqttEvent, OperationKind, OperationTimeouts, PortableMqttEngine,
        PortableNoIoMqttClient, PublishCommand, SubscribeCommand, UnsubscribeCommand,
    },
    mqtt_serde::{control_packet::MqttPacket, mqttv3::publish::MqttPublish},
    time::Timestamp,
};
use flowsdk_no_std_tests::{ack, bytes_smoke, connack, connected, options, protocol_smoke, time};

#[test]
fn qos_handshakes_all_versions() {
    protocol_smoke();
}

#[test]
fn shared_bytes_lifetime() {
    bytes_smoke();
}

#[test]
fn timestamp_boundaries_and_rounding() {
    assert!(Timestamp::try_from_uptime_millis(-1).is_err());
    assert!(Timestamp::try_from_millis(u64::MAX).is_err());
    let at = Timestamp::from_nanos(1_000_001);
    assert_eq!(at.as_millis(), 1);
    assert_eq!(at.as_millis_ceil(), 2);
    assert_eq!(time(2).as_millis_ceil(), 2);
    assert!(Timestamp::from_nanos(u64::MAX)
        .checked_add(Duration::from_nanos(1))
        .is_none());
    assert!(Timestamp::ZERO.checked_duration_since(at).is_none());
}

#[test]
fn backward_and_overflowing_time_do_not_consume_output_or_identifiers() {
    let mut engine = PortableMqttEngine::try_new_at(options(5), time(100)).unwrap();
    engine.connect_at(time(110)).unwrap();
    let old_deadline = engine.next_tick_at();
    assert!(engine.take_outgoing_at(time(109)).is_err());
    assert!(engine
        .handle_tick_at(Timestamp::from_nanos(u64::MAX))
        .is_err());
    assert_eq!(engine.next_tick_at(), old_deadline);
    assert!(engine.has_pending_output());
    assert!(engine.take_events().is_empty());
    assert_eq!(engine.take_outgoing_at(time(110)).unwrap()[0], 0x10);
    assert!(engine
        .publish_at(PublishCommand::simple("x", vec![], 1, false), time(109))
        .is_err());
    assert_eq!(engine.next_packet_id().unwrap(), 1);
    assert!(PortableMqttEngine::try_new_at(options(5), Timestamp::from_nanos(u64::MAX)).is_err());
    let mut bad = options(5);
    bad.operation_timeouts.connect = Some(Duration::MAX);
    assert!(PortableMqttEngine::try_new_at(bad, time(0)).is_err());
}

#[test]
fn nanosecond_connect_deadline_and_no_early_timeout() {
    let origin = time(900_000);
    let mut opts = options(5);
    opts.operation_timeouts.connect = Some(Duration::from_nanos(1));
    let mut client = PortableNoIoMqttClient::try_new_at(opts, origin).unwrap();
    client.connect_at(origin).unwrap();
    let deadline = origin.checked_add(Duration::from_nanos(1)).unwrap();
    assert_eq!(client.next_tick_at(), Some(deadline));
    assert!(client.handle_tick_at(origin).unwrap().is_empty());
    assert!(client
        .handle_tick_at(deadline)
        .unwrap()
        .iter()
        .any(|e| matches!(
            e,
            MqttEvent::OperationFailed {
                operation: OperationKind::Connect,
                ..
            }
        )));
}

#[test]
fn queued_publish_deadline_starts_when_output_admits_it_and_late_ack_completes() {
    let mut opts = options(5);
    opts.max_outgoing_packet_count = 1;
    opts.operation_timeouts.publish = Some(Duration::from_millis(10));
    let mut client = connected(opts, time(100));
    client
        .publish_at(PublishCommand::simple("q0", vec![], 0, false), time(101))
        .unwrap();
    let id = client
        .publish_at(PublishCommand::simple("q1", vec![], 1, false), time(102))
        .unwrap()
        .unwrap();
    assert_eq!(client.next_tick_at(), None);
    client.take_outgoing_at(time(200)).unwrap();
    assert_eq!(client.next_tick_at(), Some(time(210)));
    client.take_outgoing_at(time(200)).unwrap();
    assert!(client.handle_tick_at(time(209)).unwrap().is_empty());
    assert!(client.handle_tick_at(time(210)).unwrap().iter().any(|e| matches!(e,
        MqttEvent::OperationFailed { operation: OperationKind::Publish, packet_id: Some(pid), .. } if *pid == id)));
    assert!(client
        .handle_incoming_at(&ack(0x40, id), time(211))
        .unwrap()
        .iter()
        .any(|e| matches!(e, MqttEvent::Published(_))));
}

#[test]
fn subscribe_unsubscribe_deadlines_and_acknowledgements() {
    for version in [4, 5] {
        let mut opts = options(version);
        opts.operation_timeouts = OperationTimeouts {
            subscribe: Some(Duration::from_millis(10)),
            unsubscribe: Some(Duration::from_millis(10)),
            ..Default::default()
        };
        let mut client = connected(opts, time(100));
        let id = client
            .subscribe_at(SubscribeCommand::single("t", 1), time(101))
            .unwrap();
        assert_eq!(client.next_tick_at(), Some(time(111)));
        assert_eq!(client.take_outgoing_at(time(101)).unwrap()[0], 0x82);
        let packet = if version == 5 {
            vec![0x90, 4, (id >> 8) as u8, id as u8, 0, 1]
        } else {
            vec![0x90, 3, (id >> 8) as u8, id as u8, 1]
        };
        assert!(client
            .handle_incoming_at(&packet, time(102))
            .unwrap()
            .iter()
            .any(|e| matches!(e, MqttEvent::Subscribed(_))));
        let id = client
            .unsubscribe_at(UnsubscribeCommand::from_topics(vec!["t".into()]), time(103))
            .unwrap();
        assert_eq!(client.next_tick_at(), Some(time(113)));
        assert_eq!(client.take_outgoing_at(time(103)).unwrap()[0], 0xa2);
        let packet = if version == 5 {
            vec![0xb0, 4, (id >> 8) as u8, id as u8, 0, 0]
        } else {
            ack(0xb0, id).to_vec()
        };
        assert!(client
            .handle_incoming_at(&packet, time(104))
            .unwrap()
            .iter()
            .any(|e| matches!(e, MqttEvent::Unsubscribed(_))));
    }
}

#[test]
fn ping_timeout_and_reconnect_use_the_supplied_epoch() {
    let mut opts = options(5);
    opts.keep_alive = 1;
    opts.ping_timeout_multiplier = 2;
    opts.reconnect = true;
    opts.reconnect_base_delay_ms = 10;
    opts.reconnect_max_delay_ms = 100;
    let mut client = connected(opts, time(50_000));
    assert_eq!(client.next_tick_at(), Some(time(51_000)));
    client.handle_tick_at(time(51_000)).unwrap();
    assert_eq!(client.take_outgoing_at(time(51_000)).unwrap(), [0xc0, 0]);
    let deadline = client.next_tick_at().unwrap();
    assert_eq!(deadline, time(53_000));
    client.handle_tick_at(deadline).unwrap();
    assert!(!client.is_connected());
    let retry = client.next_tick_at().unwrap();
    assert_eq!(retry, time(53_010));
    client.schedule_reconnect_at(time(53_001)).unwrap();
    assert_eq!(client.next_tick_at(), Some(retry));
    assert!(client
        .handle_tick_at(retry)
        .unwrap()
        .iter()
        .any(|e| matches!(e, MqttEvent::ReconnectNeeded)));
}

#[test]
fn manual_ack_rejects_backward_time_before_advancing_the_exchange() {
    let mut opts = options(5);
    opts.auto_ack = false;
    let mut client = connected(opts, time(100));
    // QoS 1 PUBLISH, topic t, packet id 9, no properties, payload x.
    client
        .handle_incoming_at(&[0x32, 7, 0, 1, b't', 0, 9, 0, b'x'], time(101))
        .unwrap();
    assert!(client.take_outgoing_at(time(101)).unwrap().is_empty());
    assert!(client.puback_at(9, 0, vec![], time(100)).is_err());
    client.puback_at(9, 0, vec![], time(102)).unwrap();
    assert_eq!(client.take_outgoing_at(time(102)).unwrap()[0], 0x40);
    assert!(client.puback_at(9, 0, vec![], time(103)).is_err());
}

fn publish(id: u16) -> MqttPacket {
    MqttPacket::Publish3(MqttPublish {
        dup: false,
        qos: 1,
        retain: false,
        topic_name: "t".into(),
        message_id: Some(id),
        payload: vec![1],
    })
}

#[test]
fn equal_timestamp_replay_follows_admission_order() {
    let mut queue = InflightQueue::<Timestamp>::new(10, 5, Duration::from_secs(1));
    for id in [65535, 1, 32768] {
        queue.push_at(id, publish(id), 1, time(100)).unwrap();
    }
    assert_eq!(
        queue
            .snapshot_for_reconnect()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        [65535, 1, 32768]
    );
    queue.acknowledge(1);
    queue.push_at(1, publish(1), 1, time(100)).unwrap();
    assert_eq!(
        queue
            .snapshot_for_reconnect()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        [65535, 32768, 1]
    );
}

#[test]
fn inflight_resume_and_pubrel_replace_deadlines_and_keep_quota() {
    let mut queue = InflightQueue::<Timestamp>::new(1, 4, Duration::from_millis(10));
    queue.push_at(1, publish(1), 2, time(100)).unwrap();
    let packet =
        MqttPacket::PubRel3(flowsdk::mqtt_serde::mqttv3::pubrel::MqttPubRel { message_id: 1 });
    queue.transition_pubrel_at(1, packet, time(105)).unwrap();
    assert!(!queue.can_push_publish());
    assert_eq!(queue.next_expiration(), Some(time(115)));
    queue.resume_on_stream_at(1, 7, time(108)).unwrap();
    assert_eq!(queue.next_expiration(), Some(time(118)));
    assert!(queue.get_expired_at(time(117)).unwrap().is_empty());
    assert_eq!(
        queue.get_expired_with_stream_at(time(118)).unwrap().len(),
        1
    );
    assert_eq!(queue.next_expiration(), Some(time(128)));
    assert!(queue.get_expired_at(time(117)).is_err());
    queue.acknowledge(1);
    assert!(queue.can_push_publish());
    assert_eq!(queue.next_expiration(), None);
}

#[test]
fn zero_retransmission_timeout_retries_only_once_per_call() {
    let mut queue = InflightQueue::<Timestamp>::new(10, 4, Duration::ZERO);
    queue.push_at(1, publish(1), 1, time(0)).unwrap();
    assert_eq!(queue.get_expired_at(time(0)).unwrap().len(), 1);
    assert_eq!(queue.get(1).unwrap().retry_count, 1);
}

#[test]
fn duplicate_connack_singleton_is_rejected() {
    let mut client = PortableNoIoMqttClient::new_at(options(5), time(0));
    client.connect_at(time(0)).unwrap();
    client.take_outgoing_at(time(0)).unwrap();
    let events = client
        .handle_incoming_at(&[0x20, 9, 0, 0, 6, 0x21, 0, 1, 0x21, 0, 2], time(1))
        .unwrap();
    assert!(!client.is_connected());
    assert!(events.iter().any(|e| matches!(e, MqttEvent::Error(_))));
}

#[test]
fn split_input_keeps_the_same_nonzero_clock_origin() {
    let mut client = PortableNoIoMqttClient::new_at(options(5), time(10_000));
    client.connect_at(time(10_000)).unwrap();
    client.take_outgoing_at(time(10_000)).unwrap();
    assert!(client
        .handle_incoming_at(&connack(5)[..2], time(10_001))
        .unwrap()
        .is_empty());
    assert!(client
        .handle_incoming_at(&connack(5)[2..], time(10_002))
        .unwrap()
        .iter()
        .any(|e| matches!(e, MqttEvent::Connected(_))));
}

#[test]
fn persistent_session_replays_publish_and_pubrel_at_the_new_time() {
    for version in [4, 5] {
        for qos in [1, 2] {
            let mut opts = options(version);
            opts.clean_start = false;
            opts.session_expiry_interval = Some(3600);
            let mut client = connected(opts, time(100));
            let id = client
                .publish_at(PublishCommand::simple("t", vec![9], qos, false), time(101))
                .unwrap()
                .unwrap();
            client.take_outgoing_at(time(101)).unwrap();
            if qos == 2 {
                client
                    .handle_incoming_at(&ack(0x50, id), time(102))
                    .unwrap();
                assert_eq!(client.take_outgoing_at(time(102)).unwrap()[0], 0x62);
            }
            client.handle_connection_lost_at(time(200)).unwrap();
            client.connect_at(time(300)).unwrap();
            client.take_outgoing_at(time(300)).unwrap();
            let mut response = connack(version).to_vec();
            response[2] = 1; // Session Present
            assert!(client
                .handle_incoming_at(&response, time(301))
                .unwrap()
                .iter()
                .any(|e| matches!(e, MqttEvent::Connected(result) if result.session_present)));
            let replay = client.take_outgoing_at(time(301)).unwrap();
            assert_eq!(replay[0], if qos == 1 { 0x3a } else { 0x62 });
            assert!(client
                .handle_incoming_at(&ack(if qos == 1 { 0x40 } else { 0x70 }, id), time(302))
                .unwrap()
                .iter()
                .any(|e| matches!(e, MqttEvent::Published(_))));
        }
    }
}

#[test]
#[cfg(feature = "durable-session")]
fn session_checkpoint_restores_exchanges_with_a_new_clock_origin() {
    for version in [4, 5] {
        for qos in [1, 2] {
            let make_options = || {
                let mut opts = options(version);
                opts.clean_start = false;
                opts.session_expiry_interval = Some(3600);
                opts.operation_timeouts.connect = Some(Duration::from_millis(10));
                opts.operation_timeouts.publish = Some(Duration::from_millis(10));
                opts
            };
            let mut client = connected(make_options(), time(10_000));
            let id = client
                .publish_at(
                    PublishCommand::simple("t", vec![9], qos, false),
                    time(10_001),
                )
                .unwrap()
                .unwrap();
            client.take_outgoing_at(time(10_001)).unwrap();
            if qos == 2 {
                client
                    .handle_incoming_at(&ack(0x50, id), time(10_002))
                    .unwrap();
                assert_eq!(client.take_outgoing_at(time(10_002)).unwrap()[0], 0x62);
            }
            let saved = client.snapshot_session().unwrap();
            assert!(saved.has_session());
            drop(client);

            // A restarted application has a new uptime origin. Checkpoints
            // preserve exchanges, while CONNECT/replay establish fresh timers.
            let mut restored = PortableNoIoMqttClient::try_new_at(make_options(), time(0)).unwrap();
            restored.restore_session_state(saved).unwrap();
            assert_eq!(restored.next_tick_at(), None);
            assert!(restored.take_outgoing_at(time(0)).unwrap().is_empty());
            restored.connect_at(time(1)).unwrap();
            assert_eq!(restored.next_tick_at(), Some(time(11)));
            assert_eq!(restored.take_outgoing_at(time(1)).unwrap()[0], 0x10);
            let mut response = connack(version).to_vec();
            response[2] = 1;
            let events = restored.handle_incoming_at(&response, time(2)).unwrap();
            assert!(events
                .iter()
                .any(|e| matches!(e, MqttEvent::Connected(result) if result.session_present)));
            let replay = restored.take_outgoing_at(time(2)).unwrap();
            assert_eq!(replay[0], if qos == 1 { 0x3a } else { 0x62 });
            assert_eq!(restored.next_tick_at(), Some(time(12)));
            let events = restored
                .handle_incoming_at(&ack(if qos == 1 { 0x40 } else { 0x70 }, id), time(3))
                .unwrap();
            assert!(events.iter().any(|e| matches!(e, MqttEvent::Published(_))));
            assert_eq!(restored.next_tick_at(), None);
        }
    }
}

#[test]
fn a_broker_without_session_state_can_accept_a_fresh_reconnect() {
    for version in [4, 5] {
        let mut opts = options(version);
        opts.clean_start = false;
        opts.session_expiry_interval = Some(3600);
        let mut client = connected(opts, time(100));
        client.handle_connection_lost_at(time(200)).unwrap();
        client.connect_at(time(300)).unwrap();
        client.take_outgoing_at(time(300)).unwrap();
        assert!(client
            .handle_incoming_at(connack(version), time(301))
            .unwrap()
            .iter()
            .any(|e| matches!(e, MqttEvent::Connected(result) if !result.session_present)));
        assert!(client.is_connected());
    }
}
