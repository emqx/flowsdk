//! TLS/SSL MQTT Client Example
//!
//! This example demonstrates how to connect to an MQTT broker using TLS/SSL encryption.
//! It connects to the public broker.emqx.io broker on port 8883 (TLS port).
//!
//! # Prerequisites
//! - The 'tls' feature must be enabled in Cargo.toml
//!
//! # Running
//! ```bash
//! cargo run --example tls_client --features tls
//! ```
//!
//! # What This Example Shows
//! - Connecting to an MQTT broker over TLS using default system certificates
//! - Publishing messages over encrypted connection
//! - Subscribing to topics with TLS
//! - Clean disconnect over TLS
//!
//! # Note
//! This example uses the system's default trusted root certificates.
//! The broker.emqx.io certificate is signed by Let's Encrypt, which is
//! trusted by default on most systems.

use flowsdk::mqtt_client::client::{
    ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::{
    MqttClientError, MqttClientOptions, TokioAsyncClientConfig, TokioAsyncMqttClient,
    TokioMqttEventHandler,
};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};

/// Event handler for TLS MQTT client
struct TlsExampleHandler {
    name: String,
    message_count: Arc<Mutex<usize>>,
}

impl TlsExampleHandler {
    fn new(name: &str, message_count: Arc<Mutex<usize>>) -> Self {
        Self {
            name: name.to_string(),
            message_count,
        }
    }
}

