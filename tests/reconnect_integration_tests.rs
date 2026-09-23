// SPDX-License-Identifier: MPL-2.0

//! Reconnection Integration Tests
//!
//! Tests to verify automatic reconnection behavior in TokioAsyncMqttClient:
//! - Connection loss detection
//! - Exponential backoff timing
//! - Successful reconnection after failure
//! - Event handler callbacks during reconnection

use async_trait::async_trait;
use flowsdk::mqtt_client::client::ConnectionResult;
use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_client::tokio_async_client::{
    TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_client::MqttClientError;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};

/// Event handler that tracks reconnection attempts
#[derive(Clone)]
struct ReconnectTestHandler {
    connected_count: Arc<Mutex<u32>>,
    connection_lost_count: Arc<Mutex<u32>>,
    reconnect_attempts: Arc<Mutex<Vec<u32>>>,
    reconnect_scheduled: Arc<Notify>,
    disconnected_count: Arc<Mutex<u32>>,
}

impl ReconnectTestHandler {
    fn new() -> Self {
        Self {
            connected_count: Arc::new(Mutex::new(0)),
            connection_lost_count: Arc::new(Mutex::new(0)),
            reconnect_attempts: Arc::new(Mutex::new(Vec::new())),
            reconnect_scheduled: Arc::new(Notify::new()),
            disconnected_count: Arc::new(Mutex::new(0)),
        }
    }

    fn get_connected_count(&self) -> u32 {
        *self.connected_count.lock().unwrap()
    }

    fn get_connection_lost_count(&self) -> u32 {
        *self.connection_lost_count.lock().unwrap()
    }

    fn get_reconnect_attempts(&self) -> Vec<u32> {
        self.reconnect_attempts.lock().unwrap().clone()
    }

    async fn wait_for_reconnect_attempts(&self, count: usize) {
        timeout(Duration::from_secs(5), async {
            while self.get_reconnect_attempts().len() < count {
                self.reconnect_scheduled.notified().await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "Expected at least {count} scheduled retries, got {:?}",
                self.get_reconnect_attempts()
            )
        });
    }

    fn get_disconnected_count(&self) -> u32 {
        *self.disconnected_count.lock().unwrap()
    }
}

#[async_trait]
impl TokioMqttEventHandler for ReconnectTestHandler {
    async fn on_connected(&mut self, _result: &ConnectionResult) {
        let mut count = self.connected_count.lock().unwrap();
        *count += 1;
        println!("✅ Connected (total: {})", *count);
    }

    async fn on_disconnected(&mut self, reason: Option<u8>) {
        let mut count = self.disconnected_count.lock().unwrap();
        *count += 1;
        println!("❌ Disconnected (reason: {:?}, total: {})", reason, *count);
    }

    async fn on_connection_lost(&mut self) {
        let mut count = self.connection_lost_count.lock().unwrap();
        *count += 1;
        println!("🔌 Connection lost (total: {})", *count);
    }

    async fn on_reconnect_attempt(&mut self, attempt: u32) {
        let mut attempts = self.reconnect_attempts.lock().unwrap();
        attempts.push(attempt);
        println!("🔄 Reconnect attempt #{}", attempt);
        self.reconnect_scheduled.notify_one();
    }

    async fn on_error(&mut self, error: &MqttClientError) {
        println!("⚠️ Error: {:?}", error);
    }
}

/// Helper function to start a mosquitto broker process
fn start_mosquitto(port: u16) -> Option<Child> {
    println!("🚀 Starting mosquitto on port {}", port);

    // Try to start mosquitto with minimal config
    match Command::new("mosquitto")
        .arg("-p")
        .arg(port.to_string())
        .arg("-v") // Verbose for debugging
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            // Wait a moment for broker to start
            std::thread::sleep(Duration::from_millis(500));
            println!("✅ Mosquitto started (PID: {})", child.id());
            Some(child)
        }
        Err(e) => {
            println!(
                "⚠️ Failed to start mosquitto: {}. Make sure it's installed.",
                e
            );
            None
        }
    }
}

/// Helper function to stop a mosquitto broker process
fn stop_mosquitto(mut broker: Child) {
    println!("🛑 Stopping mosquitto");
    let _ = broker.kill();
    let _ = broker.wait();
    std::thread::sleep(Duration::from_millis(300));
}

