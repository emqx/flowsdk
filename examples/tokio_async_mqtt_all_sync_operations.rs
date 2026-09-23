// SPDX-License-Identifier: MPL-2.0

/// Comprehensive example demonstrating all synchronous MQTT operations
///
/// This example shows how to use the sync API for:
/// - connect_sync() - Wait for CONNACK
/// - subscribe_sync() - Wait for SUBACK
/// - publish_sync() - Wait for PUBACK/PUBCOMP
/// - unsubscribe_sync() - Wait for UNSUBACK
/// - ping_sync() - Wait for PINGRESP
/// - disconnect_sync() / shutdown() - Close the transport and join the worker
///
/// Run with: cargo run --example tokio_async_mqtt_all_sync_operations -- [host:port]
use flowsdk::mqtt_client::client::{
    ConnectionResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::tokio_async_client::{
    TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_client::{MqttClientError, MqttClientOptions};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  Tokio Async MQTT Client - All Synchronous Operations Demo  ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    // Create event handler
    let handler = EventTracker::new();
    let handler_clone = handler.clone();

    // Configure MQTT client using builder pattern
    let options = MqttClientOptions::builder()
        .peer(
            std::env::args()
                .nth(1)
                .unwrap_or_else(|| "localhost:1883".into()),
        )
        .client_id("tokio_all_sync_ops_client")
        .build();

    let config = TokioAsyncClientConfig::default();

    println!("📦 Creating MQTT client...");
    let client = Arc::new(TokioAsyncMqttClient::new(options, Box::new(handler), config).await?);
    println!("✅ Client created\n");

    let result = run_operations(&client, &handler_clone).await;
    // All spawned operations have been joined; release Arc before consuming the client.
    let client = Arc::try_unwrap(client).map_err(|_| "Client is still shared")?;
    let shutdown = client.shutdown().await;
    result?;
    shutdown?;
    Ok(())
}

async fn run_operations(
    client: &Arc<TokioAsyncMqttClient>,
    handler: &EventTracker,
) -> Result<(), Box<dyn std::error::Error>> {
    // ============================================================
    // 1. CONNECT SYNC - Wait for CONNACK
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 1️⃣  CONNECT SYNC - Wait for CONNACK     │");
    println!("╰─────────────────────────────────────────╯");

    match client.connect_sync().await {
        Ok(result) => {
            if !result.is_success() {
                return Err(format!("CONNECT rejected: {result:?}").into());
            }
            println!("✓ Connected successfully!");
            println!("  Reason code: {}", result.reason_code);
            println!("  Session present: {}", result.session_present);
            println!("  Success: {}\n", result.is_success());
        }
        Err(e) => {
            eprintln!("✗ Connection failed: {}\n", e);
            return Err(e.into());
        }
    }

    // ============================================================
    // 2. SUBSCRIBE SYNC - Wait for SUBACK
    // ============================================================
    println!("╭─────────────────────────────────────────╮");
    println!("│ 2️⃣  SUBSCRIBE SYNC - Wait for SUBACK    │");
    println!("╰─────────────────────────────────────────╯");

    let test_topic = "test/sync/demo/#";
    match client.subscribe_sync(test_topic, 1).await {
        Ok(result) => {
            if !result.is_success() {
                return Err(format!("SUBSCRIBE rejected: {result:?}").into());
            }
            println!("✓ Subscribed to '{}'", test_topic);
            println!("  Packet ID: {}", result.packet_id);
            println!("  Granted QoS: {:?}\n", result.reason_codes);
        }
        Err(e) => {
            eprintln!("✗ Subscribe failed: {}\n", e);
        }
    }

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
            println!("  ✓ Accepted into the output queue (no broker acknowledgement)");
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
            if !result.is_success() {
                return Err(format!("PUBLISH rejected: {result:?}").into());
            }
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
            if !result.is_success() {
                return Err(format!("PUBLISH rejected: {result:?}").into());
            }
            println!("  ✓ PUBCOMP received!");
            println!("    Packet ID: {:?}", result.packet_id);
            println!("    Reason code: {:?}", result.reason_code);
            println!("    Success: {}", result.is_success());
        }
        Err(e) => println!("  ✗ Failed: {}", e),
    }

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
                if !result.is_success() {
                    return Err(format!("PUBLISH rejected: {result:?}").into());
                }
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

    // ============================================================
    // 7. PARALLEL OPERATIONS
    // ============================================================
    println!("\n╭─────────────────────────────────────────╮");
    println!("│ 7️⃣  PARALLEL SYNCHRONOUS PUBLISHES      │");
    println!("╰─────────────────────────────────────────╯");

    let mut handles = vec![];
    for i in 1..=3 {
        let client_clone = Arc::clone(client);
        let topic = format!("test/sync/demo/parallel/{}", i);
        let handle = tokio::spawn(async move {
            let payload = format!("Parallel message {}", i);
            client_clone
                .publish_sync(&topic, payload.as_bytes(), 1, false)
                .await
        });
        handles.push((i, handle));
    }

    let mut parallel_error = None;
    for (i, handle) in handles {
        match handle.await {
            Ok(Ok(result)) if result.is_success() => {
                println!(
                    "  ✓ Parallel message {} completed: packet_id={:?}",
                    i, result.packet_id
                );
            }
            Ok(Ok(result)) => {
                parallel_error = Some(format!("Parallel PUBLISH rejected: {result:?}"));
            }
            Ok(Err(e)) => {
                println!("  ✗ Parallel message {} failed: {}", i, e);
                parallel_error = Some(e.to_string());
            }
            Err(e) => {
                println!("  ✗ Parallel message {} task error: {}", i, e);
                parallel_error = Some(e.to_string());
            }
        }
    }

    if let Some(error) = parallel_error {
        return Err(error.into());
    }

    // ============================================================
    // 8. UNSUBSCRIBE SYNC - Wait for UNSUBACK
    // ============================================================
    println!("\n╭─────────────────────────────────────────╮");
    println!("│ 8️⃣  UNSUBSCRIBE SYNC - Wait for UNSUBACK│");
    println!("╰─────────────────────────────────────────╯");

    match client.unsubscribe_sync(vec![test_topic]).await {
        Ok(result) => {
            if !result.is_success() {
                return Err(format!("UNSUBSCRIBE rejected: {result:?}").into());
            }
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
    client.disconnect_sync().await?;

    // Try to publish after disconnect (should fail gracefully)
    println!("📤 Attempting publish after disconnect:");
    match client
        .publish_sync("test/after/disconnect", b"Should fail", 1, false)
        .await
    {
        Err(MqttClientError::NotConnected) => {
            println!("  ✓ Returned NotConnected as expected");
        }
        other => return Err(format!("Expected NotConnected, got {other:?}").into()),
    }

    // ============================================================
    // 10. SUMMARY
    // ============================================================
    println!("\n╭─────────────────────────────────────────╮");
    println!("│ 📊 EVENT SUMMARY                         │");
    println!("╰─────────────────────────────────────────╯");

    let events = handler.events.lock().await;
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
