// SPDX-License-Identifier: MPL-2.0

mod common;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use flowsdk::mqtt_client::inflight::InflightQueue;
use flowsdk::priority_queue::PriorityQueue;
use std::hint::black_box;
use std::time::Duration;

const DEPTHS: [u16; 3] = [1, 32, 1024];
const RETRANSMISSION_TIMEOUT: Duration = Duration::from_secs(5);

fn inflight(version: u8, depth: u16) -> InflightQueue {
    let mut queue = InflightQueue::new(depth + 1, version, RETRANSMISSION_TIMEOUT);
    for id in 1..=depth {
        queue
            .push(id, common::publish(version, 1, 256, id), 1)
            .unwrap();
    }
    queue
}

fn inflight_benches(c: &mut Criterion) {
    for (version, label) in common::VERSIONS {
        let mut group = c.benchmark_group(format!("inflight/{label}"));
        for depth in DEPTHS {
            common::encode_checked(&common::publish(version, 1, 256, depth), version);
            let mut queue = inflight(version, depth);
            assert_eq!(queue.len(), usize::from(depth));
            assert_eq!(queue.get(depth).unwrap().packet_id, depth);
            assert_eq!(queue.acknowledge(depth).unwrap().packet_id, depth);
            assert!(!queue.contains(depth));
            queue
                .push(depth, common::publish(version, 1, 256, depth), 1)
                .unwrap();
            assert_eq!(queue.len(), usize::from(depth));

            group.throughput(Throughput::Elements(1));
            group.bench_with_input(BenchmarkId::new("lookup", depth), &depth, |b, &depth| {
                let queue = inflight(version, depth);
                b.iter(|| black_box(queue.get(black_box(depth)).unwrap()));
            });
            group.bench_with_input(BenchmarkId::new("insert", depth), &depth, |b, &depth| {
                b.iter_batched_ref(
                    || {
                        (
                            inflight(version, depth),
                            Some(common::publish(version, 1, 256, depth + 1)),
                        )
                    },
                    |(queue, packet)| {
                        queue
                            .push(black_box(depth + 1), black_box(packet.take().unwrap()), 1)
                            .unwrap();
                        black_box(queue.len())
                    },
                    BatchSize::LargeInput,
                );
            });
            group.bench_with_input(
                BenchmarkId::new("acknowledge", depth),
                &depth,
                |b, &depth| {
                    b.iter_batched_ref(
                        || inflight(version, depth),
                        |queue| black_box(queue.acknowledge(black_box(depth)).unwrap()),
                        BatchSize::LargeInput,
                    );
                },
            );

            // MQTT 5 has no active-connection retransmission; label that separately.
            let expiry_cases: &[(&str, bool)] = if version == 4 {
                &[("expire_none", false), ("expire_all", true)]
            } else {
                &[("expiry_disabled", true)]
            };
            for &(name, due) in expiry_cases {
                let setup = || {
                    let queue = inflight(version, depth);
                    let now = if due {
                        queue.get(depth).unwrap().sent_at + RETRANSMISSION_TIMEOUT
                    } else {
                        queue.get(1).unwrap().sent_at
                    };
                    (queue, now)
                };
                let expected = if due && version == 4 {
                    usize::from(depth)
                } else {
                    0
                };
                let (mut queue, now) = setup();
                assert_eq!(queue.get_expired(now).len(), expected);
                assert!(queue.get_expired(now).is_empty());
                group.throughput(Throughput::Elements(expected.max(1) as u64));
                group.bench_with_input(BenchmarkId::new(name, depth), &depth, |b, _| {
                    b.iter_batched_ref(
                        setup,
                        |(queue, now)| black_box(queue.get_expired(black_box(*now))),
                        BatchSize::LargeInput,
                    );
                });
            }
        }
        group.finish();
    }
}

fn priority_queue(depth: u16, capacity: usize) -> PriorityQueue<u8, u16> {
    let mut queue = PriorityQueue::new(capacity);
    for item in 0..depth {
        queue.enqueue((item % 8) as u8, item);
    }
    queue
}

fn priority_benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("priority_queue");
    group.throughput(Throughput::Elements(1));
    for depth in DEPTHS {
        let mut queue = priority_queue(depth, usize::from(depth) + 1);
        assert_eq!(queue.len(), usize::from(depth));
        let expected = queue.peek().map(|(&priority, &item)| (priority, item));
        assert_eq!(queue.dequeue(), expected);
        for (name, capacity) in [
            ("enqueue", usize::from(depth) + 1),
            ("enqueue_evict", usize::from(depth)),
        ] {
            let mut queue = priority_queue(depth, capacity);
            queue.enqueue(7, depth);
            assert_eq!(queue.len(), (usize::from(depth) + 1).min(capacity));
            group.bench_with_input(BenchmarkId::new(name, depth), &depth, |b, &depth| {
                b.iter_batched_ref(
                    || priority_queue(depth, capacity),
                    |queue| {
                        queue.enqueue(black_box(7), black_box(depth));
                        black_box(queue.len())
                    },
                    BatchSize::LargeInput,
                );
            });
        }
        group.bench_with_input(BenchmarkId::new("dequeue", depth), &depth, |b, &depth| {
            b.iter_batched_ref(
                || priority_queue(depth, usize::from(depth)),
                |queue| black_box(queue.dequeue().unwrap()),
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, inflight_benches, priority_benches);
criterion_main!(benches);
