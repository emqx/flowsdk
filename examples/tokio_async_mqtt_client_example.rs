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

/// Example event handler for the tokio async client
struct TokioExampleHandler {
    name: String,
    context: Arc<Mutex<Option<u16>>>,
}

impl TokioExampleHandler {
    fn new(name: &str, context: Arc<Mutex<Option<u16>>>) -> Self {
        TokioExampleHandler {
            name: name.to_string(),
            context,
        }
    }

    fn update_last_acked_packet_id(&mut self, packet_id: u16) {
        if let Ok(mut ctx) = self.context.lock() {
            println!(
                "[{}] 📦 Updating last acknowledged packet ID to {}",
                self.name, packet_id
            );
            *ctx = Some(packet_id);
        }
    }
}

#[async_trait::async_trait]
impl TokioMqttEventHandler for TokioExampleHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        if result.is_success() {
            println!(
                "[{}] ✅ Connected successfully! Session present: {}",
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
        if let Some(packet_id) = result.packet_id {
            self.update_last_acked_packet_id(packet_id);
        }
        if result.is_success() {
            println!(
                "[{}] 📤 Message published successfully (QoS: {}, ID: {:?})",
                self.name, result.qos, result.packet_id
            );
        } else {
            println!(
                "[{}] ❌ Publish failed: {:?}",
                self.name, result.reason_code
            );
        }
    }

    async fn on_subscribed(&mut self, result: &SubscribeResult) {
        self.update_last_acked_packet_id(result.packet_id);
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
        self.update_last_acked_packet_id(result.packet_id);
        println!(
            "[{}] 📤 Unsubscribe result for packet ID {:?}",
            self.name, result.packet_id
        );
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Tokio Async MQTT Client Example");

    // Configure MQTT client options using builder pattern
    let mqtt_options = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("tokio_async_example_client")
        .keep_alive(10)
        .reconnect(true)
        .auto_ack(false)
        .build();

    // Configure tokio async client settings
    let async_config = TokioAsyncClientConfig {
        auto_reconnect: true,
        reconnect_delay_ms: 1000,
        max_reconnect_delay_ms: 30000,
        max_reconnect_attempts: 5,
        command_queue_size: 1000,
        buffer_messages: true,
        max_buffer_size: 1000,
        send_buffer_size: 1000,
        recv_buffer_size: 1000,
        keep_alive_interval: 60,
        tcp_nodelay: false,
        ..Default::default()
    };

    let context = Arc::new(Mutex::new(None::<u16>));
    // Create event handler
    let event_handler = Box::new(TokioExampleHandler::new(
        "TokioAsyncClient",
        context.clone(),
    ));

    // Create tokio async MQTT client
    let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config).await?;

    println!("📡 Connecting to MQTT broker...");
    client.connect().await?;

    // Give some time for connection
    sleep(Duration::from_millis(1000)).await;

    println!("📋 Subscribing to topics...");
    client.subscribe("test/tokio/topic", 1).await?;
    client.subscribe("tokio/async/+", 2).await?;

    println!("📤 Publishing test messages...");
    client
        .publish(
            "test/tokio/topic",
            b"Hello from Tokio Async Client!",
            1,
            false,
        )
        .await?;
    client
        .publish("tokio/async/test", b"Async message with QoS 2", 2, true)
        .await?;
    client
        .publish("tokio/async/qos0", b"Quick QoS 0 message", 0, false)
        .await?;

    println!("🏓 Sending ping...");
    client.ping().await?;

    println!("📤 Unsubscribing from topics...");
    client.unsubscribe(vec!["test/tokio/topic"]).await?;

    // Wait for the last_acked_packet_id to be exactly 4
    println!("⏳ Waiting for unsubscribe acknowledgment (packet ID 4)...");
    let wait_for_ack = async {
        loop {
            if let Ok(ctx) = context.lock() {
                if *ctx == Some(4) {
                    println!("✅ Received acknowledgment for packet ID 4");
                    break;
                } else {
                    println!("❌ Unsubscribe acknowledgment not received yet {:?}", ctx);
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
    };

    match tokio::time::timeout(Duration::from_secs(5), wait_for_ack).await {
        Ok(_) => println!("✅ Successfully received acknowledgment"),
        Err(_) => println!("⚠️  Timeout waiting for acknowledgment"),
    }

    // Now testing the keep-alive and auto-reconnect features
    tokio::time::sleep(Duration::from_secs(20)).await;

    client
        .publish("tokio/async/test", b"Async message with QoS 2", 2, true)
        .await?;

    tokio::time::sleep(Duration::from_secs(5)).await;
    client
        .publish("tokio/async/test", b"Async message with QoS 2", 2, true)
        .await?;

    tokio::time::sleep(Duration::from_secs(20)).await;

    println!("👋 Disconnecting...");
    client.disconnect().await?;

    tokio::time::sleep(Duration::from_secs(20)).await;

    println!("🛑 Shutting down client...");
    client.shutdown().await?;

    println!("✅ Tokio Async MQTT Client Example completed!");
    Ok(())
}
