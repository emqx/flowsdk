use flowsdk::mqtt_client::client::ConnectionResult;
use flowsdk::mqtt_client::tokio_async_client::{
    TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_client::MqttClientOptions;
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::io;
use std::sync::Arc;

/// Simple event handler for the example
#[derive(Clone)]
struct BuilderExampleHandler;

#[async_trait::async_trait]
impl TokioMqttEventHandler for BuilderExampleHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        println!(
            "✅ Connected to broker - reason_code: {}, session_present: {}",
            result.reason_code, result.session_present
        );
    }

    async fn on_message_received(&mut self, publish: &MqttPublish) {
        println!(
            "📨 Message received on '{}': {}",
            publish.topic_name,
            String::from_utf8_lossy(&publish.payload)
        );
    }

    async fn on_error(&mut self, error: &io::Error) {
        eprintln!("⚠️  Error: {}", error);
    }
}
#[allow(clippy::field_reassign_with_default)]
#[tokio::main]
async fn main() -> io::Result<()> {
    println!("🚀 MQTT Client Options Builder Pattern Examples\n");
    println!("{}", "=".repeat(60));

    // Example 1: Minimal configuration with defaults
    println!("\n📋 Example 1: Minimal Configuration");
    println!("{}", "-".repeat(60));
    let options_minimal = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("builder_minimal_client")
        .build();

    println!("Peer: {}", options_minimal.peer);
    println!("Client ID: {}", options_minimal.client_id);
    println!("Keep Alive: {} seconds", options_minimal.keep_alive);
    println!("Clean Start: {}", options_minimal.clean_start);
    println!("Auto ACK: {}", options_minimal.auto_ack);

    // Example 2: Configuration with authentication
    println!("\n📋 Example 2: With Authentication");
    println!("{}", "-".repeat(60));
    let options_auth = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("builder_auth_client")
        .username("mqtt_user")
        .password(b"secret_password".to_vec())
        .build();

    println!("Peer: {}", options_auth.peer);
    println!("Client ID: {}", options_auth.client_id);
    println!("Username: {:?}", options_auth.username);
    println!("Password: [REDACTED]");

    // Example 3: Configuration with session expiry
    println!("\n📋 Example 3: With Session Expiry (1 hour)");
    println!("{}", "-".repeat(60));
    let options_session = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("builder_session_client")
        .clean_start(false)
        .session_expiry_interval(3600) // 1 hour
        .build();

    println!("Peer: {}", options_session.peer);
    println!("Client ID: {}", options_session.client_id);
    println!("Clean Start: {}", options_session.clean_start);
    println!(
        "Session Expiry: {:?} seconds",
        options_session.session_expiry_interval
    );

    // Example 4: Full featured configuration
    println!("\n📋 Example 4: Full Featured Configuration");
    println!("{}", "-".repeat(60));
    let options_full = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("builder_full_client")
        .username("admin")
        .password(b"admin123".to_vec())
        .clean_start(false)
        .keep_alive(120)
        .reconnect(true)
        .auto_ack(true)
        .sessionless(false)
        .session_expiry_interval(7200) // 2 hours
        .build();

    println!("Peer: {}", options_full.peer);
    println!("Client ID: {}", options_full.client_id);
    println!("Username: {:?}", options_full.username);
    println!("Keep Alive: {} seconds", options_full.keep_alive);
    println!("Clean Start: {}", options_full.clean_start);
    println!("Reconnect: {}", options_full.reconnect);
    println!("Auto ACK: {}", options_full.auto_ack);
    println!("Sessionless: {}", options_full.sessionless);
    println!(
        "Session Expiry: {:?} seconds",
        options_full.session_expiry_interval
    );

    // Example 5: Using default and then modifying
    println!("\n📋 Example 5: Using Default + Modifications");
    println!("{}", "-".repeat(60));
    let mut options_default = MqttClientOptions::default();
    options_default.peer = "localhost:8883".to_string();
    options_default.client_id = "modified_default_client".to_string();

    println!("Peer: {}", options_default.peer);
    println!("Client ID: {}", options_default.client_id);
    println!("Keep Alive: {} seconds", options_default.keep_alive);

    // Example 6: Actually create and connect a client
    println!("\n📋 Example 6: Real Client Connection (if broker available)");
    println!("{}", "-".repeat(60));

    let options = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("builder_example_working_client")
        .clean_start(true)
        .keep_alive(60)
        .reconnect(false)
        .auto_ack(true)
        .build();

    let handler = BuilderExampleHandler;
    let config = TokioAsyncClientConfig::default();

    match TokioAsyncMqttClient::new(options, Box::new(handler), config).await {
        Ok(client) => {
            let client = Arc::new(client);
            println!("✅ Client created successfully");

            // Try to connect (will only work if broker is running)
            match client.connect_sync().await {
                Ok(result) => {
                    println!("✅ Connected successfully!");
                    println!("   Reason Code: {}", result.reason_code);
                    println!("   Session Present: {}", result.session_present);

                    // Disconnect
                    if let Err(e) = client.disconnect().await {
                        eprintln!("⚠️  Error disconnecting: {}", e);
                    }
                }
                Err(e) => {
                    println!(
                        "ℹ️  Could not connect to broker (expected if not running): {}",
                        e
                    );
                }
            }
        }
        Err(e) => {
            println!("ℹ️  Could not create client: {}", e);
        }
    }

    println!("\n{}", "=".repeat(60));
    println!("✅ Builder pattern examples completed!");

    Ok(())
}
