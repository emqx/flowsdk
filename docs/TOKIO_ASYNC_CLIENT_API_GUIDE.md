# TokioAsyncMqttClient API Guide

**Version**: 1.0  
**Last Updated**: October 10, 2025

## Overview

`TokioAsyncMqttClient` is a fully asynchronous, production-ready MQTT v5.0 client built on Tokio. It provides both fire-and-forget async APIs and wait-for-acknowledgment sync APIs with configurable timeouts.

### Key Features
- ✅ Full MQTT v5.0 protocol support
- ✅ Advanced subscription options (No Local, Retain Handling, Retain As Published)
- ✅ SubscribeCommand builder pattern for complex subscriptions
- ✅ Async/await based on Tokio runtime
- ✅ Event-driven architecture with callbacks
- ✅ Automatic reconnection with exponential backoff
- ✅ Configurable operation timeouts
- ✅ Message buffering during disconnection
- ✅ QoS 0, 1, and 2 support
- ✅ Flow control (Receive Maximum, Topic Alias Maximum)
- ✅ Thread-safe and clone-friendly
- ✅ Raw Packet API for protocol compliance testing (feature-gated)

---

## Client Creation

### Basic Initialization

```rust
use flowsdk::mqtt_client::{
    MqttClientOptions,
    TokioAsyncClientConfig,
    TokioAsyncMqttClient,
    TokioMqttEventHandler,
};

// 1. Create MQTT connection options
let options = MqttClientOptions::builder()
    .peer("mqtt.example.com:1883")
    .client_id("my_client")
    .username("user")
    .password(b"password".to_vec())
    .clean_start(true)
    .keep_alive(60)
    .maximum_packet_size(268435455)  // Optional: MQTT v5 max packet size
    .request_response_information(true)  // Optional: MQTT v5 request/response
    .request_problem_information(true)   // Optional: MQTT v5 problem info
    .build();

// 2. Create event handler (implements TokioMqttEventHandler trait)
let handler = Box::new(MyEventHandler::new());

// 3. Create client configuration
let config = TokioAsyncClientConfig::builder()
    .auto_reconnect(true)
    .internet_timeouts()  // Preset timeout profile
    .receive_maximum(100)  // Optional: MQTT v5 flow control
    .topic_alias_maximum(10)  // Optional: MQTT v5 topic aliases
    .build();

// 4. Create the client
let client = TokioAsyncMqttClient::new(options, handler, config).await?;
```

### Configuration Profiles

```rust
// Local network (fast timeouts)
let config = TokioAsyncClientConfig::builder()
    .local_network_timeouts()  // 2-5 second timeouts
    .build();

// Internet/Cloud (default)
let config = TokioAsyncClientConfig::builder()
    .internet_timeouts()  // 5-30 second timeouts
    .build();

// Satellite/High-latency
let config = TokioAsyncClientConfig::builder()
    .satellite_timeouts()  // 30-120 second timeouts
    .build();

// Development (no timeouts)
let config = TokioAsyncClientConfig::builder()
    .no_timeouts()
    .auto_reconnect(false)
    .build();
```

### MQTT v5 Configuration Options

#### Client Options (MqttClientOptions)

**Maximum Packet Size**:
```rust
let options = MqttClientOptions::builder()
    .peer("mqtt.example.com:1883")
    .maximum_packet_size(1048576)  // 1MB max packet size
    .build();

// Or disable limit
let options = MqttClientOptions::builder()
    .no_maximum_packet_size()
    .build();
```

**Request/Response Information**:
```rust
let options = MqttClientOptions::builder()
    .request_response_information(true)  // Request response topic from broker
    .request_problem_information(true)   // Request detailed error info
    .build();
```

#### Client Configuration (TokioAsyncClientConfig)

**Receive Maximum** (Flow Control):
```rust
let config = TokioAsyncClientConfig::builder()
    .receive_maximum(100)  // Max 100 concurrent QoS 1/2 messages
    .build();

// Or use default (65535)
let config = TokioAsyncClientConfig::builder()
    .no_receive_maximum()
    .build();
```

**Topic Alias Maximum**:
```rust
let config = TokioAsyncClientConfig::builder()
    .topic_alias_maximum(10)  // Accept up to 10 topic aliases from server
    .build();

// Or disable topic aliases
let config = TokioAsyncClientConfig::builder()
    .no_topic_alias()  // Set to 0
    .build();
```

---

## API Categories

The client provides two types of operations:

### 1. Async APIs (Fire-and-Forget)
- Return immediately after queuing the operation
- Do not wait for broker acknowledgment
- Lower latency, higher throughput
- **Use when**: You don't need confirmation, or handle confirmations in event callbacks

### 2. Sync APIs (Wait-for-Acknowledgment)
- Block until broker sends acknowledgment (or timeout)
- Return result with packet ID and reason codes
- Higher latency, guaranteed delivery confirmation
- **Use when**: You need to ensure operation succeeded before proceeding

---

## Core Operations

### Connection Management

#### `connect()` - Async Connect
```rust
pub async fn connect(&self) -> io::Result<()>
```

**Description**: Initiates connection to broker (fire-and-forget).  
**Returns**: `Ok(())` if queued successfully  
**Acknowledgment**: Via `on_connected()` event callback  
**Use case**: When you'll handle connection result in event handler

