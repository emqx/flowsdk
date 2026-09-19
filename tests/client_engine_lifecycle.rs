// SPDX-License-Identifier: MPL-2.0

//! Wire-driven regressions for the reported client-engine lifecycle findings.
//!
//! CONNACK deadlines are configured SDK deadlines, not a fixed MQTT timeout.
//! Duplicate CONNACK and AUTH tests exercise the requested strict profile;
//! AUTH protocol errors use MQTT 5 section 4.13.1's SHOULD-close policy, not
//! an unconditional claim of a section 4.12.1 MUST violation. Only the explicit
//! re-authentication refusal tests model a failed re-authentication exchange.
//! MQTT 3.1.1 SP=0 permits closing; continuing must retain retransmission work.
//! MQTT 5 SP=0 intentionally has a different, session-discard expectation.
//!
//! These tests exercise the engine without a broker, sleeps, or raw-packet escape hatch.
//! Run: `cargo test --test client_engine_lifecycle` (default features), or add
//! `--no-default-features --features strict-protocol-compliance` for engine only.
//! Existing outbound PUBREL regressions are in `no_io_reliability`: run that
//! target with the `pubrel_replay` filter instead of duplicating those tests.
#![cfg(feature = "strict-protocol-compliance")]

use flowsdk::mqtt_client::{
    engine::MqttEngine, MqttClientError, MqttClientOptions, MqttEvent, OperationKind,
    OperationTimeouts, PublishCommand, SubscribeCommand,
};
use flowsdk::mqtt_serde::{
    control_packet::MqttPacket, mqttv5::common::properties::Property, parser::ParseOk,
};
use std::time::Duration;

const PEER_ID: u16 = 0x1234;
const METHOD: &str = "test-method";

fn options(version: u8) -> MqttClientOptions {
    MqttClientOptions::builder()
        .mqtt_version(version)
        .client_id("lifecycle-regression")
        .keep_alive(0)
        .reconnect(false)
        .build()
}

// Small, independently constructed wire fixtures. All frames fit a one-byte
// Remaining Length; malformed lifecycle sequences still use valid encodings.
fn frame(header: u8, body: &[u8]) -> Vec<u8> {
    assert!(body.len() < 128);
    [vec![header, body.len() as u8], body.to_vec()].concat()
}

fn connack(version: u8, present: bool, refused: bool) -> Vec<u8> {
    let reason = if refused {
        if version == 5 {
            0x87
        } else {
            5
        }
    } else {
        0
    };
    let mut body = vec![u8::from(present), reason];
    if version == 5 {
        body.push(0);
    }
    frame(0x20, &body)
}

fn limited_connack(present: bool, maximum: u16) -> Vec<u8> {
    let [hi, lo] = maximum.to_be_bytes();
    frame(0x20, &[u8::from(present), 0, 3, 0x21, hi, lo])
}

fn publish(version: u8, qos: u8, id: u16) -> Vec<u8> {
    let mut body = vec![0, 1, b't'];
    if qos > 0 {
        body.extend(id.to_be_bytes());
    }
    if version == 5 {
        body.push(0);
    }
    body.extend(b"payload");
    frame(0x30 | (qos << 1), &body)
}

fn suback(version: u8, id: u16, codes: &[u8]) -> Vec<u8> {
    let mut body = id.to_be_bytes().to_vec();
    if version == 5 {
        body.push(0);
    }
    body.extend(codes);
    frame(0x90, &body)
}

fn acknowledgement(header: u8, id: u16) -> Vec<u8> {
    frame(header, &id.to_be_bytes())
}

fn decode(mut bytes: &[u8], version: u8) -> Vec<MqttPacket> {
    let mut packets = Vec::new();
    while !bytes.is_empty() {
        match MqttPacket::from_bytes_with_version(bytes, version).unwrap() {
            ParseOk::Packet(packet, used) => {
                assert!(used > 0);
                packets.push(packet);
                bytes = &bytes[used..];
            }
            other => panic!("incomplete output: {other:?}"),
        }
    }
    packets
}

fn drain(engine: &mut MqttEngine) -> Vec<MqttPacket> {
    let version = engine.mqtt_version();
    decode(&engine.take_outgoing(), version)
}

fn start(engine: &mut MqttEngine) {
    engine.connect().unwrap();
    assert!(matches!(
        drain(engine).as_slice(),
        [MqttPacket::Connect3(_) | MqttPacket::Connect5(_)]
    ));
    assert!(engine.take_events().is_empty());
}

fn connected_with(options: MqttClientOptions, ack: &[u8]) -> MqttEngine {
    let mut engine = MqttEngine::new(options);
    start(&mut engine);
    let events = engine.handle_incoming(ack);
    assert!(engine.is_connected(), "setup CONNACK: {events:?}");
    assert!(matches!(events.as_slice(), [MqttEvent::Connected(r)] if r.reason_code == 0));
    assert!(drain(&mut engine).is_empty());
    assert!(engine.take_events().is_empty());
    engine
}

