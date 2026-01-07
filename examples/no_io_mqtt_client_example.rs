// SPDX-License-Identifier: MPL-2.0

// Example: Using NoIoMqttClient with custom I/O
//
// This example demonstrates how to use the Sans-I/O MQTT client with
// a simple blocking TCP socket. This pattern can be adapted for:
// - Embedded systems
// - Custom async runtimes
// - FFI bindings
// - WASM environments

use flowsdk::mqtt_client::{
    MqttClientOptions, MqttEvent, NoIoMqttClient, PublishCommand, SubscribeCommand,
};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn main() -> std::io::Result<()> {
    println!("🚀 NoIoMqttClient Example - Custom I/O Integration");
    println!("{}", "=".repeat(60));

    // 1. Create the Sans-I/O client
    let options = MqttClientOptions::builder()
        .peer("broker.emqx.io:1883")
        .client_id("no_io_example_client")
        .keep_alive(60)
        .clean_start(true)
        .build();

    let mut client = NoIoMqttClient::new(options);
    println!("✅ Created NoIoMqttClient");

    // 2. Establish TCP connection (your responsibility)
    let mut stream = TcpStream::connect("broker.emqx.io:1883")?;
    stream.set_nonblocking(true)?;
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    println!("✅ Connected to broker");

    // 3. Initiate MQTT connection
    client.connect();
    let connect_bytes = client.take_outgoing();
    stream.write_all(&connect_bytes)?;
    println!("📤 Sent CONNECT packet ({} bytes)", connect_bytes.len());

    // 4. Wait for CONNACK
    let mut buffer = vec![0u8; 4096];
    let mut connected = false;

    for _ in 0..50 {
        // Try to read (non-blocking)
        match stream.read(&mut buffer) {
            Ok(n) if n > 0 => {
                println!("📥 Received {} bytes", n);
                let events = client.handle_incoming(&buffer[..n]);

                for event in events {
                    match event {
                        MqttEvent::Connected(result) => {
                            println!(
                                "✅ Connected! Reason code: {}, Session present: {}",
                                result.reason_code, result.session_present
                            );
                            connected = true;
                        }
                        MqttEvent::Error(e) => {
                            eprintln!("❌ Error: {}", e);
                        }
                        _ => {}
                    }
                }

                // Send any outgoing bytes
                let outgoing = client.take_outgoing();
                if !outgoing.is_empty() {
                    stream.write_all(&outgoing)?;
                }

                if connected {
                    break;
                }
            }
            Ok(_) => {} // No data
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No data available, sleep briefly
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }

    if !connected {
        eprintln!("❌ Failed to connect within timeout");
        return Ok(());
    }

    // 5. Subscribe to a topic
    println!("\n📋 Subscribing to topics...");
    let sub_cmd = SubscribeCommand::builder()
        .add_topic("test/no_io/#", 1)
        .build()
        .unwrap();

    match client.subscribe(sub_cmd) {
        Ok(packet_id) => {
            println!("📤 Sent SUBSCRIBE (packet ID: {})", packet_id);
            let outgoing = client.take_outgoing();
            stream.write_all(&outgoing)?;
        }
        Err(e) => eprintln!("❌ Subscribe failed: {}", e),
    }

    // 6. Publish a message
    println!("\n📤 Publishing message...");
    let pub_cmd = PublishCommand::builder()
        .topic("test/no_io/example")
        .payload(b"Hello from NoIoMqttClient!")
        .qos(1)
        .build()
        .unwrap();

    match client.publish(pub_cmd) {
        Ok(Some(packet_id)) => {
            println!("📤 Sent PUBLISH (packet ID: {})", packet_id);
            let outgoing = client.take_outgoing();
            stream.write_all(&outgoing)?;
        }
        Ok(None) => println!("📤 Sent PUBLISH (QoS 0)"),
        Err(e) => eprintln!("❌ Publish failed: {}", e),
    }

    // 7. Main event loop
    println!("\n🔄 Entering event loop (press Ctrl+C to exit)...\n");
    let start_time = Instant::now();
    let mut _last_tick = Instant::now();

    loop {
        let now = Instant::now();

        // Handle protocol timers (keep-alive, retransmissions)
        if let Some(next_tick) = client.next_tick_at() {
            if now >= next_tick {
                let events = client.handle_tick(now);
                for event in events {
                    match event {
                        MqttEvent::PingResponse(_) => {
                            println!("🏓 Ping response received");
                        }
                        MqttEvent::ReconnectNeeded => {
                            println!("💔 Connection lost - reconnection needed");
                            break;
                        }
                        _ => {}
                    }
                }
                _last_tick = now;
            }
        }

        // Read from socket
        match stream.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let events = client.handle_incoming(&buffer[..n]);

                for event in events {
                    match event {
                        MqttEvent::MessageReceived(msg) => {
                            let payload = String::from_utf8_lossy(&msg.payload);
                            println!(
                                "📨 Message on '{}': {} (QoS: {})",
                                msg.topic_name, payload, msg.qos
                            );
                        }
                        MqttEvent::Subscribed(result) => {
                            println!("✅ Subscribed (packet ID: {})", result.packet_id);
                        }
                        MqttEvent::Published(result) => {
                            println!("✅ Published (packet ID: {:?})", result.packet_id);
                        }
                        MqttEvent::Error(e) => {
                            eprintln!("❌ Error: {}", e);
                        }
                        _ => {}
                    }
                }
            }
            Ok(_) => {} // No data
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No data available
            }
            Err(e) => {
                eprintln!("❌ Socket error: {}", e);
                break;
            }
        }

        // Send any outgoing bytes
        let outgoing = client.take_outgoing();
        if !outgoing.is_empty() {
            stream.write_all(&outgoing)?;
        }

        // Run for 30 seconds then exit
        if now.duration_since(start_time) > Duration::from_secs(30) {
            println!("\n⏱️  30 seconds elapsed, disconnecting...");
            break;
        }

        // Small sleep to avoid busy-waiting
        std::thread::sleep(Duration::from_millis(10));
    }

    // 8. Disconnect gracefully
    client.disconnect();
    let disconnect_bytes = client.take_outgoing();
    stream.write_all(&disconnect_bytes)?;
    println!("👋 Sent DISCONNECT packet");

    println!("\n✅ NoIoMqttClient example completed!");
    Ok(())
}
