// SPDX-License-Identifier: MPL-2.0

use flowsdk::mqtt_client::commands::PublishCommand;
use flowsdk::mqtt_client::engine::{MqttEngine, MqttEvent};
use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_client::MqttClientError;
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::mqttv3::connack::MqttConnAck as MqttConnAck3;
use flowsdk::mqtt_serde::mqttv3::pubrec::MqttPubRec as MqttPubRec3;
use flowsdk::mqtt_serde::mqttv5::connackv5::MqttConnAck as MqttConnAck5;
use flowsdk::mqtt_serde::mqttv5::disconnectv5::MqttDisconnect as MqttDisconnect5;
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish as MqttPublish5;
use flowsdk::mqtt_serde::mqttv5::subackv5::MqttSubAck as MqttSubAck5;
use flowsdk::mqtt_serde::mqttv5::unsubackv5::MqttUnsubAck as MqttUnsubAck5;
use flowsdk::mqtt_serde::parser::ParseOk;
use std::time::{Duration, Instant};

fn setup_engine_v5() -> MqttEngine {
    let options = MqttClientOptions::builder()
        .mqtt_version(5)
        .client_id("test_v5")
        .keep_alive(60)
        .build();
    MqttEngine::new(options)
}

fn setup_engine_v3() -> MqttEngine {
    let options = MqttClientOptions::builder()
        .mqtt_version(4) // MQTT v3.1.1 is version 4 in standard
        .client_id("test_v3")
        .keep_alive(60)
        .build();
    MqttEngine::new(options)
}

#[test]
fn connect_preserves_optional_credentials() {
    for version in [3, 4, 5] {
        for (username, password) in [
            (None, None),
            (Some("test-user"), None),
            (Some("test-user"), Some(b"test-password\x00\xff".as_slice())),
        ] {
            let mut options = MqttClientOptions::builder().mqtt_version(version).build();
            options.username = username.map(str::to_string);
            options.password = password.map(<[u8]>::to_vec);
            let mut engine = MqttEngine::new(options);
            engine.connect().unwrap();

            let bytes = engine.take_outgoing();
            let (actual_username, actual_password) =
                match MqttPacket::from_bytes_with_version(&bytes, version).unwrap() {
                    ParseOk::Packet(MqttPacket::Connect3(packet), _) => {
                        (packet.username, packet.password)
                    }
                    ParseOk::Packet(MqttPacket::Connect5(packet), _) => {
                        (packet.username, packet.password)
                    }
                    packet => panic!("Expected CONNECT, got {packet:?}"),
                };
            assert_eq!(actual_username.as_deref(), username);
            assert_eq!(actual_password.as_deref(), password);
        }
    }
}

#[test]
fn test_v5_handshake_success() {
    let mut engine = setup_engine_v5();

    // 1. Initial state
    assert!(!engine.is_connected());

    // 2. Connect
    engine.connect().unwrap();
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Connect5(p), _) => {
            assert_eq!(p.client_id, "test_v5");
            assert_eq!(p.keep_alive, 60);
        }
        _ => panic!("Expected Connect5 packet"),
    }

    // 3. Receive CONNACK
    let connack = MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None));
    let connack_bytes = connack.to_bytes().unwrap();
    let events = engine.handle_incoming(&connack_bytes);

    assert!(engine.is_connected());
    assert_eq!(events.len(), 1);
    if let MqttEvent::Connected(res) = &events[0] {
        assert_eq!(res.reason_code, 0);
    } else {
        panic!("Expected Connected event");
    }
}

#[test]
fn test_v3_handshake_success() {
    let mut engine = setup_engine_v3();

    engine.connect().unwrap();
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 4).unwrap() {
        ParseOk::Packet(MqttPacket::Connect3(p), _) => {
            assert_eq!(p.client_id, "test_v3");
            assert_eq!(p.keep_alive, 60);
        }
        _ => panic!("Expected Connect3 packet"),
    }

    let connack = MqttPacket::ConnAck3(MqttConnAck3::new(false, 0));
    let connack_bytes = connack.to_bytes().unwrap();
    let events = engine.handle_incoming(&connack_bytes);

    assert!(engine.is_connected());
    assert_eq!(events.len(), 1);
    if let MqttEvent::Connected(res) = &events[0] {
        assert_eq!(res.reason_code, 0);
    } else {
        panic!("Expected Connected event");
    }
}

