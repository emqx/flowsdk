// QUIC-enabled async MQTT client example
// Demonstrates using TokioAsyncMqttClient with QUIC transport

#[cfg(feature = "quic")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use flowsdk::mqtt_client::client::{
        ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
    };
    use flowsdk::mqtt_client::{
        MqttClientError, MqttClientOptions, TokioAsyncClientConfig, TokioAsyncMqttClient,
        TokioMqttEventHandler,
    };
    use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
    use tokio::time::{sleep, Duration};

    /// Simple event handler for the QUIC async client
    struct QuicExampleHandler {
        name: String,
    }

    impl QuicExampleHandler {
        fn new(name: &str) -> Self {
            QuicExampleHandler {
                name: name.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl TokioMqttEventHandler for QuicExampleHandler {
        async fn on_connected(&mut self, result: &ConnectionResult) {
            if result.is_success() {
                println!(
                    "[{}] ✅ Connected successfully over QUIC! Session present: {}",
                    self.name, result.session_present
                );
                if let Some(properties) = &result.properties {
                    println!("[{}] 📋 Broker properties: {:?}", self.name, properties);
                }
            } else {
                println!(
                    "[{}] ❌ Connection failed: {} (code: {})",
                    self.name,
                    result.reason_description(),
                    result.reason_code
                );
            }
        }

        async fn on_disconnected(&mut self, reason: Option<u8>) {
            match reason {
                Some(code) => println!("[{}] 👋 Disconnected (reason code: {})", self.name, code),
                None => println!("[{}] 👋 Disconnected (connection lost)", self.name),
            }
        }

        async fn on_published(&mut self, result: &PublishResult) {
            if result.is_success() {
                println!(
                    "[{}] 📤 Message published successfully (QoS: {}, ID: {:?})",
                    self.name, result.qos, result.packet_id
                );
            } else {
                println!(
                    "[{}] ❌ Publish failed: {} (code: {:?})",
                    self.name,
                    result.reason_description(),
                    result.reason_code
                );
            }
        }

        async fn on_subscribed(&mut self, result: &SubscribeResult) {
            if result.is_success() {
                println!(
                    "[{}] 📥 Subscribed successfully! ({} subscriptions)",
                    self.name,
                    result.successful_subscriptions()
                );
            } else {
                println!(
                    "[{}] ❌ Subscription failed: {:?}",
                    self.name, result.reason_codes
                );
            }
        }

        async fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
            if result.is_success() {
                println!("[{}] 📤 Unsubscribed successfully!", self.name);
            } else {
                println!(
                    "[{}] ❌ Unsubscribe failed: {:?}",
                    self.name, result.reason_codes
                );
            }
        }

        async fn on_message_received(&mut self, publish: &MqttPublish) {
            let payload_str = String::from_utf8_lossy(&publish.payload);
            println!(
                "[{}] 📨 Message received on '{}': {}",
                self.name, publish.topic_name, payload_str
            );
            println!(
                "    QoS: {}, Retain: {}, Packet ID: {:?}",
                publish.qos, publish.retain, publish.packet_id
            );
        }

        async fn on_ping_response(&mut self, result: &PingResult) {
            if result.success {
                println!("[{}] 🏓 Ping response received", self.name);
            } else {
                println!("[{}] ❌ Ping failed", self.name);
            }
        }

        async fn on_error(&mut self, error: &MqttClientError) {
            println!("[{}] ❌ Error: {}", self.name, error.user_message());
        }

        async fn on_connection_lost(&mut self) {
            println!(
                "[{}] 💔 Connection lost! Attempting to reconnect...",
                self.name
            );
        }

        async fn on_reconnect_attempt(&mut self, attempt: u32) {
            println!("[{}] 🔄 Reconnection attempt #{}", self.name, attempt);
        }

        async fn on_pending_operations_cleared(&mut self) {
            println!("[{}] 🧹 Pending operations cleared", self.name);
        }
    }

    println!("🚀 Starting Tokio Async MQTT Client with QUIC Transport");
    println!();
    println!("⚠️  NOTE: This example uses insecure_skip_verify for testing.");
    println!("    For production, use proper certificate validation!");
    println!();

    // Configure MQTT client options with quic:// scheme
    let mqtt_options = MqttClientOptions::builder()
        .peer("quic://broker.emqx.io:14567") // Use quic:// scheme
        .client_id("tokio_quic_example_client")
        .keep_alive(60)
        .clean_start(true)
        .build();

    // Configure tokio async client settings with QUIC options
    let async_config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .reconnect_delay_ms(1000)
        .max_reconnect_delay_ms(30000)
        .max_reconnect_attempts(5)
        // QUIC-specific configuration
        .quic_insecure_skip_verify(true) // ⚠️ For testing only! Use proper certs in production
        .quic_enable_0rtt(false) // Disable 0-RTT for security
        .quic_datagram_receive_buffer_size(0) // disable datagram
        // For production with custom CA:
        // let ca_pem = std::fs::read_to_string("ca.pem").unwrap();
        // .quic_custom_root_ca_pem(ca_pem)
        // .quic_insecure_skip_verify(false)  // Enable verification
        .build();

    // Create event handler
    let event_handler = Box::new(QuicExampleHandler::new("QuicClient"));

    // Create tokio async MQTT client
    let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config).await?;

    println!("📡 Connecting to MQTT broker via QUIC...");
    client.connect().await?;

    // Give some time for connection
    sleep(Duration::from_secs(2)).await;

    println!("📋 Subscribing to test topic...");
    client.subscribe("test/quic/topic", 1).await?;

    sleep(Duration::from_secs(1)).await;

    println!("📤 Publishing test message...");
    client
        .publish("test/quic/topic", b"Hello from QUIC!", 1, false)
        .await?;

    sleep(Duration::from_secs(2)).await;

    println!("🏓 Sending ping...");
    client.ping().await?;

    sleep(Duration::from_secs(2)).await;

    println!("👋 Disconnecting...");
    client.disconnect().await?;

    sleep(Duration::from_secs(1)).await;

    println!("🛑 Shutting down client...");
    client.shutdown().await?;

    println!("✅ QUIC MQTT Client Example completed!");
    Ok(())
}

#[cfg(not(feature = "quic"))]
fn main() {
    eprintln!("This example requires the `quic` feature. Run with:");
    eprintln!("cargo run --example tokio_async_mqtt_quic_example --features quic");
}