fn connected(version: u8) -> MqttEngine {
    connected_with(options(version), &connack(version, false, false))
}

#[derive(Debug)]
struct Observation {
    connected: bool,
    events: Vec<MqttEvent>,
    packets: Vec<MqttPacket>,
}

fn observe(engine: &mut MqttEngine, chunks: &[&[u8]]) -> Observation {
    let mut events = Vec::new();
    let mut packets = Vec::new();
    for chunk in chunks {
        events.extend(engine.handle_incoming(chunk));
        // Draining output may also pump buffered input: collect those events too.
        packets.extend(drain(engine));
        events.extend(engine.take_events());
    }
    Observation {
        connected: engine.is_connected(),
        events,
        packets,
    }
}

#[derive(Clone, Copy)]
enum Tail {
    None,
    Coalesced,
    Partial,
    Later,
}

fn with_tail(engine: &mut MqttEngine, first: &[u8], tail: Tail) -> Observation {
    let publication = publish(engine.mqtt_version(), 1, PEER_ID);
    match tail {
        Tail::None => observe(engine, &[first]),
        Tail::Coalesced => observe(engine, &[&[first, &publication].concat()]),
        Tail::Partial => {
            let split = publication.len() - 1;
            observe(
                engine,
                &[
                    &[first, &publication[..split]].concat(),
                    &publication[split..],
                ],
            )
        }
        Tail::Later => observe(engine, &[first, &publication]),
    }
}

fn has_delivery(events: &[MqttEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            MqttEvent::PublishReceived { .. } | MqttEvent::MessageReceived(_)
        )
    })
}

fn has_puback(packets: &[MqttPacket], id: u16) -> bool {
    packets.iter().any(|packet| match packet {
        MqttPacket::PubAck3(ack) => ack.message_id == id,
        MqttPacket::PubAck5(ack) => ack.packet_id == id,
        _ => false,
    })
}

fn assert_terminal(observed: &Observation) {
    assert!(
        !observed.connected
            && !has_delivery(&observed.events)
            && !has_puback(&observed.packets, PEER_ID)
            && !observed.events.iter().any(|event| matches!(event,
                MqttEvent::Connected(result) if result.reason_code == 0)),
        "terminal connection accepted peer traffic: {observed:#?}"
    );
}

fn assert_delivered(observed: &Observation) {
    assert!(observed.connected, "{observed:#?}");
    assert_eq!(
        observed
            .events
            .iter()
            .filter(|event| matches!(event,
                MqttEvent::MessageReceived(message)
                    if message.packet_id == Some(PEER_ID) && message.payload == b"payload"
            ))
            .count(),
        1,
        "{observed:#?}"
    );
    assert!(has_puback(&observed.packets, PEER_ID), "{observed:#?}");
    assert!(
        !observed
            .events
            .iter()
            .any(|e| matches!(e, MqttEvent::Error(_))),
        "{observed:#?}"
    );
}

fn expired_attempt(version: u8) -> MqttEngine {
    let mut opts = options(version);
    opts.operation_timeouts = OperationTimeouts {
        connect: Some(Duration::from_millis(17)),
        ..OperationTimeouts::default()
    };
    let mut engine = MqttEngine::new(opts);
    start(&mut engine);
    let deadline = engine.next_tick_at().expect("configured CONNECT deadline");
    let events = engine.handle_tick(deadline);
    assert!(
        events.iter().any(|event| matches!(
            event,
            MqttEvent::OperationFailed {
                operation: OperationKind::Connect,
                packet_id: None,
                error: MqttClientError::OperationTimeout { .. },
            }
        )),
        "deadline did not expire: {events:?}"
    );
    assert!(!engine.is_connected());
    assert!(drain(&mut engine).is_empty());
    engine
}

fn late_connack(version: u8, tail: Tail) {
    let mut engine = expired_attempt(version);
    let observed = with_tail(&mut engine, &connack(version, false, false), tail);
    assert_terminal(&observed);
}

#[derive(Clone, Copy)]
enum ConnackState {
    BeforeConnect,
    Connected,
    LocallyDisconnected,
}

fn unexpected_connack(version: u8, state: ConnackState, refused: bool, tail: Tail) {
    let mut engine = match state {
        ConnackState::BeforeConnect => MqttEngine::new(options(version)),
        ConnackState::Connected | ConnackState::LocallyDisconnected => connected(version),
    };
    if matches!(state, ConnackState::LocallyDisconnected) {
        engine.disconnect().unwrap();
        assert!(!engine.is_connected());
        drain(&mut engine);
        engine.take_events();
    }
    let observed = with_tail(&mut engine, &connack(version, false, refused), tail);
    assert!(
        !observed
            .events
            .iter()
            .any(|e| matches!(e, MqttEvent::Connected(_))),
        "unsolicited CONNACK created a connection result: {observed:#?}"
    );
    assert_terminal(&observed);
}