**Example**:
```rust
client.connect().await?;
// Connection result arrives in on_connected() callback
```

---

#### `connect_sync()` - Sync Connect
```rust
pub async fn connect_sync(&self) -> Result<ConnectionResult, MqttClientError>
```

**Description**: Connects to broker and waits for CONNACK.  
**Returns**: `ConnectionResult` with session info and properties  
**Timeout**: Configured via `connect_timeout_ms` (default: 30 seconds)  
**Use case**: When you need to ensure connection before proceeding

**Example**:
```rust
match client.connect_sync().await {
    Ok(result) => {
        println!("Connected! Session present: {}", result.session_present);
        println!("Reason code: {}", result.reason_code);
    }
    Err(MqttClientError::OperationTimeout { operation, timeout_ms }) => {
        eprintln!("Connection timed out after {}ms", timeout_ms);
    }
    Err(e) => eprintln!("Connection failed: {}", e),
}
```

**ConnectionResult fields**:
- `reason_code: u8` - 0 = success, non-zero = error
- `session_present: bool` - Whether broker has existing session
- `properties: Option<Vec<Property>>` - MQTT v5 properties from broker
- `assigned_client_id: Option<String>` - Server-assigned client ID (if any)

---

#### `connect_sync_with_timeout()` - Sync Connect with Custom Timeout
```rust
pub async fn connect_sync_with_timeout(
    &self,
    timeout_ms: u64
) -> Result<ConnectionResult, MqttClientError>
```

**Description**: Like `connect_sync()` but with custom timeout override.  
**Use case**: Networks requiring longer/shorter timeouts than default

**Example**:
```rust
// 60 second timeout for satellite network
let result = client.connect_sync_with_timeout(60000).await?;
```

---

#### `disconnect()` - Graceful Disconnect
```rust
pub async fn disconnect(&self) -> io::Result<()>
```

**Description**: Sends DISCONNECT packet to broker and closes connection.  
**Returns**: `Ok(())` if disconnect initiated successfully  
**Acknowledgment**: Via `on_disconnected()` event callback

**Example**:
```rust
client.disconnect().await?;
```

---

#### `shutdown()` - Client Shutdown
```rust
pub async fn shutdown(&self) -> io::Result<()>
```

**Description**: Shuts down client worker task and releases resources.  
**Use case**: Final cleanup before dropping client

**Example**:
```rust
client.disconnect().await?;
tokio::time::sleep(Duration::from_secs(1)).await;
client.shutdown().await?;
```

---

### Subscribe Operations

#### `subscribe()` - Async Subscribe
```rust
pub async fn subscribe(&self, topic: &str, qos: u8) -> io::Result<()>
```

**Description**: Subscribe to topic (fire-and-forget).  
**Parameters**:
- `topic: &str` - Topic filter (supports wildcards: `+`, `#`)
- `qos: u8` - Requested QoS level (0, 1, or 2)

**Returns**: `Ok(())` if queued successfully  
**Acknowledgment**: Via `on_subscribed()` event callback

**Example**:
```rust
client.subscribe("sensors/+/temperature", 1).await?;
client.subscribe("alerts/#", 2).await?;
```

---

#### `subscribe_sync()` - Sync Subscribe
```rust
pub async fn subscribe_sync(
    &self,
    topic: &str,
    qos: u8
) -> Result<SubscribeResult, MqttClientError>
```

**Description**: Subscribe and wait for SUBACK.  
**Timeout**: Configured via `subscribe_timeout_ms` (default: 10 seconds)  
**Returns**: `SubscribeResult` with granted QoS levels

**Example**:
```rust
match client.subscribe_sync("sensors/#", 1).await {
    Ok(result) => {
        println!("Subscribed! Packet ID: {}", result.packet_id);
        println!("Granted QoS: {:?}", result.reason_codes);
    }
    Err(e) => eprintln!("Subscribe failed: {}", e),
}
```

**SubscribeResult fields**:
- `packet_id: u16` - MQTT packet identifier
- `reason_codes: Vec<u8>` - One per topic filter (0-2 = granted QoS, 0x80+ = failure)
- `properties: Option<Vec<Property>>` - MQTT v5 properties

---

#### `subscribe_sync_with_timeout()` - Sync Subscribe with Custom Timeout
```rust
pub async fn subscribe_sync_with_timeout(
    &self,
    topic: &str,
    qos: u8,
    timeout_ms: u64
) -> Result<SubscribeResult, MqttClientError>
```

**Example**:
```rust
// 5 second timeout for time-critical subscription
let result = client.subscribe_sync_with_timeout("alerts/#", 2, 5000).await?;
```

---

#### `subscribe_with_command()` - Advanced Subscribe
```rust
pub async fn subscribe_with_command(
    &self,
    command: SubscribeCommand
) -> io::Result<()>
```

**Description**: Subscribe with full MQTT v5 options using the builder pattern.  
**Use case**: When you need subscription options or multiple topics

**Example using Builder Pattern** (Recommended):
```rust
use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;

// Simple subscription
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/temp", 1)
    .build()
    .unwrap();

client.subscribe_with_command(cmd).await?;

// Advanced: Subscription with MQTT v5 options
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/+/temp", 1)
    .with_no_local(true)           // Don't receive own messages
    .with_retain_handling(2)       // Don't send retained messages
    .with_subscription_id(42)      // Track which subscription matched
    .build()
    .unwrap();

client.subscribe_with_command(cmd).await?;

// Multiple topics with different options
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/temp", 1)
    .with_no_local(true)
    .add_topic("sensors/humidity", 2)
    .with_retain_as_published(true)
    .with_retain_handling(1)
    .build()
    .unwrap();

client.subscribe_with_command(cmd).await?;
```

