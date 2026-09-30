// SPDX-License-Identifier: MPL-2.0

mod common;

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
use flowsdk::mqtt_serde::parser::leveled::{
    parse_headers_only, parse_raw_body, parse_type_only, LeveledParseOk, ParseLevel, ParsedPacket,
    VariableHeader,
};
use flowsdk::mqtt_serde::parser::{parse_remaining_length, ParseOk};
use std::hint::black_box;

/// Verify the direct APIs before timing, including the bytes retained by each level.
fn validate(packet: &MqttPacket, frame: &Bytes, version: u8) {
    match MqttPacket::from_bytes_with_version(frame, version).unwrap() {
        ParseOk::Packet(decoded, consumed) => {
            assert_eq!(&decoded, packet);
            assert_eq!(consumed, frame.len());
        }
        other => panic!("expected a complete packet: {other:?}"),
    }

    let (_, vbi_len) = parse_remaining_length(&frame[1..]).unwrap();
    for (level, parsed) in [
        (
            ParseLevel::HeadersParsed,
            parse_headers_only(frame.clone(), version).unwrap(),
        ),
        (ParseLevel::RawBody, parse_raw_body(frame.clone()).unwrap()),
        (ParseLevel::TypeOnly, parse_type_only(frame).unwrap()),
    ] {
        let LeveledParseOk::Packet(parsed, consumed) = parsed else {
            panic!("expected a complete leveled packet");
        };
        assert_eq!(consumed, frame.len());
        assert_eq!(parsed.packet_type(), packet.packet_type());
        match (level, parsed) {
            (ParseLevel::HeadersParsed, ParsedPacket::HeadersParsed(header)) => {
                assert_eq!(header.mqtt_version, version);
                assert_eq!(header.flags, frame[0] & 0x0f);
                let payload = match (&header.variable_header, packet) {
                    (
                        VariableHeader::PublishV3 {
                            topic_name,
                            qos,
                            message_id,
                            ..
                        },
                        MqttPacket::Publish3(expected),
                    ) => {
                        assert_eq!(topic_name, &expected.topic_name);
                        assert_eq!(*qos, expected.qos);
                        assert_eq!(*message_id, expected.message_id);
                        &expected.payload
                    }
                    (
                        VariableHeader::PublishV5 {
                            topic_name,
                            qos,
                            packet_id,
                            properties,
                            ..
                        },
                        MqttPacket::Publish5(expected),
                    ) => {
                        assert_eq!(topic_name, &expected.topic_name);
                        assert_eq!(*qos, expected.qos);
                        assert_eq!(*packet_id, expected.packet_id);
                        assert_eq!(properties, &expected.properties);
                        &expected.payload
                    }
                    _ => panic!("wrong PUBLISH header variant"),
                };
                assert_eq!(header.raw_payload.as_ref(), payload.as_slice());
            }
            (ParseLevel::RawBody, ParsedPacket::RawBody(raw)) => {
                assert_eq!(raw.flags, frame[0] & 0x0f);
                assert_eq!(raw.remaining.as_ref(), &frame[1 + vbi_len..]);
            }
            (ParseLevel::TypeOnly, ParsedPacket::TypeOnly(header)) => {
                assert_eq!(header.flags, frame[0] & 0x0f)
            }
            _ => panic!("wrong parser level"),
        }
    }
}

fn leveled_parser(c: &mut Criterion) {
    for (version, label) in common::VERSIONS {
        let mut fixtures: Vec<_> = common::PAYLOAD_SIZES
            .into_iter()
            .map(|size| {
                (
                    "publish/qos1",
                    size,
                    common::publish(version, 1, size, common::PACKET_ID),
                )
            })
            .collect();
        if version == 5 {
            let mut command = common::command(1, 256, common::PACKET_ID);
            command.properties = (0..8)
                .map(|i| Property::UserProperty(format!("key-{i}"), format!("value-{i}")))
                .collect();
            fixtures.push((
                "publish/qos1/user_properties_8",
                256,
                MqttPacket::Publish5(command.to_mqtt_publish()),
            ));
        }
        let fixtures: Vec<_> = fixtures
            .into_iter()
            .map(|(name, size, packet)| {
                let frame = Bytes::from(common::encode_checked(&packet, version));
                validate(&packet, &frame, version);
                (name, size, frame)
            })
            .collect();

        for (level, name) in [
            (ParseLevel::Full, "full"),
            (ParseLevel::HeadersParsed, "headers"),
            (ParseLevel::RawBody, "raw"),
            (ParseLevel::TypeOnly, "type_only"),
        ] {
            let mut group = c.benchmark_group(format!("leveled_parser/{label}/{name}"));
            for (case, size, frame) in &fixtures {
                group.throughput(Throughput::Bytes(frame.len() as u64));
                group.bench_with_input(BenchmarkId::new(*case, size), frame, |b, frame| {
                    // Match outside the measured loop. Bytes-taking APIs include a
                    // shared Bytes clone, and every case includes result destruction.
                    match level {
                        ParseLevel::Full => b.iter(|| {
                            black_box(
                                MqttPacket::from_bytes_with_version(
                                    black_box(frame.as_ref()),
                                    version,
                                )
                                .unwrap(),
                            )
                        }),
                        ParseLevel::HeadersParsed => b.iter(|| {
                            black_box(
                                parse_headers_only(black_box(frame.clone()), version).unwrap(),
                            )
                        }),
                        ParseLevel::RawBody => {
                            b.iter(|| black_box(parse_raw_body(black_box(frame.clone())).unwrap()))
                        }
                        ParseLevel::TypeOnly => b.iter(|| {
                            black_box(parse_type_only(black_box(frame.as_ref())).unwrap())
                        }),
                    }
                });
            }
            group.finish();
        }
    }
}

criterion_group!(benches, leveled_parser);
criterion_main!(benches);
