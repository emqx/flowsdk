// SPDX-License-Identifier: MPL-2.0

// QUIC-enabled async MQTT client example
// Demonstrates using TokioAsyncMqttClient with QUIC transport

use flowsdk::mqtt_client::client::{
    ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::{
    MqttClientError, MqttClientOptions, MqttMessage, TokioAsyncClientConfig, TokioAsyncMqttClient,
    TokioMqttEventHandler,
};
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

    async fn on_message_received(&mut self, publish: &MqttMessage) {
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

async fn run_example(test_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Tokio Async MQTT Client with QUIC Transport");
    println!();
    println!("⚠️  NOTE: This example uses insecure_skip_verify for testing.");
    println!("    For production, use proper certificate validation!");
    println!();

    let broker_addr = "broker.emqx.io:14567";
    let peer_url = format!("quic://{}", broker_addr);
    println!("📡 Connecting to broker: {}", peer_url);
    println!();

    // Configure MQTT client options with quic:// scheme
    let mqtt_options = MqttClientOptions::builder()
        .peer(&peer_url)
        .client_id("tokio_quic_example_client")
        .keep_alive(60)
        .clean_start(true)
        .build();

    // Configure tokio async client settings with QUIC options
    let async_config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .max_reconnect_delay_ms(1000)
        .max_reconnect_attempts(5)
        .quic_insecure_skip_verify(true) // ⚠️ For testing only!
        .quic_enable_0rtt(false)
        .quic_datagram_receive_buffer_size(0) // disable datagram
        .build();

    // Create event handler
    let event_handler = Box::new(QuicExampleHandler::new("QuicClient"));

    // Create tokio async MQTT client
    let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config).await?;

    println!("📡 Initiating QUIC connection...");
    client.connect().await?;

    sleep(Duration::from_secs(2)).await;

    println!("📋 Subscribing to test topic...");
    client.subscribe("test/quic/topic", 1).await?;

    sleep(Duration::from_secs(1)).await;

    if test_mode {
        println!("📤 Publishing test messages...");
        for i in 1..=3 {
            let message = format!("Hello from QUIC! Message #{}", i);
            match client
                .publish("test/quic/topic", message.as_bytes(), 1, false)
                .await
            {
                Ok(_) => println!("📤 Published message #{}", i),
                Err(e) => eprintln!("❌ Failed to publish message #{}: {}", i, e),
            }
            sleep(Duration::from_millis(500)).await;
        }
    } else {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        println!("📤 Publishing messages continuously (Press Ctrl-C to stop)...");

        // Set up Ctrl-C handler
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        ctrlc::set_handler(move || {
            println!("\n🛑 Ctrl-C received, stopping...");
            r.store(false, Ordering::SeqCst);
        })
        .expect("Error setting Ctrl-C handler");

        let mut counter = 0u64;
        while running.load(Ordering::SeqCst) {
            counter += 1;
            let message = format!("Hello from QUIC! Message #{}", counter);

            match client
                .publish("test/quic/topic", message.as_bytes(), 1, false)
                .await
            {
                Ok(_) => println!("📤 Published message #{}", counter),
                Err(e) => eprintln!("❌ Failed to publish message #{}: {}", counter, e),
            }

            sleep(Duration::from_secs(1)).await;
        }
    }

    println!("🏓 Sending ping...");
    client.ping().await?;

    sleep(Duration::from_secs(1)).await;

    println!("👋 Disconnecting...");
    client.disconnect().await?;

    sleep(Duration::from_secs(1)).await;

    println!("🛑 Shutting down client...");
    client.shutdown().await?;

    println!("✅ QUIC MQTT Client Example completed!");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_example(false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_example() {
        run_example(true).await.unwrap();
    }
}