#[test]
fn test_keep_alive_ping() {
    let mut engine = setup_engine_v5();
    let now = Instant::now();

    // Connect first
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    let connack = MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
        .to_bytes()
        .unwrap();
    engine.handle_incoming(&connack);

    // Advance time past keep_alive (60s)
    let later = now + Duration::from_secs(61);
    let _events = engine.handle_tick(later);

    // Should have sent PINGREQ
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::PingReq5(_), _) => {}
        _ => panic!("Expected PingReq5 packet"),
    }
}

#[test]
fn test_keep_alive_timeout() {
    let mut engine = setup_engine_v5();
    let now = Instant::now();

    // Connect
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    let connack = MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
        .to_bytes()
        .unwrap();
    engine.handle_incoming(&connack);

    // First expire the idle keep-alive interval so the engine sends PINGREQ.
    let ping_at = now + Duration::from_secs(61);
    assert!(engine.handle_tick(ping_at).is_empty());
    assert!(engine.is_connected());

    // The timeout starts when PINGREQ is sent, not at the start of the idle
    // interval. No PINGRESP within 60s * 2 marks the connection as lost.
    let timeout_at = ping_at + Duration::from_secs(121);
    let events = engine.handle_tick(timeout_at);

    assert!(!engine.is_connected());
    assert!(events
        .iter()
        .any(|e| matches!(e, MqttEvent::Disconnected(_))));
}

#[test]
fn test_v3_qos1_retransmission_after_reconnect() {
    let mut engine = setup_engine_v3();

    // 1. Connect and send QoS 1 publish
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck3(MqttConnAck3::new(false, 0))
            .to_bytes()
            .unwrap(),
    );

    let cmd = PublishCommand::simple("topic", b"payload".to_vec(), 1, false);
    let pid = engine.publish(cmd).unwrap().unwrap();
    let _publish_packet = engine.take_outgoing();

    // 2. Simulate connection loss
    engine.handle_connection_lost();
    assert!(!engine.is_connected());

    // 3. Reconnect with session_present = true
    engine.connect().unwrap();
    let _ = engine.take_outgoing(); // New CONNECT

    let _events = engine.handle_incoming(
        &MqttPacket::ConnAck3(MqttConnAck3::new(true, 0))
            .to_bytes()
            .unwrap(),
    );

    // 4. Verify retransmission
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 4).unwrap() {
        ParseOk::Packet(MqttPacket::Publish3(p), _) => {
            assert!(p.dup);
            assert_eq!(p.topic_name, "topic");
            assert_eq!(p.payload, b"payload");
            assert_eq!(p.qos, 1);
            assert_eq!(p.message_id, Some(pid));
        }
        _ => panic!("Expected Publish3 packet"),
    }
}

#[test]
fn test_v3_pubrel_retransmission_after_reconnect() {
    let mut engine = setup_engine_v3();

    // 1. Connect and start QoS 2 flow
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck3(MqttConnAck3::new(false, 0))
            .to_bytes()
            .unwrap(),
    );

    let cmd = PublishCommand::simple("topic", b"payload".to_vec(), 2, false);
    let pid = engine.publish(cmd).unwrap().unwrap();
    let _publish_packet = engine.take_outgoing();

    // Receive PUBREC
    engine.handle_incoming(
        &MqttPacket::PubRec3(MqttPubRec3::new(pid))
            .to_bytes()
            .unwrap(),
    );

    // Should have sent PUBREL
    let pubrel = engine.take_outgoing();
    assert_eq!(pubrel[0] & 0xF0, 0x60); // PUBREL is 0x6x

    // 2. Connection loss before PUBCOMP
    engine.handle_connection_lost();

    // 3. Reconnect with session_present = true
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck3(MqttConnAck3::new(true, 0))
            .to_bytes()
            .unwrap(),
    );

    // 4. Verify PUBREL retransmission
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 4).unwrap() {
        ParseOk::Packet(MqttPacket::PubRel3(p), _) => {
            assert_eq!(p.message_id, pid);
        }
        _ => panic!("Expected PubRel3 packet"),
    }
}

