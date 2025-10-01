use flowsdk::mqtt_client::client::{
    ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::{
    MqttClientOptions, TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::io;
use tokio::time::{sleep, Duration};

/// Example event handler for the tokio async client
struct TokioExampleHandler {
    name: String,
}

impl TokioExampleHandler {
    fn new(name: &str) -> Self {
        TokioExampleHandler {
            name: name.to_string(),
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

    async fn on_error(&mut self, error: &io::Error) {
        println!("[{}] ❌ Error: {}", self.name, error);
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

    // Configure MQTT client options
    let mqtt_options = MqttClientOptions {
        peer: "localhost:1883".to_string(),
        client_id: "tokio_async_example_client".to_string(),
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        will: None,
        reconnect: true,
        sessionless: false,
        subscription_topics: vec![],
        auto_ack: false,
    };

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
        tcp_nodelay: true,
    };

    // Create event handler
    let event_handler = Box::new(TokioExampleHandler::new("TokioAsyncClient"));

    // Create tokio async MQTT client
    let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config).await?;

    println!("📡 Connecting to MQTT broker...");
    client.connect().await?;

    // Give some time for connection
    sleep(Duration::from_millis(1000)).await;

    println!("📋 Subscribing to topics...");
    client.subscribe("test/tokio/topic", 1).await?;
    client.subscribe("tokio/async/+", 2).await?;

    // Give some time for subscriptions
    sleep(Duration::from_millis(500)).await;

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

    // Give some time for publishes
    sleep(Duration::from_millis(500)).await;

    println!("🏓 Sending ping...");
    client.ping().await?;

    // Give some time for ping
    sleep(Duration::from_millis(500)).await;

    println!("📤 Unsubscribing from topics...");
    client.unsubscribe(vec!["test/tokio/topic"]).await?;

    // Give some time for unsubscribes
    sleep(Duration::from_millis(500)).await;

    println!("👋 Disconnecting...");
    client.disconnect().await?;

    // Give some time for disconnect
    sleep(Duration::from_millis(500)).await;

    println!("🛑 Shutting down client...");
    client.shutdown().await?;

    println!("✅ Tokio Async MQTT Client Example completed!");
    Ok(())
}
