// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct BenchStats {
    pub messages_sent: AtomicU64,
    pub messages_acked: AtomicU64,
    pub errors: AtomicU64,
    pub clients_connected: AtomicU64,
    pub clients_done: AtomicU64,
    pub start_time: Instant,
    pub stopped: AtomicBool,
}

impl BenchStats {
    pub fn new() -> Self {
        Self {
            messages_sent: AtomicU64::new(0),
            messages_acked: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            clients_connected: AtomicU64::new(0),
            clients_done: AtomicU64::new(0),
            start_time: Instant::now(),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            sent: self.messages_sent.load(Ordering::Relaxed),
            acked: self.messages_acked.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            connected: self.clients_connected.load(Ordering::Relaxed),
            done: self.clients_done.load(Ordering::Relaxed),
            elapsed: self.start_time.elapsed(),
        }
    }
}

pub struct StatsSnapshot {
    pub sent: u64,
    pub acked: u64,
    pub errors: u64,
    pub connected: u64,
    pub done: u64,
    pub elapsed: Duration,
}

pub fn merge_latency_samples(worker_samples: Vec<Vec<Duration>>) -> Vec<Duration> {
    let total: usize = worker_samples.iter().map(|v| v.len()).sum();
    let mut merged = Vec::with_capacity(total);
    for mut samples in worker_samples {
        merged.append(&mut samples);
    }
    merged.sort_unstable();
    merged
}

pub fn percentile(sorted: &[Duration], pct: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * pct / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn print_final_summary(
    config: &crate::config::BenchConfig,
    snap: &StatsSnapshot,
    latency_samples: &[Duration],
) {
    let duration_secs = snap.elapsed.as_secs_f64();
    let throughput = if duration_secs > 0.0 {
        snap.sent as f64 / duration_secs
    } else {
        0.0
    };

    let sep = "=".repeat(60);
    let dash = "-".repeat(60);
    println!("\n{sep}");
    println!("{:^60}", "MQTT Bench Results");
    println!("{sep}");
    let transport = if config.quic { "QUIC" } else { "TCP" };
    println!(
        "  Broker:          {}:{} ({})",
        config.host, config.port, transport
    );
    println!("  MQTT Version:    {}", config.mqtt_version);
    println!("  Clients:         {}", config.clients);
    println!("  Workers:         {}", config.workers);
    println!("  QoS:             {}", config.qos);
    println!("  Payload Size:    {} bytes", config.payload_size);
    println!("  Messages/Client: {}", config.messages);
    println!("  Socket Buffers:  {} bytes", config.socket_buf);
    println!("  Parser Buffer:   {} bytes", config.parser_buf);
    println!("{dash}");
    println!("  Total Sent:      {}", snap.sent);
    if config.qos > 0 {
        println!("  Total Acked:     {}", snap.acked);
    }
    println!("  Errors:          {}", snap.errors);
    println!("  Duration:        {:.2} s", duration_secs);
    println!("  Throughput:      {:.2} msg/s", throughput);

    if !latency_samples.is_empty() {
        println!("{dash}");
        let label = if config.qos == 0 {
            "Latency (enqueue time)"
        } else {
            "Latency (send -> ACK)"
        };
        println!("  {}:", label);
        let avg: Duration = latency_samples.iter().sum::<Duration>() / latency_samples.len() as u32;
        println!("    Min:           {:.2} ms", ms(latency_samples[0]));
        println!(
            "    Max:           {:.2} ms",
            ms(*latency_samples.last().unwrap())
        );
        println!("    Avg:           {:.2} ms", ms(avg));
        println!(
            "    P50:           {:.2} ms",
            ms(percentile(latency_samples, 50.0))
        );
        println!(
            "    P95:           {:.2} ms",
            ms(percentile(latency_samples, 95.0))
        );
        println!(
            "    P99:           {:.2} ms",
            ms(percentile(latency_samples, 99.0))
        );
        println!("    Samples:       {}", latency_samples.len());
    }
    println!("{sep}");
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
