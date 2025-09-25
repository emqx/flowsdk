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
        Ok(_) => println!("Connected to MQTT broker"),
        Err(e) => eprintln!("Error connecting to MQTT broker: {}", e),
    }

    // Subscribe to a topic
    match client.subscribed("example/topic", 1) {
        Ok(_) => println!("Subscribed to topic"),
        Err(e) => eprintln!("Error subscribing to topic: {}", e),
    }

    match client.published("example/topic", b"Hello, MQTT!", 0, false) {
        Ok(_) => println!("Message Qos0 published"),
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    match client.published("example/topic", b"Hello, MQTT!", 1, false) {
        Ok(_) => println!("Message Qos1 published"),
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    match client.published("example/topic", b"Hello, MQTT!", 2, false) {
        Ok(_) => println!("Message Qos2 published"),
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    for _i in 0..5 {
        std::thread::sleep(std::time::Duration::from_secs(10));
        match client.pingd() {
            Ok(_) => println!("Ping successful"),
            Err(e) => eprintln!("Error sending ping: {}", e),
        }
    }

    match client.published("example/topic", b"Hello, MQTT!", 0, false) {
        Ok(_) => println!("loopback QoS 0 msg received"),
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
        Ok(_) => println!("Unsubscribed from topic"),
        Err(e) => eprintln!("Error unsubscribing from topic: {}", e),
    }

    match client.published("example/topic", b"Hello again, MQTT!", 1, false) {
        Ok(_) => println!("Message Qos1 published"),
        Err(e) => eprintln!("Error publishing message: {}", e),
    }

    match client.disconnected() {
        Ok(_) => println!("Disconnected successfully"),
        Err(e) => eprintln!("Error disconnecting: {}", e),
    }

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