#[test]
fn test_v5_qos1_retransmission_after_reconnect() {
    let mut engine = setup_engine_v5();

    // 1. Connect and send QoS 1 publish
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    let cmd = PublishCommand::simple("topic", b"payload".to_vec(), 1, false);
    let pid = engine.publish(cmd).unwrap().unwrap();
    let _publish_packet = engine.take_outgoing();

    // 2. Connection loss
    engine.handle_connection_lost();

    // 3. Reconnect with session_present = true
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(true, 0, None))
            .to_bytes()
            .unwrap(),
    );

    // 4. Verify retransmission
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Publish5(p), _) => {
            assert!(p.dup);
            assert_eq!(p.topic_name, "topic");
            assert_eq!(p.payload, b"payload");
            assert_eq!(p.qos, 1);
            assert_eq!(p.packet_id, Some(pid));
        }
        _ => panic!("Expected Publish5 packet"),
    }
}

#[test]
fn test_outgoing_buffer_full() {
    let options = MqttClientOptions {
        max_outgoing_packet_count: 1,
        ..Default::default()
    };

    let mut engine = MqttEngine::new(options);

    // Initial CONNECT
    engine.connect().unwrap();

    // Second packet should fail
    let res = engine.enqueue_packet(MqttPacket::PingReq5(
        flowsdk::mqtt_serde::mqttv5::pingreqv5::MqttPingReq::new(),
    ));
    assert!(res.is_err());
    // Note: MqttClientError is not directly comparable easily with matches! due to String fields, but we check for failure
}

#[test]
fn test_back_pressure_event_buffer() {
    let options = MqttClientOptions {
        max_event_count: 1,
        ..Default::default()
    };

    let mut engine = MqttEngine::new(options);
    engine.connect().unwrap();
    engine.take_outgoing();

    let connack = MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
        .to_bytes()
        .unwrap();
    let pingresp =
        MqttPacket::PingResp5(flowsdk::mqtt_serde::mqttv5::pingrespv5::MqttPingResp::new())
            .to_bytes()
            .unwrap();

    let mut data = connack;
    data.extend_from_slice(&pingresp);

    let events = engine.handle_incoming(&data);
    assert_eq!(events.len(), 1); // Only room for 1 event (Connected)

    // Take events and process remaining by calling handle_incoming with empty data
    let events2 = engine.handle_incoming(&[]);
    assert_eq!(events2.len(), 1); // Should have the PingResponse event now
}

#[test]
fn test_qos2_full_handshake() {
    let mut engine = setup_engine_v5();
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    // 1. Publish QoS 2
    let cmd = PublishCommand::simple("t", b"p".to_vec(), 2, false);
    let pid = engine.publish(cmd).unwrap().unwrap();
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Publish5(p), _) => {
            assert_eq!(p.topic_name, "t");
            assert_eq!(p.payload, b"p");
            assert_eq!(p.qos, 2);
            assert_eq!(p.packet_id, Some(pid));
        }
        _ => panic!("Expected Publish5 packet"),
    }

    // 2. Receive PUBREC
    let events = engine.handle_incoming(
        &MqttPacket::PubRec5(flowsdk::mqtt_serde::mqttv5::pubrecv5::MqttPubRec::new(
            pid,
            0,
            Vec::new(),
        ))
        .to_bytes()
        .unwrap(),
    );
    assert!(events.is_empty()); // PUBREC doesn't emit event yet, it triggers PUBREL

    // 3. Check for PUBREL
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::PubRel5(p), _) => {
            assert_eq!(p.packet_id, pid);
        }
        _ => panic!("Expected PubRel5 packet"),
    }

    // 4. Receive PUBCOMP
    let events = engine.handle_incoming(
        &MqttPacket::PubComp5(flowsdk::mqtt_serde::mqttv5::pubcompv5::MqttPubComp::new(
            pid,
            0,
            Vec::new(),
        ))
        .to_bytes()
        .unwrap(),
    );
    assert_eq!(events.len(), 1);
    if let MqttEvent::Published(res) = &events[0] {
        assert_eq!(res.qos, 2);
    } else {
        panic!("Expected Published event");
    }
}

