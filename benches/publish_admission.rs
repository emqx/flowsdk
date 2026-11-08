// SPDX-License-Identifier: Apache-2.0

//! Admission latency with separately verified output. This uses a loopback
//! broker stub, not an external broker or a VeloFlux pipeline.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use flowsdk::mqtt_client::{
    MqttClientError, MqttClientOptions, MqttEngine, MqttEvent, PublishAdmissionConfig,
    PublishCommand, TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_serde::{control_packet::MqttPacket, parser::ParseOk};
use futures::{stream::FuturesUnordered, StreamExt};
use std::hint::black_box;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const TOPIC: &str = "bench/admission";
const DEADLINE: Duration = Duration::from_secs(5);
const MAX_MESSAGES: usize = 32;

#[derive(Clone, Copy)]
enum Mode {
    LegacySequential,
    Sequential,
    Concurrent,
    Batch,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::LegacySequential => "legacy_sequential_single",
            Self::Sequential => "sequential_single",
            Self::Concurrent => "concurrent_single",
            Self::Batch => "batch",
        }
    }
}

struct Handler;

#[async_trait::async_trait]
impl TokioMqttEventHandler for Handler {}

fn options(version: u8) -> MqttClientOptions {
    MqttClientOptions::builder()
        .mqtt_version(version)
        .client_id("admission-benchmark")
        .keep_alive(0)
        .reconnect(false)
        .max_outgoing_packet_count(4 * MAX_MESSAGES)
        .max_outgoing_buffer_bytes(4 * 1024 * 1024)
        .build()
}

fn commands(first: u64, count: usize, payload_size: usize) -> Vec<PublishCommand> {
    (0..count)
        .map(|offset| {
            let mut payload = vec![0x5a; payload_size];
            payload[..8].copy_from_slice(&(first + offset as u64).to_be_bytes());
            PublishCommand::simple(TOPIC, payload, 0, false)
        })
        .collect()
}

fn connack(version: u8) -> &'static [u8] {
    if version == 5 {
        &[0x20, 3, 0, 0, 0]
    } else {
        &[0x20, 2, 0, 0]
    }
}

/// Verify an iteration's exact sequence-number set, allowing concurrent callers
/// to enter the admission queue in any order. Duplicates and omissions fail.
struct Verification {
    first: u64,
    seen: Vec<bool>,
    remaining: usize,
    payload_size: usize,
}

impl Verification {
    fn new(count: usize, payload_size: usize) -> Self {
        Self {
            first: 0,
            seen: vec![false; count],
            remaining: count,
            payload_size,
        }
    }

    fn publish(&mut self, packet: MqttPacket) -> Option<u64> {
        let (topic, payload, qos, retain, dup) = match packet {
            MqttPacket::Publish3(p) => (p.topic_name, p.payload, p.qos, p.retain, p.dup),
            MqttPacket::Publish5(p) => (p.topic_name, p.payload, p.qos, p.retain, p.dup),
            other => panic!("expected PUBLISH, got {other:?}"),
        };
        assert_eq!(topic, TOPIC);
        assert_eq!((qos, retain, dup), (0, false, false));
        assert_eq!(payload.len(), self.payload_size);
        assert!(payload[8..].iter().all(|byte| *byte == 0x5a));
        let sequence = u64::from_be_bytes(payload[..8].try_into().unwrap());
        let offset = sequence.checked_sub(self.first).expect("old publication");
        assert!(
            offset < self.seen.len() as u64,
            "publication outside current iteration"
        );
        let seen = &mut self.seen[offset as usize];
        assert!(!*seen, "duplicate publication {sequence}");
        *seen = true;
        self.remaining -= 1;
        if self.remaining != 0 {
            return None;
        }
        self.first += self.seen.len() as u64;
        self.remaining = self.seen.len();
        self.seen.fill(false);
        Some(self.first)
    }

    fn complete(&self) -> bool {
        self.remaining == self.seen.len()
    }
}

