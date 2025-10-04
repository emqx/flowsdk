/// Comprehensive example demonstrating all synchronous MQTT operations
///
/// This example shows how to use the sync API for:
/// - connect_sync() - Wait for CONNACK
/// - subscribe_sync() - Wait for SUBACK
/// - publish_sync() - Wait for PUBACK/PUBCOMP
/// - unsubscribe_sync() - Wait for UNSUBACK
/// - ping_sync() - Wait for PINGRESP
///
/// Run with: cargo run --example tokio_async_mqtt_all_sync_operations
use flowsdk::mqtt_client::client::{
    ConnectionResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::tokio_async_client::{
    TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_client::{MqttClientError, MqttClientOptions};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::io;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Event handler that tracks all events
#[derive(Clone)]
struct EventTracker {
    events: Arc<Mutex<Vec<String>>>,
}

impl EventTracker {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn log(&self, msg: String) {
        println!("  [Event] {}", msg);
        self.events.lock().await.push(msg);
    }
}

#[async_trait::async_trait]
impl TokioMqttEventHandler for EventTracker {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        self.log(format!(
            "Connected: reason_code={}, session_present={}",
            result.reason_code, result.session_present
        ))
        .await;
    }

    async fn on_subscribed(&mut self, result: &SubscribeResult) {
        self.log(format!(
            "Subscribed: packet_id={}, reason_codes={:?}",
            result.packet_id, result.reason_codes
        ))
        .await;
    }

    async fn on_published(&mut self, result: &PublishResult) {
        self.log(format!(
            "Published: packet_id={:?}, qos={}",
            result.packet_id, result.qos
        ))
        .await;
    }

    async fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
        self.log(format!(
            "Unsubscribed: packet_id={}, reason_codes={:?}",
            result.packet_id, result.reason_codes
        ))
        .await;
    }

    async fn on_message_received(&mut self, publish: &MqttPublish) {
        let payload = String::from_utf8_lossy(&publish.payload);
        self.log(format!(
            "Message received: topic='{}', payload='{}'",
            publish.topic_name, payload
        ))
        .await;
    }

    async fn on_error(&mut self, error: &MqttClientError) {
        self.log(format!("Error: {}", error.user_message())).await;
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  Tokio Async MQTT Client - All Synchronous Operations Demo  ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    // Create event handler
    let handler = EventTracker::new();
    let handler_clone = handler.clone();

    // Configure MQTT client using builder pattern
    let options = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("tokio_all_sync_ops_client")
        .build();

    let config = TokioAsyncClientConfig::default();

    println!("📦 Creating MQTT client...");
    let client = Arc::new(TokioAsyncMqttClient::new(options, Box::new(handler), config).await?);
    println!("✅ Client created\n");

    // ============================================================
    // 1. CONNECT SYNC - Wait for CONNACK
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 1️⃣  CONNECT SYNC - Wait for CONNACK     │");
    println!("╰─────────────────────────────────────────╯");

    match client.connect_sync().await {
        Ok(result) => {
            println!("✓ Connected successfully!");
            println!("  Reason code: {}", result.reason_code);
            println!("  Session present: {}", result.session_present);
            println!("  Success: {}\n", result.is_success());
        }
        Err(e) => {
            eprintln!("✗ Connection failed: {}\n", e);
            return Err(e);
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // ============================================================
    // 2. SUBSCRIBE SYNC - Wait for SUBACK
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 2️⃣  SUBSCRIBE SYNC - Wait for SUBACK    │");
    println!("╰─────────────────────────────────────────╯");

    let test_topic = "test/sync/demo/#";
    match client.subscribe_sync(test_topic, 1).await {
        Ok(result) => {
            println!("✓ Subscribed to '{}'", test_topic);
            println!("  Packet ID: {}", result.packet_id);
            println!("  Granted QoS: {:?}\n", result.reason_codes);
        }
        Err(e) => {
            eprintln!("✗ Subscribe failed: {}\n", e);
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // ============================================================
    // 3. PUBLISH SYNC - Wait for PUBACK/PUBCOMP
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 3️⃣  PUBLISH SYNC - Wait for ACKs        │");
    println!("╰─────────────────────────────────────────╯");

    // QoS 0 - Fire and forget
    println!("📤 QoS 0 (fire-and-forget):");
    match client
        .publish_sync("test/sync/demo/qos0", b"QoS 0 message", 0, false)
        .await
    {
        Ok(result) => {
            println!("  ✓ Sent immediately (no ack)");
            println!("    Packet ID: {:?}", result.packet_id);
        }
        Err(e) => println!("  ✗ Failed: {}", e),
    }

    // QoS 1 - Wait for PUBACK
    println!("\n📤 QoS 1 (wait for PUBACK):");
    match client
        .publish_sync("test/sync/demo/qos1", b"QoS 1 message", 1, false)
        .await
    {
        Ok(result) => {
            println!("  ✓ PUBACK received!");
            println!("    Packet ID: {:?}", result.packet_id);
            println!("    Reason code: {:?}", result.reason_code);
            println!("    Success: {}", result.is_success());
        }
        Err(e) => println!("  ✗ Failed: {}", e),
    }

    // QoS 2 - Wait for PUBCOMP
    println!("\n📤 QoS 2 (wait for PUBCOMP):");
    match client
        .publish_sync("test/sync/demo/qos2", b"QoS 2 message", 2, false)
        .await
    {
        Ok(result) => {
            println!("  ✓ PUBCOMP received!");
            println!("    Packet ID: {:?}", result.packet_id);
            println!("    Reason code: {:?}", result.reason_code);
            println!("    Success: {}", result.is_success());
        }
        Err(e) => println!("  ✗ Failed: {}", e),
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // ============================================================
    // 4. PING SYNC - Wait for PINGRESP
    // ============================================================
    println!("\n╭─────────────────────────────────────────╮");
    println!("│ 4️⃣  PING SYNC - Wait for PINGRESP       │");
    println!("╰─────────────────────────────────────────╯");

    match client.ping_sync().await {
        Ok(result) => {
            println!("✓ PINGRESP received!");
            println!("  Success: {}\n", result.success);
        }
        Err(e) => {
            eprintln!("✗ Ping failed: {}\n", e);
        }
    }

    // ============================================================
    // 5. PING WITH TIMEOUT
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 5️⃣  PING SYNC with Timeout              │");
    println!("╰─────────────────────────────────────────╯");

    match tokio::time::timeout(tokio::time::Duration::from_secs(5), client.ping_sync()).await {
        Ok(Ok(result)) => {
            println!("✓ Ping completed within timeout");
            println!("  Success: {}\n", result.success);
        }
        Ok(Err(e)) => {
            println!("✗ Ping error: {}\n", e);
        }
        Err(_) => {
            println!("✗ Ping timed out after 5 seconds\n");
        }
    }

    // ============================================================
    // 6. BATCH OPERATIONS
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 6️⃣  BATCH SYNCHRONOUS PUBLISHES         │");
    println!("╰─────────────────────────────────────────╯");

    for i in 1..=5 {
        let topic = format!("test/sync/demo/batch/{}", i);
        let payload = format!("Batch message {}", i);

        match client
            .publish_sync(&topic, payload.as_bytes(), 1, false)
            .await
        {
            Ok(result) => {
                println!(
                    "  ✓ Message {} acknowledged (packet_id: {:?})",
                    i, result.packet_id
                );
            }
            Err(e) => {
                println!("  ✗ Message {} failed: {}", i, e);
            }
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // ============================================================
    // 7. PARALLEL OPERATIONS
    // ============================================================
    println!("\n╭─────────────────────────────────────────╮");
    println!("│ 7️⃣  PARALLEL SYNCHRONOUS PUBLISHES      │");
    println!("╰─────────────────────────────────────────╯");

    let mut handles = vec![];
    for i in 1..=3 {
        let client_clone = client.clone();
        let topic = format!("test/sync/demo/parallel/{}", i);
        let handle = tokio::spawn(async move {
            let payload = format!("Parallel message {}", i);
            client_clone
                .publish_sync(&topic, payload.as_bytes(), 1, false)
                .await
        });
        handles.push((i, handle));
    }

    for (i, handle) in handles {
        match handle.await {
            Ok(Ok(result)) => {
                println!(
                    "  ✓ Parallel message {} completed: packet_id={:?}",
                    i, result.packet_id
                );
            }
            Ok(Err(e)) => {
                println!("  ✗ Parallel message {} failed: {}", i, e);
            }
            Err(e) => {
                println!("  ✗ Parallel message {} task error: {}", i, e);
            }
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // ============================================================
    // 8. UNSUBSCRIBE SYNC - Wait for UNSUBACK
    // ============================================================
    println!("\n╭─────────────────────────────────────────╮");
    println!("│ 8️⃣  UNSUBSCRIBE SYNC - Wait for UNSUBACK│");
    println!("╰─────────────────────────────────────────╯");

    match client.unsubscribe_sync(vec![test_topic]).await {
        Ok(result) => {
            println!("✓ Unsubscribed from '{}'", test_topic);
            println!("  Packet ID: {}", result.packet_id);
            println!("  Reason codes: {:?}\n", result.reason_codes);
        }
        Err(e) => {
            eprintln!("✗ Unsubscribe failed: {}\n", e);
        }
    }

    // ============================================================
    // 9. ERROR HANDLING DEMONSTRATION
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 9️⃣  ERROR HANDLING                      │");
    println!("╰─────────────────────────────────────────╯");

    // Disconnect first
    println!("🔌 Disconnecting...");
    client.disconnect().await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Try to publish after disconnect (should fail gracefully)
    println!("📤 Attempting publish after disconnect:");
    match tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        client.publish_sync("test/after/disconnect", b"Should fail", 1, false),
    )
    .await
    {
        Ok(Ok(result)) => {
            println!("  ⚠️  Unexpected success: {:?}", result);
        }
        Ok(Err(e)) => {
            println!("  ✓ Failed as expected: {}", e);
        }
        Err(_) => {
            println!("  ✓ Timed out as expected");
        }
    }

    // ============================================================
    // 10. SUMMARY
    // ============================================================
    println!("\n╭─────────────────────────────────────────╮");
    println!("│ 📊 EVENT SUMMARY                         │");
    println!("╰─────────────────────────────────────────╯");

    let events = handler_clone.events.lock().await;
    println!("Total events captured: {}", events.len());
    println!("\nRecent events:");
    for (i, event) in events.iter().rev().take(10).enumerate() {
        println!("  {}. {}", i + 1, event);
    }

    println!("\n╔═══════════════════════════════════════════════════════════════╗");
    println!("║              ✅ All operations completed!                     ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");

    Ok(())
}