#[test]
fn test_malformed_packet_error() {
    let mut engine = setup_engine_v5();
    // Feed invalid fixed header (packet type 0 is reserved)
    let events = engine.handle_incoming(&[0x00, 0x00]);
    assert!(events
        .iter()
        .any(|event| matches!(event, MqttEvent::Error(_))));
    assert!(!engine.is_connected());
}

#[test]
fn test_next_tick_at_priority() {
    let mut engine = setup_engine_v5();
    let now = Instant::now();

    // 1. Disconnected, no reconnect scheduled
    assert_eq!(engine.next_tick_at(), None);

    // 2. Schedule reconnect
    engine.schedule_reconnect(now);
    let reconnect_at = engine.next_tick_at().unwrap();
    assert!(reconnect_at > now);

    // 3. Connected (keep-alive 60s)
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    let next = engine.next_tick_at().unwrap();
    // Should be keep-alive (60s) or timeout (120s)
    let diff = next.duration_since(Instant::now()).as_secs();
    assert!((59..=61).contains(&diff));
}

#[test]
fn test_packet_id_rollover() {
    let mut engine = setup_engine_v5();
    engine.connect().unwrap();
    // Establish session
    let connack = MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
        .to_bytes()
        .unwrap();
    engine.handle_incoming(&connack);

    // Fast-forward packet ID by calling next_packet_id many times
    // Since we can't easily access the inner session, we just use the public API
    let mut last_pid = 0;
    for _ in 0..65535 {
        last_pid = engine.next_packet_id().unwrap();
    }

    assert_eq!(last_pid, 65535);
    let first_pid = engine.next_packet_id().unwrap();
    assert_eq!(first_pid, 1);
}

#[test]
fn test_v5_subscribe_unsub_flow() {
    let mut engine = setup_engine_v5();
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    // 1. Subscribe
    use flowsdk::mqtt_client::commands::SubscribeCommand;
    let cmd = SubscribeCommand::single("topic", 1);
    let pid = engine.subscribe(cmd).unwrap();

    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Subscribe5(p), _) => {
            assert_eq!(p.packet_id, pid);
            assert_eq!(p.subscriptions[0].topic_filter, "topic");
        }
        _ => panic!("Expected Subscribe5 packet"),
    }

    // 2. Receive SUBACK
    let events = engine.handle_incoming(
        &MqttPacket::SubAck5(MqttSubAck5::new_success(pid, 1))
            .to_bytes()
            .unwrap(),
    );
    assert_eq!(events.len(), 1);
    if let MqttEvent::Subscribed(res) = &events[0] {
        assert_eq!(res.packet_id, pid);
        assert_eq!(res.reason_codes, vec![0x00]);
    } else {
        panic!("Expected Subscribed event");
    }

    // 3. Unsubscribe
    use flowsdk::mqtt_client::commands::UnsubscribeCommand;
    let cmd = UnsubscribeCommand::from_topics(vec!["topic".to_string()]);
    let pid2 = engine.unsubscribe(cmd).unwrap();

    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Unsubscribe5(p), _) => {
            assert_eq!(p.packet_id, pid2);
            assert_eq!(p.topic_filters, vec!["topic".to_string()]);
        }
        _ => panic!("Expected Unsubscribe5 packet"),
    }

    // 4. Receive UNSUBACK
    let events = engine.handle_incoming(
        &MqttPacket::UnsubAck5(MqttUnsubAck5::new_success(pid2, 1))
            .to_bytes()
            .unwrap(),
    );
    assert_eq!(events.len(), 1);
    if let MqttEvent::Unsubscribed(res) = &events[0] {
        assert_eq!(res.packet_id, pid2);
    } else {
        panic!("Expected Unsubscribed event");
    }
}

#[test]
fn test_incoming_publish_qos1() {
    let mut engine = setup_engine_v5();
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    // 1. Receive QoS 1 Publish from server
    let publish = MqttPacket::Publish5(MqttPublish5::new(
        1,
        "t".to_string(),
        Some(100),
        b"data".to_vec(),
        false,
        false,
    ));
    let events = engine.handle_incoming(&publish.to_bytes().unwrap());

    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        MqttEvent::PublishReceived {
            packet_id: Some(100),
            stream: None
        }
    ));
    match &events[1] {
        MqttEvent::MessageReceived(p) => {
            assert_eq!(p.topic_name, "t");
            assert_eq!(p.payload, b"data");
            assert_eq!(p.qos, 1);
            assert_eq!(p.packet_id, Some(100));
        }
        _ => panic!("Expected MessageReceived event"),
    }

    // 2. Verify auto-PUBACK
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::PubAck5(p), _) => {
            assert_eq!(p.packet_id, 100);
        }
        _ => panic!("Expected PubAck5 packet"),
    }
}

