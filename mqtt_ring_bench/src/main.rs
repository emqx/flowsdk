// SPDX-License-Identifier: MPL-2.0

// Linux-only: this bench is built on io_uring. On non-Linux platforms we
// fall through to a stub `main` below so workspace builds (e.g. macOS CI)
// still succeed.

#[cfg(target_os = "linux")]
mod config;
#[cfg(target_os = "linux")]
mod connection;
#[cfg(target_os = "linux")]
mod stats;
#[cfg(target_os = "linux")]
mod worker_common;
#[cfg(all(target_os = "linux", feature = "quic"))]
mod worker_quic;
#[cfg(target_os = "linux")]
mod worker_tcp;

#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("mqtt_ring_bench is Linux-only (built on io_uring).");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    let config = Arc::new(config::parse_args());
    let stats = Arc::new(stats::BenchStats::new());

    print_banner(&config);

    // Build TLS config for QUIC (if enabled)
    #[cfg(feature = "quic")]
    let quic_crypto: Option<Arc<rustls::ClientConfig>> = if config.quic {
        Some(Arc::new(build_quic_crypto(&config)))
    } else {
        None
    };

    // Ctrl+C handler
    let stats_ctrlc = Arc::clone(&stats);
    let shutdown_mode = config.shutdown_mode;
    ctrlc::set_handler(move || {
        if shutdown_mode == config::ShutdownMode::Immediate {
            eprintln!("\nInterrupted, exiting immediately...");
            unsafe {
                libc::_exit(130);
            }
        }

        if stats_ctrlc.stopped.swap(true, Ordering::SeqCst) {
            eprintln!("\nInterrupted again, forcing exit...");
            unsafe {
                libc::_exit(130);
            }
        }
        eprintln!("\nInterrupted, cancelling pending I/O...");
    })
    .expect("failed to set Ctrl+C handler");

    let workers = config.workers.min(config.clients);
    let clients_per_worker = config.clients / workers;
    let remainder = config.clients % workers;

    // Spawn worker threads
    let mut handles = Vec::with_capacity(workers);
    let mut offset = 0;
    for w in 0..workers {
        let n = clients_per_worker + if w < remainder { 1 } else { 0 };
        let range = offset..offset + n;
        offset += n;

        let cfg = Arc::clone(&config);
        let st = Arc::clone(&stats);

        #[cfg(feature = "quic")]
        if let Some(ref crypto) = quic_crypto {
            let crypto = Arc::clone(crypto);
            handles.push(thread::spawn(move || {
                worker_quic::run_quic_worker(w, range, cfg, st, crypto)
            }));
            continue;
        }

        handles.push(thread::spawn(move || {
            worker_tcp::run_worker(w, range, cfg, st)
        }));
    }

    // Stats reporter thread
    let stats_reporter = Arc::clone(&stats);
    let total_clients = config.clients as u64;
    let total_messages = (config.clients as u64).saturating_mul(config.messages);
    let action = config.action;
    let reporter = thread::spawn(move || {
        let mut prev_messages: u64 = 0;
        loop {
            thread::sleep(Duration::from_secs(1));
            let snap = stats_reporter.snapshot();
            let messages = match action {
                config::BenchAction::Pub => snap.sent,
                config::BenchAction::Sub => snap.received,
            };
            let rate = messages.saturating_sub(prev_messages);
            prev_messages = messages;
            let secs = snap.elapsed.as_secs();
            match action {
                config::BenchAction::Pub => eprint!(
                    "\r[{:>4}s] Connected: {}/{} | Sent: {}/{} | Acked: {} | PubAckNoMatch: {} | Errors: {} (socket:{} connect:{} send:{} recv:{} mqtt:{} [conn:{} sub:{} pub:{} client:{} disc:{}]) | Rate: {} msg/s   ",
                    secs, snap.connected, total_clients, snap.sent, total_messages, snap.acked,
                    snap.puback_no_match, snap.errors,
                    snap.socket_errors, snap.connect_errors, snap.send_errors, snap.receive_errors,
                    snap.mqtt_errors, snap.mqtt_connect_errors, snap.mqtt_subscribe_errors,
                    snap.mqtt_publish_errors, snap.mqtt_client_errors,
                    snap.mqtt_disconnect_errors, rate
                ),
                config::BenchAction::Sub => {
                    let target = if total_messages == 0 {
                        "unbounded".to_string()
                    } else {
                        total_messages.to_string()
                    };
                    eprint!(
                        "\r[{:>4}s] Connected: {}/{} | Subscribed: {}/{} | Received: {}/{} | Errors: {} (socket:{} connect:{} send:{} recv:{} mqtt:{} [conn:{} sub:{} client:{} disc:{}]) | Rate: {} msg/s   ",
                        secs, snap.connected, total_clients, snap.subscribed, total_clients,
                        snap.received, target, snap.errors, snap.socket_errors,
                        snap.connect_errors, snap.send_errors, snap.receive_errors,
                        snap.mqtt_errors, snap.mqtt_connect_errors, snap.mqtt_subscribe_errors,
                        snap.mqtt_client_errors, snap.mqtt_disconnect_errors, rate
                    );
                }
            }
            if snap.done >= total_clients || stats_reporter.stopped.load(Ordering::Relaxed) {
                eprintln!();
                break;
            }
        }
    });

    // Wait for all workers
    let mut all_samples: Vec<Vec<Duration>> = Vec::new();
    for h in handles {
        match h.join() {
            Ok(result) => all_samples.push(result.latency_samples),
            Err(_) => eprintln!("worker thread panicked"),
        }
    }
    let _ = reporter.join();

    let merged = stats::merge_latency_samples(all_samples);
    let snap = stats.snapshot();
    stats::print_final_summary(&config, &snap, &merged);
    if stats.stopped.load(Ordering::SeqCst) {
        std::process::exit(130);
    }
}

