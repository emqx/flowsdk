// SPDX-License-Identifier: MPL-2.0

use flowsdk::mqtt_client::commands::PublishCommand;
use flowsdk::mqtt_client::engine::{MqttEvent, QuicMqttEngine};
use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_client::SubscribeCommand;
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mqtt_opts = MqttClientOptions::builder()
        .client_id("no-io-quic-client")
        .peer("broker.emqx.io:14567")
        .keep_alive(30)
        .build();

    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs()? {
        root_store.add(cert).ok();
    }

    let crypto = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let mut engine = QuicMqttEngine::new(mqtt_opts)?;
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_nonblocking(true)?;
    let server_addr = ("broker.emqx.io", 14567u16)
        .to_socket_addrs()?
        .next()
        .ok_or("DNS Failure")?;

    engine.connect(server_addr, "broker.emqx.io", crypto, Instant::now())?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    let mut last_tick = Instant::now();
    let mut published = false;
    let mut subscribed = false;

    loop {
        if !running.load(Ordering::SeqCst) {
            break Ok(());
        }
        let now = Instant::now();

        let mut buf = [0u8; 2048];
        while let Ok((len, remote)) = socket.recv_from(&mut buf) {
            if remote == server_addr {
                engine.handle_datagram(buf[..len].to_vec(), remote, now);
            }
        }

        if now.duration_since(last_tick) >= Duration::from_millis(10) {
            let events = engine.handle_tick(now);
            for event in events {
                match event {
                    MqttEvent::Connected(_) => println!("MQTT Connected over QUIC!"),
                    MqttEvent::Subscribed(res) => println!("Subscribed: ID={:?}", res.packet_id),
                    MqttEvent::Published(res) => {
                        println!("Message Published: ID={:?}", res.packet_id)
                    }
                    MqttEvent::Disconnected(reason) => {
                        println!("MQTT Disconnected: {:?}", reason);
                        return Ok(());
                    }
                    _ => {}
                }
            }
            last_tick = now;
        }

        let mut dags = engine.take_outgoing_datagrams();
        while let Some((dest, data)) = dags.pop_front() {
            let _ = socket.send_to(&data, dest);
        }

        if engine.is_connected() && !subscribed {
            let sub_cmd = SubscribeCommand::builder()
                .add_topic("test/quic/topic", 1)
                .build()
                .unwrap();
            let _ = engine.subscribe(sub_cmd);
            subscribed = true;
        }

        if engine.is_connected() && !published {
            let pub_cmd = PublishCommand::builder()
                .topic("test/quic/topic")
                .payload("Hello!".to_string())
                .qos(1)
                .build();
            if let Ok(cmd) = pub_cmd {
                let _ = engine.publish(cmd);
            }
            published = true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