#[test]
fn test_reconnect_backoff_multi() {
    let options = MqttClientOptions::builder()
        .reconnect_base_delay_ms(100)
        .reconnect_max_delay_ms(500)
        .max_reconnect_attempts(3)
        .build();
    let mut engine = MqttEngine::new(options);
    let now = Instant::now();

    // 1. First attempt: 100ms
    engine.schedule_reconnect(now);
    if let MqttEvent::ReconnectScheduled { attempt, delay } = engine.take_events().remove(0) {
        assert_eq!(attempt, 1);
        assert_eq!(delay, Duration::from_millis(100));
    }

    engine.handle_tick(now + Duration::from_millis(100));

    // 2. Second attempt: 200ms
    engine.schedule_reconnect(now);
    if let MqttEvent::ReconnectScheduled { attempt, delay } = engine.take_events().remove(0) {
        assert_eq!(attempt, 2);
        assert_eq!(delay, Duration::from_millis(200));
    }

    engine.handle_tick(now + Duration::from_millis(200));

    // 3. Third attempt: 400ms
    engine.schedule_reconnect(now);
    if let MqttEvent::ReconnectScheduled { attempt, delay } = engine.take_events().remove(0) {
        assert_eq!(attempt, 3);
        assert_eq!(delay, Duration::from_millis(400));
    }

    engine.handle_tick(now + Duration::from_millis(400));

    // 4. Fourth attempt: should give up (max=3)
    engine.schedule_reconnect(now);
    assert!(engine.take_events().is_empty());
    assert_eq!(engine.next_tick_at(), None);
}

#[test]
fn test_v5_receive_maximum_enforcement() {
    let mut engine = setup_engine_v5();
    engine.connect().unwrap();
    let _ = engine.take_outgoing();

    // Connect with Receive Maximum = 1
    use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
    let connack = MqttPacket::ConnAck5(MqttConnAck5::new(
        false,
        0,
        Some(vec![Property::ReceiveMaximum(1)]),
    ));
    engine.handle_incoming(&connack.to_bytes().unwrap());

    // 1. Send first publish (QoS 1) - should be sent
    let cmd1 = PublishCommand::simple("t1", b"p1".to_vec(), 1, false);
    engine.publish(cmd1).unwrap();
    let outgoing1 = engine.take_outgoing();
    assert!(!outgoing1.is_empty());

    // 2. Send second publish (QoS 1) - should be queued but NOT sent due to ReceiveMaximum=1
    let cmd2 = PublishCommand::simple("t2", b"p2".to_vec(), 1, false);
    engine.publish(cmd2).unwrap();
    let outgoing2 = engine.take_outgoing();
    assert!(outgoing2.is_empty()); // Blocked by ReceiveMaximum

    // 3. Receive PUBACK for first publish
    match MqttPacket::from_bytes_with_version(&outgoing1, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Publish5(p), _) => {
            let pid = p.packet_id.unwrap();
            engine.handle_incoming(
                &MqttPacket::PubAck5(flowsdk::mqtt_serde::mqttv5::pubackv5::MqttPubAck::new(
                    pid,
                    0,          // reason_code
                    Vec::new(), // properties
                ))
                .to_bytes()
                .unwrap(),
            );
        }
        _ => panic!("Expected Publish5"),
    }

    // 4. Now second publish should be sent automatically
    let outgoing2 = engine.take_outgoing();
    assert!(!outgoing2.is_empty());
    match MqttPacket::from_bytes_with_version(&outgoing2, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Publish5(p), _) => {
            assert_eq!(p.topic_name, "t2");
        }
        _ => panic!("Expected Publish5"),
    }
}