**MQTT v5 Subscription Options**:
- `no_local`: If true, server won't forward messages published by this client
- `retain_as_published`: If true, retain flag is kept as-is from publisher
- `retain_handling`:
  - `0`: Send retained messages at subscribe time (default)
  - `1`: Send retained only if subscription doesn't exist
  - `2`: Don't send retained messages
- `subscription_id`: Identifier to track which subscription matched (0 = none)

**Builder Methods**:
- `add_topic(topic, qos)` - Add topic with default options
- `add_topic_with_options(topic, qos, no_local, retain_as_published, retain_handling)` - Full control
- `with_no_local(bool)` - Modify last added topic
- `with_retain_as_published(bool)` - Modify last added topic
- `with_retain_handling(u8)` - Modify last added topic (0-2)
- `with_subscription_id(u32)` - Add subscription identifier
- `add_property(Property)` - Add custom MQTT v5 property
- `with_packet_id(u16)` - Set packet identifier
- `build()` - Build the command (validates at least one topic)

---

#### `subscribe_with_command_sync()` - Sync Advanced Subscribe
```rust
pub async fn subscribe_with_command_sync(
    &self,
    command: SubscribeCommand
) -> Result<SubscribeResult, MqttClientError>
```

**Description**: Like `subscribe_with_command()` but waits for SUBACK.

**Example**:
```rust
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/#", 1)
    .with_no_local(true)
    .build()
    .unwrap();

match client.subscribe_with_command_sync(cmd).await {
    Ok(result) => println!("Subscribed: {:?}", result.reason_codes),
    Err(e) => eprintln!("Failed: {}", e),
}
```
    vec![]  // properties
);
client.subscribe_with_command(command).await?;
```

---

### Unsubscribe Operations

#### `unsubscribe()` - Async Unsubscribe
```rust
pub async fn unsubscribe(&self, topics: Vec<&str>) -> io::Result<()>
```

**Description**: Unsubscribe from topics (fire-and-forget).  
**Parameters**: `topics` - List of topic filters to unsubscribe from

**Example**:
```rust
client.unsubscribe(vec!["sensors/#", "alerts/#"]).await?;
```

---

#### `unsubscribe_sync()` - Sync Unsubscribe
```rust
pub async fn unsubscribe_sync(
    &self,
    topics: Vec<&str>
) -> Result<UnsubscribeResult, MqttClientError>
```

**Description**: Unsubscribe and wait for UNSUBACK.  
**Timeout**: Configured via `unsubscribe_timeout_ms` (default: 10 seconds)

**Example**:
```rust
let result = client.unsubscribe_sync(vec!["sensors/#"]).await?;
println!("Unsubscribed! Reason codes: {:?}", result.reason_codes);
```

---

#### `unsubscribe_sync_with_timeout()` - Sync Unsubscribe with Custom Timeout
```rust
pub async fn unsubscribe_sync_with_timeout(
    &self,
    topics: Vec<&str>,
    timeout_ms: u64
) -> Result<UnsubscribeResult, MqttClientError>
```

**Example**:
```rust
let result = client.unsubscribe_sync_with_timeout(
    vec!["sensors/#"],
    15000  // 15 second timeout
).await?;
```

---

### Publish Operations

#### `publish()` - Async Publish
```rust
pub async fn publish(
    &self,
    topic: &str,
    payload: &[u8],
    qos: u8,
    retain: bool
) -> io::Result<()>
```

**Description**: Publish message (fire-and-forget).  
**Parameters**:
- `topic: &str` - Topic to publish to (no wildcards)
- `payload: &[u8]` - Message payload bytes
- `qos: u8` - Quality of Service (0, 1, or 2)
- `retain: bool` - Whether broker should retain message

**Returns**: `Ok(())` if queued successfully  
**Acknowledgment**: Via `on_published()` event callback (QoS 1/2 only)

**Example**:
```rust
// QoS 0 - fire and forget
client.publish("sensors/temp", b"23.5", 0, false).await?;

// QoS 1 - at least once delivery
client.publish("events/alert", b"WARNING", 1, false).await?;