fn subscribe_three(engine: &mut MqttEngine) -> u16 {
    let id = engine
        .subscribe(
            SubscribeCommand::builder()
                .add_topic("one", 0)
                .add_topic("two/+", 1)
                .add_topic("three/#", 2)
                .build()
                .unwrap(),
        )
        .unwrap();
    let sent = drain(engine);
    assert!(
        matches!(sent.as_slice(),
            [MqttPacket::Subscribe3(packet)] if packet.message_id == id && packet.subscriptions.len() == 3
        ) || matches!(sent.as_slice(),
            [MqttPacket::Subscribe5(packet)] if packet.packet_id == id && packet.subscriptions.len() == 3
        ),
        "three-filter SUBSCRIBE was not transmitted: {sent:?}"
    );
    id
}

fn mismatched_suback(version: u8, count: usize, tail: Tail) {
    let mut engine = connected(version);
    let id = subscribe_three(&mut engine);
    let observed = with_tail(&mut engine, &suback(version, id, &vec![0; count]), tail);
    assert!(
        observed
            .events
            .iter()
            .any(|e| matches!(e, MqttEvent::Error(_)))
            && !observed
                .events
                .iter()
                .any(|e| matches!(e, MqttEvent::Subscribed(_))),
        "SUBACK with {count} results completed three filters: {observed:#?}"
    );
    // MQTT 3.1.1 4.8 requires closing once detected. MQTT 5 follows the selected
    // strict close policy here, without applying it universally to unknown IDs.
    assert_terminal(&observed);
}

fn after_local_disconnect(version: u8, partial: bool) {
    let mut engine = connected(version);
    let packet = publish(version, 1, PEER_ID);
    let remaining = if partial {
        let split = packet.len() - 1;
        let prefix = observe(&mut engine, &[&packet[..split]]);
        assert!(prefix.events.is_empty() && prefix.packets.is_empty());
        &packet[split..]
    } else {
        packet.as_slice()
    };
    engine.disconnect().unwrap();
    assert!(!engine.is_connected());
    let observed = observe(&mut engine, &[remaining]);
    assert_terminal(&observed);
}

fn after_refused_connack(version: u8, tail: Tail) {
    let mut engine = MqttEngine::new(options(version));
    start(&mut engine);
    let observed = with_tail(&mut engine, &connack(version, false, true), tail);
    assert_terminal(&observed);
}

macro_rules! cases {
    ($($name:ident => $body:expr;)*) => {$(
        #[test]
        fn $name() { $body }
    )*};
}