#[async_trait::async_trait]
impl TokioMqttEventHandler for TlsExampleHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        if result.is_success() {
            println!("[{}] 🔒 ✅ Connected securely via TLS!", self.name);
            println!(
                "[{}]    Session present: {}",
                self.name, result.session_present
            );
        } else {
            println!(
                "[{}] ❌ TLS Connection failed: {} (code: {})",
                self.name,
                result.reason_description(),
                result.reason_code
            );
        }
    }

    async fn on_disconnected(&mut self, reason: Option<u8>) {
        match reason {
            Some(code) => println!("[{}] 🔒 👋 Disconnected (reason code: {})", self.name, code),
            None => println!("[{}] 🔒 👋 Connection lost", self.name),
        }
    }

    async fn on_published(&mut self, result: &PublishResult) {
        if result.is_success() {
            println!(
                "[{}] 🔒 📤 Message published securely (QoS: {}, ID: {:?})",
                self.name, result.qos, result.packet_id
            );
        } else {
            println!(
                "[{}] ❌ Publish failed (code: {:?})",
                self.name, result.reason_code
            );
        }
    }

    async fn on_subscribed(&mut self, result: &SubscribeResult) {
        if result.is_success() {
            println!("[{}] 🔒 📥 Subscribed successfully!", self.name);
            println!(
                "[{}]    Granted QoS levels: {:?}",
                self.name, result.reason_codes
            );
        } else {
            println!(
                "[{}] ❌ Subscribe failed: {:?}",
                self.name, result.reason_codes
            );
        }
    }

    async fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
        if result.is_success() {
            println!("[{}] 🔒 📤 Unsubscribed successfully", self.name);
        } else {
            println!(
                "[{}] ❌ Unsubscribe failed: {:?}",
                self.name, result.reason_codes
            );
        }
    }

    async fn on_message_received(&mut self, publish: &MqttPublish) {
        let mut count = self.message_count.lock().unwrap();
        *count += 1;

        let payload = String::from_utf8_lossy(&publish.payload);
        println!(
            "[{}] 🔒 📨 Message #{} received on topic '{}':",
            self.name, count, publish.topic_name
        );
        println!("[{}]    Payload: {}", self.name, payload);
        println!(
            "[{}]    QoS: {}, Retain: {}, Packet ID: {:?}",
            self.name, publish.qos, publish.retain, publish.packet_id
        );
    }

    async fn on_ping_response(&mut self, result: &PingResult) {
        if result.success {
            println!("[{}] 🔒 🏓 Ping successful", self.name);
        } else {
            println!("[{}] ❌ Ping failed", self.name);
        }
    }

    async fn on_error(&mut self, error: &MqttClientError) {
        println!("[{}] ❌ Client error: {}", self.name, error.user_message());
    }

    async fn on_connection_lost(&mut self) {
        println!("[{}] 💔 Connection lost!", self.name);
    }

    async fn on_reconnect_attempt(&mut self, attempt: u32) {
        println!("[{}] 🔄 Reconnection attempt #{}", self.name, attempt);
    }

    async fn on_pending_operations_cleared(&mut self) {
        println!("[{}] 🧹 Pending operations cleared", self.name);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔒 TLS MQTT Client Example");
    println!("==========================\n");

    // Create shared context for message counting
    let message_count = Arc::new(Mutex::new(0));

    // Create event handler
    let handler = Box::new(TlsExampleHandler::new("TLS-Client", message_count.clone()));

    // Configure MQTT client options with TLS enabled
    let mqtt_options = MqttClientOptions::builder()
        .peer("mqtts://broker.emqx.io:8883") // EMQX public TLS broker (mqtts:// scheme)
        .client_id("flowsdk_tls_example")
        .clean_start(true)
        .keep_alive(60)
        .build();

    // Configure the TLS feature
    #[cfg(feature = "tls")]
    let mqtt_options = {
        mqtt_options.enable_tls() // Enable TLS with default system certificates
    };

    println!("📋 Client Configuration:");
    println!("   Broker: {}", mqtt_options.peer);
    println!("   Client ID: {}", mqtt_options.client_id);
    #[cfg(feature = "tls")]
    println!("   TLS Enabled: {}", mqtt_options.tls_config.is_some());
    println!("   Clean Start: {}", mqtt_options.clean_start);
    println!("   Keep Alive: {} seconds\n", mqtt_options.keep_alive);

    // Configure tokio async client settings
    let async_config = TokioAsyncClientConfig::builder()
        .auto_reconnect(false)
        .command_queue_size(1000)
        .buffer_messages(true)
        .max_buffer_size(1000)
        .send_buffer_size(1000)
        .recv_buffer_size(1000)
        .tcp_nodelay(true)
        .build();

    // Create and connect client
    println!("🔌 Connecting to broker.emqx.io:8883 via TLS...");
    let client = TokioAsyncMqttClient::new(mqtt_options, handler, async_config).await?;

    client.connect().await?;

    // Wait for connection to establish
    sleep(Duration::from_secs(2)).await;

    // Subscribe to a test topic
    let topic = "flowsdk/tls/test";
    println!("\n📥 Subscribing to topic: {}", topic);

    client.subscribe(topic, 1).await?;

    sleep(Duration::from_secs(1)).await;

    // Publish a test message
    println!("\n📤 Publishing test message...");
    client
        .publish(topic, b"Hello from FlowSDK TLS client!", 1, false)
        .await?;

    // Wait to receive the message
    println!("\n⏳ Waiting to receive message (5 seconds)...");
    sleep(Duration::from_secs(5)).await;

    // Send a ping to verify connection is still alive
    println!("\n🏓 Sending PING to broker...");
    client.ping().await?;

    sleep(Duration::from_secs(1)).await;

    // Publish a few more messages
    println!("\n📤 Publishing additional messages...");
    for i in 1..=3 {
        let message = format!("TLS message #{}", i);
        client.publish(topic, message.as_bytes(), 1, false).await?;

        sleep(Duration::from_millis(500)).await;
    }

    // Wait to receive all messages
    sleep(Duration::from_secs(2)).await;

    // Print final statistics
    let final_count = *message_count.lock().unwrap();
    println!("\n📊 Session Summary:");
    println!("   Total messages received: {}", final_count);

    // Disconnect gracefully
    println!("\n👋 Disconnecting from broker...");
    client.disconnect().await?;

    sleep(Duration::from_secs(1)).await;

    println!("✅ TLS example completed successfully!\n");

    Ok(())
}
