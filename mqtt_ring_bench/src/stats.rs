// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct BenchStats {
    pub messages_sent: AtomicU64,
    pub messages_received: AtomicU64,
    pub messages_acked: AtomicU64,
    pub puback_no_match: AtomicU64,
    pub errors: AtomicU64,
    pub socket_errors: AtomicU64,
    pub connect_errors: AtomicU64,
    pub send_errors: AtomicU64,
    pub receive_errors: AtomicU64,
    pub mqtt_errors: AtomicU64,
    pub mqtt_connect_errors: AtomicU64,
    pub mqtt_subscribe_errors: AtomicU64,
    pub mqtt_publish_errors: AtomicU64,
    pub mqtt_client_errors: AtomicU64,
    pub mqtt_disconnect_errors: AtomicU64,
    pub clients_connected: AtomicU64,
    pub clients_subscribed: AtomicU64,
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
    MqttConnect,
    MqttSubscribe,
    MqttPublish,
    MqttClient,
    MqttDisconnect,
}

impl BenchStats {
    pub fn new() -> Self {
        Self {
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            messages_acked: AtomicU64::new(0),
            puback_no_match: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            socket_errors: AtomicU64::new(0),
            connect_errors: AtomicU64::new(0),
            send_errors: AtomicU64::new(0),
            receive_errors: AtomicU64::new(0),
            mqtt_errors: AtomicU64::new(0),
            mqtt_connect_errors: AtomicU64::new(0),
            mqtt_subscribe_errors: AtomicU64::new(0),
            mqtt_publish_errors: AtomicU64::new(0),
            mqtt_client_errors: AtomicU64::new(0),
            mqtt_disconnect_errors: AtomicU64::new(0),
            clients_connected: AtomicU64::new(0),
            clients_subscribed: AtomicU64::new(0),
            clients_done: AtomicU64::new(0),
            start_time: Instant::now(),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn record_error(&self, kind: ErrorKind) {
        self.record_errors(kind, 1);
    }

    pub fn record_puback_no_match(&self, count: u64) {
        self.puback_no_match.fetch_add(count, Ordering::Relaxed);
    }

    pub fn record_errors(&self, kind: ErrorKind, count: u64) {
        if count == 0 {
            return;
        }
        self.errors.fetch_add(count, Ordering::Relaxed);
        match kind {
            ErrorKind::Socket => {
                self.socket_errors.fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::Connect => {
                self.connect_errors.fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::Send => {
                self.send_errors.fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::Receive => {
                self.receive_errors.fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::MqttConnect => {
                self.mqtt_errors.fetch_add(count, Ordering::Relaxed);
                self.mqtt_connect_errors.fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::MqttSubscribe => {
                self.mqtt_errors.fetch_add(count, Ordering::Relaxed);
                self.mqtt_subscribe_errors
                    .fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::MqttPublish => {
                self.mqtt_errors.fetch_add(count, Ordering::Relaxed);
                self.mqtt_publish_errors.fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::MqttClient => {
                self.mqtt_errors.fetch_add(count, Ordering::Relaxed);
                self.mqtt_client_errors.fetch_add(count, Ordering::Relaxed);
            }
            ErrorKind::MqttDisconnect => {
                self.mqtt_errors.fetch_add(count, Ordering::Relaxed);
                self.mqtt_disconnect_errors
                    .fetch_add(count, Ordering::Relaxed);
            }
        };
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            sent: self.messages_sent.load(Ordering::Relaxed),
            received: self.messages_received.load(Ordering::Relaxed),
            acked: self.messages_acked.load(Ordering::Relaxed),
            puback_no_match: self.puback_no_match.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            socket_errors: self.socket_errors.load(Ordering::Relaxed),
            connect_errors: self.connect_errors.load(Ordering::Relaxed),
            send_errors: self.send_errors.load(Ordering::Relaxed),
            receive_errors: self.receive_errors.load(Ordering::Relaxed),
            mqtt_errors: self.mqtt_errors.load(Ordering::Relaxed),
            mqtt_connect_errors: self.mqtt_connect_errors.load(Ordering::Relaxed),
            mqtt_subscribe_errors: self.mqtt_subscribe_errors.load(Ordering::Relaxed),
            mqtt_publish_errors: self.mqtt_publish_errors.load(Ordering::Relaxed),
            mqtt_client_errors: self.mqtt_client_errors.load(Ordering::Relaxed),
            mqtt_disconnect_errors: self.mqtt_disconnect_errors.load(Ordering::Relaxed),
            connected: self.clients_connected.load(Ordering::Relaxed),
            subscribed: self.clients_subscribed.load(Ordering::Relaxed),
            done: self.clients_done.load(Ordering::Relaxed),
            elapsed: self.start_time.elapsed(),
        }
    }
}

pub struct StatsSnapshot {
    pub sent: u64,
    pub received: u64,
    pub acked: u64,
    pub puback_no_match: u64,
    pub errors: u64,
    pub socket_errors: u64,
    pub connect_errors: u64,
    pub send_errors: u64,
    pub receive_errors: u64,
    pub mqtt_errors: u64,
    pub mqtt_connect_errors: u64,
    pub mqtt_subscribe_errors: u64,
    pub mqtt_publish_errors: u64,
    pub mqtt_client_errors: u64,
    pub mqtt_disconnect_errors: u64,
    pub connected: u64,
    pub subscribed: u64,
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
    let messages = match config.action {
        crate::config::BenchAction::Pub => snap.sent,
        crate::config::BenchAction::Sub => snap.received,
    };
    let throughput = if duration_secs > 0.0 {
        messages as f64 / duration_secs
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
    println!("  Action:          {}", config.action.as_str());
    println!("  Clients:         {}", config.clients);
    println!("  Workers:         {}", config.workers);
    println!("  QoS:             {}", config.qos);
    match config.action {
        crate::config::BenchAction::Pub => {
            println!("  Payload Size:    {} bytes", config.payload_size);
            println!("  Messages/Client: {}", config.messages);
        }
        crate::config::BenchAction::Sub => {
            let target = if config.messages == 0 {
                "unbounded".to_string()
            } else {
                config.messages.to_string()
            };
            println!("  Receive Target:  {} per client", target);
            println!("  Subscribed:      {}", snap.subscribed);
        }
    }
    println!("  Socket Buffers:  {} bytes", config.socket_buf);
    println!("  Parser Buffer:   {} bytes", config.parser_buf);
    println!("{dash}");
    match config.action {
        crate::config::BenchAction::Pub => {
            println!("  Total Sent:      {}", snap.sent);
            if config.qos > 0 {
                println!("  Total Acked:     {}", snap.acked);
                println!("  PubAckNoMatch:   {}", snap.puback_no_match);
            }
        }
        crate::config::BenchAction::Sub => {
            println!("  Total Received:  {}", snap.received);
        }
    }
    println!("  Errors:          {}", snap.errors);
    println!("    Socket:        {}", snap.socket_errors);
    println!("    Connect:       {}", snap.connect_errors);
    println!("    Send:          {}", snap.send_errors);
    println!("    Receive:       {}", snap.receive_errors);
    println!("    MQTT:          {}", snap.mqtt_errors);
    println!("      CONNECT:     {}", snap.mqtt_connect_errors);
    println!("      SUBSCRIBE:   {}", snap.mqtt_subscribe_errors);
    println!("      PUBLISH:     {}", snap.mqtt_publish_errors);
    println!("      Client:      {}", snap.mqtt_client_errors);
    println!("      Disconnect:  {}", snap.mqtt_disconnect_errors);
    println!("  Duration:        {:.2} s", duration_secs);
    println!("  Throughput:      {:.2} msg/s", throughput);

    if config.action == crate::config::BenchAction::Pub && !latency_samples.is_empty() {
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
            ErrorKind::MqttConnect,
            ErrorKind::MqttSubscribe,
            ErrorKind::MqttPublish,
            ErrorKind::MqttClient,
            ErrorKind::MqttDisconnect,
        ] {
            stats.record_error(kind);
        }
        stats.record_puback_no_match(2);

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.errors, 9);
        assert_eq!(snapshot.puback_no_match, 2);
        assert_eq!(snapshot.socket_errors, 1);
        assert_eq!(snapshot.connect_errors, 1);
        assert_eq!(snapshot.send_errors, 1);
        assert_eq!(snapshot.receive_errors, 1);
        assert_eq!(snapshot.mqtt_errors, 5);
        assert_eq!(snapshot.mqtt_connect_errors, 1);
        assert_eq!(snapshot.mqtt_subscribe_errors, 1);
        assert_eq!(snapshot.mqtt_publish_errors, 1);
        assert_eq!(snapshot.mqtt_client_errors, 1);
        assert_eq!(snapshot.mqtt_disconnect_errors, 1);
    }
}