macro_rules! shared_cases {
    () => {
        cases! {
            late_connack_cannot_revive_expired_attempt => late_connack(VERSION, Tail::None);
            late_connack_cannot_deliver_or_ack_tail => late_connack(VERSION, Tail::Coalesced);
            connack_before_connect_is_rejected => unexpected_connack(VERSION, ConnackState::BeforeConnect, false, Tail::None);
            connack_before_connect_cannot_deliver_tail => unexpected_connack(VERSION, ConnackState::BeforeConnect, false, Tail::Coalesced);
            duplicate_success_is_rejected => unexpected_connack(VERSION, ConnackState::Connected, false, Tail::None);
            duplicate_success_cannot_deliver_tail => unexpected_connack(VERSION, ConnackState::Connected, false, Tail::Coalesced);
            connack_after_local_disconnect_is_rejected => unexpected_connack(VERSION, ConnackState::LocallyDisconnected, false, Tail::None);
            connack_after_local_disconnect_cannot_deliver_tail => unexpected_connack(VERSION, ConnackState::LocallyDisconnected, false, Tail::Coalesced);
            duplicate_refusal_cannot_deliver_tail => unexpected_connack(VERSION, ConnackState::Connected, true, Tail::Coalesced);
            duplicate_refusal_cannot_deliver_later_publish => unexpected_connack(VERSION, ConnackState::Connected, true, Tail::Later);
            suback_with_two_results_for_three_filters_is_rejected => mismatched_suback(VERSION, 2, Tail::None);
            suback_with_four_results_for_three_filters_is_rejected => mismatched_suback(VERSION, 4, Tail::None);
            short_suback_cannot_deliver_tail => mismatched_suback(VERSION, 2, Tail::Coalesced);
            long_suback_cannot_deliver_tail => mismatched_suback(VERSION, 4, Tail::Coalesced);
            short_suback_cannot_deliver_partial_tail => mismatched_suback(VERSION, 2, Tail::Partial);
            long_suback_cannot_deliver_partial_tail => mismatched_suback(VERSION, 4, Tail::Partial);
            local_disconnect_suppresses_later_publish => after_local_disconnect(VERSION, false);
            local_disconnect_suppresses_previously_buffered_partial_publish => after_local_disconnect(VERSION, true);
            refused_connack_suppresses_later_publish => after_refused_connack(VERSION, Tail::Later);
            refused_connack_suppresses_partial_tail => after_refused_connack(VERSION, Tail::Partial);
        }

        #[test]
        fn valid_connack_delivers_and_acknowledges_tail() {
            let mut engine = MqttEngine::new(options(VERSION));
            start(&mut engine);
            let observed = with_tail(&mut engine, &connack(VERSION, false, false), Tail::Coalesced);
            assert_eq!(observed.events.iter().filter(|e| matches!(e, MqttEvent::Connected(_))).count(), 1);
            assert_delivered(&observed);
        }

        #[test]
        fn new_connect_after_expired_attempt_can_succeed() {
            let mut engine = expired_attempt(VERSION);
            engine.reset_for_new_transport();
            start(&mut engine);
            let observed = with_tail(&mut engine, &connack(VERSION, false, false), Tail::Coalesced);
            assert_delivered(&observed);
        }

        #[test]
        fn suback_with_three_results_completes_three_filters() {
            let mut engine = connected(VERSION);
            let id = subscribe_three(&mut engine);
            let observed = with_tail(&mut engine, &suback(VERSION, id, &[0, 1, 2]), Tail::Coalesced);
            assert!(observed.events.iter().any(|event| matches!(event,
                MqttEvent::Subscribed(result) if result.packet_id == id && result.reason_codes == [0, 1, 2]
            )), "{observed:#?}");
            assert_delivered(&observed);
        }

        #[test]
        fn partial_publish_completes_on_an_active_connection() {
            let mut engine = connected(VERSION);
            let packet = publish(VERSION, 1, PEER_ID);
            let split = packet.len() - 1;
            let prefix = observe(&mut engine, &[&packet[..split]]);
            assert!(prefix.events.is_empty() && prefix.packets.is_empty());
            assert_delivered(&observe(&mut engine, &[&packet[split..]]));
        }

        #[test]
        fn local_disconnect_discards_complete_publish_buffered_by_backpressure() {
            let mut opts = options(VERSION);
            opts.max_event_count = 1;
            let mut engine = connected_with(opts, &connack(VERSION, false, false));
            let bytes = [publish(VERSION, 0, 0), publish(VERSION, 1, PEER_ID)].concat();
            let events = engine.handle_incoming(&bytes);
            assert!(matches!(events.as_slice(), [MqttEvent::PublishReceived { packet_id: None, .. }, MqttEvent::MessageReceived(_)]));
            // Do not drain output here: that would pump the buffered PUBLISH.
            engine.disconnect().unwrap();
            assert_terminal(&observe(&mut engine, &[&[]]));
        }

        #[test]
        fn local_disconnect_suppresses_stream_ingress() {
            let mut engine = connected(VERSION);
            engine.disconnect().unwrap();
            drain(&mut engine);
            let packet = decode(&publish(VERSION, 1, PEER_ID), VERSION).pop().unwrap();
            let (events, bytes) = engine.ingest_stream_packet(packet, 4);
            assert_terminal(&Observation {
                connected: engine.is_connected(), events, packets: decode(&bytes, VERSION),
            });
        }

        #[test]
        fn reduced_parsing_cannot_accept_duplicate_connack() {
            use flowsdk::mqtt_serde::parser::leveled::ParseLevel;
            for level in [ParseLevel::HeadersParsed, ParseLevel::TypeOnly] {
                let mut engine = connected(VERSION);
                engine.try_set_parse_level(level).unwrap();
                let observed = with_tail(&mut engine, &connack(VERSION, false, false), Tail::Coalesced);
                assert_terminal(&observed);
                assert!(observed.events.iter().any(|e| matches!(e, MqttEvent::Error(_))), "{observed:#?}");
            }
        }

        #[test]
        fn new_transport_after_local_disconnect_can_connect() {
            let mut engine = connected(VERSION);
            engine.disconnect().unwrap();
            drain(&mut engine);
            engine.reset_for_new_transport();
            start(&mut engine);
            assert_delivered(&with_tail(&mut engine, &connack(VERSION, false, false), Tail::Coalesced));
        }
    };
}

#[derive(Clone, Copy)]
enum OutboundStage {
    Qos1Publish,
    Qos2Publish,
    PubRel,
}

