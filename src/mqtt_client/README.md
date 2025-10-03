# Tokio Async MQTT Client

A production-ready, fully-featured MQTT v5 client built on Tokio. This client provides both async (fire-and-forget) and sync (wait-for-acknowledgment) APIs for all MQTT operations.

## Features

- ✅ **MQTT v5 Protocol Support** - Full compliance with MQTT v5 specification
- 🚀 **High Performance** - Built on Tokio for efficient async I/O
- 🔄 **Auto Reconnection** - Configurable automatic reconnection with exponential backoff
- 📦 **Message Buffering** - Queue messages during disconnection
- 🔐 **Enhanced Authentication** - Support for SCRAM, OAuth, and custom auth flows
- ⚡ **Sync & Async APIs** - Choose between fire-and-forget or wait-for-acknowledgment
- 🎯 **Event-Driven** - Flexible event handler trait for all MQTT events
- 📊 **QoS 0, 1, 2** - Full Quality of Service support
- 🔧 **Builder Pattern** - Clean, fluent API for configuration
- 🔒 **Session Management** - Support for persistent and clean sessions

## Quick Start

### Basic Example

```rust
use flowsdk::mqtt_client::{
    MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig, TokioMqttEventHandler
};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::io;

// 1. Create an event handler
#[derive(Clone)]
struct MyHandler;

#[async_trait::async_trait]
impl TokioMqttEventHandler for MyHandler {
    async fn on_message_received(&mut self, publish: &MqttPublish) {
        println!("Message: {}", String::from_utf8_lossy(&publish.payload));
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // 2. Configure the client using builder pattern
    let options = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("my_client")
        .clean_start(true)
        .keep_alive(60)
        .build();

    // 3. Create the client
    let client = TokioAsyncMqttClient::new(
        options,
        Box::new(MyHandler),
        TokioAsyncClientConfig::default()
    ).await?;

    // 4. Connect to broker
    client.connect().await?;

    // 5. Subscribe to topics
    client.subscribe("sensors/+/temperature", 1).await?;

    // 6. Publish messages
    client.publish("sensors/room1/temperature", b"23.5", 1, false).await?;

    Ok(())
}
```

## API Overview

### Async APIs (Fire-and-Forget)

Non-blocking operations that return immediately after queueing the operation:

```rust
// Connect to broker (non-blocking)
client.connect().await?;

// Subscribe to topics
client.subscribe("topic/filter", qos).await?;

// Publish message
client.publish("topic", b"payload", qos, retain).await?;

// Unsubscribe from topics
client.unsubscribe(vec!["topic1", "topic2"]).await?;

// Send ping
client.ping().await?;

// Disconnect
client.disconnect().await?;
```

### Sync APIs (Wait-for-Acknowledgment)

Blocking operations that wait for broker acknowledgment before returning:

```rust
// Connect and wait for CONNACK
let result = client.connect_sync().await?;
println!("Session present: {}", result.session_present);

// Subscribe and wait for SUBACK
let result = client.subscribe_sync("topic/#", 1).await?;
println!("Granted QoS: {:?}", result.reason_codes);

// Publish and wait for PUBACK/PUBCOMP (QoS 1/2)
let result = client.publish_sync("topic", b"data", 2, false).await?;
println!("Published with packet ID: {:?}", result.packet_id);

// Unsubscribe and wait for UNSUBACK
let result = client.unsubscribe_sync(vec!["topic"]).await?;

// Ping and wait for PINGRESP
let result = client.ping_sync().await?;
```

### Advanced Publish/Subscribe Commands

For full control over MQTT v5 features:

```rust
use flowsdk::mqtt_client::tokio_async_client::{PublishCommand, SubscribeCommand};
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;

// Advanced publish with properties
let cmd = PublishCommand::new(
    "topic".to_string(),
    b"payload".to_vec(),
    2,                    // QoS
    false,                // retain
    false,                // dup
    None,                 // packet_id (auto-generated)
    vec![
        Property::MessageExpiryInterval(3600),
        Property::ContentType("application/json".to_string()),
    ]
);
client.publish_with_command(cmd).await?;

// Advanced subscribe with options
let subscription = TopicSubscription::new(
    "topic/#".to_string(),
    2,      // QoS
    false,  // no_local
    true,   // retain_as_published
    0       // retain_handling
);
let cmd = SubscribeCommand::new(None, vec![subscription], vec![]);
client.subscribe_with_command(cmd).await?;
```

