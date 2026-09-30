// SPDX-License-Identifier: MPL-2.0

mod common;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
use std::hint::black_box;

fn codec(c: &mut Criterion) {
    for (version, label) in common::VERSIONS {
        let mut packets: Vec<(String, MqttPacket)> = common::control_packets(version)
            .into_iter()
            .map(|(name, packet)| (name.into(), packet))
            .collect();
        for size in common::PAYLOAD_SIZES {
            packets.push((
                format!("publish/qos1/{size}"),
                common::publish(version, 1, size, common::PACKET_ID),
            ));
        }
        for qos in [0, 2] {
            packets.push((
                format!("publish/qos{qos}/256"),
                common::publish(version, qos, 256, common::PACKET_ID),
            ));
        }
        if version == 5 {
            let mut command = common::command(1, 256, common::PACKET_ID);
            command.properties = (0..8)
                .map(|i| Property::UserProperty(format!("key-{i}"), format!("value-{i}")))
                .collect();
            packets.push((
                "publish/qos1/256/user_properties_8".into(),
                MqttPacket::Publish5(command.to_mqtt_publish()),
            ));
        }

        let mut group = c.benchmark_group(format!("codec/{label}"));
        for (name, packet) in packets {
            let bytes = common::encode_checked(&packet, version);
            group.throughput(Throughput::Bytes(bytes.len() as u64));
            group.bench_with_input(BenchmarkId::new("encode", &name), &packet, |b, packet| {
                b.iter(|| black_box(black_box(packet).to_bytes().unwrap()));
            });
            group.bench_with_input(BenchmarkId::new("decode", &name), &bytes, |b, bytes| {
                b.iter(|| {
                    black_box(
                        MqttPacket::from_bytes_with_version(black_box(bytes), version).unwrap(),
                    )
                });
            });
        }
        group.finish();
    }
}

criterion_group!(benches, codec);
criterion_main!(benches);