#[test]
fn rejected_pubrec_finishes_publish_without_pubrel() {
    use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
    use flowsdk::mqtt_serde::mqttv5::pubrecv5::MqttPubRec;

    for reason_code in [0x80, 0x87] {
        let mut engine = setup_engine_v5();
        engine.connect().unwrap();
        engine.take_outgoing();
        engine.handle_incoming(
            &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
                .to_bytes()
                .unwrap(),
        );
        let pid = engine
            .publish(PublishCommand::simple(
                "test/topic",
                b"payload".to_vec(),
                2,
                false,
            ))
            .unwrap()
            .unwrap();
        assert!(!engine.take_outgoing().is_empty());

        let properties = vec![Property::ReasonString("Publish rejected".to_string())];
        let pubrec = MqttPacket::PubRec5(MqttPubRec::new(pid, reason_code, properties.clone()))
            .to_bytes()
            .unwrap();
        let events = engine.handle_incoming(&pubrec);
        assert_eq!(events.len(), 1);
        match &events[0] {
            MqttEvent::Published(result) => {
                assert_eq!(result.packet_id, Some(pid));
                assert_eq!(result.reason_code, Some(reason_code));
                assert_eq!(result.qos, 2);
                assert_eq!(result.properties.as_ref(), Some(&properties));
                assert!(!result.is_success());
            }
            event => panic!("Expected publish result, got {event:?}"),
        }
        assert!(engine.take_outgoing().is_empty());
        assert!(engine.handle_incoming(&pubrec).iter().any(|event| matches!(
            event,
            MqttEvent::Error(MqttClientError::InvalidPacketId { .. })
        )));
        engine.handle_tick(Instant::now() + Duration::from_secs(6));
        assert!(engine.take_outgoing().is_empty());
    }
}

#[test]
fn test_incoming_publish_qos2() {
    let mut engine = setup_engine_v5();
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    // 1. Receive QoS 2 Publish
    let pid = 200;
    let publish = MqttPacket::Publish5(MqttPublish5::new(
        2,
        "qos2".into(),
        Some(pid),
        b"payload".into(),
        false,
        false,
    ));
    let events = engine.handle_incoming(&publish.to_bytes().unwrap());

    assert_eq!(events.len(), 2);
    match events[0] {
        MqttEvent::PublishReceived { packet_id, stream } => {
            assert_eq!(packet_id, Some(pid));
            assert_eq!(stream, None);
        }
        _ => panic!("Expected PublishReceived event"),
    }
    match &events[1] {
        MqttEvent::MessageReceived(p) => {
            assert_eq!(p.qos, 2);
            assert_eq!(p.packet_id, Some(pid));
        }
        _ => panic!("Expected MessageReceived"),
    }

    // 2. Verify auto-PUBREC
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::PubRec5(p), _) => assert_eq!(p.packet_id, pid),
        _ => panic!("Expected PubRec5"),
    }

    // 3. Receive PUBREL
    use flowsdk::mqtt_serde::mqttv5::pubrelv5::MqttPubRel as MqttPubRel5;
    let pubrel = MqttPacket::PubRel5(MqttPubRel5::new(pid, 0, Vec::new()));
    engine.handle_incoming(&pubrel.to_bytes().unwrap());

    // 4. Verify auto-PUBCOMP
    let outgoing2 = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing2, 5).unwrap() {
        ParseOk::Packet(MqttPacket::PubComp5(p), _) => assert_eq!(p.packet_id, pid),
        _ => panic!("Expected PubComp5"),
    }
}

#[test]
fn test_v5_auth() {
    use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
    let mut engine = MqttEngine::new(
        MqttClientOptions::builder()
            .connect_properties(vec![Property::AuthenticationMethod("test".into())])
            .build(),
    );
    engine.connect().unwrap();
    engine.take_outgoing();
    assert!(engine.auth(0, vec![]).is_err());
    engine.auth(0x18, Vec::new()).unwrap();
    let outgoing = engine.take_outgoing();
    assert!(
        matches!(MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap(), ParseOk::Packet(MqttPacket::Auth(a), _) if a.reason_code == 0x18)
    );
}

