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

### Timeout Configuration

All sync operations support configurable timeouts to prevent indefinite blocking:

```rust
use flowsdk::mqtt_client::tokio_async_client::TokioAsyncClientConfig;
use flowsdk::mqtt_client::MqttClientError;

// Configure default timeouts for all operations
let config = TokioAsyncClientConfig {
    connect_timeout_ms: Some(30000),        // 30 seconds for CONNECT
    subscribe_timeout_ms: Some(10000),      // 10 seconds for SUBSCRIBE
    publish_ack_timeout_ms: Some(10000),    // 10 seconds for PUBLISH ACK
    unsubscribe_timeout_ms: Some(10000),    // 10 seconds for UNSUBSCRIBE
    ping_timeout_ms: Some(5000),            // 5 seconds for PING
    default_operation_timeout_ms: 30000,    // Default fallback
    ..Default::default()
};

let client = TokioAsyncMqttClient::new(options, handler, config).await?;

// Use default timeouts
match client.connect_sync().await {
    Ok(result) => println!("Connected: {:?}", result),
    Err(MqttClientError::OperationTimeout { operation, timeout_ms }) => {
        eprintln!("{} timed out after {}ms", operation, timeout_ms);
    }
    Err(e) => eprintln!("Error: {}", e),
}

// Or override with custom timeout for specific operation
let result = client.connect_sync_with_timeout(60000).await?; // 60 second timeout
let result = client.subscribe_sync_with_timeout("topic/#", 1, 5000).await?; // 5 second timeout
let result = client.publish_sync_with_timeout("topic", b"data", 2, false, 3000).await?; // 3 second timeout
let result = client.unsubscribe_sync_with_timeout(vec!["topic"], 15000).await?; // 15 second timeout
let result = client.ping_sync_with_timeout(2000).await?; // 2 second timeout
```

**Timeout Behavior:**
- `Some(ms)`: Operation times out after specified milliseconds
- `None`: No timeout, operation waits indefinitely
- On timeout: Returns `MqttClientError::OperationTimeout` with operation name and timeout duration
- Custom timeouts override defaults for individual operations

**Use Cases for Custom Timeouts:**
- **Satellite/High-latency networks**: Longer timeouts (60s+)
- **Time-critical operations**: Shorter timeouts (1-3s)
- **Health checks**: Very short ping timeouts (500ms-2s)
- **Bulk operations**: Extended timeouts for large payloads
- **Development/testing**: Disable timeouts with `None`

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

All sync operations return `Result<T, MqttClientError>` for comprehensive error information:

```rust
use flowsdk::mqtt_client::MqttClientError;

// Handling MqttClientError
match client.connect_sync().await {
    Ok(result) => println!("Connected: {:?}", result),
    Err(MqttClientError::OperationTimeout { operation, timeout_ms }) => {
        eprintln!("⏱️  {} timed out after {}ms", operation, timeout_ms);
        // Consider retrying with longer timeout
    }
    Err(MqttClientError::ConnectionRefused { reason_code, description }) => {
        eprintln!("🚫 Connection refused: {} (code: 0x{:02X})", description, reason_code);
        // Check authentication, authorization, broker config
    }
    Err(MqttClientError::NetworkError { kind, message }) => {
        eprintln!("🌐 Network error ({:?}): {}", kind, message);
        // Check network connectivity
    }
    Err(e) => eprintln!("❌ Error: {}", e.user_message()),
}

// Check if error is recoverable
if error.is_recoverable() {
    // Retry the operation
}

// Check if error should trigger reconnection
if error.should_reconnect() {
    // Initiate reconnection
}

// Convert to io::Error for backward compatibility
let io_result: io::Result<_> = client.connect_sync().await.map_err(Into::into);

// Error handling in event handler
async fn on_error(&mut self, error: &MqttClientError) {
    match error {
        MqttClientError::ConnectionLost { reason } => {
            println!("Connection lost: {}", reason);
        }
        MqttClientError::PublishFailed { packet_id, reason_code, reason_string } => {
            eprintln!("Publish failed: packet_id={:?}, code={}, reason={:?}", 
                     packet_id, reason_code, reason_string);
        }
        _ => eprintln!("Error: {}", error.user_message()),
    }
}
```

**Error Categories:**
- **Timeout Errors**: `OperationTimeout` - operation exceeded configured timeout
- **Connection Errors**: `ConnectionRefused`, `ConnectionLost`, `NotConnected`
- **Network Errors**: `NetworkError` - I/O errors with detailed context
- **Protocol Errors**: `ProtocolViolation`, `PacketParsing`, `UnexpectedPacket`
- **Operation Errors**: `PublishFailed`, `SubscribeFailed`, `UnsubscribeFailed`
- **State Errors**: `InvalidState`, `AlreadyConnected`, `NoActiveSession`
- **Resource Errors**: `BufferFull`, `PacketIdExhausted`

**Backward Compatibility:**
MqttClientError implements `From<MqttClientError>` for `io::Error`, allowing seamless integration with code expecting `io::Result<T>`:
- `OperationTimeout` → `io::ErrorKind::TimedOut`
- `NetworkError` → Original `io::ErrorKind`
- `NotConnected` → `io::ErrorKind::NotConnected`
- Other errors → `io::ErrorKind::Other`

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

1. **Configure Appropriate Timeouts**: Set timeouts based on your network characteristics
   - Local networks: 1-5 seconds
   - Internet/Cloud: 10-30 seconds
   - Satellite/High-latency: 60+ seconds
   - Development: Disable with `None` for easier debugging

2. **Use Sync APIs for Critical Operations**: When you need to ensure delivery before proceeding
   - Use `*_sync()` methods with default timeouts for most cases
   - Use `*_sync_with_timeout()` for operations needing custom timeouts

3. **Handle Timeout Errors Gracefully**: 
   ```rust
   match client.publish_sync(topic, payload, qos, retain).await {
       Err(MqttClientError::OperationTimeout { .. }) => {
           // Retry with longer timeout or fail gracefully
           client.publish_sync_with_timeout(topic, payload, qos, retain, 30000).await?
       }
       result => result
   }
   ```

4. **Event Handler for Business Logic**: Handle incoming messages in `on_message_received`

5. **Builder Pattern for Configuration**: Clean and explicit configuration

6. **Don't wrap in Arc unnecessarily**: The client is already clone-safe via internal `mpsc::Sender`

7. **Handle Reconnection**: Implement `on_connection_lost` and `on_reconnect_attempt` for robust reconnection handling

8. **QoS Selection**: Use QoS 0 for telemetry, QoS 1 for events, QoS 2 for critical commands

9. **Check Error Recoverability**: Use `error.is_recoverable()` to determine retry strategy

10. **Monitor Timeout Patterns**: If operations frequently timeout, consider:
    - Increasing timeout values
    - Checking network quality
    - Verifying broker performance
    - Reducing message size or frequency

## Performance Tips

- Enable `tcp_nodelay` for low-latency (default: enabled)
- Increase buffer sizes for high-throughput scenarios
- Use async APIs when you don't need acknowledgment
- Batch subscriptions using `SubscribeCommand` with multiple topics
- Consider connection pooling for very high message rates

## License

See the repository's LICENSE file for details.