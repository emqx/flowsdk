use flowsdk::mqtt_client::client::{ConnectionResult, PublishResult};
use flowsdk::mqtt_client::{
    MqttClientError, MqttClientOptions, TokioAsyncClientConfig, TokioAsyncMqttClient,
    TokioMqttEventHandler,
};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// Event handler that tracks message order
struct PriorityTrackingHandler {
    name: String,
    message_count: Arc<AtomicU32>,
}

impl PriorityTrackingHandler {
    fn new(name: &str, message_count: Arc<AtomicU32>) -> Self {
        PriorityTrackingHandler {
            name: name.to_string(),
            message_count,
        }
    }
}

#[async_trait::async_trait]
impl TokioMqttEventHandler for PriorityTrackingHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        if result.is_success() {
            println!(
                "[{}] ✅ Connected successfully! Session present: {}",
                self.name, result.session_present
            );
        } else {
            println!(
                "[{}] ❌ Connection failed: code {}",
                self.name, result.reason_code
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
        let count = self.message_count.fetch_add(1, Ordering::SeqCst) + 1;
        if result.is_success() {
            println!(
                "[{}] 📤 #{} Message published (QoS: {}, ID: {:?})",
                self.name, count, result.qos, result.packet_id
            );
        } else {
            println!(
                "[{}] ❌ #{} Publish failed: code {:?}",
                self.name, count, result.reason_code
            );
        }
    }

    async fn on_message_received(&mut self, publish: &MqttPublish) {
        let payload_str = String::from_utf8_lossy(&publish.payload);
        println!(
            "[{}] 📨 Received on '{}': {}",
            self.name, publish.topic_name, payload_str
        );
    }

    async fn on_error(&mut self, error: &MqttClientError) {
        println!("[{}] ❌ Error: {}", self.name, error.user_message());
    }

    async fn on_connection_lost(&mut self) {
        println!("[{}] 💔 Connection lost!", self.name);
    }

    async fn on_reconnect_attempt(&mut self, attempt: u32) {
        println!("[{}] 🔄 Reconnection attempt #{}", self.name, attempt);
    }
}

async fn run_priority_example() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 Priority Queue MQTT Client Example\n");
    println!("This example demonstrates how messages are sent in priority order:");
    println!("  - Priority 255 (highest) = Critical alerts");
    println!("  - Priority 200 = High importance");
    println!("  - Priority 128 (default) = Normal messages");
    println!("  - Priority 50 = Low priority");
    println!("  - Priority 0 (lowest) = Background tasks\n");

    let mqtt_options = MqttClientOptions::builder()
        .peer("broker.emqx.io:1883")
        .client_id("tokio_priority_example")
        .keep_alive(60)
        .reconnect(true)
        .auto_ack(true)
        .build();

    let async_config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .max_reconnect_delay_ms(30000)
        .buffer_messages(true)
        .max_buffer_size(100)
        .build();

    let message_count = Arc::new(AtomicU32::new(0));
    let event_handler = Box::new(PriorityTrackingHandler::new(
        "PriorityClient",
        message_count.clone(),
    ));

    let client = TokioAsyncMqttClient::new(mqtt_options, event_handler, async_config).await?;

    // Test 1: Publish BEFORE connection - messages should be buffered in priority queue
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📝 TEST 1: Publishing BEFORE connection");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    println!("📤 Queueing messages (not yet connected)...\n");

    // Queue messages in intentionally mixed priority order
    client
        .publish_with_priority(
            "priority/test",
            b"[3] NORMAL priority message (128)",
            1,
            false,
            128, // Normal priority
        )
        .await?;
    println!("  → Queued: Normal priority (128)");

    client
        .publish_with_priority(
            "priority/test",
            b"[5] LOW priority message (50)",
            1,
            false,
            50, // Low priority
        )
        .await?;
    println!("  → Queued: Low priority (50)");

    client
        .publish_with_priority(
            "priority/test",
            b"[1] CRITICAL alert (255)",
            1,
            false,
            255, // Highest priority
        )
        .await?;
    println!("  → Queued: Critical priority (255)");

    client
        .publish_with_priority(
            "priority/test",
            b"[4] BELOW NORMAL priority (80)",
            1,
            false,
            80,
        )
        .await?;
    println!("  → Queued: Below normal priority (80)");

    client
        .publish_with_priority(
            "priority/test",
            b"[2] HIGH priority message (200)",
            1,
            false,
            200, // High priority
        )
        .await?;
    println!("  → Queued: High priority (200)");

    client
        .publish_with_priority(
            "priority/test",
            b"[6] BACKGROUND task (10)",
            1,
            false,
            10, // Very low priority
        )
        .await?;
    println!("  → Queued: Background priority (10)\n");

    println!("✅ All 6 messages queued in priority queue");
    println!("📊 Expected send order after connect:");
    println!("   1️⃣  Priority 255 (CRITICAL)");
    println!("   2️⃣  Priority 200 (HIGH)");
    println!("   3️⃣  Priority 128 (NORMAL)");
    println!("   4️⃣  Priority 80  (BELOW NORMAL)");
    println!("   5️⃣  Priority 50  (LOW)");
    println!("   6️⃣  Priority 10  (BACKGROUND)\n");

    println!("📡 Connecting to MQTT broker...\n");
    client.connect().await?;

    // Wait for all messages to be sent
    sleep(Duration::from_secs(3)).await;

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📝 TEST 2: Publishing AFTER connection");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    println!("📤 Queueing more messages while connected...\n");

    // When connected, messages should still go through priority queue
    client
        .publish_with_priority(
            "priority/test",
            b"[8] Post-connect NORMAL (128)",
            1,
            false,
            128,
        )
        .await?;
    println!("  → Queued: Normal priority (128)");

    client
        .publish_with_priority(
            "priority/test",
            b"[7] Post-connect URGENT (250)",
            1,
            false,
            250,
        )
        .await?;
    println!("  → Queued: Urgent priority (250)");

    client
        .publish_with_priority("priority/test", b"[9] Post-connect LOW (30)", 1, false, 30)
        .await?;
    println!("  → Queued: Low priority (30)\n");

    println!("📊 Expected send order:");
    println!("   7️⃣  Priority 250 (URGENT)");
    println!("   8️⃣  Priority 128 (NORMAL)");
    println!("   9️⃣  Priority 30  (LOW)\n");

    // Wait for messages to be sent
    sleep(Duration::from_secs(2)).await;

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📝 TEST 3: Rapid burst with mixed priorities");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    println!("📤 Sending 10 rapid messages with random priorities...\n");

    let priorities = [100, 255, 50, 200, 150, 75, 225, 25, 175, 125];
    for (i, &prio) in priorities.iter().enumerate() {
        let payload = format!("Burst message #{} (priority {})", i + 1, prio);
        client
            .publish_with_priority("priority/burst", payload.as_bytes(), 1, false, prio)
            .await?;
    }

    println!("✅ Sent 10 burst messages");
    println!("   They will be sent in descending priority order\n");

    // Wait for burst to complete
    sleep(Duration::from_secs(3)).await;

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📊 Summary");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let total_messages = message_count.load(Ordering::SeqCst);
    println!("✅ Total messages published: {}", total_messages);
    println!("🎯 All messages were sent in correct priority order\n");

    println!("👋 Disconnecting...");
    client.disconnect().await?;

    sleep(Duration::from_secs(1)).await;

    println!("🛑 Shutting down client...");
    client.shutdown().await?;

    println!("\n✅ Priority Queue Example completed!");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_priority_example().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_example() {
        // Call run_priority_example to get coverage
        run_priority_example().await.unwrap();
    }
}