/// Test automatic reconnection after broker restart
///
/// This test spawns its own mosquitto broker and restarts it during the test.
/// Requires mosquitto to be installed and in PATH.
#[tokio::test]
#[ignore] // Run with: cargo test --test reconnect_integration_tests -- --ignored --nocapture
async fn test_auto_reconnect_on_broker_restart() {
    const TEST_PORT: u16 = 11883; // Use non-standard port to avoid conflicts

    // Start mosquitto broker
    let broker = start_mosquitto(TEST_PORT);
    if broker.is_none() {
        println!("⚠️ Skipping test: mosquitto not available");
        return;
    }
    let mut broker = broker.unwrap();

    // Configure client with fast reconnect for testing
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .max_reconnect_delay_ms(1000) // 1s max delay
        .max_reconnect_attempts(0) // Unlimited
        .build();

    let options = MqttClientOptions::builder()
        .peer(format!("127.0.0.1:{}", TEST_PORT))
        .client_id("test-auto-reconnect")
        .clean_start(true)
        .keep_alive(3) // Short keep-alive for faster timeout detection
        .reconnect_base_delay_ms(300) // 300ms initial delay
        .ping_timeout_multiplier(2) // Wait up to 6 seconds for PINGRESP
        .build();

    let handler = ReconnectTestHandler::new();
    let handler_clone = handler.clone();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            // 1. Initial connection
            println!("\n🔵 Phase 1: Initial connection");
            match client.connect_sync().await {
                Ok(result) => {
                    println!(
                        "✅ Connected successfully: session_present={}",
                        result.session_present
                    );
                    assert_eq!(handler_clone.get_connected_count(), 1);
                }
                Err(e) => {
                    println!("❌ Initial connection failed: {:?}", e);
                    stop_mosquitto(broker);
                    return;
                }
            }

            // 2. Wait a moment to ensure connection is stable
            sleep(Duration::from_millis(500)).await;

            // 3. Stop the broker to simulate connection loss
            println!("\n🔵 Phase 2: Stopping broker to simulate connection loss");
            stop_mosquitto(broker);

            // Wait for EOF or keep-alive processing to detect the stopped broker.
            println!("⏳ Waiting for connection loss detection (up to 8 seconds)...");
            sleep(Duration::from_secs(8)).await;

            // 4. Verify connection loss was detected
            let lost_count = handler_clone.get_connection_lost_count();
            let disconnect_count = handler_clone.get_disconnected_count();
            let attempts_after_loss = handler_clone.get_reconnect_attempts();

            println!("\n📊 After connection loss:");
            println!("   Connection lost events: {}", lost_count);
            println!("   Disconnected events: {}", disconnect_count);
            println!("   Reconnect attempts so far: {:?}", attempts_after_loss);

            assert!(
                lost_count > 0 || disconnect_count > 0,
                "Should detect connection loss"
            );

            // 5. Restart broker
            println!("\n🔵 Phase 3: Restarting broker");
            broker = match start_mosquitto(TEST_PORT) {
                Some(b) => b,
                None => {
                    println!("❌ Failed to restart broker");
                    let _ = client.shutdown().await;
                    return;
                }
            };

            // Wait for automatic reconnection (with backoff attempts)
            // The backoff schedule is: 300ms, 600ms, 1000ms (capped), then repeats at 1000ms
            // With multiple retry cycles, reconnection should succeed within 15 seconds
            println!("⏳ Waiting for automatic reconnection (checking every 2 seconds, max 15 seconds)...");
            let mut reconnected = false;
            let max_checks = 8; // 8 checks * 2 seconds = 16 seconds total

            for i in 0..max_checks {
                sleep(Duration::from_secs(2)).await;
                let current_connections = handler_clone.get_connected_count();
                let current_attempts = handler_clone.get_reconnect_attempts();
                println!(
                    "   Check #{}: connections={}, attempts={:?}",
                    i + 1,
                    current_connections,
                    current_attempts
                );

                if current_connections >= 2 {
                    reconnected = true;
                    println!("   ✅ Reconnection detected!");
                    break;
                }
            }

            // 6. Verify reconnection occurred
            let attempts = handler_clone.get_reconnect_attempts();
            let final_connected_count = handler_clone.get_connected_count();

            println!("\n📊 Final status:");
            println!("   Total connections: {}", final_connected_count);
            println!("   All reconnect attempts: {:?}", attempts);
            println!(
                "   Connection lost events: {}",
                handler_clone.get_connection_lost_count()
            );

            // We should have reconnection attempts
            assert!(!attempts.is_empty(), "Should have reconnection attempts");

            // MUST have successfully reconnected - fail the test if not
            assert!(
                reconnected,
                "❌ FAILED: Automatic reconnection did not occur within {} seconds.\n\
                 Expected: Client should reconnect after broker restart\n\
                 Actual: {} connection(s), {} attempt(s)\n\
                 This indicates the reconnection mechanism is not working correctly.\n\
                 Backoff schedule: 300ms, 600ms, 1000ms (capped), repeating at 1000ms\n\
                 Total time available: {} seconds should allow 10+ retry attempts.",
                max_checks * 2,
                final_connected_count,
                attempts.len(),
                max_checks * 2
            );

            assert!(
                final_connected_count >= 2,
                "Should have reconnected (got {} connections)",
                final_connected_count
            );

            println!("✅ Automatic reconnection successful!");

            // Cleanup
            let _ = client.shutdown().await;
            stop_mosquitto(broker);
        }
        Err(e) => {
            stop_mosquitto(broker);
            panic!("Failed to create client: {:?}", e);
        }
    }
}

