// SPDX-License-Identifier: MPL-2.0

mod common;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use flowsdk::mqtt_client::{MqttClientOptions, MqttEngine, MqttEvent, PublishCommand};
use flowsdk::mqtt_serde::control_packet::{ControlPacketType, MqttPacket};
use flowsdk::mqtt_serde::{mqttv3, mqttv5};
use std::hint::black_box;

fn connected_engine(version: u8) -> MqttEngine {
    let options = MqttClientOptions::builder()
        .mqtt_version(version)
        .client_id("microbench")
        .keep_alive(0)
        .build();
    let mut engine = MqttEngine::new(options);
    engine.connect().unwrap();
    assert!(!engine.take_outgoing().is_empty());
    let connack = match version {
        4 => MqttPacket::ConnAck3(mqttv3::connack::MqttConnAck::new(false, 0)),
        5 => MqttPacket::ConnAck5(mqttv5::connack::MqttConnAck::new(false, 0, None)),
        _ => unreachable!(),
    };
    let events = engine.handle_incoming(&connack.to_bytes().unwrap());
    assert!(matches!(events.as_slice(), [MqttEvent::Connected(_)]));
    assert!(engine.is_connected());
    assert!(engine.take_outgoing().is_empty());
    assert!(engine.take_events().is_empty());
    engine
}

struct Exchange {
    frames: [Vec<u8>; 2],
    events: [Vec<MqttEvent>; 3],
}

struct Acks {
    puback: Vec<u8>,
    pubrec: Vec<u8>,
    pubrel: Vec<u8>,
    pubcomp: Vec<u8>,
}

impl Acks {
    fn new(version: u8) -> Self {
        let encode =
            |kind| common::encode_checked(&common::ack(version, kind, common::PACKET_ID), version);
        Self {
            puback: encode(ControlPacketType::PUBACK),
            pubrec: encode(ControlPacketType::PUBREC),
            pubrel: encode(ControlPacketType::PUBREL),
            pubcomp: encode(ControlPacketType::PUBCOMP),
        }
    }
}

fn send(engine: &mut MqttEngine, command: PublishCommand, acks: &Acks) -> Exchange {
    let qos = command.qos;
    black_box(engine.publish(black_box(command)).unwrap());
    let publish = engine.take_outgoing();
    let mut events = [engine.take_events(), Vec::new(), Vec::new()];
    let mut pubrel = Vec::new();
    if qos == 1 {
        events[1] = engine.handle_incoming(black_box(&acks.puback));
    } else if qos == 2 {
        events[1] = engine.handle_incoming(black_box(&acks.pubrec));
        pubrel = engine.take_outgoing();
        events[2] = engine.handle_incoming(black_box(&acks.pubcomp));
    }
    Exchange {
        frames: [publish, pubrel],
        events,
    }
}

fn receive(engine: &mut MqttEngine, publish: &[u8], qos: u8, acks: &Acks) -> Exchange {
    let mut events = [
        engine.handle_incoming(black_box(publish)),
        Vec::new(),
        Vec::new(),
    ];
    let response = engine.take_outgoing();
    let mut pubcomp = Vec::new();
    if qos == 2 {
        events[1] = engine.handle_incoming(black_box(&acks.pubrel));
        pubcomp = engine.take_outgoing();
    }
    Exchange {
        frames: [response, pubcomp],
        events,
    }
}

fn validate_exchange(engine: &mut MqttEngine, exchange: &Exchange) {
    for event in exchange.events.iter().flatten() {
        assert!(
            !matches!(
                event,
                MqttEvent::Error(_)
                    | MqttEvent::OperationFailed { .. }
                    | MqttEvent::Disconnected(_)
            ),
            "{event:?}"
        );
    }
    assert!(engine.is_connected());
    assert!(engine.take_events().is_empty());
    assert!(engine.take_outgoing().is_empty());
}

fn engine(c: &mut Criterion) {
    for (version, label) in common::VERSIONS {
        let acks = Acks::new(version);
        let mut group = c.benchmark_group(format!("engine/{label}"));
        // Each operation is one full in-memory exchange, not network latency.
        group.throughput(Throughput::Elements(1));
        for qos in 0..=2 {
            for size in [256, 4096] {
                let command = common::command(qos, size, common::PACKET_ID);
                let publish = common::encode_checked(
                    &common::publish(version, qos, size, common::PACKET_ID),
                    version,
                );
                let mut sender = connected_engine(version);
                let mut receiver = connected_engine(version);
                // Reusing the packet ID proves that both handshakes complete.
                for _ in 0..2 {
                    let sent = send(&mut sender, command.clone(), &acks);
                    validate_exchange(&mut sender, &sent);
                    assert_eq!(sent.frames[0], publish);
                    assert_eq!(
                        sent.frames[1].as_slice(),
                        if qos == 2 {
                            acks.pubrel.as_slice()
                        } else {
                            &[]
                        }
                    );
                    let completed: Vec<_> = sent
                        .events
                        .iter()
                        .flatten()
                        .filter_map(|event| {
                            if let MqttEvent::Published(result) = event {
                                Some(result)
                            } else {
                                None
                            }
                        })
                        .collect();
                    // The engine emits Published only for acknowledged QoS 1/2.
                    // QoS 0 completes when its encoded output is drained above.
                    assert_eq!(completed.len(), usize::from(qos > 0));
                    if qos > 0 {
                        assert_eq!(completed[0].qos, qos);
                        assert_eq!(completed[0].packet_id, command.packet_id);
                        assert!(completed[0].is_success());
                    }

                    let received = receive(&mut receiver, &publish, qos, &acks);
                    validate_exchange(&mut receiver, &received);
                    let expected_ack = match qos {
                        1 => acks.puback.as_slice(),
                        2 => acks.pubrec.as_slice(),
                        _ => &[],
                    };
                    assert_eq!(received.frames[0], expected_ack);
                    assert_eq!(
                        received.frames[1].as_slice(),
                        if qos == 2 {
                            acks.pubcomp.as_slice()
                        } else {
                            &[]
                        }
                    );
                    let messages: Vec<_> = received
                        .events
                        .iter()
                        .flatten()
                        .filter_map(|event| {
                            if let MqttEvent::MessageReceived(message) = event {
                                Some(message)
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_eq!(messages.len(), 1);
                    assert_eq!(messages[0].topic_name, common::TOPIC);
                    assert_eq!(messages[0].payload, command.payload);
                    assert_eq!(messages[0].qos, qos);
                }

                group.bench_function(BenchmarkId::new(format!("send/qos{qos}"), size), |b| {
                    b.iter_batched_ref(
                        || (connected_engine(version), Some(command.clone())),
                        |(engine, command)| black_box(send(engine, command.take().unwrap(), &acks)),
                        BatchSize::LargeInput,
                    );
                });
                group.bench_function(BenchmarkId::new(format!("receive/qos{qos}"), size), |b| {
                    b.iter_batched_ref(
                        || connected_engine(version),
                        |engine| black_box(receive(engine, &publish, qos, &acks)),
                        BatchSize::LargeInput,
                    );
                });
            }
        }
        group.finish();
    }
}

criterion_group!(benches, engine);
criterion_main!(benches);
