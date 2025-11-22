use flowsdk::mqtt_client::{MqttClient, MqttClientOptions};
use std::io;

fn main() -> io::Result<()> {
    println!("MQTT v3.1.1 Client Example");
    println!("==========================\n");

    // Create options for MQTT v3.1.1
    let options = MqttClientOptions::builder()
        .peer("broker.emqx.io:1883")
        .client_id("flowsdk_v3_example")
        .mqtt_version(3) // Use MQTT v3.1.1
        .clean_start(true)
        .keep_alive(60)
        .build();

    println!("Connecting to {} using MQTT v3.1.1...", options.peer);

    // Create client
    let mut client = MqttClient::new(options);

    // Connect to broker
    match client.connected() {
        Ok(result) => {
            if result.is_success() {
                println!("✓ Connected successfully!");
                println!("  Session present: {}", result.session_present);
                println!(
                    "  Reason code: {} ({})",
                    result.reason_code,
                    result.reason_description()
                );
            } else {
                println!("✗ Connection failed!");
                println!(
                    "  Reason code: {} ({})",
                    result.reason_code,
                    result.reason_description()
                );
                return Ok(());
            }
        }
        Err(e) => {
            eprintln!("✗ Connection error: {}", e);
            return Err(e);
        }
    }

    // Subscribe to a topic
    let topic = "flowsdk/v3/test";
    println!("\nSubscribing to topic: {}", topic);

    match client.subscribed(topic, 1) {
        Ok(result) => {
            if result.is_success() {
                println!("✓ Subscribed successfully!");
                println!("  Packet ID: {}", result.packet_id);
                println!("  Granted QoS: {:?}", result.reason_codes);
            } else {
                println!("✗ Subscription failed!");
                println!("  Reason codes: {:?}", result.reason_codes);
            }
        }
        Err(e) => {
            eprintln!("✗ Subscription error: {}", e);
        }
    }

    // Note: publish, disconnect, and other methods need to be updated for v3 support
    // This example demonstrates the basic connection and subscription with MQTT v3.1.1

    println!("\n✓ MQTT v3.1.1 example completed successfully!");
    println!("Note: Full v3 support for publish/disconnect/ping is in progress");

    Ok(())
}