fn persistent_exchange(version: u8, stage: OutboundStage) -> (MqttEngine, u16) {
    let mut opts = options(version);
    opts.clean_start = false;
    if version == 5 {
        opts.session_expiry_interval = Some(3600);
    }
    let mut engine = connected_with(opts, &connack(version, false, false));
    let qos = if matches!(stage, OutboundStage::Qos1Publish) {
        1
    } else {
        2
    };
    let id = engine
        .publish(PublishCommand::simple(
            "outbound",
            b"retained work".to_vec(),
            qos,
            false,
        ))
        .unwrap()
        .unwrap();
    let sent = drain(&mut engine);
    assert!(
        matches!(sent.as_slice(),
            [MqttPacket::Publish3(p)] if p.message_id == Some(id) && p.qos == qos && !p.dup
        ) || matches!(sent.as_slice(),
            [MqttPacket::Publish5(p)] if p.packet_id == Some(id) && p.qos == qos && !p.dup
        ),
        "PUBLISH was not transmitted: {sent:?}"
    );
    if matches!(stage, OutboundStage::PubRel) {
        let received = observe(&mut engine, &[&acknowledgement(0x50, id)]);
        assert!(
            matches!(received.packets.as_slice(),
                [MqttPacket::PubRel3(p)] if p.message_id == id
            ) || matches!(received.packets.as_slice(),
                [MqttPacket::PubRel5(p)] if p.packet_id == id
            ),
            "PUBREL was not transmitted: {received:?}"
        );
    }
    engine.handle_connection_lost();
    start(&mut engine);
    (engine, id)
}

mod mqtt311 {
    use super::*;
    const VERSION: u8 = 4;
    shared_cases!();

    #[test]
    fn detected_unknown_suback_closes_connection() {
        let mut engine = connected(VERSION);
        let observed = observe(&mut engine, &[&suback(VERSION, 0x7777, &[0])]);
        assert!(
            observed.events.iter().any(|event| matches!(
                event,
                MqttEvent::Error(MqttClientError::InvalidPacketId { packet_id: 0x7777 })
            )),
            "test requires a detected protocol error: {observed:?}"
        );
        assert!(!observed
            .events
            .iter()
            .any(|e| matches!(e, MqttEvent::Subscribed(_))));
        // MQTT 3.1.1 section 4.8, after detection rather than assumed detection.
        assert_terminal(&observed);
    }

    fn sp_zero_retries_or_closes(stage: OutboundStage) {
        let (mut engine, id) = persistent_exchange(VERSION, stage);
        let observed = observe(&mut engine, &[&connack(VERSION, false, false)]);
        // Section 3.2.2.2 permits disconnecting on disagreement about SP. If the
        // client continues, section 4.4 requires the original identifier/work.
        if !observed.connected {
            return;
        }
        let retried = observed.packets.iter().any(|packet| match (stage, packet) {
            (OutboundStage::Qos1Publish, MqttPacket::Publish3(p)) => {
                p.qos == 1 && p.dup && p.message_id == Some(id) && p.payload == b"retained work"
            }
            (OutboundStage::Qos2Publish, MqttPacket::Publish3(p)) => {
                p.qos == 2 && p.dup && p.message_id == Some(id) && p.payload == b"retained work"
            }
            (OutboundStage::PubRel, MqttPacket::PubRel3(p)) => p.message_id == id,
            _ => false,
        });
        assert!(
            retried,
            "SP=0 continued but lost transmitted work {id}: {observed:#?}"
        );
    }

    cases! {
        persistent_sp_zero_retries_publish_awaiting_puback_or_closes => sp_zero_retries_or_closes(OutboundStage::Qos1Publish);
        persistent_sp_zero_retries_publish_awaiting_pubrec_or_closes => sp_zero_retries_or_closes(OutboundStage::Qos2Publish);
        persistent_sp_zero_retries_pubrel_awaiting_pubcomp_or_closes => sp_zero_retries_or_closes(OutboundStage::PubRel);
    }
}

mod mqtt5 {
    use super::*;
    const VERSION: u8 = 5;
    shared_cases!();

    #[test]
    fn duplicate_connack_cannot_replace_negotiated_receive_maximum() {
        let mut engine = connected_with(options(VERSION), &limited_connack(false, 1));
        let id = engine
            .publish(PublishCommand::simple("first", vec![], 1, false))
            .unwrap()
            .unwrap();
        assert!(
            matches!(drain(&mut engine).as_slice(), [MqttPacket::Publish5(p)] if p.packet_id == Some(id))
        );
        engine
            .publish(PublishCommand::simple("queued", vec![], 1, false))
            .unwrap();
        assert!(
            drain(&mut engine).is_empty(),
            "initial Receive Maximum must block new PUBLISH"
        );
        let observed = observe(&mut engine, &[&limited_connack(false, 2)]);
        assert!(
            !observed.connected
                && !observed
                    .events
                    .iter()
                    .any(|e| matches!(e, MqttEvent::Connected(_)))
                && !observed
                    .packets
                    .iter()
                    .any(|p| matches!(p, MqttPacket::Publish5(_))),
            "duplicate CONNACK changed the live connection's quota: {observed:#?}"
        );
    }

    #[test]
    fn unknown_suback_is_rejected_without_a_universal_close_expectation() {
        let mut engine = connected(VERSION);
        let observed = observe(&mut engine, &[&suback(VERSION, 0x7777, &[0])]);
        assert!(
            observed
                .events
                .iter()
                .any(|e| matches!(e, MqttEvent::Error(_))),
            "{observed:?}"
        );
        assert!(
            !observed
                .events
                .iter()
                .any(|e| matches!(e, MqttEvent::Subscribed(_))),
            "{observed:?}"
        );
    }

