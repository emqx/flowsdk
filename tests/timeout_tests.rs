//! Timeout Integration Tests
//!
//! Tests to verify timeout behavior for sync operations in TokioAsyncMqttClient:
//! - Default timeout configuration
//! - Custom timeout overrides
//! - Timeout expiration behavior
//! - Error handling and recovery

use async_trait::async_trait;
use flowsdk::mqtt_client::client::ConnectionResult;
use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_client::tokio_async_client::{
    TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_client::MqttClientError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Simple event handler for testing
#[derive(Clone)]
struct TestEventHandler {
    connected_count: Arc<Mutex<u32>>,
}

impl TestEventHandler {
    fn new() -> Self {
        Self {
            connected_count: Arc::new(Mutex::new(0)),
        }
    }

    fn get_connected_count(&self) -> u32 {
        *self.connected_count.lock().unwrap()
    }
}

#[async_trait]
impl TokioMqttEventHandler for TestEventHandler {
    async fn on_connected(&mut self, _result: &ConnectionResult) {
        let mut count = self.connected_count.lock().unwrap();
        *count += 1;
    }
}

/// Test that connect_sync respects the configured timeout
#[tokio::test]
async fn test_connect_sync_with_default_timeout() {
    // Create a config with a short connect timeout
    let config = TokioAsyncClientConfig::builder()
        .connect_timeout_ms(100) // 100ms timeout
        .build();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("test-connect-timeout")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    // This should either succeed quickly or timeout
    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            let start = std::time::Instant::now();
            let result = client.connect_sync().await;
            let duration = start.elapsed();

            match result {
                Ok(_) => {
                    // Connection succeeded within timeout
                    assert!(
                        duration < Duration::from_millis(150),
                        "Connect should complete within timeout + margin"
                    );
                }
                Err(MqttClientError::OperationTimeout {
                    operation,
                    timeout_ms,
                }) => {
                    // Expected timeout behavior
                    assert_eq!(operation, "connect");
                    assert_eq!(timeout_ms, 100);
                    assert!(
                        duration >= Duration::from_millis(95)
                            && duration <= Duration::from_millis(150),
                        "Timeout should occur around the configured duration, got {:?}",
                        duration
                    );
                }
                Err(e) => {
                    // Other errors (e.g., connection refused) are also acceptable
                    println!("Connect failed with non-timeout error: {}", e);
                }
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}

/// Test custom timeout override for connect_sync
#[tokio::test]
async fn test_connect_sync_with_custom_timeout() {
    let config = TokioAsyncClientConfig::default();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("test-connect-custom-timeout")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            // Use a very short custom timeout (50ms)
            let start = std::time::Instant::now();
            let result = client.connect_sync_with_timeout(50).await;
            let duration = start.elapsed();

            match result {
                Ok(_) => {
                    // Connection succeeded within custom timeout
                    assert!(
                        duration < Duration::from_millis(100),
                        "Connect should complete within custom timeout + margin"
                    );
                }
                Err(MqttClientError::OperationTimeout {
                    operation,
                    timeout_ms,
                }) => {
                    // Expected timeout behavior with custom timeout
                    assert_eq!(operation, "connect");
                    assert_eq!(timeout_ms, 50);
                    assert!(
                        duration >= Duration::from_millis(45)
                            && duration <= Duration::from_millis(100),
                        "Custom timeout should occur around 50ms, got {:?}",
                        duration
                    );
                }
                Err(e) => {
                    println!("Connect failed with non-timeout error: {}", e);
                }
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}

/// Test that successful operations complete before timeout
#[tokio::test]
async fn test_successful_operation_within_timeout() {
    let config = TokioAsyncClientConfig::builder()
        .connect_timeout_ms(5000) // 5 second timeout
        .build();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("test-success-within-timeout")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    match TokioAsyncMqttClient::new(options, Box::new(handler.clone()), config).await {
        Ok(client) => {
            let start = std::time::Instant::now();
            match client.connect_sync().await {
                Ok(result) => {
                    let duration = start.elapsed();

                    // Successful connection should complete well before timeout
                    assert!(
                        duration < Duration::from_secs(5),
                        "Successful connect should not take full timeout duration"
                    );

                    println!("✅ Connected successfully in {:?}", duration);
                    println!("   Reason code: {}", result.reason_code);
                    println!("   Session present: {}", result.session_present);

                    // Verify the event handler was called
                    assert_eq!(handler.get_connected_count(), 1);

                    // Clean disconnect
                    let _ = client.disconnect().await;
                }
                Err(e) => {
                    println!("Connect failed (broker may not be running): {}", e);
                }
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}

/// Test timeout error conversion to io::Error
#[tokio::test]
async fn test_timeout_error_conversion_to_io_error() {
    let config = TokioAsyncClientConfig::builder()
        .connect_timeout_ms(100) // 100ms timeout
        .build();

    let options = MqttClientOptions::builder()
        .peer("192.0.2.1:1883") // TEST-NET-1: Should not be routable
        .client_id("test-timeout-conversion")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            // Test that MqttClientError can be converted to io::Error
            let result: Result<_, std::io::Error> = client.connect_sync().await.map_err(Into::into);

            match result {
                Ok(_) => {
                    println!("Unexpectedly connected");
                }
                Err(io_err) => {
                    // Verify it converted to io::Error
                    println!("Got io::Error: {:?} - {}", io_err.kind(), io_err);

                    // Timeout should map to io::ErrorKind::TimedOut
                    if io_err.to_string().contains("timed out") {
                        assert_eq!(io_err.kind(), std::io::ErrorKind::TimedOut);
                    }
                }
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}

/// Test publish timeout with custom override
#[tokio::test]
async fn test_publish_sync_with_timeout() {
    let config = TokioAsyncClientConfig::default();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("test-publish-timeout")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            // Try to connect first
            if let Ok(_) = client.connect_sync().await {
                // Try publishing with a custom timeout
                let result = client
                    .publish_sync_with_timeout("test/topic", b"test payload", 1, false, 5000)
                    .await;

                match result {
                    Ok(pub_result) => {
                        println!("✅ Published successfully");
                        println!("   Packet ID: {:?}", pub_result.packet_id);
                        println!("   Reason code: {:?}", pub_result.reason_code);
                    }
                    Err(MqttClientError::OperationTimeout {
                        operation,
                        timeout_ms,
                    }) => {
                        assert_eq!(operation, "publish");
                        assert_eq!(timeout_ms, 5000);
                        println!("⏱️  Publish timed out as expected");
                    }
                    Err(e) => {
                        println!("Publish failed: {}", e);
                    }
                }

                let _ = client.disconnect().await;
            } else {
                println!("Could not connect (broker may not be running)");
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}

/// Test subscribe timeout with custom override
#[tokio::test]
async fn test_subscribe_sync_with_timeout() {
    let config = TokioAsyncClientConfig::default();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("test-subscribe-timeout")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            if let Ok(_) = client.connect_sync().await {
                // Try subscribing with a custom timeout
                let result = client
                    .subscribe_sync_with_timeout("test/topic/#", 1, 3000)
                    .await;

                match result {
                    Ok(sub_result) => {
                        println!("✅ Subscribed successfully");
                        println!("   Packet ID: {:?}", sub_result.packet_id);
                        println!("   Reason codes: {:?}", sub_result.reason_codes);
                    }
                    Err(MqttClientError::OperationTimeout {
                        operation,
                        timeout_ms,
                    }) => {
                        assert_eq!(operation, "subscribe");
                        assert_eq!(timeout_ms, 3000);
                        println!("⏱️  Subscribe timed out as expected");
                    }
                    Err(e) => {
                        println!("Subscribe failed: {}", e);
                    }
                }

                let _ = client.disconnect().await;
            } else {
                println!("Could not connect (broker may not be running)");
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}

/// Test ping timeout
#[tokio::test]
async fn test_ping_sync_with_timeout() {
    let config = TokioAsyncClientConfig::default();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("test-ping-timeout")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            if let Ok(_) = client.connect_sync().await {
                // Test ping with custom timeout
                let start = std::time::Instant::now();
                let result = client.ping_sync_with_timeout(2000).await;
                let duration = start.elapsed();

                match result {
                    Ok(_) => {
                        println!("✅ Ping succeeded in {:?}", duration);
                        assert!(duration < Duration::from_secs(2));
                    }
                    Err(MqttClientError::OperationTimeout {
                        operation,
                        timeout_ms,
                    }) => {
                        assert_eq!(operation, "ping");
                        assert_eq!(timeout_ms, 2000);
                        println!("⏱️  Ping timed out as expected");
                    }
                    Err(e) => {
                        println!("Ping failed: {}", e);
                    }
                }

                let _ = client.disconnect().await;
            } else {
                println!("Could not connect (broker may not be running)");
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}

/// Test that None timeout means no timeout (operation waits indefinitely)
#[tokio::test]
async fn test_none_timeout_means_no_timeout() {
    let config = TokioAsyncClientConfig::builder()
        .no_connect_timeout() // No timeout
        .build();

    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("test-no-timeout")
        .clean_start(true)
        .build();

    let handler = TestEventHandler::new();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            // With None timeout, this should wait indefinitely or until success/failure
            // We wrap it in our own timeout to prevent test hanging
            let result = tokio::time::timeout(Duration::from_secs(10), client.connect_sync()).await;

            match result {
                Ok(Ok(_)) => {
                    println!("✅ Connected successfully with no timeout configured");
                    let _ = client.disconnect().await;
                }
                Ok(Err(e)) => {
                    // Should not be OperationTimeout since we set timeout to None
                    match e {
                        MqttClientError::OperationTimeout { .. } => {
                            panic!("Should not get OperationTimeout when timeout is None");
                        }
                        _ => {
                            println!("Connect failed with non-timeout error: {}", e);
                        }
                    }
                }
                Err(_) => {
                    // Our test timeout expired, but client should still be trying
                    println!("Test timeout expired (expected with None timeout config)");
                }
            }
        }
        Err(e) => {
            println!("Failed to create client: {}", e);
        }
    }
}
