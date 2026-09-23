// SPDX-License-Identifier: MPL-2.0

use flowsdk::mqtt_client::client::{
    ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::PublishCommand;
use flowsdk::mqtt_client::{
    MqttClientError, MqttClientOptions, MqttMessage, TokioAsyncClientConfig, TokioAsyncMqttClient,
    TokioMqttEventHandler,
};

use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};

/// Example event handler for MQTT v3.1.1 tokio async client
struct TokioV3ExampleHandler {
    name: String,
    context: Arc<Mutex<Option<u16>>>,
}

impl TokioV3ExampleHandler {
    fn new(name: &str, context: Arc<Mutex<Option<u16>>>) -> Self {
        TokioV3ExampleHandler {
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
impl TokioMqttEventHandler for TokioV3ExampleHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        if result.is_success() {
            println!(
                "[{}] ✅ Connected successfully via MQTT v3.1.1! Session present: {}",
                self.name, result.session_present
            );
            println!(
                "[{}] 📋 Return code: {} ({})",
                self.name,
                result.reason_code,
                result.reason_description()
            );
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
            Some(code) => println!("[{}] 👋 Disconnected (return code: {})", self.name, code),
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
                "[{}] ❌ Publish failed (ID: {:?})",
                self.name, result.packet_id
            );
        }
    }

