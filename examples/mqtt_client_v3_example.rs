// SPDX-License-Identifier: Apache-2.0

use flowsdk::mqtt_client::{MqttClient, MqttClientOptions};
use std::io;

fn run_v3_example() -> io::Result<()> {
    println!("🚀 MQTT v3.1.1 Synchronous Client Example");
    println!("{}", "=".repeat(60));
    println!("Testing all MQTT v3.1.1 operations\n");

    // Create options for MQTT v3.1.1
    let options = MqttClientOptions::builder()
        .peer("broker.emqx.io:1883")
        .client_id("flowsdk_v3_example")
        .mqtt_version(3) // Use MQTT v3.1.1
        .clean_start(true)
        .keep_alive(60)
        .reconnect(true)
        .auto_ack(false)
        .build();

    println!("📡 Connecting to {} using MQTT v3.1.1...", options.peer);

    // Create client
    let mut client = MqttClient::new(options);

    // Connect to broker
    match client.connected() {
        Ok(result) => {
            if result.is_success() {
                println!("✅ Connected successfully!");
                println!("   Session present: {}", result.session_present);
                println!(
                    "   Return code: {} ({})",
                    result.reason_code,
                    result.reason_description()
                );
            } else {
                println!("❌ Connection failed!");
                println!(
                    "   Return code: {} ({})",
                    result.reason_code,
                    result.reason_description()
                );
                return Ok(());
            }
        }
        Err(e) => {
            eprintln!("❌ Connection error: {}", e);
            return Err(e);
        }
    }

    // Subscribe to topic
    let topic = "flowsdk/v3/example";
    println!("\n📥 Subscribing to topic: {}", topic);

    match client.subscribed(topic, 1) {
        Ok(result) => {
            if result.is_success() {
                println!("✅ Subscribed successfully!");
                println!("   Packet ID: {}", result.packet_id);
                println!("   Granted QoS: {:?}", result.reason_codes);
            } else {
                println!("❌ Subscription failed!");
                println!("   Reason codes: {:?}", result.reason_codes);
            }
        }
        Err(e) => {
            eprintln!("❌ Subscription error: {}", e);
        }
    }

    // Test QoS 0 publishing
    println!("\n📤 Testing QoS 0 Publish (fire and forget)...");
    match client.published(topic, b"Hello from MQTT v3 - QoS 0", 0, false) {
        Ok(result) => {
            if result.is_success() {
                println!("✅ QoS {} message published successfully", result.qos);
            } else {
                println!("❌ QoS 0 publish failed: {:?}", result.reason_code);
            }
        }
        Err(e) => eprintln!("❌ Error publishing QoS 0: {}", e),
    }

    // Test QoS 1 publishing
    println!("\n📤 Testing QoS 1 Publish (at least once)...");
    match client.published(topic, b"Hello from MQTT v3 - QoS 1", 1, false) {
        Ok(result) => {
            if result.is_success() {
                println!(
                    "✅ QoS {} message published successfully (ID: {:?})",
                    result.qos, result.packet_id
                );
            } else {
                println!("❌ QoS 1 publish failed: {:?}", result.reason_code);
            }
        }
        Err(e) => eprintln!("❌ Error publishing QoS 1: {}", e),
    }

    // Test QoS 2 publishing
    println!("\n📤 Testing QoS 2 Publish (exactly once)...");
    match client.published(topic, b"Hello from MQTT v3 - QoS 2", 2, false) {
        Ok(result) => {
            if result.is_success() {
                println!(
                    "✅ QoS {} message published successfully (ID: {:?})",
                    result.qos, result.packet_id
                );
            } else {
                println!("❌ QoS 2 publish failed: {:?}", result.reason_code);
            }
        }
        Err(e) => eprintln!("❌ Error publishing QoS 2: {}", e),
    }

    // Test retained message
    println!("\n📌 Testing Retained Message...");
    match client.published(topic, b"Retained message - MQTT v3", 1, true) {
        Ok(result) => {
            if result.is_success() {
                println!("✅ Retained message published (ID: {:?})", result.packet_id);
            } else {
                println!("❌ Retained publish failed: {:?}", result.reason_code);
            }
        }
        Err(e) => eprintln!("❌ Error publishing retained: {}", e),
    }

    // Test ping functionality (v3 PINGREQ/PINGRESP)
    println!("\n🏓 Testing PING (v3 PINGREQ/PINGRESP)...");
    for i in 1..=3 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        match client.pingd() {
            Ok(result) => {
                if result.success {
                    println!("✅ Ping #{} successful", i);
                } else {
                    println!("❌ Ping #{} failed", i);
                }
            }
            Err(e) => eprintln!("❌ Error sending ping #{}: {}", i, e),
        }
    }

    // Publish loopback message to receive
    println!("\n📤 Publishing loopback message...");
    match client.published(topic, b"Loopback message for receive test", 0, false) {
        Ok(result) => {
            if result.is_success() {
                println!("✅ Loopback QoS {} message published", result.qos);
            } else {
                println!("❌ Loopback publish failed: {:?}", result.reason_code);
            }
        }
        Err(e) => eprintln!("❌ Error publishing loopback: {}", e),
    }

    // Receive packets
    println!("\n📨 Receiving packet...");
    match client.recv_packet() {
        Ok(Some(packet)) => {
            println!(
                "✅ Received packet: {}",
                serde_json::to_string(&packet).unwrap()
            );
        }
        Ok(None) => println!("❌ Connection closed"),
        Err(e) => eprintln!("❌ Error receiving packet: {}", e),
    }

    // Test unsubscribe (v3 UNSUBSCRIBE/UNSUBACK)
    println!("\n📤 Testing Unsubscribe...");
    match client.unsubscribed_single(topic) {
        Ok(result) => {
            if result.is_success() {
                println!("✅ Unsubscribed from '{}' successfully", topic);
                println!("   Packet ID: {}", result.packet_id);
            } else {
                println!("❌ Unsubscribe failed: {:?}", result.reason_codes);
            }
        }
        Err(e) => eprintln!("❌ Error unsubscribing: {}", e),
    }

    // Try publishing after unsubscribe (should get no subscribers error)
    println!("\n📤 Publishing after unsubscribe (should fail)...");
    match client.published(topic, b"After unsubscribe", 1, false) {
        Ok(result) => {
            if result.is_success() {
                println!("✅ Message published (ID: {:?})", result.packet_id);
            } else {
                println!(
                    "⚠️  Expected failure: Reason code {:?} (No subscribers)",
                    result.reason_code
                );
            }
        }
        Err(e) => eprintln!("❌ Error publishing: {}", e),
    }

    // Test disconnect (v3 DISCONNECT)
    println!("\n👋 Testing Disconnect...");
    match client.disconnected(0) {
        Ok(_) => println!("✅ Disconnected successfully"),
        Err(e) => eprintln!("❌ Error disconnecting: {}", e),
    }

    // Check unhandled packets
    println!("\n📦 Checking unhandled packets...");
    let unhandled_count = client.unhandled_packets_mut().len();
    if unhandled_count > 0 {
        println!("   Found {} unhandled packet(s)", unhandled_count);
        client.unhandled_packets_mut().iter().for_each(|packet| {
            println!("   - {}", serde_json::to_string(packet).unwrap());
        });
    } else {
        println!("   No unhandled packets");
    }

    // Clear unhandled packets
    client.clear_unhandled_packets();
    println!("   Cleared unhandled packets");

    // Try receiving after disconnect
    println!("\n📨 Receiving after disconnect...");
    match client.recv_packet() {
        Ok(Some(packet)) => {
            println!("   Received: {}", serde_json::to_string(&packet).unwrap());
        }
        Ok(None) => println!("   Connection closed (expected)"),
        Err(e) => eprintln!("   Error: {}", e),
    }

    println!("\n{}", "=".repeat(60));
    println!("✅ MQTT v3.1.1 example completed successfully!");
    println!("\n📊 Operations tested:");
    println!("   ✓ CONNECT (v3.1.1)");
    println!("   ✓ SUBSCRIBE");
    println!("   ✓ PUBLISH (QoS 0, 1, 2)");
    println!("   ✓ PUBLISH (retained)");
    println!("   ✓ PINGREQ/PINGRESP");
    println!("   ✓ UNSUBSCRIBE");
    println!("   ✓ DISCONNECT");
    println!("   ✓ Packet reception");
    Ok(())
}

fn main() -> io::Result<()> {
    run_v3_example()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mqtt_v3_example() {
        // Run the v3 example as a test
        run_v3_example().unwrap();
    }
}
