// QUIC transport example for flowsdk with MQTT v5.0
// Demonstrates:
// - QuicConfig builder API (ALPN, 0-RTT, custom root CA from PEM file)
// - Sending MQTT CONNECT and receiving CONNACK over QUIC
// - Using the flowsdk MQTT serialization library

#[cfg(feature = "quic")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    use flowsdk::mqtt_client::transport::quic::{QuicConfig, QuicTransport};
    use flowsdk::mqtt_client::transport::Transport;
    use flowsdk::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
    use flowsdk::mqtt_serde::mqttv5::connectv5::MqttConnect;
    use flowsdk::mqtt_serde::parser::ParseOk;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let addr = "192.168.64.17:14567";

    // Build QUIC config with custom root CA (optional - comment out to use system roots)
    // To use a custom root CA, create a file `ca.pem` in the current directory
    //
    // NOTE: If connecting to an IP address, the server certificate must include that IP
    // in its Subject Alternative Names (SAN). Otherwise, you'll get an "UnknownIssuer" error.
    // To test with your own broker:
    // 1. Use a hostname instead of IP (e.g., "mqtt.example.com:14567")
    // 2. Or provide a custom root CA that signed the server's certificate
    // 3. Or use insecure_skip_verify(true) for testing ONLY (not for production!)
    let cfg = if std::path::Path::new("ca.pem").exists() {
        println!("📄 Loading custom root CA from ca.pem");
        QuicConfig::builder()
            .alpn(b"mqtt") // Advertise MQTT ALPN protocol
            .enable_0rtt(false) // Disable 0-RTT for initial connection
            .custom_roots_from_pem_file("ca.pem")? // Load custom root certificates
            .insecure_skip_verify(true) // ⚠️ Skip cert verification for testing
            .build()
    } else {
        println!("🔒 Using system root certificates (ca.pem not found)");
        println!("⚠️  Enabling insecure_skip_verify for testing - DO NOT USE IN PRODUCTION!");
        QuicConfig::builder()
            .alpn(b"mqtt")
            .enable_0rtt(false)
            .insecure_skip_verify(true) // ⚠️ Skip cert verification for testing
            .build()
    };

    println!("🔌 Connecting to {} via QUIC...", addr);

    let mut transport = match QuicTransport::connect_with_config(addr, cfg).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("❌ Failed to connect: {}", e);
            eprintln!();
            eprintln!("💡 Common issues:");
            eprintln!("   - Server certificate doesn't match the hostname/IP");
            eprintln!("   - Server requires a custom root CA (create ca.pem)");
            eprintln!("   - Server doesn't support QUIC on this port");
            eprintln!("   - Network connectivity issues");
            eprintln!();
            eprintln!("📝 To fix certificate issues:");
            eprintln!("   1. Use a hostname instead of IP address");
            eprintln!("   2. Or provide the server's root CA in ca.pem");
            eprintln!("   3. Ensure the server certificate is valid");
            return Err(e.into());
        }
    };
    println!(
        "✅ Connected: peer={} local={}",
        transport.peer_addr()?,
        transport.local_addr()?
    );

    // Create MQTT v5.0 CONNECT packet
    let connect_packet = MqttConnect::new(
        "quic_example_client".to_string(), // client_id
        None,                              // username
        None,                              // password
        None,                              // will
        60,                                // keep_alive (seconds)
        true,                              // clean_start
        vec![],                            // properties
    );

    // Serialize CONNECT packet to bytes
    let connect_bytes = connect_packet.to_bytes()?;
    println!("📤 Sending MQTT CONNECT ({} bytes)...", connect_bytes.len());

    // Send CONNECT packet
    transport.write_all(&connect_bytes).await?;
    transport.flush().await?;

    // Read CONNACK response
    println!("⏳ Waiting for CONNACK...");
    let mut buffer = vec![0u8; 2048];
    let n = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        transport.read(&mut buffer),
    )
    .await??;

    if n == 0 {
        eprintln!("❌ Connection closed by server");
        return Ok(());
    }

    println!("📥 Received {} bytes", n);

    // Parse CONNACK packet
    match MqttPacket::from_bytes_v5(&buffer[..n])? {
        ParseOk::Packet(MqttPacket::ConnAck5(connack), _) => {
            println!("✅ CONNACK received:");
            println!("   Session present: {}", connack.session_present);
            println!(
                "   Reason code: {} (0x{:02X})",
                connack.reason_code, connack.reason_code
            );

            if connack.reason_code == 0 {
                println!("🎉 Successfully connected to MQTT broker over QUIC!");
            } else {
                println!("⚠️  Connection accepted with non-zero reason code");
            }

            if let Some(props) = &connack.properties {
                println!("   Properties: {} property(ies)", props.len());
                for prop in props {
                    println!("     - {:?}", prop);
                }
            }
        }
        ParseOk::Packet(other, _) => {
            eprintln!("❌ Unexpected packet type: {:?}", other);
        }
        ParseOk::Continue(hint, _) => {
            eprintln!("⚠️  Incomplete packet (need {} more bytes)", hint);
        }
        ParseOk::TopicName(_, _) => {
            eprintln!("❌ Unexpected topic name parse result");
        }
    }

    // Close connection gracefully
    transport.close().await?;
    println!("👋 Connection closed");

    Ok(())
}

#[cfg(not(feature = "quic"))]
fn main() {
    eprintln!("This example requires the `quic` feature. Run with `cargo run --example quic_client_example --features quic`");
}