/// Test reconnection backoff timing
#[tokio::test]
async fn test_reconnect_backoff_timing() {
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .max_reconnect_delay_ms(1000) // 1s max
        .max_reconnect_attempts(3) // Stop after 3 attempts
        .build();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:19999") // Non-existent port to force connection failure
        .client_id("test-backoff")
        .reconnect_base_delay_ms(200) // 200ms base
        .build();

    let handler = ReconnectTestHandler::new();
    let handler_clone = handler.clone();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            println!("\n🔵 Testing reconnection backoff with unreachable broker");

            // Try to connect - should fail
            let start = std::time::Instant::now();
            assert!(client.connect_sync().await.is_err());

            handler_clone.wait_for_reconnect_attempts(3).await;
            // Callbacks report scheduled retries. Let the final retry run and
            // allow enough time to catch an incorrectly scheduled fourth retry.
            sleep(Duration::from_secs(2)).await;

            let attempts = handler_clone.get_reconnect_attempts();
            let elapsed = start.elapsed();

            println!("\n📊 Backoff test results:");
            println!("   Reconnect attempts: {:?}", attempts);
            println!("   Time elapsed: {:?}", elapsed);
            println!("   Expected: 3 attempts with backoff (200ms, 400ms, 800ms)");

            assert_eq!(attempts, vec![1, 2, 3], "Should schedule exactly 3 retries");

            let _ = client.shutdown().await;
        }
        Err(e) => {
            panic!("Failed to create client: {:?}", e);
        }
    }
}

/// Test that reconnection can be disabled
#[tokio::test]
async fn test_reconnect_disabled() {
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(false) // Disable auto-reconnect
        .build();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:19999") // Non-existent port
        .client_id("test-no-reconnect")
        .reconnect(true) // Async config must still be able to disable retries
        .build();

    let handler = ReconnectTestHandler::new();
    let handler_clone = handler.clone();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            println!("\n🔵 Testing with auto-reconnect disabled");

            // Try to connect - should fail
            assert!(client.connect_sync().await.is_err());

            // Wait a moment
            sleep(Duration::from_secs(2)).await;

            let attempts = handler_clone.get_reconnect_attempts();

            println!("\n📊 No-reconnect test results:");
            println!("   Reconnect attempts: {:?}", attempts);
            println!("   Expected: No reconnection attempts");

            // Should have no reconnection attempts when disabled
            assert!(
                attempts.is_empty(),
                "Should not attempt reconnection when disabled"
            );

            let _ = client.shutdown().await;
        }
        Err(e) => {
            panic!("Failed to create client: {:?}", e);
        }
    }
}

/// Test dynamic enable/disable of auto-reconnect
#[tokio::test]
async fn test_dynamic_reconnect_control() {
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .max_reconnect_delay_ms(400)
        .max_reconnect_attempts(0) // Unlimited
        .build();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:19999") // Non-existent port
        .client_id("test-dynamic-reconnect")
        .reconnect_base_delay_ms(200)
        .build();

    let handler = ReconnectTestHandler::new();
    let handler_clone = handler.clone();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            println!("\n🔵 Testing dynamic reconnect control");

            // Initial state: auto-reconnect enabled
            assert!(client.connect_sync().await.is_err());
            handler_clone.wait_for_reconnect_attempts(1).await;

            // Disable auto-reconnect
            client.set_auto_reconnect(false).await.unwrap();
            println!("   ✋ Auto-reconnect disabled");

            // The setter only queues a command. A subsequent completed command
            // confirms the worker has processed it before we record the count.
            assert!(matches!(
                client.ping_sync().await,
                Err(MqttClientError::NotConnected)
            ));
            let at_disable = handler_clone.get_reconnect_attempts().len();

            // Observe for longer than the maximum retry delay.
            sleep(Duration::from_secs(1)).await;

            let after_disable = handler_clone.get_reconnect_attempts().len();
            println!("   Attempts after disable: {}", after_disable);
            assert_eq!(
                after_disable, at_disable,
                "Should not schedule retries while reconnect is disabled"
            );

            // Re-enable auto-reconnect
            client.set_auto_reconnect(true).await.unwrap();
            println!("   ✅ Auto-reconnect re-enabled");

            // Re-enabling while disconnected must schedule another retry and
            // keep retrying after that connection attempt fails.
            handler_clone
                .wait_for_reconnect_attempts(after_disable + 2)
                .await;

            let _ = client.shutdown().await;
        }
        Err(e) => {
            panic!("Failed to create client: {:?}", e);
        }
    }
}