// QoS 2 - exactly once delivery with retain
client.publish("config/settings", b"{\"rate\":100}", 2, true).await?;
```

---

#### `publish_sync()` - Sync Publish
```rust
pub async fn publish_sync(
    &self,
    topic: &str,
    payload: &[u8],
    qos: u8,
    retain: bool
) -> Result<PublishResult, MqttClientError>
```

**Description**: Publish and wait for acknowledgment.  
**Timeout**: Configured via `publish_ack_timeout_ms` (default: 10 seconds)  
**Behavior**:
- QoS 0: Returns immediately (no ACK)
- QoS 1: Waits for PUBACK
- QoS 2: Waits for PUBCOMP (full flow)

**Example**:
```rust
match client.publish_sync("sensors/temp", b"23.5", 1, false).await {
    Ok(result) => {
        println!("Published! Packet ID: {:?}", result.packet_id);
        println!("Reason code: {:?}", result.reason_code);
    }
    Err(MqttClientError::OperationTimeout { .. }) => {
        eprintln!("Publish acknowledgment timeout");
    }
    Err(e) => eprintln!("Publish failed: {}", e),
}
```

**PublishResult fields**:
- `packet_id: Option<u16>` - MQTT packet identifier (None for QoS 0)
- `qos: u8` - QoS level used
- `reason_code: Option<u8>` - Reason code from broker (if any)
- `properties: Option<Vec<Property>>` - MQTT v5 properties

---

#### `publish_sync_with_timeout()` - Sync Publish with Custom Timeout
```rust
pub async fn publish_sync_with_timeout(
    &self,
    topic: &str,
    payload: &[u8],
    qos: u8,
    retain: bool,
    timeout_ms: u64
) -> Result<PublishResult, MqttClientError>
```

**Example**:
```rust
// 3 second timeout for critical alert
let result = client.publish_sync_with_timeout(
    "alerts/critical",
    b"SYSTEM FAILURE",
    2,
    false,
    3000
).await?;
```

---

#### `publish_with_command()` - Advanced Publish
```rust
pub async fn publish_with_command(
    &self,
    command: PublishCommand
) -> io::Result<()>
```

**Description**: Publish with full MQTT v5 options.  
**Use case**: When you need message properties or advanced options

**Example**:
```rust
use flowsdk::mqtt_client::tokio_async_client::PublishCommand;
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;

let command = PublishCommand::new(
    "events/alert".to_string(),
    b"WARNING".to_vec(),
    1,      // qos
    false,  // retain
    false,  // dup
    None,   // packet_id (auto-generated)
    vec![
        Property::MessageExpiryInterval(3600),
        Property::ContentType("application/json".to_string()),
        Property::ResponseTopic("responses/alert".to_string()),
    ]
);
client.publish_with_command(command).await?;
```

---

### Ping Operations

#### `ping()` - Async Ping
```rust
pub async fn ping(&self) -> io::Result<()>
```

**Description**: Send PINGREQ to broker (fire-and-forget).  
**Acknowledgment**: Via `on_ping_response()` event callback

**Example**:
```rust
client.ping().await?;
```

---

#### `ping_sync()` - Sync Ping
```rust
pub async fn ping_sync(&self) -> Result<PingResult, MqttClientError>
```

**Description**: Send PINGREQ and wait for PINGRESP.  
**Timeout**: Configured via `ping_timeout_ms` (default: 5 seconds)  
**Use case**: Connection health checks

**Example**:
```rust
match client.ping_sync().await {
    Ok(_) => println!("Broker is responsive"),
    Err(MqttClientError::OperationTimeout { .. }) => {
        eprintln!("Ping timeout - connection may be dead");
    }
    Err(e) => eprintln!("Ping failed: {}", e),
}
```

---

#### `ping_sync_with_timeout()` - Sync Ping with Custom Timeout
```rust
pub async fn ping_sync_with_timeout(
    &self,
    timeout_ms: u64
) -> Result<PingResult, MqttClientError>
```

**Example**:
```rust
// Quick 2 second health check
let result = client.ping_sync_with_timeout(2000).await?;
```

---

## Event Handler

All asynchronous events are delivered through the `TokioMqttEventHandler` trait.

### Event Handler Trait

```rust
#[async_trait::async_trait]
pub trait TokioMqttEventHandler: Send + Sync {
    /// Called when successfully connected to broker
    async fn on_connected(&mut self, result: &ConnectionResult) {}
    
    /// Called when disconnected from broker
    async fn on_disconnected(&mut self, reason: Option<u8>) {}
    
    /// Called when message is published (QoS 1/2 acknowledgment)
    async fn on_published(&mut self, result: &PublishResult) {}
    
    /// Called when subscription is acknowledged
    async fn on_subscribed(&mut self, result: &SubscribeResult) {}
    
    /// Called when unsubscription is acknowledged
    async fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {}
    
    /// Called when message is received from broker
    async fn on_message_received(&mut self, publish: &MqttPublish) {}
    
    /// Called when ping response is received
    async fn on_ping_response(&mut self, result: &PingResult) {}
    
    /// Called when an error occurs
    async fn on_error(&mut self, error: &MqttClientError) {}
    
    /// Called when connection is lost
    async fn on_connection_lost(&mut self) {}
    
    /// Called on each reconnection attempt
    async fn on_reconnect_attempt(&mut self, attempt: u32) {}
    
    /// Called when pending operations are cleared
    async fn on_pending_operations_cleared(&mut self) {}
}
```

### Example Event Handler Implementation

```rust
use async_trait::async_trait;

struct MyHandler;

#[async_trait]
impl TokioMqttEventHandler for MyHandler {
    async fn on_connected(&mut self, result: &ConnectionResult) {
        if result.is_success() {
            println!("✅ Connected! Session: {}", result.session_present);
        }
    }
    
    async fn on_message_received(&mut self, publish: &MqttPublish) {
        let payload = String::from_utf8_lossy(&publish.payload);
        println!("📨 [{}] {}", publish.topic_name, payload);
    }
    
    async fn on_error(&mut self, error: &MqttClientError) {
        eprintln!("❌ Error: {}", error.user_message());
    }
    