async fn read_packet(reader: &mut BufReader<TcpStream>, version: u8) -> MqttPacket {
    let mut frame = vec![reader.read_u8().await.unwrap()];
    let mut remaining = 0usize;
    for shift in [0, 7, 14, 21] {
        let byte = reader.read_u8().await.unwrap();
        frame.push(byte);
        remaining |= usize::from(byte & 127) << shift;
        if byte & 128 == 0 {
            let offset = frame.len();
            frame.resize(offset + remaining, 0);
            reader.read_exact(&mut frame[offset..]).await.unwrap();
            return match MqttPacket::from_bytes_with_version(&frame, version).unwrap() {
                ParseOk::Packet(packet, consumed) if consumed == frame.len() => packet,
                other => panic!("invalid complete frame: {other:?}"),
            };
        }
    }
    panic!("invalid MQTT remaining length");
}

async fn broker(
    listener: TcpListener,
    version: u8,
    count: usize,
    payload_size: usize,
    verified: mpsc::Sender<u64>,
) -> u64 {
    let (stream, _) = listener.accept().await.unwrap();
    stream.set_nodelay(true).unwrap();
    let mut reader = BufReader::new(stream);
    let connect = read_packet(&mut reader, version).await;
    assert!(matches!(
        connect,
        MqttPacket::Connect3(_) | MqttPacket::Connect5(_)
    ));
    reader.get_mut().write_all(connack(version)).await.unwrap();
    let mut verification = Verification::new(count, payload_size);
    loop {
        let packet = read_packet(&mut reader, version).await;
        if matches!(
            packet,
            MqttPacket::Disconnect3(_) | MqttPacket::Disconnect5(_)
        ) {
            assert!(
                verification.complete(),
                "shutdown with incomplete publications"
            );
            return verification.first;
        }
        if let Some(total) = verification.publish(packet) {
            verified.send(total).await.unwrap();
        }
    }
}

struct Loopback {
    client: TokioAsyncMqttClient,
    verified: mpsc::Receiver<u64>,
    broker: JoinHandle<u64>,
    sent: u64,
}

impl Loopback {
    async fn open(version: u8, count: usize, payload_size: usize, mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut options = options(version);
        options.peer = format!("mqtt://{}", listener.local_addr().unwrap());
        let (verified_tx, verified) = mpsc::channel(1);
        let broker = tokio::spawn(broker(listener, version, count, payload_size, verified_tx));
        let mut config = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .buffer_messages(false)
            .priority_queue_enabled(false)
            .command_queue_size(4 * MAX_MESSAGES);
        if !matches!(mode, Mode::LegacySequential) {
            config = config.publish_admission(PublishAdmissionConfig {
                max_batch_messages: MAX_MESSAGES,
                max_batch_bytes: 256 * 1024,
                max_pending_messages: 4 * MAX_MESSAGES,
                max_pending_bytes: 4 * 1024 * 1024,
            });
        }
        let config = config.build();
        let client = TokioAsyncMqttClient::new(options, Box::new(Handler), config)
            .await
            .unwrap();
        assert!(client.connect_sync().await.unwrap().is_success());
        Self {
            client,
            verified,
            broker,
            sent: 0,
        }
    }

    async fn submit(
        &self,
        mode: Mode,
        commands: Vec<PublishCommand>,
    ) -> Vec<Result<Option<u16>, MqttClientError>> {
        match mode {
            Mode::LegacySequential | Mode::Sequential => {
                let mut results = Vec::with_capacity(commands.len());
                for command in commands {
                    results.push(self.client.publish_with_command_admitted(command).await);
                }
                results
            }
            Mode::Concurrent => {
                let mut pending: FuturesUnordered<_> = commands
                    .into_iter()
                    .map(|command| self.client.publish_with_command_admitted(command))
                    .collect();
                let mut results = Vec::with_capacity(pending.len());
                while let Some(result) = pending.next().await {
                    results.push(result);
                }
                results
            }
            Mode::Batch => {
                self.client
                    .publish_batch_admitted(commands)
                    .await
                    .unwrap()
                    .results
            }
        }
    }

