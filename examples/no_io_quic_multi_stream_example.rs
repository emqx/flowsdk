// SPDX-License-Identifier: MPL-2.0

//! Demonstrates MQTT-over-QUIC multi-stream support with the sans-I/O
//! `QuicMqttEngine`.
//!
//! The engine opens a primary *control* stream that carries only session-level
//! packets (CONNECT/CONNACK, PING, DISCONNECT, AUTH). This example then opens two
//! dedicated data streams once connected:
//!
//! - a **sub stream**: SUBSCRIBE is sent via `subscribe_on`; the SUBACK and any
//!   delivered messages — plus the PUBACK we send for QoS 1 deliveries — flow on
//!   that same stream.
//! - a **pub stream**: a PUBLISH is sent via `publish_on`; the QoS 1 PUBACK comes
//!   back on that same stream.
//!
//! Responses never cross-fire between streams or onto the control stream.

use flowsdk::mqtt_client::commands::{PublishCommand, SubscribeCommand};
use flowsdk::mqtt_client::engine::{MqttEvent, QuicMqttEngine};
use flowsdk::mqtt_client::opts::MqttClientOptions;
use std::future::Future;
use std::net::{ToSocketAddrs, UdpSocket};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

struct ProtocolDriver {
    engine: QuicMqttEngine,
    socket: UdpSocket,
    server_addr: std::net::SocketAddr,
    last_tick: Instant,
    /// Handles of the dedicated streams, opened once connected.
    sub_stream: Option<u64>,
    pub_stream: Option<u64>,
    setup_done: bool,
    running: Arc<AtomicBool>,
    exit_when_rcvd: bool,
}

impl Future for ProtocolDriver {
    type Output = Result<(), Box<dyn std::error::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.running.load(Ordering::SeqCst) {
            return Poll::Ready(Ok(()));
        }

        let now = Instant::now();
        let mut buf = [0u8; 2048];
        while let Ok((len, remote)) = self.socket.recv_from(&mut buf) {
            if remote == self.server_addr {
                self.engine
                    .handle_datagram(buf[..len].to_vec(), remote, now);
            }
        }

        if now.duration_since(self.last_tick) >= Duration::from_millis(10) {
            let events = self.engine.handle_tick(now);
            for event in events {
                match event {
                    MqttEvent::Connected(_) => println!("Multi-stream: MQTT Connected!"),
                    MqttEvent::Published(res) => {
                        println!("Multi-stream: Message Published: ID={:?}", res.packet_id)
                    }
                    MqttEvent::Disconnected(reason) => {
                        println!("Multi-stream: MQTT Disconnected: {:?}", reason);
                        return Poll::Ready(Ok(()));
                    }
                    _ => {}
                }
            }
            self.last_tick = now;
        }

        let mut dags = self.engine.take_outgoing_datagrams();
        while let Some((dest, data)) = dags.pop_front() {
            let _ = self.socket.send_to(&data, dest);
        }

        // Once connected, open a dedicated sub stream and pub stream, then
        // subscribe and publish on them respectively.
        if self.engine.is_connected() && !self.setup_done {
            let sub_stream = self.engine.open_data_stream()?;
            let pub_stream = self.engine.open_data_stream()?;
            self.sub_stream = Some(sub_stream);
            self.pub_stream = Some(pub_stream);
            println!(
                "Multi-stream: opened sub stream {} and pub stream {} ({} data streams total)",
                sub_stream,
                pub_stream,
                self.engine.data_stream_count()
            );

            // SUBSCRIBE on the sub stream — SUBACK arrives on the same stream.
            if let Ok(sub_cmd) = SubscribeCommand::builder()
                .add_topic("test/topic/multistream", 1)
                .build()
            {
                let _ = self.engine.subscribe_on(sub_stream, sub_cmd);
            }

            // PUBLISH on the pub stream — the QoS 1 PUBACK arrives on that stream.
            if let Ok(pub_cmd) = PublishCommand::builder()
                .topic("test/topic/multistream")
                .payload("Hello from a dedicated QUIC pub stream!".to_string())
                .qos(1)
                .build()
            {
                let _ = self.engine.publish_on(pub_stream, pub_cmd);
            }

            self.setup_done = true;
            if self.exit_when_rcvd {
                self.running.store(false, Ordering::SeqCst);
            }
        }

        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Demonstrates the multi-stream `QuicMqttEngine` API against a broker.
pub fn run_example(exit_when_rcvd: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Install the process-default rustls crypto provider. The OpenSSL provider is
    // only available on the size-optimised `quic-openssl` builds; the mainstream
    // `quic` build uses ring.
    #[cfg(feature = "quic-proto-openssl")]
    let _ = rustls_openssl::default_provider().install_default();
    #[cfg(not(feature = "quic-proto-openssl"))]
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mqtt_opts = MqttClientOptions::builder()
        .client_id("no-io-quic-multi-stream-client")
        .peer("broker.emqx.io:14567")
        .build();

    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs()? {
        root_store.add(cert).ok();
    }
    let crypto = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_nonblocking(true)?;
    let server_addr = ("broker.emqx.io", 14567u16)
        .to_socket_addrs()?
        .next()
        .ok_or("DNS Failure")?;

    let mut engine = QuicMqttEngine::new(mqtt_opts)?;
    engine.connect(server_addr, "broker.emqx.io", crypto, Instant::now())?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    let driver = ProtocolDriver {
        engine,
        socket,
        server_addr,
        last_tick: Instant::now(),
        sub_stream: None,
        pub_stream: None,
        setup_done: false,
        running,
        exit_when_rcvd,
    };

    futures::executor::block_on(driver)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_example(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires network access to broker.emqx.io:14567"]
    fn test_example() {
        run_example(true).unwrap();
    }
}