    fn receive_quota_resets(awaiting_pubcomp: bool) {
        let mut opts = options(VERSION);
        opts.clean_start = false;
        opts.session_expiry_interval = Some(3600);
        opts.incoming_receive_maximum = Some(1);
        opts.auto_ack = false;
        let mut engine = connected_with(opts, &connack(VERSION, false, false));
        let first = observe(&mut engine, &[&publish(VERSION, 2, 7)]);
        assert!(has_delivery(&first.events), "{first:?}");
        engine.pubrec(7, 0, vec![]).unwrap();
        assert!(
            matches!(drain(&mut engine).as_slice(), [MqttPacket::PubRec5(p)] if p.packet_id == 7)
        );
        if awaiting_pubcomp {
            let rel = observe(&mut engine, &[&acknowledgement(0x62, 7)]);
            assert!(rel
                .events
                .iter()
                .any(|e| matches!(e, MqttEvent::PubRelReceived { packet_id: 7, .. })));
            assert!(rel.packets.is_empty());
        }
        engine.handle_connection_lost();
        start(&mut engine);
        let resumed = observe(&mut engine, &[&connack(VERSION, true, false)]);
        assert!(resumed.connected, "{resumed:?}");

        // MQTT 5 section 4.9: the old QoS 2 state survives, its connection quota
        // does not. Do not complete the old exchange before sending this PUBLISH.
        let observed = observe(&mut engine, &[&publish(VERSION, 1, PEER_ID)]);
        assert!(
            observed.connected
                && has_delivery(&observed.events)
                && !observed
                    .events
                    .iter()
                    .any(|e| matches!(e, MqttEvent::Error(_))),
            "old QoS 2 exchange consumed new connection quota: {observed:#?}"
        );
        engine.puback(PEER_ID, 0, vec![]).unwrap();
        assert!(has_puback(&drain(&mut engine), PEER_ID));
        // Also require the old receive stage to survive; simply clearing all
        // received session state on reconnect is not a valid quota fix.
        if !awaiting_pubcomp {
            engine.pubrec(7, 0, vec![]).unwrap();
            assert!(
                matches!(drain(&mut engine).as_slice(), [MqttPacket::PubRec5(p)] if p.packet_id == 7)
            );
            let rel = observe(&mut engine, &[&acknowledgement(0x62, 7)]);
            assert!(rel
                .events
                .iter()
                .any(|e| matches!(e, MqttEvent::PubRelReceived { packet_id: 7, .. })));
        }
        engine.pubcomp(7, 0, vec![]).unwrap();
        assert!(
            matches!(drain(&mut engine).as_slice(), [MqttPacket::PubComp5(p)] if p.packet_id == 7 && p.reason_code == 0)
        );
    }

    cases! {
        new_connection_quota_excludes_old_qos2_waiting_for_pubrel => receive_quota_resets(false);
        new_connection_quota_excludes_old_qos2_waiting_for_pubcomp => receive_quota_resets(true);
    }

    fn resume_with_old_received_publish(send_pubrec: bool) -> MqttEngine {
        let mut opts = options(VERSION);
        opts.clean_start = false;
        opts.session_expiry_interval = Some(3600);
        opts.incoming_receive_maximum = Some(1);
        opts.auto_ack = false;
        let mut engine = connected_with(opts, &connack(VERSION, false, false));
        assert!(has_delivery(
            &observe(&mut engine, &[&publish(VERSION, 2, 7)]).events
        ));
        if send_pubrec {
            engine.pubrec(7, 0, vec![]).unwrap();
            drain(&mut engine);
        }
        engine.handle_connection_lost();
        start(&mut engine);
        assert!(observe(&mut engine, &[&connack(VERSION, true, false)]).connected);
        engine
    }

    #[test]
    fn new_connection_still_enforces_receive_maximum() {
        let mut engine = resume_with_old_received_publish(true);
        let first = observe(&mut engine, &[&publish(VERSION, 1, PEER_ID)]);
        assert!(first.connected && has_delivery(&first.events), "{first:#?}");
        let excess = observe(&mut engine, &[&publish(VERSION, 1, PEER_ID + 1)]);
        assert_terminal(&excess);
        assert!(excess.events.iter().any(|e| matches!(
            e,
            MqttEvent::Error(MqttClientError::ProtocolViolation { .. })
        )));
    }

