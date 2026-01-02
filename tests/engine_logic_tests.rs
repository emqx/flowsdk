use flowsdk::mqtt_client::commands::PublishCommand;
use flowsdk::mqtt_client::engine::{MqttEngine, MqttEvent};
use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::mqttv3::connack::MqttConnAck as MqttConnAck3;
use flowsdk::mqtt_serde::mqttv3::pubrec::MqttPubRec as MqttPubRec3;
use flowsdk::mqtt_serde::mqttv5::connackv5::MqttConnAck as MqttConnAck5;
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
fn test_v5_handshake_success() {
    let mut engine = setup_engine_v5();

    // 1. Initial state
    assert!(!engine.is_connected());

    // 2. Connect
    engine.connect();
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

    engine.connect();
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
    engine.connect();
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
    engine.connect();
    let _ = engine.take_outgoing();
    let connack = MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
        .to_bytes()
        .unwrap();
    engine.handle_incoming(&connack);

    // Advance time past keep_alive * multiplier (60s * 2 = 120s)
    let much_later = now + Duration::from_secs(121);
    let events = engine.handle_tick(much_later);

    assert!(!engine.is_connected());
    assert!(events
        .iter()
        .any(|e| matches!(e, MqttEvent::ReconnectNeeded)));
}

#[test]
fn test_v3_qos1_retransmission_after_reconnect() {
    let mut engine = setup_engine_v3();

    // 1. Connect and send QoS 1 publish
    engine.connect();
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
    engine.connect();
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
    engine.connect();
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
    engine.connect();
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
    engine.connect();
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
    engine.connect();
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
    let mut options = MqttClientOptions::default();
    options.max_outgoing_packet_count = 1;
    let mut engine = MqttEngine::new(options);

    // Initial CONNECT
    engine.connect();

    // Second packet should fail
    let res = engine.enqueue_packet(MqttPacket::PingReq5(
        flowsdk::mqtt_serde::mqttv5::pingreqv5::MqttPingReq::new(),
    ));
    assert!(res.is_err());
    // Note: MqttClientError is not directly comparable easily with matches! due to String fields, but we check for failure
}

#[test]
fn test_back_pressure_event_buffer() {
    let mut options = MqttClientOptions::default();
    options.max_event_count = 1;
    let mut engine = MqttEngine::new(options);

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
    engine.connect();
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
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], MqttEvent::Error(_)));
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
    engine.connect();
    let _ = engine.take_outgoing();
    engine.handle_incoming(
        &MqttPacket::ConnAck5(MqttConnAck5::new(false, 0, None))
            .to_bytes()
            .unwrap(),
    );

    let next = engine.next_tick_at().unwrap();
    // Should be keep-alive (60s) or timeout (120s)
    let diff = next.duration_since(Instant::now()).as_secs();
    assert!(diff >= 59 && diff <= 61);
}

#[test]
fn test_packet_id_rollover() {
    let mut engine = setup_engine_v5();
    engine.connect();
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
