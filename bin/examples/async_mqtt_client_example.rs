use flowsdk::mqtt_client::client::{
    ConnectionResult, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::{
    AsyncClientConfig, AsyncMqttClient, MqttClientOptions, MqttEventHandler,
};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::io;
use std::time::Duration;

/// Example event handler implementation
struct MyEventHandler {
    client_name: String,
}

impl MyEventHandler {
    fn new(name: &str) -> Self {
        MyEventHandler {
            client_name: name.to_string(),
        }
    }
}

impl MqttEventHandler for MyEventHandler {
    fn on_connected(&mut self, result: &ConnectionResult) {
        if result.is_success() {
            println!(
                "[{}] ✅ Connected successfully! Session present: {}",
                self.client_name, result.session_present
            );
            if let Some(props) = &result.properties {
                println!("[{}] 📋 Broker properties: {:?}", self.client_name, props);
            }
        } else {
            println!(
                "[{}] ❌ Connection failed: {} (code: {})",
                self.client_name,
                result.reason_description(),
                result.reason_code
            );
        }
    }

    fn on_disconnected(&mut self, reason: Option<u8>) {
        match reason {
            Some(0) => println!("[{}] 👋 Disconnected normally", self.client_name),
            Some(code) => println!(
                "[{}] ⚠️ Disconnected with reason code: {}",
                self.client_name, code
            ),
            None => println!("[{}] 💥 Disconnected unexpectedly", self.client_name),
        }
    }

    fn on_published(&mut self, result: &PublishResult) {
        if result.is_success() {
            println!(
                "[{}] 📤 Message published successfully (QoS: {}, ID: {:?})",
                self.client_name, result.qos, result.packet_id
            );
        } else {
            println!(
                "[{}] ❌ Publish failed: {:?}",
                self.client_name, result.reason_code
            );
        }
    }

    fn on_subscribed(&mut self, result: &SubscribeResult) {
        if result.is_success() {
            println!(
                "[{}] 📥 Subscribed successfully! ({} subscriptions)",
                self.client_name,
                result.successful_subscriptions()
            );
        } else {
            println!(
                "[{}] ❌ Subscribe failed: {:?}",
                self.client_name, result.reason_codes
            );
        }
    }

    fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
        if result.is_success() {
            println!("[{}] 📤 Unsubscribed successfully", self.client_name);
        } else {
            println!(
                "[{}] ❌ Unsubscribe failed: {:?}",
                self.client_name, result.reason_codes
            );
        }
    }

    fn on_message_received(&mut self, publish: &MqttPublish) {
        println!(
            "[{}] 📨 Message received on '{}': {:?}",
            self.client_name,
            publish.topic_name,
            String::from_utf8_lossy(&publish.payload)
        );
    }

    fn on_ping_response(&mut self, result: &PingResult) {
        if result.success {
            println!("[{}] 🏓 Ping successful", self.client_name);
        } else {
            println!("[{}] ❌ Ping failed", self.client_name);
        }
    }

    fn on_error(&mut self, error: &io::Error) {
        println!("[{}] ⚠️ Error: {}", self.client_name, error);
    }

    fn on_connection_lost(&mut self) {
        println!(
            "[{}] 💔 Connection lost! Attempting to reconnect...",
            self.client_name
        );
    }

    fn on_reconnect_attempt(&mut self, attempt: u32) {
        println!(
            "[{}] 🔄 Reconnection attempt #{}",
            self.client_name, attempt
        );
    }

    fn on_pending_operations_cleared(&mut self) {
        println!("[{}] 🧹 Pending operations cleared", self.client_name);
    }
}

fn main() -> io::Result<()> {
    println!("🚀 Starting Async MQTT Client Example");

    // Configure MQTT client options
    let mqtt_options = MqttClientOptions {
        peer: "localhost:1883".to_string(),
        client_id: "async_example_client".to_string(),
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        will: None,
        reconnect: false,
        sessionless: false,
        subscription_topics: vec![],
        auto_ack: false,
    };

    // Configure async client settings
    let async_config = AsyncClientConfig {
        auto_reconnect: true,
        reconnect_delay_ms: 1000,
        max_reconnect_delay_ms: 30000,
        max_reconnect_attempts: 5,
        command_queue_size: 100,
        buffer_messages: true,
        max_buffer_size: 1000,
    };

    // Create event handler
    let event_handler = Box::new(MyEventHandler::new("AsyncClient"));

    // Create async MQTT client
    let client = AsyncMqttClient::new(mqtt_options, event_handler, async_config)?;

    println!("📡 Connecting to MQTT broker...");
    client.connect()?;

    // Wait a bit for connection
    //std::thread::sleep(Duration::from_millis(2000));

    println!("📋 Subscribing to topics...");
    client.subscribe("async/test/+", 1)?;
    client.subscribe("async/demo/#", 2)?;

    // Wait a bit for subscriptions
    //std::thread::sleep(Duration::from_millis(1000));

    println!("📤 Publishing test messages...");
    client.publish("async/test/qos0", b"Hello QoS 0!", 0, false)?;
    client.publish("async/test/qos1", b"Hello QoS 1!", 1, false)?;
    client.publish("async/demo/retained", b"Hello Retained!", 1, true)?;

    // Wait for message processing
    //std::thread::sleep(Duration::from_millis(2000));

    println!("🏓 Sending ping...");
    client.ping()?;

    // Wait for ping response
    //std::thread::sleep(Duration::from_millis(1000));

    println!("📤 Unsubscribing from topics...");
    client.unsubscribe(vec!["async/test/+", "async/demo/#"])?;

    // Wait for unsubscription
    //std::thread::sleep(Duration::from_millis(1000));

    println!("👋 Disconnecting...");
    client.disconnect()?;

    // Wait for disconnect
    //std::thread::sleep(Duration::from_millis(1000));

    println!("🛑 Shutting down client...");
    client.shutdown()?;

    println!("✅ Example completed successfully!");
    Ok(())
}