    #[test]
    fn resumed_publish_consumes_new_quota_without_redelivery() {
        let mut engine = resume_with_old_received_publish(false);
        let mut duplicate = publish(VERSION, 2, 7);
        duplicate[0] |= 8;
        let replay = observe(&mut engine, &[&duplicate]);
        assert!(
            replay.connected && !has_delivery(&replay.events),
            "{replay:#?}"
        );
        engine.pubrec(7, 0, vec![]).unwrap();
        drain(&mut engine);
        // A successful PUBREC does not replenish the quota.
        let excess = observe(&mut engine, &[&publish(VERSION, 1, PEER_ID)]);
        assert_terminal(&excess);
        assert!(excess.events.iter().any(|e| matches!(
            e,
            MqttEvent::Error(MqttClientError::ProtocolViolation { .. })
        )));
    }

    #[test]
    fn old_pubcomp_replenishes_new_connection_quota_without_exceeding_limit() {
        let mut engine = resume_with_old_received_publish(true);
        assert!(has_delivery(
            &observe(&mut engine, &[&publish(VERSION, 1, 10)]).events
        ));
        observe(&mut engine, &[&acknowledgement(0x62, 7)]);
        engine.pubcomp(7, 0, vec![]).unwrap();
        drain(&mut engine);
        assert!(has_delivery(
            &observe(&mut engine, &[&publish(VERSION, 1, 11)]).events
        ));
        let excess = observe(&mut engine, &[&publish(VERSION, 1, PEER_ID)]);
        assert_terminal(&excess);
    }

    #[test]
    fn sp_zero_discards_old_session_work() {
        for stage in [
            OutboundStage::Qos1Publish,
            OutboundStage::Qos2Publish,
            OutboundStage::PubRel,
        ] {
            let (mut engine, id) = persistent_exchange(VERSION, stage);
            let observed = observe(&mut engine, &[&connack(VERSION, false, false)]);
            assert!(
                observed.connected && observed.packets.is_empty(),
                "{observed:#?}"
            );
            assert!(observed.events.iter().any(|event| matches!(event,
                MqttEvent::OperationFailed { packet_id: Some(pid), error: MqttClientError::SessionExpired, .. } if *pid == id
            )), "MQTT 5 must report discarded session work: {observed:#?}");
        }
    }

    fn auth(reason: u8, method: Option<&str>) -> Vec<u8> {
        let mut properties = Vec::new();
        if let Some(method) = method {
            properties.push(0x15);
            properties.extend((method.len() as u16).to_be_bytes());
            properties.extend(method.as_bytes());
        }
        let mut body = vec![reason, properties.len() as u8];
        body.extend(properties);
        frame(0xf0, &body)
    }

    fn auth_options() -> MqttClientOptions {
        let mut opts = options(VERSION);
        opts.connect_properties = vec![Property::AuthenticationMethod(METHOD.into())];
        opts
    }

    fn auth_connack() -> Vec<u8> {
        let auth = auth(0, Some(METHOD));
        frame(0x20, &[&[0, 0][..], &auth[3..]].concat())
    }

    fn authenticated() -> MqttEngine {
        connected_with(auth_options(), &auth_connack())
    }

    fn reauthenticating() -> MqttEngine {
        let mut engine = authenticated();
        engine.auth(0x19, vec![]).unwrap();
        assert!(
            matches!(drain(&mut engine).as_slice(), [MqttPacket::Auth(a)]
            if a.reason_code == 0x19 && a.properties.contains(&Property::AuthenticationMethod(METHOD.into())))
        );
        engine
    }

    fn reject_auth(engine: &mut MqttEngine, packet: &[u8]) {
        let observed = observe(engine, &[packet]);
        assert!(
            observed
                .events
                .iter()
                .any(|e| matches!(e, MqttEvent::Error(_)))
                && !observed
                    .events
                    .iter()
                    .any(|e| matches!(e, MqttEvent::AuthReceived(_))),
            "invalid AUTH surfaced as an authentication result: {observed:#?}"
        );
        assert_terminal(&observed);
    }

    #[test]
    fn initial_auth_success_cannot_replace_connack() {
        let mut engine = MqttEngine::new(auth_options());
        start(&mut engine);
        // Assert rejection of AuthReceived, not an invented Connected event.
        reject_auth(&mut engine, &auth(0, Some(METHOD)));
    }

    #[test]
    fn unsolicited_auth_continue_after_connack_is_rejected() {
        reject_auth(&mut authenticated(), &auth(0x18, Some(METHOD)));
    }

    #[test]
    fn unsolicited_auth_success_after_connack_is_rejected() {
        reject_auth(&mut authenticated(), &auth(0, Some(METHOD)));
    }

    #[test]
    fn minimal_auth_success_must_include_negotiated_method_in_strict_profile() {
        reject_auth(&mut reauthenticating(), &[0xf0, 0]);
    }

    fn auth_error_closes(packet: &[u8], tail: Tail) {
        let mut engine = reauthenticating();
        let observed = with_tail(&mut engine, packet, tail);
        assert!(
            observed.events.iter().any(|event| matches!(
                event,
                MqttEvent::Error(MqttClientError::ProtocolViolation { .. })
            )),
            "test requires a detected AUTH error: {observed:#?}"
        );
        assert!(
            !observed
                .events
                .iter()
                .any(|e| matches!(e, MqttEvent::AuthReceived(_))),
            "{observed:#?}"
        );
        // Strict SHOULD-close profile, not automatically an explicit refusal.
        assert_terminal(&observed);
    }