## Configuration

### Client Options (MqttClientOptions)

Configure using the builder pattern:

```rust
let options = MqttClientOptions::builder()
    .peer("mqtt.example.com:1883")
    .client_id("unique_client_id")
    .clean_start(true)
    .keep_alive(60)
    .username("user")
    .password("pass")
    .session_expiry_interval(3600)  // Session expires after 1 hour
    .sessionless(false)
    .auto_ack(true)
    .add_subscription_topic("sensors/#", 1)
    .build();
```

**Available Options:**
- `peer(addr)` - Broker address (required)
- `client_id(id)` - Client identifier (required)
- `clean_start(bool)` - Start with clean session (default: true)
- `keep_alive(secs)` - Keep-alive interval in seconds (default: 60)
- `username(user)` - Authentication username
- `password(pass)` - Authentication password
- `will(message)` - Last Will and Testament message
- `reconnect(bool)` - Enable reconnection (default: true)
- `sessionless(bool)` - Run without persistent session
- `session_expiry_interval(secs)` - Session expiry in seconds (MQTT v5)
- `subscription_topics(topics)` - Auto-subscribe on connect
- `auto_ack(bool)` - Automatically acknowledge messages

### Client Configuration (TokioAsyncClientConfig)

Advanced runtime configuration:

```rust
let config = TokioAsyncClientConfig {
    auto_reconnect: true,
    reconnect_delay_ms: 1000,
    max_reconnect_delay_ms: 30000,
    max_reconnect_attempts: 0,      // 0 = infinite
    command_queue_size: 1000,
    buffer_messages: true,
    max_buffer_size: 1000,
    send_buffer_size: 1000,
    recv_buffer_size: 1000,
    keep_alive_interval: 60,
    tcp_nodelay: true,
};
```

## Event Handler

Implement `TokioMqttEventHandler` to handle MQTT events:

```rust
use async_trait::async_trait;

struct MyHandler;

#[async_trait]
impl TokioMqttEventHandler for MyHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        println!("Connected! Session present: {}", result.session_present);
    }

    async fn on_disconnected(&mut self, reason: Option<u8>) {
        println!("Disconnected: {:?}", reason);
    }

    async fn on_message_received(&mut self, publish: &MqttPublish) {
        println!("Message on {}: {:?}", publish.topic_name, publish.payload);
    }

    async fn on_published(&mut self, result: &PublishResult) {
        println!("Published: {:?}", result.packet_id);
    }

    async fn on_subscribed(&mut self, result: &SubscribeResult) {
        println!("Subscribed: {:?}", result.reason_codes);
    }

    async fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
        println!("Unsubscribed: {:?}", result.packet_id);
    }

    async fn on_ping_response(&mut self, result: &PingResult) {
        println!("Ping response received");
    }

    async fn on_error(&mut self, error: &io::Error) {
        eprintln!("Error: {}", error);
    }

    async fn on_connection_lost(&mut self) {
        println!("Connection lost, will attempt reconnect...");
    }

    async fn on_reconnect_attempt(&mut self, attempt: u32) {
        println!("Reconnect attempt #{}", attempt);
    }

    async fn on_auth_received(&mut self, result: &AuthResult) {
        println!("AUTH packet received: {:?}", result);
    }
}
```

## Architecture

### Client Structure

The client consists of two components:

1. **`TokioAsyncMqttClient`** - The public API client
   - Sends commands to worker via mpsc channel
   - Provides async and sync methods
   - Can be cloned cheaply (uses `mpsc::Sender` internally)

2. **`TokioClientWorker`** - Background worker task
   - Handles all I/O operations
   - Manages connection state
   - Processes incoming/outgoing packets
   - Invokes event handler callbacks