#[cfg(not(feature = "strict-protocol-compliance"))]
#[test]
fn connack_auth_method_validation_is_optional() {
    use flowsdk::mqtt_serde::mqttv5::common::properties::Property;

    for (connect_method, connack_method) in [
        (Some("test"), None),
        (Some("test"), Some("changed")),
        (None, Some("unsolicited")),
    ] {
        let properties = |method: Option<&str>| {
            method
                .map(|method| vec![Property::AuthenticationMethod(method.into())])
                .unwrap_or_default()
        };
        let mut engine = MqttEngine::new(
            MqttClientOptions::builder()
                .mqtt_version(5)
                .connect_properties(properties(connect_method))
                .build(),
        );
        engine.connect().unwrap();
        engine.take_outgoing();
        let connack = MqttPacket::ConnAck5(MqttConnAck5::new(
            false,
            0,
            Some(properties(connack_method)),
        ));
        let events = engine.handle_incoming(&connack.to_bytes().unwrap());
        assert!(engine.is_connected(), "{events:#?}");
        assert!(
            matches!(events.as_slice(), [MqttEvent::Connected(result)] if result.reason_code == 0),
            "{events:#?}"
        );
    }
}

#[test]
fn test_disconnect_flows() {
    let mut engine = setup_engine_v5();
    engine.connect().unwrap();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );
    assert!(engine.is_connected());

    // 1. Client initiates disconnect
    engine.disconnect().unwrap();
    assert!(!engine.is_connected());
    let outgoing = engine.take_outgoing();
    match MqttPacket::from_bytes_with_version(&outgoing, 5).unwrap() {
        ParseOk::Packet(MqttPacket::Disconnect5(p), _) => assert_eq!(p.reason_code, 0),
        _ => panic!("Expected Disconnect5"),
    }

    // 2. Server initiates disconnect
    let mut engine2 = setup_engine_v5();
    engine2.connect().unwrap();
    let _ = engine2.take_outgoing();
    engine2.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    let server_disc = MqttPacket::Disconnect5(MqttDisconnect5::new(0x81, Vec::new()));
    let events = engine2.handle_incoming(&server_disc.to_bytes().unwrap());
    assert!(!engine2.is_connected());
    assert_eq!(events.len(), 1);
    match &events[0] {
        MqttEvent::DisconnectReceived { reason_code, .. } => assert_eq!(*reason_code, 0x81),
        _ => panic!("Expected Disconnected event"),
    }
}

#[test]
fn test_publish_priority() {
    let mut engine = setup_engine_v5();
    // 1. Publish messages with different priorities while DISCONNECTED
    // Higher number = higher priority
    engine
        .publish(PublishCommand::with_priority(
            "low",
            b"p".to_vec(),
            0,
            false,
            10,
        ))
        .unwrap();
    engine
        .publish(PublishCommand::with_priority(
            "high",
            b"p".to_vec(),
            0,
            false,
            200,
        ))
        .unwrap();
    engine
        .publish(PublishCommand::with_priority(
            "mid",
            b"p".to_vec(),
            0,
            false,
            128,
        ))
        .unwrap();

    // Verify nothing sent yet
    assert!(engine.take_outgoing().is_empty());

    // 2. Connect
    engine.connect().unwrap();
    let conn_bytes = engine.take_outgoing();
    assert!(!conn_bytes.is_empty()); // This is the CONNECT packet

    // 3. Receive CONNACK -> triggers process_queue
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    // 4. Verify order in outgoing buffer
    let outgoing = engine.take_outgoing();
    let mut parser = flowsdk::mqtt_serde::parser::stream::MqttParser::new(1024, 5);
    parser.feed(&outgoing);

    // Should be: high (200), mid (128), low (10)
    let p1 = parser.next_packet().unwrap().unwrap();
    let p2 = parser.next_packet().unwrap().unwrap();
    let p3 = parser.next_packet().unwrap().unwrap();

    if let MqttPacket::Publish5(p) = p1 {
        assert_eq!(p.topic_name, "high");
    } else {
        panic!("Expected high priority");
    }
    if let MqttPacket::Publish5(p) = p2 {
        assert_eq!(p.topic_name, "mid");
    } else {
        panic!("Expected mid priority");
    }
    if let MqttPacket::Publish5(p) = p3 {
        assert_eq!(p.topic_name, "low");
    } else {
        panic!("Expected low priority");
    }
}
