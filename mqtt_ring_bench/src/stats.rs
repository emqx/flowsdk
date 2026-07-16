// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct BenchStats {
    pub messages_sent: AtomicU64,
    pub messages_acked: AtomicU64,
    pub errors: AtomicU64,
    pub socket_errors: AtomicU64,
    pub connect_errors: AtomicU64,
    pub send_errors: AtomicU64,
    pub receive_errors: AtomicU64,
    pub mqtt_errors: AtomicU64,
    pub clients_connected: AtomicU64,
    pub clients_done: AtomicU64,
    pub start_time: Instant,
    pub stopped: AtomicBool,
}

#[derive(Clone, Copy)]
pub enum ErrorKind {
    Socket,
    Connect,
    Send,
    Receive,
    Mqtt,
}

impl BenchStats {
    pub fn new() -> Self {
        Self {
            messages_sent: AtomicU64::new(0),
            messages_acked: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            socket_errors: AtomicU64::new(0),
            connect_errors: AtomicU64::new(0),
            send_errors: AtomicU64::new(0),
            receive_errors: AtomicU64::new(0),
            mqtt_errors: AtomicU64::new(0),
            clients_connected: AtomicU64::new(0),
            clients_done: AtomicU64::new(0),
            start_time: Instant::now(),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn record_error(&self, kind: ErrorKind) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        match kind {
            ErrorKind::Socket => self.socket_errors.fetch_add(1, Ordering::Relaxed),
            ErrorKind::Connect => self.connect_errors.fetch_add(1, Ordering::Relaxed),
            ErrorKind::Send => self.send_errors.fetch_add(1, Ordering::Relaxed),
            ErrorKind::Receive => self.receive_errors.fetch_add(1, Ordering::Relaxed),
            ErrorKind::Mqtt => self.mqtt_errors.fetch_add(1, Ordering::Relaxed),
        };
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            sent: self.messages_sent.load(Ordering::Relaxed),
            acked: self.messages_acked.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            socket_errors: self.socket_errors.load(Ordering::Relaxed),
            connect_errors: self.connect_errors.load(Ordering::Relaxed),
            send_errors: self.send_errors.load(Ordering::Relaxed),
            receive_errors: self.receive_errors.load(Ordering::Relaxed),
            mqtt_errors: self.mqtt_errors.load(Ordering::Relaxed),
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
    pub socket_errors: u64,
    pub connect_errors: u64,
    pub send_errors: u64,
    pub receive_errors: u64,
    pub mqtt_errors: u64,
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
    println!("    Socket:        {}", snap.socket_errors);
    println!("    Connect:       {}", snap.connect_errors);
    println!("    Send:          {}", snap.send_errors);
    println!("    Receive:       {}", snap.receive_errors);
    println!("    MQTT:          {}", snap.mqtt_errors);
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

#[cfg(test)]
mod tests {
    use super::{BenchStats, ErrorKind};

    #[test]
    fn records_error_breakdown() {
        let stats = BenchStats::new();
        for kind in [
            ErrorKind::Socket,
            ErrorKind::Connect,
            ErrorKind::Send,
            ErrorKind::Receive,
            ErrorKind::Mqtt,
        ] {
            stats.record_error(kind);
        }

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.errors, 5);
        assert_eq!(snapshot.socket_errors, 1);
        assert_eq!(snapshot.connect_errors, 1);
        assert_eq!(snapshot.send_errors, 1);
        assert_eq!(snapshot.receive_errors, 1);
        assert_eq!(snapshot.mqtt_errors, 1);
    }
}
