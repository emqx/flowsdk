// SPDX-License-Identifier: Apache-2.0

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use flowsdk::mqtt_client::{MqttClientOptions, MqttEngine, MqttEvent, PublishCommand};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

// This benchmark is single-threaded. Count requested heap bytes, not allocator
// size classes or RSS. The counters also add overhead to the timed measurements.
struct CountingAllocator;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);

fn allocated(size: usize) {
    ALLOCS.fetch_add(1, Relaxed);
    ALLOCATED.fetch_add(size, Relaxed);
    LIVE.fetch_add(size, Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, size);
        if !new_ptr.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            allocated(size);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy)]
enum Mode {
    Vec,
    Unique,
    Shared,
    Slice,
    Static,
}

const MODES: [Mode; 5] = [
    Mode::Vec,
    Mode::Unique,
    Mode::Shared,
    Mode::Slice,
    Mode::Static,
];
const SIZE: usize = 4096;
static STATIC_PAYLOAD: [u8; SIZE] = [42; SIZE];

enum Input {
    Vec(Vec<u8>),
    Bytes {
        payload: Bytes,
        application: Option<Bytes>,
    },
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Vec => "vec",
            Self::Unique => "unique",
            Self::Shared => "shared",
            Self::Slice => "shared_slice_64k",
            Self::Static => "static",
        }
    }

    fn input(self) -> Input {
        match self {
            Self::Vec => Input::Vec(vec![42; SIZE]),
            Self::Unique => Input::Bytes {
                payload: Bytes::from(vec![42; SIZE]),
                application: None,
            },
            Self::Shared => {
                let application = Bytes::from(vec![42; SIZE]);
                Input::Bytes {
                    payload: application.clone(),
                    application: Some(application),
                }
            }
            Self::Slice => {
                let application = Bytes::from(vec![42; 16 * SIZE]);
                Input::Bytes {
                    payload: application.slice(17..17 + SIZE),
                    application: Some(application),
                }
            }
            Self::Static => Input::Bytes {
                payload: Bytes::from_static(&STATIC_PAYLOAD),
                application: None,
            },
        }
    }
}

#[derive(Clone, Copy)]
enum Path {
    SharedBytes,
    VecConversion,
}

impl Path {
    fn name(self) -> &'static str {
        match self {
            Self::SharedBytes => "bytes",
            Self::VecConversion => "vec_conversion_baseline",
        }
    }
}

// The baseline explicitly reintroduces the removed Bytes -> Vec conversion,
// then uses today's command/packet types. It is not a whole 0.7 SDK benchmark.
fn command(input: Input, path: Path) -> (PublishCommand, Option<Bytes>, usize) {
    let (mut command, application, original_ptr) = match input {
        Input::Vec(payload) => {
            let ptr = payload.as_ptr();
            (
                PublishCommand::simple("telemetry", payload, 1, false),
                None,
                ptr,
            )
        }
        Input::Bytes {
            payload,
            application,
        } => {
            let ptr = payload.as_ptr();
            let command = match path {
                Path::SharedBytes => PublishCommand::from_bytes("telemetry", payload, 1, false),
                Path::VecConversion => {
                    PublishCommand::simple("telemetry", Vec::from(payload), 1, false)
                }
            };
            (command, application, ptr)
        }
    };
    command.packet_id = Some(1);
    // These paths either retain the input or copy exactly the visible payload.
    // Pointer identity detects the replacement, not wire/transport copies.
    let copied = usize::from(command.payload.as_ptr() != original_ptr) * SIZE;
    (command, application, copied)
}

fn allocation_report() {
    println!("mode,path,payload_bytes,prepare_allocations,prepare_allocated_bytes,replacement_payload_copy_bytes,retained_heap_with_application,retained_heap_sdk_only");
    for mode in MODES {
        for path in [Path::SharedBytes, Path::VecConversion] {
            let start_live = LIVE.load(Relaxed);
            let input = mode.input();
            let start_allocs = ALLOCS.load(Relaxed);
            let start_bytes = ALLOCATED.load(Relaxed);
            let (command, application, copied) = command(input, path);
            let packet = command.into_mqtt_publish();
            let allocations = ALLOCS.load(Relaxed) - start_allocs;
            let allocated_bytes = ALLOCATED.load(Relaxed) - start_bytes;
            let retained = LIVE.load(Relaxed) - start_live;
            drop(application);
            let sdk_only = LIVE.load(Relaxed) - start_live;
            assert_eq!(packet.payload.as_ref(), &STATIC_PAYLOAD);
            if matches!(path, Path::SharedBytes) {
                assert_eq!(copied, 0);
            }
            drop(packet);
            assert_eq!(LIVE.load(Relaxed), start_live);
            println!(
                "{},{},{SIZE},{allocations},{allocated_bytes},{copied},{retained},{sdk_only}",
                mode.name(),
                path.name()
            );
        }
    }
}

fn connected() -> MqttEngine {
    let mut engine = MqttEngine::new(MqttClientOptions::builder().keep_alive(0).build());
    engine.connect().unwrap();
    engine.take_outgoing();
    engine.handle_incoming(&[0x20, 3, 0, 0, 0]);
    engine
}

fn shared_payload(c: &mut Criterion) {
    allocation_report();
    let mut prepare = c.benchmark_group("shared_payload/prepare");
    for mode in MODES {
        for path in [Path::SharedBytes, Path::VecConversion] {
            prepare.bench_function(BenchmarkId::new(mode.name(), path.name()), |b| {
                b.iter_batched(
                    || mode.input(),
                    |input| {
                        let (command, application, copied) = command(black_box(input), path);
                        black_box((command.into_mqtt_publish(), application, copied))
                    },
                    BatchSize::LargeInput,
                );
            });
        }
    }
    prepare.finish();
    let mut exchange = c.benchmark_group("shared_payload/v5_qos1_encode_ack");
    for mode in MODES {
        for path in [Path::SharedBytes, Path::VecConversion] {
            exchange.bench_function(BenchmarkId::new(mode.name(), path.name()), |b| {
                b.iter_batched_ref(
                    || (connected(), Some(mode.input())),
                    |(engine, input)| {
                        let (command, application, _) = command(input.take().unwrap(), path);
                        assert_eq!(engine.publish(black_box(command)).unwrap(), Some(1));
                        black_box(engine.take_outgoing());
                        let events = engine.handle_incoming(&[0x40, 2, 0, 1]);
                        assert!(matches!(events.as_slice(), [MqttEvent::Published(_)]));
                        black_box(application);
                    },
                    BatchSize::LargeInput,
                );
            });
        }
    }
    exchange.finish();
}

criterion_group!(benches, shared_payload);
criterion_main!(benches);