#[cfg(target_os = "linux")]
fn print_banner(config: &config::BenchConfig) {
    let transport = if config.quic { "QUIC" } else { "TCP" };
    eprintln!("mqtt_ring_bench - io_uring MQTT benchmark");
    eprintln!(
        "  Target:     {}:{} ({})",
        config.host, config.port, transport
    );
    eprintln!("  Clients:    {}", config.clients);
    eprintln!("  Workers:    {}", config.workers);
    eprintln!("  Action:     {}", config.action.as_str());
    if config.action == config::BenchAction::Sub && config.messages == 0 {
        eprintln!("  Messages:   unbounded");
    } else {
        eprintln!("  Messages:   {} per client", config.messages);
    }
    eprintln!("  Topic:      {}", config.topic);
    eprintln!("  QoS:        {}", config.qos);
    if config.action == config::BenchAction::Pub {
        eprintln!("  Payload:    {} bytes", config.payload_size);
        eprintln!(
            "  Interval:   {} ms",
            if config.interval_ms == 0 {
                "max speed".to_string()
            } else {
                config.interval_ms.to_string()
            }
        );
    }
    eprintln!("  MQTT:       v{}", config.mqtt_version);
    eprintln!("  Socket buf: {} bytes", config.socket_buf);
    eprintln!("  Parser buf: {} bytes", config.parser_buf);
    eprintln!("  Shutdown:   {}", config.shutdown_mode.as_str());
    if config.ifaddrs.is_empty() {
        eprintln!("  Bind addrs: (OS default)");
    } else {
        eprintln!(
            "  Bind addrs: {} IPs ({}..{})",
            config.ifaddrs.len(),
            config.ifaddrs.first().unwrap(),
            config.ifaddrs.last().unwrap(),
        );
    }
    if config.quic {
        let sni = config.server_name.as_deref().unwrap_or(&config.host);
        eprintln!("  TLS SNI:    {}", sni);
        if config.quic_insecure {
            eprintln!("  TLS verify: DISABLED (insecure)");
        }
    }
    eprintln!();
}

#[cfg(all(target_os = "linux", feature = "quic"))]
fn build_quic_crypto(config: &config::BenchConfig) -> rustls::ClientConfig {
    if config.quic_insecure {
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier))
            .with_no_client_auth()
    } else {
        let mut root_store = rustls::RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().unwrap_or_default() {
            root_store.add(cert).ok();
        }
        if root_store.is_empty() {
            eprintln!("Warning: no system root certificates found; TLS verification may fail.");
            eprintln!("         Use --quic-insecure for testing without certificate verification.");
        }
        rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth()
    }
}

/// Certificate verifier that accepts any server certificate (for --quic-insecure).
#[cfg(all(target_os = "linux", feature = "quic"))]
#[derive(Debug)]
struct InsecureCertVerifier;

#[cfg(all(target_os = "linux", feature = "quic"))]
impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