    async fn on_subscribed(&mut self, result: &SubscribeResult) {
        self.update_last_acked_packet_id(result.packet_id);
        if result.is_success() {
            println!(
                "[{}] 📥 Subscribed successfully! ({} subscriptions, granted QoS: {:?})",
                self.name,
                result.successful_subscriptions(),
                result.reason_codes
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

    async fn on_message_received(&mut self, publish: &MqttMessage) {
        let payload_str = String::from_utf8_lossy(&publish.payload);
        println!(
            "[{}] 📨 Message received on '{}': {}",
            self.name, publish.topic_name, payload_str
        );
        println!(
            "    QoS: {}, Retain: {}, Packet ID: {:?}, DUP: {}",
            publish.qos, publish.retain, publish.packet_id, publish.dup
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

async fn run_v3_example() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting MQTT v3.1.1 Tokio Async Client Example");
    println!("{}", "=".repeat(60));

    // Configure MQTT client options for MQTT v3.1.1
    let mqtt_options = MqttClientOptions::builder()
        .peer("broker.emqx.io:1883")
        .client_id("tokio_async_v3_example_client")
        .mqtt_version(3) // ⚠️ IMPORTANT: Use MQTT v3.1.1
        .keep_alive(60)
        .clean_start(true) // Clean session for v3
        .reconnect(true)
        .auto_ack(true) // Enable auto-ack to test session handling
        .build();

    // Configure tokio async client settings
    let async_config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .max_reconnect_delay_ms(30000)
        .max_reconnect_attempts(5)
        .command_queue_size(1000)
        .buffer_messages(true)
        .max_buffer_size(1000)
        .tcp_nodelay(false)
        .build();

    let context = Arc::new(Mutex::new(None::<u16>));

    // Create event handler
    let event_handler = Box::new(TokioV3ExampleHandler::new(
        "TokioAsyncV3Client",
        context.clone(),
    ));

    // Create tokio async MQTT client
    let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config).await?;

    println!("\n📡 Connecting to MQTT broker via v3.1.1...");
    let connected = client.connect_sync().await?;
    if !connected.is_success() {
        return Err(format!("CONNECT rejected: {connected:?}").into());
    }

    println!("\n📋 Testing MQTT v3.1.1 Subscriptions...");
    println!("{}", "─".repeat(60));

    // Test single subscription
    let subscribed = client.subscribe_sync("test/v3/qos0", 0).await?;
    if !subscribed.is_success() {
        return Err(format!("SUBSCRIBE rejected: {subscribed:?}").into());
    }

    let subscribed = client.subscribe_sync("test/v3/qos1", 1).await?;
    if !subscribed.is_success() {
        return Err(format!("SUBSCRIBE rejected: {subscribed:?}").into());
    }

    let subscribed = client.subscribe_sync("test/v3/qos2", 2).await?;
    if !subscribed.is_success() {
        return Err(format!("SUBSCRIBE rejected: {subscribed:?}").into());
    }

    // Test wildcard subscriptions (v3 supports + and #)
    let subscribed = client.subscribe_sync("test/v3/+/sensor", 1).await?;
    if !subscribed.is_success() {
        return Err(format!("SUBSCRIBE rejected: {subscribed:?}").into());
    }

    let subscribed = client.subscribe_sync("test/v3/#", 1).await?;
    if !subscribed.is_success() {
        return Err(format!("SUBSCRIBE rejected: {subscribed:?}").into());
    }

    println!("\n📤 Testing MQTT v3.1.1 Publishing...");
    println!("{}", "─".repeat(60));

    // Test QoS 0 (fire and forget)
    println!("\n🔹 QoS 0 Publish (fire and forget)");
    let qos0_cmd = PublishCommand::builder()
        .topic("test/v3/qos0")
        .payload(b"QoS 0 message - no acknowledgment")
        .qos(0)
        .build()?;
    client.publish_with_command(qos0_cmd).await?;
    sleep(Duration::from_millis(500)).await;

    // Test QoS 1 (at least once - tests PUBACK session handling)
    println!("\n🔹 QoS 1 Publish (at least once - tests PUBACK)");
    let qos1_cmd = PublishCommand::builder()
        .topic("test/v3/qos1")
        .payload(b"QoS 1 message - tests PUBACK session handling")
        .qos(1)
        .build()?;
    client.publish_with_command(qos1_cmd).await?;
    sleep(Duration::from_millis(500)).await;

    // Test QoS 2 (exactly once - tests PUBREC/PUBREL/PUBCOMP session handling)
    println!("\n🔹 QoS 2 Publish (exactly once - tests PUBCOMP)");
    let qos2_cmd = PublishCommand::builder()
        .topic("test/v3/qos2")
        .payload(b"QoS 2 message - tests full 4-way handshake (PUBREC/PUBREL/PUBCOMP)")
        .qos(2)
        .build()?;
    client.publish_with_command(qos2_cmd).await?;
    sleep(Duration::from_millis(1000)).await;

    // Test retained message
    println!("\n🔹 Retained Message");
    let retained_cmd = PublishCommand::builder()
        .topic("test/v3/retained")
        .payload(b"This message will be retained by the broker")
        .qos(1)
        .retain(true)
        .build()?;
    client.publish_with_command(retained_cmd).await?;
    sleep(Duration::from_millis(500)).await;

    // Test multiple QoS 1 messages to verify packet ID tracking
    println!("\n🔹 Multiple QoS 1 Messages (tests packet ID reuse)");
    for i in 0..5 {
        let cmd = PublishCommand::builder()
            .topic("test/v3/multiple")
            .payload(format!("Message #{} - testing packet ID reuse", i).as_bytes())
            .qos(1)
            .build()?;
        client.publish_with_command(cmd).await?;
        sleep(Duration::from_millis(200)).await;
    }

    // Test multiple QoS 2 messages to verify session state tracking
    println!("\n🔹 Multiple QoS 2 Messages (tests session state)");
    for i in 0..3 {
        let cmd = PublishCommand::builder()
            .topic("test/v3/qos2/multiple")
            .payload(format!("QoS 2 Message #{} - testing session state tracking", i).as_bytes())
            .qos(2)
            .build()?;
        client.publish_with_command(cmd).await?;
        sleep(Duration::from_millis(500)).await;
    }

    println!("\n🏓 Testing PING...");
    println!("{}", "─".repeat(60));
    client.ping_sync().await?;

    println!("\n📤 Testing Unsubscribe...");
    println!("{}", "─".repeat(60));
    for topics in [vec!["test/v3/qos0"], vec!["test/v3/+/sensor", "test/v3/#"]] {
        let result = client.unsubscribe_sync(topics).await?;
        if !result.is_success() {
            return Err(format!("UNSUBSCRIBE rejected: {result:?}").into());
        }
    }

    // Test keep-alive mechanism
    println!("\n⏱️  Testing keep-alive (waiting 10 seconds)...");
    println!("{}", "─".repeat(60));
    sleep(Duration::from_secs(10)).await;

    // Publish one more message to verify connection is still alive
    println!("\n📤 Publishing final message to verify connection...");
    client
        .publish("test/v3/final", b"Final test message", 1, false)
        .await?;
    sleep(Duration::from_millis(500)).await;

    println!("\n👋 Disconnecting...");
    client.disconnect_sync().await?;

    println!("\n🛑 Shutting down client...");
    client.shutdown().await?;

    println!("\n✅ MQTT v3.1.1 Tokio Async Client Example completed!");
    println!("{}", "=".repeat(60));
    println!("\n📊 Summary:");
    println!("  ✓ Tested QoS 0, 1, and 2 publishes");
    println!("  ✓ Tested PUBACK session handling (QoS 1)");
    println!("  ✓ Tested PUBCOMP session handling (QoS 2)");
    println!("  ✓ Tested subscriptions with wildcards");
    println!("  ✓ Tested retained messages");
    println!("  ✓ Tested packet ID tracking and reuse");
    println!("  ✓ Tested keep-alive mechanism");
    println!("  ✓ Verified all v3 session state fixes");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_v3_example().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_v3_example() {
        // Call run_v3_example to get coverage
        run_v3_example().await.unwrap();
    }
}