    cases! {
        changed_auth_method_closes_connection_in_strict_profile => auth_error_closes(&auth(0x18, Some("changed-method")), Tail::None);
        changed_auth_method_suppresses_tail_in_strict_profile => auth_error_closes(&auth(0x18, Some("changed-method")), Tail::Coalesced);
        changed_auth_method_suppresses_later_publish_in_strict_profile => auth_error_closes(&auth(0x18, Some("changed-method")), Tail::Later);
        missing_auth_method_closes_connection_in_strict_profile => auth_error_closes(&auth(0x18, None), Tail::None);
        server_reauthenticate_reason_closes_connection_in_strict_profile => auth_error_closes(&auth(0x19, Some(METHOD)), Tail::None);
    }

    fn reauth_refusal(tail: Tail) {
        let mut engine = reauthenticating();
        let observed = with_tail(&mut engine, &[0xe0, 2, 0x87, 0], tail);
        assert!(
            observed.events.iter().any(|event| matches!(
                event,
                MqttEvent::DisconnectReceived {
                    reason_code: 0x87,
                    ..
                }
            )),
            "explicit re-authentication refusal was not reported: {observed:#?}"
        );
        assert_terminal(&observed);
    }

    cases! {
        explicit_reauth_refusal_suppresses_later_publish => reauth_refusal(Tail::Later);
        explicit_reauth_refusal_closes_connection => reauth_refusal(Tail::None);
        explicit_reauth_refusal_suppresses_same_buffer_tail => reauth_refusal(Tail::Coalesced);
    }

    #[test]
    fn initial_auth_continue_then_connack_succeeds() {
        let mut engine = MqttEngine::new(auth_options());
        start(&mut engine);
        let challenge = observe(&mut engine, &[&auth(0x18, Some(METHOD))]);
        assert!(!challenge.connected);
        assert!(
            matches!(challenge.events.as_slice(), [MqttEvent::AuthReceived(a)] if a.reason_code == 0x18)
        );
        engine.auth(0x18, vec![]).unwrap();
        assert!(
            matches!(drain(&mut engine).as_slice(), [MqttPacket::Auth(a)] if a.reason_code == 0x18)
        );
        let observed = with_tail(&mut engine, &auth_connack(), Tail::Coalesced);
        assert_delivered(&observed);
    }

    #[test]
    fn client_initiated_reauthentication_can_continue_and_succeed() {
        let mut engine = reauthenticating();
        let challenge = observe(&mut engine, &[&auth(0x18, Some(METHOD))]);
        assert!(challenge.connected);
        assert!(
            matches!(challenge.events.as_slice(), [MqttEvent::AuthReceived(a)] if a.reason_code == 0x18)
        );
        engine.auth(0x18, vec![]).unwrap();
        drain(&mut engine);
        let observed = with_tail(&mut engine, &auth(0, Some(METHOD)), Tail::Coalesced);
        assert!(observed
            .events
            .iter()
            .any(|e| matches!(e, MqttEvent::AuthReceived(a) if a.reason_code == 0)));
        assert_delivered(&observed);
    }

    #[test]
    fn completed_reauthentication_rejects_unsolicited_auth() {
        let mut engine = reauthenticating();
        let success = observe(&mut engine, &[&auth(0, Some(METHOD))]);
        assert!(
            matches!(success.events.as_slice(), [MqttEvent::AuthReceived(a)] if a.reason_code == 0)
        );
        reject_auth(&mut engine, &auth(0x18, Some(METHOD)));
    }

    #[test]
    fn subsequent_reauthentication_can_start_and_allows_publish_traffic() {
        let mut engine = authenticated();
        for _ in 0..2 {
            engine.auth(0x19, vec![]).unwrap();
            drain(&mut engine);
            assert_delivered(&observe(&mut engine, &[&publish(VERSION, 1, PEER_ID)]));
            let success = observe(&mut engine, &[&auth(0, Some(METHOD))]);
            assert!(success.connected);
            assert!(
                matches!(success.events.as_slice(), [MqttEvent::AuthReceived(a)] if a.reason_code == 0)
            );
        }
    }

    #[test]
    fn failed_auth_enqueue_does_not_begin_reauthentication() {
        let mut opts = auth_options();
        opts.max_outgoing_packet_count = 1;
        let mut engine = connected_with(opts, &auth_connack());
        engine.send_ping().unwrap();
        assert!(matches!(
            engine.auth(0x19, vec![]),
            Err(MqttClientError::BufferFull { .. })
        ));
        drain(&mut engine);
        reject_auth(&mut engine, &auth(0x18, Some(METHOD)));
    }
}