    async fn on_connection_lost(&mut self) {
        println!("💔 Connection lost, will auto-reconnect");
    }
}
```

---

## Error Handling

All sync operations return `Result<T, MqttClientError>`.

### Error Types

```rust
pub enum MqttClientError {
    // Timeout errors
    OperationTimeout { operation: String, timeout_ms: u64 },
    
    // Connection errors
    ConnectionRefused { reason_code: u8, description: String },
    ConnectionLost { reason: String },
    NotConnected,
    AlreadyConnected,
    
    // Network errors
    NetworkError { kind: io::ErrorKind, message: String },
    
    // Protocol errors
    ProtocolViolation { message: String },
    PacketParsing { parse_error: String, raw_data: Vec<u8> },
    
    // Operation errors
    PublishFailed { packet_id: Option<u16>, reason_code: u8, reason_string: Option<String> },
    SubscribeFailed { topics: Vec<String>, reason_codes: Vec<u8> },
    UnsubscribeFailed { topics: Vec<String>, reason_codes: Vec<u8> },
    
    // Resource errors
    BufferFull { buffer_type: String, capacity: usize },
    ChannelClosed { channel: String },
    PacketIdExhausted,
    
    // Other
    InvalidConfiguration { field: String, reason: String },
    InternalError { message: String },
}
```

### Error Handling Patterns

```rust
match client.connect_sync().await {
    Ok(result) => { /* success */ }
    
    // Handle specific error types
    Err(MqttClientError::OperationTimeout { operation, timeout_ms }) => {
        eprintln!("{} timed out after {}ms", operation, timeout_ms);
        // Retry with longer timeout
        client.connect_sync_with_timeout(60000).await?
    }
    
    Err(MqttClientError::ConnectionRefused { reason_code, description }) => {
        eprintln!("Connection refused: {} (code: 0x{:02X})", description, reason_code);
        // Check credentials, broker config
    }
    
    Err(MqttClientError::NetworkError { kind, message }) => {
        eprintln!("Network error ({:?}): {}", kind, message);
        // Check network connectivity
    }
    
    Err(e) => {
        eprintln!("Error: {}", e.user_message());
    }
}
```

### Error Helper Methods

```rust
// Check if error is recoverable (retry possible)
if error.is_recoverable() {
    // Retry the operation
}

// Check if error should trigger reconnection
if error.should_reconnect() {
    // Initiate reconnection
}

// Check if error is fatal (client should stop)
if error.is_fatal() {
    // Shutdown client
}

// Check if error is authentication-related
if error.is_auth_error() {
    // Fix credentials
}
```

### Backward Compatibility with io::Error

```rust
// Automatic conversion for functions expecting io::Error
fn legacy_function() -> io::Result<()> {
    let result = client.connect_sync().await?;  // Auto-converts
    Ok(())
}

// Explicit conversion
let io_result: io::Result<_> = client.connect_sync()
    .await
    .map_err(Into::into);
```

---

## Common Usage Patterns

### 1. Simple Publish/Subscribe

```rust
// Create client
let client = TokioAsyncMqttClient::new(options, handler, config).await?;

// Connect
client.connect_sync().await?;

// Subscribe
client.subscribe_sync("sensors/+/temp", 1).await?;

// Publish
client.publish_sync("sensors/room1/temp", b"23.5", 1, false).await?;

// Disconnect
client.disconnect().await?;
```

### 2. High-Throughput Publishing

```rust
// Use async APIs for maximum throughput
for i in 0..10000 {
    let payload = format!("Message {}", i);
    client.publish("data/stream", payload.as_bytes(), 0, false).await?;
}
// Acknowledgments handled in on_published() callback
```

### 3. Reliable Message Delivery

```rust
// Use sync APIs with QoS 2 for guaranteed delivery
let result = client.publish_sync(
    "critical/command",
    b"SHUTDOWN",
    2,  // QoS 2 - exactly once
    false
).await?;

if result.is_success() {
    println!("Message delivered successfully");
}
```

### 4. Connection Health Monitoring

```rust
// Periodic ping to check connection health
loop {
    tokio::time::sleep(Duration::from_secs(30)).await;
    
    match client.ping_sync_with_timeout(5000).await {
        Ok(_) => println!("Connection healthy"),
        Err(_) => {
            eprintln!("Connection may be dead, reconnecting...");
            client.connect_sync().await?;
        }
    }
}
```

### 5. Graceful Shutdown

```rust
// 1. Unsubscribe from all topics if desired
client.unsubscribe_sync(vec!["sensors/#", "alerts/#"]).await?;

// 2. Disconnect gracefully
client.disconnect().await?;

// 3. Wait for disconnect to complete 
// @TODO: transport flush 
tokio::time::sleep(Duration::from_secs(1)).await;

