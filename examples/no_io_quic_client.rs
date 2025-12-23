use flowsdk::mqtt_client::commands::PublishCommand;
use flowsdk::mqtt_client::engine::{MqttEvent, QuicMqttEngine};
use flowsdk::mqtt_client::opts::MqttClientOptions;
// quinn_proto::ClientConfig is no longer needed in main
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // rustls 0.23+ requires a crypto provider to be installed
    #[cfg(feature = "quic")]
    let _ = rustls::crypto::ring::default_provider().install_default();

    println!("Starting No-IO MQTT over QUIC Example...");

    // 1. Configure MQTT
    let mqtt_opts = MqttClientOptions::builder()
        .client_id("no-io-quic-client")
        .peer("broker.emqx.io:14567")
        .keep_alive(30)
        .build();

    // 2. Configure QUIC (using rustls) but disable peer cert verify...
    // Use SkipServerVerification for testing
    // let crypto = rustls::ClientConfig::builder()
    //     .dangerous()
    //     .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
    //     .with_no_client_auth();

    // 2. Configure QUIC (using rustls)
    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs()? {
        root_store.add(cert).ok(); // ignore errors for now
    }

    let crypto = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    // 3. Initialize Engine
    let mut engine = QuicMqttEngine::new(mqtt_opts)?;

    // 4. Setup UDP Socket
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_nonblocking(true)?;
    let server_host = "broker.emqx.io";
    let server_port = 14567u16;
    let server_addr = (server_host, server_port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| format!("Could not resolve {}:{}", server_host, server_port))?;

    // 5. Connect
    let now = Instant::now();
    engine.connect(server_addr, "broker.emqx.io", crypto, now)?;

    println!(
        "UDP socket bound to {}, connecting to {}...",
        socket.local_addr()?,
        server_addr
    );

    // 6. Simple Event Loop
    let mut last_tick = Instant::now();
    let mut published = false;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl-C, shutting down...");
        r.store(false, Ordering::SeqCst);
    })?;

    loop {
        if !running.load(Ordering::SeqCst) {
            println!("Exiting loop...");
            break Ok(());
        }
        let now = Instant::now();

        // A. Handle incoming UDP packets
        let mut buf = [0u8; 2048];
        while let Ok((len, remote)) = socket.recv_from(&mut buf) {
            if remote == server_addr {
                engine.handle_datagram(buf[..len].to_vec(), remote, now);
            }
        }

        // B. Drive Engine Tick
        if now.duration_since(last_tick) >= Duration::from_millis(10) {
            let events = engine.handle_tick(now);
            for event in events {
                match event {
                    MqttEvent::Connected(_) => {
                        println!("MQTT Connected over QUIC!");
                    }
                    MqttEvent::MessageReceived(msg) => {
                        println!(
                            "Received Message: Topic={}, Payload={:?}",
                            msg.topic_name, msg.payload
                        );
                    }
                    MqttEvent::Published(res) => {
                        println!("Message Published: ID={:?}", res.packet_id);
                    }
                    MqttEvent::Disconnected(reason) => {
                        println!("MQTT Disconnected: Reason={:?}", reason);
                        return Ok(());
                    }
                    _ => {}
                }
            }
            last_tick = now;
        }

        // C. Send outgoing UDP packets
        let mut dags = engine.take_outgoing_datagrams();
        while let Some((dest, data)) = dags.pop_front() {
            socket.send_to(&data, dest)?;
        }

        // D. Example: Publish a message once connected
        if engine.is_connected() && !published {
            println!("Publishing test message...");
            let pub_cmd = PublishCommand::builder()
                .topic("test/topic")
                .payload("Hello QUIC world!".to_string())
                .qos(1)
                .build();
            engine.publish(pub_cmd?)?;
            published = true;
        }

        // Avoid pegged CPU in this simple example
        std::thread::sleep(Duration::from_millis(1));
    }
}
