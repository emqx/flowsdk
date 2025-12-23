use flowsdk::mqtt_client::commands::PublishCommand;
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
    published: bool,
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
                    MqttEvent::Connected(_) => println!("Async Driver: MQTT Connected!"),
                    MqttEvent::Published(res) => {
                        println!("Async Driver: Message Published: ID={:?}", res.packet_id)
                    }
                    MqttEvent::Subscribed(res) => {
                        println!("Async Driver: Subscribed: ID={:?}", res.packet_id)
                    }
                    MqttEvent::Disconnected(reason) => {
                        println!("Async Driver: MQTT Disconnected: {:?}", reason);
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

        if self.engine.is_connected() && !self.published {
            let pub_cmd = PublishCommand::builder()
                .topic("test/topic/async")
                .payload("Hello from non-tokio async!".to_string())
                .qos(1)
                .build();

            if let Ok(cmd) = pub_cmd {
                let _ = self.engine.publish(cmd);
            }
            self.published = true;
            if self.exit_when_rcvd {
                self.running.store(false, Ordering::SeqCst);
            }
        }

        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Demonstrates how to use the QuicMqttEngine as a non-tokio async driver.

pub fn run_example(exit_when_rcvd: bool) -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mqtt_opts = MqttClientOptions::builder()
        .client_id("no-io-quic-async-client")
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
        published: false,
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
    fn test_example() {
        run_example(true).unwrap();
    }
}