// 4. Shutdown client worker
client.shutdown().await?;
```

---

## QoS Level Guidelines

### QoS 0 (At most once)
- **No acknowledgment**: Fire and forget
- **Use for**: High-frequency telemetry, sensor data where occasional loss is acceptable
- **Latency**: Lowest
- **Example**: Temperature readings every second

### QoS 1 (At least once)
- **Acknowledgment**: PUBACK required
- **Use for**: Important events that must be delivered (duplicates acceptable)
- **Latency**: Medium
- **Example**: System alerts, user actions

### QoS 2 (Exactly once)
- **Acknowledgment**: Full 4-step handshake (PUBREC, PUBREL, PUBCOMP)
- **Use for**: Critical commands where duplicates would cause problems
- **Latency**: Highest
- **Example**: Financial transactions, device control commands

---

## Configuration Reference

### TokioAsyncClientConfig Fields

```rust
pub struct TokioAsyncClientConfig {
    // Reconnection
    pub auto_reconnect: bool,                    // Default: true
    pub reconnect_delay_ms: u64,                 // Default: 1000 (1 sec)
    pub max_reconnect_delay_ms: u64,             // Default: 30000 (30 sec)
    pub max_reconnect_attempts: u32,             // Default: 0 (infinite)
    
    // Timeouts (None = no timeout)
    pub connect_timeout_ms: Option<u64>,         // Default: Some(30000)
    pub subscribe_timeout_ms: Option<u64>,       // Default: Some(10000)
    pub publish_ack_timeout_ms: Option<u64>,     // Default: Some(10000)
    pub unsubscribe_timeout_ms: Option<u64>,     // Default: Some(10000)
    pub ping_timeout_ms: Option<u64>,            // Default: Some(5000)
    pub default_operation_timeout_ms: u64,       // Default: 30000
    
    // Buffers
    pub command_queue_size: usize,               // Default: 1000
    pub buffer_messages: bool,                   // Default: true
    pub max_buffer_size: usize,                  // Default: 1000
    pub send_buffer_size: usize,                 // Default: 1000
    pub recv_buffer_size: usize,                 // Default: 1000
    
    // Other
    pub keep_alive_interval: u64,                // Default: 60 (seconds)
    pub tcp_nodelay: bool,                       // Default: true
}
```

---

## Best Practices

### 1. Choose the Right API
- **Use async APIs** when you don't need immediate confirmation
- **Use sync APIs** when operation success must be verified before continuing
- **Use custom timeouts** for operations with special latency requirements

### 2. Handle Timeouts Gracefully
```rust
match client.publish_sync(topic, payload, qos, retain).await {
    Err(MqttClientError::OperationTimeout { operation, timeout_ms }) => {
        eprintln!("{} timed out after {}ms - connection may be unresponsive", operation, timeout_ms);
        
        // Timeout indicates broker is not responding - disconnect and reconnect
        client.disconnect().await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        match client.connect_sync().await {
            Ok(_) => {
                // Retry operation after successful reconnection
                client.publish_sync(topic, payload, qos, retain).await?
            }
            Err(e) => {
                eprintln!("Reconnection failed: {}", e);
                // Alternative: try different broker peer/endpoint
                // Alternative: queue message for delivery via another channel
                return Err(e);
            }
        }
    }
    result => result?
}
```

### 3. Implement Comprehensive Event Handling
```rust
async fn on_error(&mut self, error: &MqttClientError) {
    match error {
        MqttClientError::ConnectionLost { .. } => {
            // Log, trigger alert, etc.
        }
        MqttClientError::PublishFailed { packet_id, .. } => {
            // Retry specific message
        }
        _ => {
            // Handle other errors
        }
    }
}
```

### 4. Configure Timeouts for Your Network
```rust
// Local development
let config = TokioAsyncClientConfig::builder()
    .local_network_timeouts()
    .build();

// Production cloud
let config = TokioAsyncClientConfig::builder()
    .internet_timeouts()
    .max_reconnect_attempts(10)
    .build();

// IoT/Satellite
let config = TokioAsyncClientConfig::builder()
    .satellite_timeouts()
    .max_reconnect_delay_ms(120000)
    .build();