### Async Streams

- **Egress Stream**: Buffers outgoing MQTT packets
- **Ingress Stream**: Parses incoming bytes into MQTT packets
- Both use mpsc channels for efficient async communication

### Keep-Alive Mechanism

The client implements MQTT keep-alive correctly:
- Tracks the last control packet **sent** (not just PINGREQ)
- ANY control packet (PUBLISH, SUBSCRIBE, etc.) satisfies keep-alive
- PINGREQ is only sent if no other packet was sent during the period
- Dynamic timer adjusts based on last packet sent time

## Session Management

### Persistent Session

Maintain session state across reconnections:

```rust
let options = MqttClientOptions::builder()
    .client_id("persistent_client")
    .clean_start(false)              // Resume existing session
    .session_expiry_interval(3600)   // Keep session for 1 hour
    .build();
```

### Sessionless Mode

Disable session tracking entirely:

```rust
let options = MqttClientOptions::builder()
    .client_id("sessionless_client")
    .sessionless(true)
    .build();
```

### Clean Session

Start fresh on each connection:

```rust
let options = MqttClientOptions::builder()
    .client_id("clean_client")
    .clean_start(true)
    .build();
```

## Enhanced Authentication (MQTT v5)

Support for multi-step authentication flows:

```rust
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;

// In your event handler
async fn on_auth_received(&mut self, result: &AuthResult) {
    // Continue authentication
    let props = vec![
        Property::AuthenticationMethod("SCRAM-SHA-256".to_string()),
        Property::AuthenticationData(compute_response(&result.data)),
    ];
    
    if let Some(client) = &self.client {
        client.auth_continue(props).await.ok();
    }
}

// Send AUTH to continue authentication
client.auth_continue(properties).await?;

// Send AUTH to re-authenticate
client.auth_re_authenticate(properties).await?;
```

## Error Handling

All operations return `io::Result<T>`:

```rust
use tokio::time::{timeout, Duration};

// With timeout
match timeout(Duration::from_secs(5), client.connect_sync()).await {
    Ok(Ok(result)) => println!("Connected: {:?}", result),
    Ok(Err(e)) => eprintln!("Connection error: {}", e),
    Err(_) => eprintln!("Connection timeout"),
}

// Error handling in event handler
async fn on_error(&mut self, error: &io::Error) {
    match error.kind() {
        io::ErrorKind::ConnectionReset => println!("Connection reset"),
        io::ErrorKind::BrokenPipe => println!("Broken pipe"),
        _ => eprintln!("Error: {}", error),
    }
}
```

## Examples

See the `examples/` directory for complete examples:

- `tokio_async_mqtt_client_example.rs` - Basic usage with all MQTT operations
- `tokio_async_mqtt_all_sync_operations.rs` - Demonstrates all sync APIs
- `tokio_async_mqtt_auth_example.rs` - Enhanced authentication example

Run an example:

```bash
cargo run --example tokio_async_mqtt_client_example
```

## Best Practices

1. **Use Sync APIs for Critical Operations**: When you need to ensure delivery before proceeding
2. **Event Handler for Business Logic**: Handle incoming messages in `on_message_received`
3. **Builder Pattern for Configuration**: Clean and explicit configuration
4. **Don't wrap in Arc unnecessarily**: The client is already clone-safe via internal `mpsc::Sender`
5. **Handle Reconnection**: Implement `on_connection_lost` and `on_reconnect_attempt` for robust reconnection handling
6. **Set Appropriate Timeouts**: Use `tokio::time::timeout` for sync operations
7. **QoS Selection**: Use QoS 0 for telemetry, QoS 1 for events, QoS 2 for critical commands

## Performance Tips

- Enable `tcp_nodelay` for low-latency (default: enabled)
- Increase buffer sizes for high-throughput scenarios
- Use async APIs when you don't need acknowledgment
- Batch subscriptions using `SubscribeCommand` with multiple topics
- Consider connection pooling for very high message rates

## License

See the repository's LICENSE file for details.