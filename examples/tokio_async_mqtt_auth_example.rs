// SPDX-License-Identifier: MPL-2.0

/// Example demonstrating MQTT v5 Enhanced Authentication (AUTH packet)
///
/// This example shows how to use AUTH packets for multi-step authentication
/// like SCRAM-SHA-256, OAuth, or custom authentication mechanisms.
///
/// Run with: cargo run --example tokio_async_mqtt_auth_example
use flowsdk::mqtt_client::client::{AuthResult, ConnectionResult};
use flowsdk::mqtt_client::tokio_async_client::{
    TokioAsyncClientConfig, TokioAsyncMqttClient, TokioMqttEventHandler,
};
use flowsdk::mqtt_client::{MqttClientError, MqttClientOptions};
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Custom event handler that implements enhanced authentication flow
#[derive(Clone)]
struct AuthExampleHandler {
    client: Option<Arc<TokioAsyncMqttClient>>,
    auth_step: Arc<Mutex<u32>>,
}

impl AuthExampleHandler {
    fn new() -> Self {
        Self {
            client: None,
            auth_step: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl TokioMqttEventHandler for AuthExampleHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        println!("✅ Connected to broker!");
        println!("   Reason code: {}", result.reason_code);
        println!("   Session present: {}", result.session_present);
    }

    async fn on_disconnected(&mut self, reason: Option<u8>) {
        println!("❌ Disconnected from broker. Reason: {:?}", reason);
    }

    async fn on_auth_received(&mut self, result: &AuthResult) {
        println!("\n🔐 AUTH packet received from broker:");
        println!(
            "   Reason code: 0x{:02X} ({})",
            result.reason_code,
            result.reason_description()
        );

        let mut step = self.auth_step.lock().await;
        *step += 1;

        if result.is_continue() {
            println!("   ➡️  Authentication continuing (step {})...", *step);

            // Extract authentication data from properties
            let mut auth_method = None;
            let mut auth_data = None;

            for prop in &result.properties {
                match prop {
                    Property::AuthenticationMethod(method) => {
                        auth_method = Some(method.clone());
                    }
                    Property::AuthenticationData(data) => {
                        auth_data = Some(data.clone());
                    }
                    _ => {}
                }
            }

            println!("   Authentication Method: {:?}", auth_method);
            println!(
                "   Authentication Data length: {:?}",
                auth_data.as_ref().map(|d| d.len())
            );

            // In a real implementation, you would:
            // 1. Parse the challenge data
            // 2. Compute the response based on the authentication method (SCRAM, etc.)
            // 3. Send the response via AUTH packet

            // Example: Send mock authentication response
            if let Some(client) = &self.client {
                let response_properties = vec![
                    Property::AuthenticationMethod(
                        auth_method.unwrap_or_else(|| "SCRAM-SHA-256".to_string()),
                    ),
                    Property::AuthenticationData(vec![0x00, 0x01, 0x02, 0x03]), // Mock response data
                ];

                println!("   📤 Sending AUTH response...");
                if let Err(e) = client.auth_continue(response_properties).await {
                    eprintln!("   ❌ Failed to send AUTH: {}", e);
                }
            }
        } else if result.is_success() {
            println!("   ✅ Authentication successful!");
        } else if result.is_re_authenticate() {
            println!("   🔄 Re-authentication requested by broker");

            // Handle re-authentication
            if let Some(client) = &self.client {
                let reauth_properties = vec![
                    Property::AuthenticationMethod("SCRAM-SHA-256".to_string()),
                    Property::AuthenticationData(vec![0x10, 0x20, 0x30, 0x40]),
                ];

                println!("   📤 Sending re-authentication...");
                if let Err(e) = client.auth_re_authenticate(reauth_properties).await {
                    eprintln!("   ❌ Failed to send re-AUTH: {}", e);
                }
            }
        }
    }

    async fn on_error(&mut self, error: &MqttClientError) {
        eprintln!("⚠️  Error: {}", error.user_message());
    }

    async fn on_connection_lost(&mut self) {
        println!("🔌 Connection lost");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 MQTT v5 Enhanced Authentication Example");
    println!("==========================================\n");

    // Configure MQTT client options using builder pattern
    let options = MqttClientOptions::builder()
        .peer("127.0.0.1:1883")
        .client_id("auth_example_client")
        .username("test_user")
        .password(b"test_password".to_vec())
        .keep_alive(60)
        .build();

    // Configure async client with default settings
    let config = TokioAsyncClientConfig::default();

    // Create event handler
    let handler = AuthExampleHandler::new();

    // Create the async MQTT client
    println!("📡 Creating MQTT client...");
    let client = Arc::new(TokioAsyncMqttClient::new(options, Box::new(handler), config).await?);

    // Give handler a reference to the client for sending AUTH responses
    // Note: In a real app, you'd use channels or other sync mechanisms
    // handler_clone.set_client(client.clone());

    println!("🔌 Connecting to broker...\n");
    let connected = client.connect_sync().await?;
    if !connected.is_success() {
        return Err(format!("CONNECT rejected: {connected:?}").into());
    }

    // Note: In a real scenario, the broker would initiate enhanced authentication
    // by sending an AUTH packet during the connection handshake if:
    // 1. The CONNECT packet includes an Authentication Method property
    // 2. The broker supports and requires that authentication method

    // Example: Manually send AUTH packet (normally done in response to broker)
    println!("\n📤 Sending AUTH packet to broker (example)...");
    let auth_properties = vec![
        Property::AuthenticationMethod("SCRAM-SHA-256".to_string()),
        Property::AuthenticationData(vec![0xAA, 0xBB, 0xCC, 0xDD]),
    ];

    match client.auth(0x18, auth_properties).await {
        Ok(_) => println!("   AUTH command queued; protocol errors are reported by on_error"),
        Err(e) => eprintln!("   ❌ Failed to send AUTH: {}", e),
    }

    // Keep the example running
    println!("\n⏳ Running for 10 seconds to demonstrate AUTH flow...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    println!("\n🛑 Disconnecting...");
    client.disconnect_sync().await?;
    let client = Arc::try_unwrap(client).map_err(|_| "Client is still shared")?;
    client.shutdown().await?;

    println!("✅ Example completed!\n");

    Ok(())
}