```

### 5. Monitor Timeout Patterns
If operations frequently timeout:
- **DO NOT** simply retry with longer timeout values
- **DO** disconnect and reconnect to establish fresh connection
- **DO** check network quality and latency
- **DO** verify broker performance and resource availability
- **DO** consider message size or frequency reduction
- **DO** implement failover to alternate broker peers/endpoints
- **DO** queue messages for delivery via alternate channels

### 6. Timeout Recovery Strategies

#### Strategy 1: Disconnect and Reconnect
```rust
async fn handle_timeout_with_reconnect(
    client: &TokioAsyncMqttClient,
    operation: impl Fn() -> Future<Output = Result<T, MqttClientError>>
) -> Result<T, MqttClientError> {
    match operation().await {
        Err(MqttClientError::OperationTimeout { .. }) => {
            // Connection is unresponsive - force reconnection
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            client.connect_sync().await?;
            
            // Retry once after reconnection
            operation().await
        }
        result => result
    }
}
```

#### Strategy 2: Failover to Alternate Broker
```rust
async fn publish_with_failover(
    primary_client: &TokioAsyncMqttClient,
    backup_client: &TokioAsyncMqttClient,
    topic: &str,
    payload: &[u8],
    qos: u8
) -> Result<PublishResult, MqttClientError> {
    match primary_client.publish_sync(topic, payload, qos, false).await {
        Err(MqttClientError::OperationTimeout { .. }) => {
            eprintln!("Primary broker timeout - failing over to backup");
            backup_client.publish_sync(topic, payload, qos, false).await
        }
        result => result
    }
}
```

#### Strategy 3: Queue for Alternative Delivery
```rust
async fn publish_with_queue_fallback(
    client: &TokioAsyncMqttClient,
    fallback_queue: &mut Vec<(String, Vec<u8>)>,
    topic: &str,
    payload: &[u8],
    qos: u8
) -> Result<(), MqttClientError> {
    match client.publish_sync(topic, payload, qos, false).await {
        Err(MqttClientError::OperationTimeout { .. }) => {
            eprintln!("Timeout - queueing message for alternative delivery");
            fallback_queue.push((topic.to_string(), payload.to_vec()));
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(e) => Err(e)
    }
}
```

---

## Troubleshooting

### Connection Issues

**Problem**: `ConnectionRefused` error  
**Solutions**:
- Verify broker address and port
- Check authentication credentials
- Ensure broker is running and accessible
- Check firewall rules

**Problem**: `OperationTimeout` on connect  
**Solutions**:
- **First**: Verify broker is responsive (check broker logs, ping server)
- **Then**: Check network latency and packet loss
- **Avoid**: Simply increasing `connect_timeout_ms` without investigation
- **Consider**: Network-appropriate timeout preset (local_network_timeouts, internet_timeouts, satellite_timeouts)

### Publish Issues

**Problem**: Messages not being delivered  
**Solutions**:
- Use sync API to verify acknowledgment
- Check QoS level (use QoS 1 or 2 for guaranteed delivery)
- Verify subscription on receiving end
- Check message buffer isn't full

**Problem**: `OperationTimeout` on publish  
**Solutions**:
- **DO NOT** simply increase timeout - this indicates broker is unresponsive
- **DO** disconnect and reconnect to establish fresh connection
- **DO** implement failover to alternate broker peer
- **DO** queue messages for delivery via alternative channel
- **Then** investigate: check broker load, network quality, message size
- **Alternative**: Use async API if you can handle acknowledgments in `on_published()` callback

### Performance Issues

**Problem**: High latency  
**Solutions**:
- Use async APIs instead of sync
- Reduce QoS level where acceptable
- Enable `tcp_nodelay` for low-latency scenarios
- Increase buffer sizes

**Problem**: Messages being dropped  
**Solutions**:
- Increase `max_buffer_size`
- Enable `buffer_messages` during reconnection
- Slow down publishing rate
- Use higher QoS level

---

## API Quick Reference

### Core Operations

| Operation | Async (Fire-and-Forget) | Sync (Wait-for-ACK) | Sync with Timeout |
|-----------|------------------------|---------------------|-------------------|
| **Connect** | `connect()` | `connect_sync()` | `connect_sync_with_timeout(ms)` |
| **Disconnect** | `disconnect()` | - | - |
| **Subscribe** | `subscribe(topic, qos)` | `subscribe_sync(topic, qos)` | `subscribe_sync_with_timeout(topic, qos, ms)` |
| **Subscribe (Advanced)** | `subscribe_with_command(cmd)` | `subscribe_with_command_sync(cmd)` | `subscribe_with_command_sync_with_timeout(cmd, ms)` |
| **Unsubscribe** | `unsubscribe(topics)` | `unsubscribe_sync(topics)` | `unsubscribe_sync_with_timeout(topics, ms)` |
| **Publish** | `publish(topic, payload, qos, retain)` | `publish_sync(topic, payload, qos, retain)` | `publish_sync_with_timeout(topic, payload, qos, retain, ms)` |
| **Publish (Advanced)** | `publish_with_command(cmd)` | `publish_with_command_sync(cmd)` | `publish_with_command_sync_with_timeout(cmd, ms)` |
| **Ping** | `ping()` | `ping_sync()` | `ping_sync_with_timeout(ms)` |

### SubscribeCommand Builder

```rust
// Simple subscription
SubscribeCommand::builder()
    .add_topic("sensors/temp", 1)
    .build()?

// With MQTT v5 options
SubscribeCommand::builder()
    .add_topic("sensors/+/temp", 1)
    .with_no_local(true)           // Don't receive own messages
    .with_retain_handling(2)       // Don't send retained messages
    .with_retain_as_published(true) // Keep retain flag as-is
    .with_subscription_id(42)      // Track subscription matching
    .build()?

// Multiple topics
SubscribeCommand::builder()
    .add_topic("sensors/temp", 1)
    .with_no_local(true)
    .add_topic("sensors/humidity", 2)
    .with_retain_as_published(true)
    .build()?
```

### MQTT v5 Configuration

**Client Options** (MqttClientOptions):
- `maximum_packet_size(size)` / `no_maximum_packet_size()`
- `request_response_information(bool)`
- `request_problem_information(bool)`

**Client Config** (TokioAsyncClientConfig):
- `receive_maximum(max)` / `no_receive_maximum()`
- `topic_alias_maximum(max)` / `no_topic_alias()`

---

## Raw Packet API (Protocol Testing)

⚠️ **DANGER: Test-Only API** - Only use for protocol compliance testing!

The raw packet API is available behind the `protocol-testing` feature flag and provides low-level packet manipulation capabilities for testing MQTT protocol compliance.

### Enabling the Feature

```toml
[dependencies]
flowsdk = { version = "0.1", features = ["protocol-testing"] }
```

### RawPacketBuilder

Create and manipulate raw MQTT packets:

```rust
#[cfg(feature = "protocol-testing")]
use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;

// Create from valid packet
let packet = MqttPacket::Connect5(connect);
let mut builder = RawPacketBuilder::from_packet(packet)?;

// Manipulate bytes
builder.set_byte(0, 0x01);  // Set specific byte
builder.set_fixed_header_flags(0x01);  // Modify flags
builder.set_packet_type(15);  // Change packet type
builder.corrupt_variable_length();  // Corrupt encoding
builder.insert_bytes(10, &[0xFF, 0xFF]);  // Insert bytes
builder.remove_bytes(10, 2);  // Remove bytes
builder.truncate(50);  // Truncate to size
builder.append_bytes(&[0x00, 0x01]);  // Append bytes

let raw_packet = builder.build();
```

### RawTestClient

Direct TCP client bypassing MQTT protocol:

```rust
#[cfg(feature = "protocol-testing")]
use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;

// Connect directly via TCP
let mut client = RawTestClient::connect("localhost:1883").await?;

// Send raw bytes
let malformed_packet = vec![0x10, 0x00];  // Invalid CONNECT
client.send_raw(malformed_packet).await?;

// Receive response
let response = client.receive_raw(5000).await?;

// Expect disconnect
client.send_expect_disconnect(malformed_packet, 5000).await?;

// Send and receive
let response = client.send_and_receive(packet, 5000).await?;

// Close connection
client.close().await?;
```

### MalformedPacketGenerator

Pre-built malformed packets for testing server rejection:

```rust
#[cfg(feature = "protocol-testing")]
use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;

// Reserved bits violations
let packet = MalformedPacketGenerator::connect_reserved_flag()?;
let packet = MalformedPacketGenerator::subscribe_reserved_bits()?;

// Encoding violations
let packet = MalformedPacketGenerator::non_minimal_variable_length()?;
let packet = MalformedPacketGenerator::qos0_with_packet_id()?;
let packet = MalformedPacketGenerator::invalid_qos_both_bits()?;

// Protocol violations
let packet = MalformedPacketGenerator::incorrect_protocol_name()?;
let packet = MalformedPacketGenerator::second_connect()?;
let packet = MalformedPacketGenerator::invalid_protocol_version()?;

// Property violations
let packet = MalformedPacketGenerator::topic_alias_zero()?;
let packet = MalformedPacketGenerator::subscription_identifier_zero()?;
let packet = MalformedPacketGenerator::duplicate_topic_alias()?;

// Topic violations
let packet = MalformedPacketGenerator::topic_with_wildcards()?;
let packet = MalformedPacketGenerator::empty_topic_name()?;

// Payload violations
let packet = MalformedPacketGenerator::will_flag_without_payload()?;
let packet = MalformedPacketGenerator::client_id_too_long()?;

// Size violations
let packet = MalformedPacketGenerator::remaining_length_too_large()?;

// Valid baseline packets
let packet = MalformedPacketGenerator::valid_connect()?;
let packet = MalformedPacketGenerator::valid_pingreq()?;
```

### Testing Pattern Example

```rust
#[tokio::test]
#[cfg(feature = "protocol-testing")]
#[ignore]  // Requires live broker
async fn test_mqtt_reserved_bits_rejection() {
    use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    
    // Connect via raw TCP
    let mut client = RawTestClient::connect("localhost:1883").await.unwrap();
    
    // Send malformed CONNECT with reserved bit set
    let malformed = MalformedPacketGenerator::connect_reserved_flag().unwrap();
    
    // Server should disconnect (MQTT-2.1.3-1)
    client.send_expect_disconnect(malformed, 5000).await.unwrap();
}
```

### Safety Warnings

⚠️ **Never use in production**:
- These APIs create malformed packets that violate MQTT specification
- Can crash brokers or cause undefined behavior
- Only for testing server compliance with MQTT normative statements
- Always use behind `#[cfg(feature = "protocol-testing")]`
- Mark tests with `#[ignore]` to prevent accidental execution

### Available Generators (20+ methods)

Each generator method creates a packet that violates a specific MQTT normative statement:

- **MQTT-1.5.5-1**: Non-minimal variable byte integers
- **MQTT-2.1.3-1**: Reserved flag bits
- **MQTT-2.2.1-2**: QoS 0 with packet identifier
- **MQTT-3.1.0-2**: Second CONNECT packet
- **MQTT-3.1.2-1**: Incorrect protocol name
- **MQTT-3.3.1-4**: Both QoS bits set to 1
- **MQTT-3.3.2-8**: Topic Alias = 0
- **MQTT-3.3.4-6**: Subscription Identifier = 0
- **MQTT-3.8.3-1-2**: SUBSCRIBE reserved bits
- And many more...

See `src/mqtt_client/raw_packet/malformed.rs` for complete list.

---

## Additional Resources

- **Full Example**: See `examples/tokio_async_mqtt_client_example.rs`
- **Timeout Tests**: See `tests/timeout_tests.rs`
- **Protocol Compliance**: See `tests/protocol_compliance_tests.rs`
- **MQTT v5 Spec**: See `docs/source/mqtt-v5.0.pdf`
- **Raw Packet API Reference**: See `docs/RAW_PACKET_API_QUICK_REFERENCE.md`
- **Test Coverage Analysis**: See `docs/MQTT_PROTOCOL_TEST_COVERAGE_ANALYSIS.md`

---

**Last Updated**: October 10, 2025  
**Version**: 1.0