    async fn measure(
        &mut self,
        mode: Mode,
        iterations: u64,
        count: usize,
        size: usize,
    ) -> Duration {
        let mut elapsed = Duration::ZERO;
        for _ in 0..iterations {
            // Allocation/preparation and output verification are outside timing.
            let commands = commands(self.sent, count, size);
            let start = Instant::now();
            let results = timeout(DEADLINE, self.submit(mode, commands))
                .await
                .unwrap();
            elapsed += start.elapsed();
            assert_eq!(results.len(), count);
            for result in results {
                assert_eq!(result.unwrap(), None);
            }
            self.sent += count as u64;
            let received = timeout(DEADLINE, self.verified.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, self.sent);
        }
        elapsed
    }

    async fn close(self) {
        timeout(DEADLINE, self.client.shutdown())
            .await
            .unwrap()
            .unwrap();
        let received = timeout(DEADLINE, self.broker).await.unwrap().unwrap();
        assert_eq!(received, self.sent);
    }
}

fn connected_engine(version: u8) -> MqttEngine {
    let mut engine = MqttEngine::new(options(version));
    engine.connect().unwrap();
    assert!(!engine.take_outgoing().is_empty());
    let events = engine.handle_incoming(connack(version));
    assert!(matches!(events.as_slice(), [MqttEvent::Connected(result)] if result.is_success()));
    assert!(engine.is_connected());
    engine
}

fn verify_output(engine: &mut MqttEngine, version: u8, verification: &mut Verification) {
    let bytes = engine.take_outgoing();
    let mut offset = 0;
    let mut completed = false;
    while offset < bytes.len() {
        match MqttPacket::from_bytes_with_version(&bytes[offset..], version).unwrap() {
            ParseOk::Packet(packet, consumed) => {
                assert!(consumed > 0);
                assert!(!completed, "too many publications");
                completed = verification.publish(packet).is_some();
                offset += consumed;
            }
            other => panic!("invalid engine output: {other:?}"),
        }
    }
    assert!(completed, "missing publications");
    assert!(engine.take_events().is_empty());
}

fn publish_admission(c: &mut Criterion) {
    // Keep the caller, worker, and verifier on one runtime thread. This is not a
    // multi-core scalability test; measured latency includes their scheduling.
    let runtime: Runtime = Builder::new_current_thread().enable_all().build().unwrap();
    for (version, label) in [(4, "v3_1_1"), (5, "v5")] {
        for size in [64, 4096] {
            let mut group = c.benchmark_group(format!("publish_admission/{label}/payload{size}"));
            for count in [1, 8, MAX_MESSAGES] {
                group.throughput(Throughput::Elements(count as u64));
                for mode in [
                    Mode::LegacySequential,
                    Mode::Sequential,
                    Mode::Concurrent,
                    Mode::Batch,
                ] {
                    group.bench_function(BenchmarkId::new(mode.name(), count), |b| {
                        let mut connection = runtime
                            .block_on(async {
                                timeout(DEADLINE, Loopback::open(version, count, size, mode)).await
                            })
                            .unwrap();
                        b.iter_custom(|iterations| {
                            runtime.block_on(connection.measure(mode, iterations, count, size))
                        });
                        runtime.block_on(connection.close());
                    });
                }
                group.bench_function(BenchmarkId::new("engine_direct", count), |b| {
                    let mut engine = connected_engine(version);
                    let mut verification = Verification::new(count, size);
                    b.iter_custom(|iterations| {
                        let mut elapsed = Duration::ZERO;
                        for _ in 0..iterations {
                            let commands = commands(verification.first, count, size);
                            let start = Instant::now();
                            for command in commands {
                                assert_eq!(black_box(engine.publish(command)).unwrap(), None);
                            }
                            elapsed += start.elapsed();
                            verify_output(&mut engine, version, &mut verification);
                        }
                        elapsed
                    });
                });
            }
            group.finish();
        }
    }
}

criterion_group!(benches, publish_admission);
criterion_main!(benches);
