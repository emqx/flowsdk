// SPDX-License-Identifier: MPL-2.0

use clap::{ArgAction, Parser};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

pub struct BenchConfig {
    pub addr: SocketAddr,
    pub host: String,
    pub port: u16,
    pub clients: usize,
    pub messages: u64,
    pub qos: u8,
    pub topic: String,
    pub payload_size: usize,
    pub interval_ms: u64,
    pub keep_alive: u16,
    pub mqtt_version: u8,
    pub workers: usize,
    pub connect_rate: usize,
    pub socket_buf: usize,
    pub parser_buf: usize,
    /// Source IP addresses to bind outgoing connections to (round-robin).
    /// Empty means OS picks the source address.
    pub ifaddrs: Vec<IpAddr>,
    /// Use MQTT over QUIC (UDP) instead of TCP.
    pub quic: bool,
    /// Skip TLS certificate verification for QUIC (testing only).
    pub quic_insecure: bool,
    /// TLS SNI server name for QUIC. Defaults to --host value.
    pub server_name: Option<String>,
}

#[derive(Debug, Parser)]
#[command(name = "mqtt_ring_bench", about = "High-connection-count MQTT publish benchmark (io_uring)")]
struct BenchArgs {
    #[arg(long, default_value = "localhost")]
    host: String,

    #[arg(long, default_value_t = 1883)]
    port: u16,

    #[arg(long, default_value_t = 1000)]
    clients: usize,

    #[arg(long, default_value_t = 1000)]
    messages: u64,

    #[arg(long, default_value_t = 0)]
    qos: u8,

    #[arg(long, default_value = "bench/test")]
    topic: String,

    #[arg(long = "payload-size", default_value_t = 256)]
    payload_size: usize,

    #[arg(long, default_value_t = 0)]
    interval: u64,

    #[arg(long = "keep-alive", default_value_t = 60)]
    keep_alive: u16,

    #[arg(long = "mqtt-version", default_value_t = 5)]
    mqtt_version: u8,

    #[arg(long, default_value_t = num_cpus())]
    workers: usize,

    #[arg(long = "connect-rate", default_value_t = 1000)]
    connect_rate: usize,

    #[arg(long = "socket-buf", default_value_t = 2048)]
    socket_buf: usize,

    #[arg(long = "parser-buf", default_value_t = 1500)]
    parser_buf: usize,

    /// Source IP addresses to bind outgoing connections to (round-robin).
    /// Supports single, comma-separated, or last-octet range syntax.
    #[arg(long = "ifaddr")]
    ifaddr: Option<String>,

    #[arg(long, action = ArgAction::SetTrue)]
    quic: bool,

    #[arg(long = "quic-insecure", action = ArgAction::SetTrue)]
    quic_insecure: bool,

    #[arg(long = "server-name")]
    server_name: Option<String>,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 1883)),
            host: "localhost".to_string(),
            port: 1883,
            clients: 1000,
            messages: 1000,
            qos: 0,
            topic: "bench/test".to_string(),
            payload_size: 256,
            interval_ms: 0,
            keep_alive: 60,
            mqtt_version: 5,
            workers: num_cpus(),
            connect_rate: 1000,
            socket_buf: 2048,
            parser_buf: 1500,
            ifaddrs: Vec::new(),
            quic: false,
            quic_insecure: false,
            server_name: None,
        }
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

pub fn parse_args() -> BenchConfig {
    let args = BenchArgs::parse();

    #[cfg(not(feature = "quic"))]
    if args.quic {
        eprintln!("Error: --quic requires the 'quic' feature. Rebuild with: --features quic");
        std::process::exit(1);
    }

    let mut config = BenchConfig {
        host: args.host,
        port: args.port,
        clients: args.clients,
        messages: args.messages,
        qos: args.qos,
        topic: args.topic,
        payload_size: args.payload_size,
        interval_ms: args.interval,
        keep_alive: args.keep_alive,
        mqtt_version: args.mqtt_version,
        workers: args.workers,
        connect_rate: args.connect_rate,
        socket_buf: args.socket_buf,
        parser_buf: args.parser_buf,
        ifaddrs: Vec::new(),
        quic: args.quic,
        quic_insecure: args.quic_insecure,
        server_name: args.server_name,
        ..BenchConfig::default()
    };

    if let Some(val) = args.ifaddr {
        config.ifaddrs = parse_ifaddrs(&val);
        if config.ifaddrs.is_empty() {
            eprintln!("Error: no valid addresses parsed from --ifaddr '{}'", val);
            std::process::exit(1);
        }
    }

    let addr_str = format!("{}:{}", config.host, config.port);
    config.addr = addr_str
        .to_socket_addrs()
        .unwrap_or_else(|e| {
            eprintln!("Error resolving {}: {}", addr_str, e);
            std::process::exit(1);
        })
        .next()
        .unwrap_or_else(|| {
            eprintln!("Error: no addresses found for {}", addr_str);
            std::process::exit(1);
        });

    if config.workers == 0 {
        config.workers = 1;
    }
    if config.clients == 0 {
        eprintln!("Error: --clients must be > 0");
        std::process::exit(1);
    }
    if config.qos > 2 {
        eprintln!("Error: --qos must be 0, 1, or 2");
        std::process::exit(1);
    }
    if !matches!(config.mqtt_version, 3 | 4 | 5) {
        eprintln!("Error: --mqtt-version must be 3, 4, or 5");
        std::process::exit(1);
    }

    config
}

/// Parse --ifaddr value. Supports:
///   Single:   192.168.1.100
///   List:     192.168.1.100,192.168.1.101,192.168.1.102
///   Range:    192.168.1.100-200   (expands last octet from 100 to 200 inclusive)
fn parse_ifaddrs(val: &str) -> Vec<IpAddr> {
    let mut addrs = Vec::new();
    for part in val.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Check for range notation: a.b.c.X-Y
        if let Some(dash_pos) = part.rfind('-') {
            // Ensure dash is after the last dot (range on last octet)
            if let Some(dot_pos) = part.rfind('.') {
                if dash_pos > dot_pos {
                    let prefix = &part[..dot_pos + 1]; // "a.b.c."
                    let start_str = &part[dot_pos + 1..dash_pos];
                    let end_str = &part[dash_pos + 1..];
                    if let (Ok(start), Ok(end)) =
                        (start_str.parse::<u8>(), end_str.parse::<u8>())
                    {
                        for i in start..=end {
                            let ip_str = format!("{}{}", prefix, i);
                            if let Ok(ip) = ip_str.parse::<IpAddr>() {
                                addrs.push(ip);
                            }
                        }
                        continue;
                    }
                }
            }
        }
        // Plain IP address
        match part.parse::<IpAddr>() {
            Ok(ip) => addrs.push(ip),
            Err(e) => {
                eprintln!("Warning: invalid IP address '{}': {}", part, e);
            }
        }
    }
    addrs
}
