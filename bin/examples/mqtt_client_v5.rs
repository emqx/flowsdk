use flowsdk::mqtt_client::{MqttClient, MqttClientOptions};

fn main() {
    let opts: MqttClientOptions = MqttClientOptions {
        client_id: "example_client".to_string(),
        clean_start: true,
        keep_alive: 10,
        username: None,
        password: None,
        will: None,
        reconnect: true,
        sessionless: false,
        subscription_topics: vec![],
        auto_ack: false,
    };
    // Example usage of MqttClient
    let mut client = MqttClient::new("localhost:1883".to_string(), opts);

    match client.connected() {
        Ok(result) => {
            if result.is_success() {
                println!(
                    "Connected to MQTT broker successfully! Session present: {}",
                    result.session_present
                );
                if let Some(props) = &result.properties {
                    println!("Broker properties: {:?}", props);
                }
            } else {
                println!(
                    "Connection failed: {} (code: {})",
                    result.reason_description(),
                    result.reason_code
                );
            }
        }
        Err(e) => eprintln!("Error connecting to MQTT broker: {}", e),
    }

    // Subscribe to a topic
    match client.subscribed("example/topic", 1) {
        Ok(result) => {
            if result.is_success() {
                println!(
                    "Subscribed to topic successfully! ({} successful subscriptions)",
                    result.successful_subscriptions()
                );
            } else {
                println!(
                    "Subscription failed. Reason codes: {:?}",
                    result.reason_codes
                );
            }
        }
        Err(e) => eprintln!("Error subscribing to topic: {}", e),
    }

    match client.published("example/topic", b"Hello, MQTT 0!", 0, false) {
        Ok(result) => {
            if result.is_success() {
                println!("Message QoS {} published successfully", result.qos);
            } else {
                println!(
                    "Message publish failed. Reason code: {:?}",
                    result.reason_code
                );
            }
        }
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    match client.published("example/topic", b"Hello, MQTT 1!", 1, false) {
        Ok(result) => {
            if result.is_success() {
                println!(
                    "Message QoS {} published successfully (ID: {:?})",
                    result.qos, result.packet_id
                );
            } else {
                println!(
                    "Message publish failed. Reason code: {:?}",
                    result.reason_code
                );
            }
        }
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    match client.published("example/topic", b"Hello, MQTT 2!", 2, false) {
        Ok(result) => {
            if result.is_success() {
                println!(
                    "Message QoS {} published successfully (ID: {:?})",
                    result.qos, result.packet_id
                );
            } else {
                println!(
                    "Message publish failed. Reason code: {:?}",
                    result.reason_code
                );
            }
        }
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    for _i in 0..5 {
        std::thread::sleep(std::time::Duration::from_secs(10));
        match client.pingd() {
            Ok(result) => {
                if result.success {
                    println!("Ping successful");
                } else {
                    println!("Ping failed");
                }
            }
            Err(e) => eprintln!("Error sending ping: {}", e),
        }
    }

    match client.published("example/topic", b"Hello, MQTT!", 0, false) {
        Ok(result) => {
            if result.is_success() {
                println!("loopback QoS {} msg published successfully", result.qos);
            } else {
                println!(
                    "loopback msg publish failed. Reason code: {:?}",
                    result.reason_code
                );
            }
        }
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    match client.recv_packet() {
        Ok(Some(packet)) => println!(
            "Received packet: {}",
            serde_json::to_string(&packet).unwrap()
        ),
        Ok(None) => println!("Connection Closed"),
        Err(e) => eprintln!("Error receiving packet: {}", e),
    }

    match client.unsubscribed_single("example/topic") {
        Ok(result) => {
            if result.is_success() {
                println!("Unsubscribed from topic successfully");
            } else {
                println!(
                    "Unsubscribe failed. Reason codes: {:?}",
                    result.reason_codes
                );
            }
        }
        Err(e) => eprintln!("Error unsubscribing from topic: {}", e),
    }

    match client.published("example/topic", b"Hello again, MQTT!", 1, false) {
        Ok(result) => {
            if result.is_success() {
                println!(
                    "Message QoS {} published successfully (ID: {:?})",
                    result.qos, result.packet_id
                );
            } else {
                println!(
                    "Message publish failed. Reason code: {:?}",
                    result.reason_code
                );
            }
        }
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    match client.disconnected() {
        Ok(_) => println!("Disconnected successfully"),
        Err(e) => eprintln!("Error disconnecting: {}", e),
    }

    client.unhandled_packets_mut().iter().for_each(|packet| {
        println!(
            "Unhandled packet: {}",
            serde_json::to_string(packet).unwrap()
        )
    });

    client.clear_unhandled_packets();

    client.unhandled_packets_mut().iter().for_each(|packet| {
        println!(
            "Unhandled packet: {}",
            serde_json::to_string(packet).unwrap()
        )
    });
    
    match client.recv_packet() {
        Ok(Some(packet)) => println!(
            "Received packet: {}",
            serde_json::to_string(&packet).unwrap()
        ),
        Ok(None) => {
            println!("Connection Closed without receiving any packets due to unsubscribed topics")
        }
        Err(e) => eprintln!("Error receiving packet: {}", e),
    }
}
