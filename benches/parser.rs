// SPDX-License-Identifier: MPL-2.0

mod common;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use flowsdk::mqtt_serde::parser::leveled::{ParseLevel, ParsedPacket};
use flowsdk::mqtt_serde::parser::stream::MqttParser;
use std::hint::black_box;

/// Feed every chunk and drain all results, as a transport would do.
fn parse_stream(parser: &mut MqttParser, bytes: &[u8], chunk_size: usize) -> usize {
    let mut count = 0;
    for chunk in bytes.chunks(chunk_size) {
        parser.feed(black_box(chunk));
        while let Some(packet) = parser.next_parsed().unwrap() {
            black_box(packet);
            count += 1;
        }
    }
    count
}

fn parser(c: &mut Criterion) {
    for (version, label) in common::VERSIONS {
        for (level, level_name) in [
            (ParseLevel::Full, "full"),
            (ParseLevel::HeadersParsed, "headers"),
            (ParseLevel::RawBody, "raw"),
            (ParseLevel::TypeOnly, "type_only"),
        ] {
            let mut group = c.benchmark_group(format!("parser/{label}/{level_name}"));
            // Representative cases, avoiding a full payload x chunk x batch product.
            let cases = common::PAYLOAD_SIZES
                .map(|size| ("complete", size, usize::MAX, 1))
                .into_iter()
                .chain([
                    ("fragmented_64", 4096, 64, 1),
                    ("batch_16", 256, usize::MAX, 16),
                ]);
            for (name, size, chunk_size, packet_count) in cases {
                let packet = common::publish(version, 1, size, common::PACKET_ID);
                let frame = common::encode_checked(&packet, version);
                let bytes = frame.repeat(packet_count);
                let mut parser = MqttParser::with_level(bytes.len(), version, level);

                // Verify the requested path and full result before timing.
                parser.feed(&frame);
                let parsed = parser.next_parsed().unwrap().unwrap();
                assert_eq!(parsed.packet_type(), packet.packet_type());
                match (&parsed, level) {
                    (ParsedPacket::Full(decoded), ParseLevel::Full) => assert_eq!(decoded, &packet),
                    (ParsedPacket::HeadersParsed(decoded), ParseLevel::HeadersParsed) => {
                        assert_eq!(decoded.raw_payload.as_ref(), vec![0x5a; size]);
                    }
                    (ParsedPacket::RawBody(_), ParseLevel::RawBody)
                    | (ParsedPacket::TypeOnly(_), ParseLevel::TypeOnly) => {}
                    _ => panic!("wrong parser level"),
                }
                drop(parsed);
                assert!(parser.buffer_mut().is_empty());
                // Exercise repeated reuse, including fragmented input and batches.
                for _ in 0..2 {
                    assert_eq!(parse_stream(&mut parser, &bytes, chunk_size), packet_count);
                    assert!(parser.buffer_mut().is_empty());
                }

                group.throughput(Throughput::Bytes(bytes.len() as u64));
                group.bench_with_input(BenchmarkId::new(name, size), &bytes, |b, bytes| {
                    // Reuse the drained parser. Buffer recycling and packet drops are timed.
                    b.iter(|| black_box(parse_stream(&mut parser, black_box(bytes), chunk_size)));
                });
            }
            group.finish();
        }
    }
}

criterion_group!(benches, parser);
criterion_main!(benches);
