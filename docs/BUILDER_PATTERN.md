# Builder Pattern Documentation

## Overview

FlowSDK provides comprehensive builder patterns for configuring MQTT clients. This document covers:

1. **MqttClientOptions Builder** - Configure connection options (peer, credentials, session settings)
2. **TokioAsyncClientConfig Builder** - Configure async client behavior (timeouts, reconnection, QUIC/TLS)
3. **PublishCommand Builder** - Create MQTT PUBLISH packets with QoS, retain, and MQTT v5 properties
4. **SubscribeCommand Builder** - Create MQTT SUBSCRIBE packets with subscription options and identifiers

The builder patterns provide fluent, chainable APIs for clean, type-safe configuration with sensible defaults.

---

## 1. MqttClientOptions Builder

The `MqttClientOptions` builder configures connection parameters and MQTT protocol settings.

### Default Values

When using `MqttClientOptions::default()` or `MqttClientOptions::builder()`:

```rust
MqttClientOptions {
    peer: "localhost:1883",
    client_id: "mqtt_client",
    clean_start: true,
    keep_alive: 60,
    username: None,
    password: None,
    will: None,
    reconnect: false,
    sessionless: false,
    subscription_topics: Vec::new(),
    auto_ack: true,
    session_expiry_interval: None,
    maximum_packet_size: None,
    request_response_information: None,
    request_problem_information: None,
    #[cfg(feature = "tls")]
    tls_config: None,
}
```

### Usage Examples

#### 1. Simple Connection

```rust
use flowsdk::mqtt_client::MqttClientOptions;

let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("my_awesome_client")
    .build();
```

#### 2. Secure Connection with TLS

```rust
use flowsdk::mqtt_client::MqttClientOptions;

// Using mqtts:// URL scheme (requires 'tls' feature)
let options = MqttClientOptions::builder()
    .peer("mqtts://localhost:8883")
    .client_id("secure_client")
    .username("mqtt_user")
    .password(b"secret_password".to_vec())
    .build();

// Or with explicit TLS config
#[cfg(feature = "tls")]
{
    use flowsdk::mqtt_client::transport::tls::TlsConfig;
    
    let tls_config = TlsConfig::builder()
        .enable_sni(true)
        .build();
    
    let options = MqttClientOptions::builder()
        .peer("localhost:8883")
        .client_id("secure_client")
        .tls_config(tls_config)
        .build();
}
```

#### 3. QUIC Transport Connection

```rust
// Using quic:// URL scheme (requires 'quic' feature)
let options = MqttClientOptions::builder()
    .peer("quic://localhost:14567")
    .client_id("quic_client")
    .username("mqtt_user")
    .password(b"secret_password".to_vec())
    .build();
```

#### 4. Full Configuration with Session Management

```rust
let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("persistent_client")
    .username("mqtt_user")
    .password(b"secret_password".to_vec())
    .clean_start(false)
    .keep_alive(120)
    .reconnect(true)
    .auto_ack(true)
    .session_expiry_interval(3600)  // 1 hour
    .maximum_packet_size(1048576)   // 1 MB
    .request_response_information(true)
    .request_problem_information(true)
    .build();
```

#### 5. With Auto-Subscribe Topics

```rust
use flowsdk::mqtt_client::MqttClientOptions;
use flowsdk::mqtt_serde::mqttv5::subscribev5::TopicSubscription;

let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("subscriber_client")
    .add_subscription_topic(TopicSubscription {
        topic: "sensors/temperature".to_string(),
        qos: 1,
        no_local: false,
        retain_as_published: false,
        retain_handling: 0,
    })
    .add_subscription_topic(TopicSubscription {
        topic: "sensors/humidity".to_string(),
        qos: 1,
        no_local: false,
        retain_as_published: false,
        retain_handling: 0,
    })
    .build();
```

### MqttClientOptions Builder Methods Reference

| Method | Parameter Type | Description |
|--------|---------------|-------------|
| `builder()` | - | Create a new builder with default values |
| `peer()` | `impl Into<String>` | Set broker address (supports `mqtt://`, `mqtts://`, `quic://` schemes) |
| `client_id()` | `impl Into<String>` | Set MQTT client identifier |
| `clean_start()` | `bool` | Set clean start flag (MQTT v5) |
| `keep_alive()` | `u16` | Set keep-alive interval in seconds |
| `username()` | `impl Into<String>` | Set authentication username |
| `password()` | `impl Into<Vec<u8>>` | Set authentication password |
| `will()` | `Will` | Set Last Will and Testament |
| `reconnect()` | `bool` | Enable/disable auto-reconnect |
| `sessionless()` | `bool` | Enable/disable sessionless mode |
| `subscription_topics()` | `Vec<TopicSubscription>` | Set all auto-subscribe topics |
| `add_subscription_topic()` | `TopicSubscription` | Add single auto-subscribe topic |
| `auto_ack()` | `bool` | Enable/disable automatic message acknowledgment |
| `session_expiry_interval()` | `u32` | Set session expiry in seconds (MQTT v5) |
| `maximum_packet_size()` | `u32` | Set maximum packet size in bytes (MQTT v5) |
| `no_maximum_packet_size()` | - | Remove maximum packet size limit |
| `request_response_information()` | `bool` | Request response information from server (MQTT v5) |
| `request_problem_information()` | `bool` | Request problem information in responses (MQTT v5) |
| `tls_config()` | `TlsConfig` | Set TLS configuration (requires `tls` feature) |
| `enable_tls()` | - | Enable TLS with default config (requires `tls` feature) |
| `disable_tls()` | - | Disable TLS (requires `tls` feature) |
| `build()` | - | Consume builder and return configured options |

---

## 2. TokioAsyncClientConfig Builder

The `TokioAsyncClientConfig` builder configures async client behavior including reconnection, timeouts, and transport-specific options.

### Default Values

```rust
TokioAsyncClientConfig {
    auto_reconnect: true,
    reconnect_delay_ms: 1000,
    max_reconnect_delay_ms: 30000,
    max_reconnect_attempts: 0,  // infinite
    command_queue_size: 1000,
    buffer_messages: true,
    max_buffer_size: 1000,
    send_buffer_size: 1000,
    recv_buffer_size: 1000,
    keep_alive_interval: 60,
    tcp_nodelay: true,
    connect_timeout_ms: Some(30000),      // 30 seconds
    subscribe_timeout_ms: Some(10000),    // 10 seconds
    publish_ack_timeout_ms: Some(10000),  // 10 seconds
    unsubscribe_timeout_ms: Some(10000),  // 10 seconds
    ping_timeout_ms: Some(5000),          // 5 seconds
    default_operation_timeout_ms: 30000,  // 30 seconds
    receive_maximum: None,                // Use MQTT v5 default (65535)
    topic_alias_maximum: None,            // Topic aliases not supported by default
    
    // QUIC-specific (requires 'quic' feature)
    #[cfg(feature = "quic")]
    quic_enable_0rtt: false,              // Disable 0-RTT by default
    #[cfg(feature = "quic")]
    quic_insecure_skip_verify: false,     // Enable verification by default
    #[cfg(feature = "quic")]
    quic_custom_root_ca_pem: None,        // Use system CAs
    #[cfg(feature = "quic")]
    quic_client_cert_pem: None,           // No client cert
    #[cfg(feature = "quic")]
    quic_client_key_pem: None,            // No client key
}
```

### Usage Examples

#### 1. Basic Configuration

```rust
use flowsdk::mqtt_client::TokioAsyncClientConfig;

let config = TokioAsyncClientConfig::builder()
    .auto_reconnect(true)
    .reconnect_delay_ms(2000)
    .max_reconnect_attempts(10)
    .build();
```

#### 2. High-Performance Configuration

```rust
let config = TokioAsyncClientConfig::builder()
    .tcp_nodelay(true)
    .send_buffer_size(2000)
    .recv_buffer_size(2000)
    .command_queue_size(5000)
    .connect_timeout_ms(Some(10000))
    .build();
```

#### 3. QUIC Configuration (Testing)

```rust
#[cfg(feature = "quic")]
{
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .quic_insecure_skip_verify(true)  // ⚠️ For testing only!
        .quic_enable_0rtt(false)
        .build();
}
```

#### 4. QUIC Configuration (Production with Custom CA)

```rust
#[cfg(feature = "quic")]
{
    let ca_pem = std::fs::read_to_string("ca.pem")?;
    
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .quic_custom_root_ca_pem(ca_pem)
        .quic_insecure_skip_verify(false)  // Enable verification
        .quic_enable_0rtt(false)
        .build();
}
```

#### 5. QUIC with mTLS

```rust
#[cfg(feature = "quic")]
{
    let ca_pem = std::fs::read_to_string("ca.pem")?;
    let cert_pem = std::fs::read_to_string("client.pem")?;
    let key_pem = std::fs::read_to_string("client.key")?;
    
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .quic_custom_root_ca_pem(ca_pem)
        .quic_client_cert_pem(cert_pem)
        .quic_client_key_pem(key_pem)
        .quic_enable_0rtt(false)
        .build();
}
```

#### 6. Flow Control and Topic Aliases

```rust
let config = TokioAsyncClientConfig::builder()
    .receive_maximum(100)         // Limit in-flight QoS 1/2 messages
    .topic_alias_maximum(10)       // Accept up to 10 topic aliases
    .build();
```

### TokioAsyncClientConfig Builder Methods Reference

| Method | Parameter Type | Description |
|--------|---------------|-------------|
| `builder()` | - | Create a new builder with default values |
| **Reconnection** |||
| `auto_reconnect()` | `bool` | Enable/disable automatic reconnection |
| `reconnect_delay_ms()` | `u64` | Initial delay between reconnection attempts |
| `max_reconnect_delay_ms()` | `u64` | Maximum delay between reconnection attempts |
| `max_reconnect_attempts()` | `u32` | Max reconnection attempts (0 = infinite) |
| **Buffers & Queues** |||
| `command_queue_size()` | `usize` | Size of command queue |
| `buffer_messages()` | `bool` | Enable/disable message buffering |
| `max_buffer_size()` | `usize` | Maximum buffered messages |
| `send_buffer_size()` | `usize` | Send buffer size |
| `recv_buffer_size()` | `usize` | Receive buffer size |
| **Network** |||
| `tcp_nodelay()` | `bool` | Enable/disable TCP_NODELAY (Nagle's algorithm) |
| `keep_alive_interval()` | `u16` | Keep-alive interval in seconds |
| **Timeouts** |||
| `connect_timeout_ms()` | `Option<u64>` | Connection timeout in milliseconds |
| `subscribe_timeout_ms()` | `Option<u64>` | Subscribe timeout in milliseconds |
| `publish_ack_timeout_ms()` | `Option<u64>` | Publish acknowledgment timeout |
| `unsubscribe_timeout_ms()` | `Option<u64>` | Unsubscribe timeout in milliseconds |
| `ping_timeout_ms()` | `Option<u64>` | Ping timeout in milliseconds |
| `default_operation_timeout_ms()` | `u64` | Default operation timeout |
| **Flow Control (MQTT v5)** |||
| `receive_maximum()` | `u16` | Maximum in-flight QoS 1/2 messages |
| `no_receive_maximum()` | - | Remove receive maximum limit |
| `topic_alias_maximum()` | `u16` | Maximum topic aliases accepted |
| `no_topic_alias()` | - | Disable topic alias support |
| **QUIC Transport** (requires `quic` feature) |||
| `quic_enable_0rtt()` | `bool` | Enable/disable 0-RTT for faster reconnection |
| `quic_insecure_skip_verify()` | `bool` | ⚠️ Skip TLS certificate verification (testing only!) |
| `quic_custom_root_ca_pem()` | `String` | Custom root CA certificates (PEM format) |
| `quic_client_cert_pem()` | `String` | Client certificate for mTLS (PEM format) |
| `quic_client_key_pem()` | `String` | Client private key for mTLS (PEM format) |
| **Build** |||
| `build()` | - | Consume builder and return configured client config |

---

## Complete Example

### Using Both Builders Together

```rust
use std::sync::Arc;
use flowsdk::mqtt_client::{
    MqttClientOptions, 
    TokioAsyncMqttClient, 
    TokioAsyncClientConfig,
    TokioMqttEventHandler,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure connection options
    let options = MqttClientOptions::builder()
        .peer("mqtt://broker.example.com:1883")
        .client_id("my_client")
        .username("user")
        .password(b"pass".to_vec())
        .clean_start(true)
        .keep_alive(60)
        .session_expiry_interval(3600)
        .build();

    // Configure async client behavior
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .reconnect_delay_ms(1000)
        .max_reconnect_delay_ms(30000)
        .tcp_nodelay(true)
        .connect_timeout_ms(Some(30000))
        .build();

    // Create event handler
    let handler = Box::new(MyEventHandler);

    // Create the client
    let client = TokioAsyncMqttClient::new(options, handler, config).await?;

    // Connect and use
    client.connect().await?;
    client.subscribe("sensors/#", 1).await?;
    client.publish("sensors/temp", b"23.5", 1, false).await?;

    Ok(())
}
```

### QUIC Transport Example

```rust
#[cfg(feature = "quic")]
{
    use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncClientConfig};

    // Configure for QUIC transport
    let options = MqttClientOptions::builder()
        .peer("quic://localhost:14567")  // Use quic:// scheme
        .client_id("quic_client")
        .build();

    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .quic_insecure_skip_verify(true)  // ⚠️ Testing only!
        .quic_enable_0rtt(false)
        .build();

    let client = TokioAsyncMqttClient::new(options, handler, config).await?;
}
```

---

## URL Schemes and Transport Selection

The `peer()` method supports multiple URL schemes for automatic transport selection:

| URL Scheme | Transport | Feature Required | Default Port |
|------------|-----------|------------------|--------------|
| `mqtt://host:port` | TCP | None | 1883 |
| `mqtts://host:port` | TLS | `tls` | 8883 |
| `quic://host:port` | QUIC | `quic` | 14567 |
| `host:port` | TCP | None | (from port) |

**Examples:**
```rust
// TCP transport
.peer("mqtt://broker.example.com:1883")
.peer("broker.example.com:1883")

// TLS transport (requires 'tls' feature)
.peer("mqtts://broker.example.com:8883")

// QUIC transport (requires 'quic' feature)
.peer("quic://broker.example.com:14567")
```
---

## Comparison: Traditional vs Builder Pattern

### Traditional Struct Initialization (Still Supported)

```rust
let options = MqttClientOptions {
    peer: "localhost:1883".to_string(),
    client_id: "my_client".to_string(),
    clean_start: true,
    keep_alive: 60,
    username: None,
    password: None,
    will: None,
    reconnect: false,
    sessionless: false,
    subscription_topics: Vec::new(),
    auto_ack: true,
    session_expiry_interval: None,
    maximum_packet_size: None,
    request_response_information: None,
    request_problem_information: None,
    #[cfg(feature = "tls")]
    tls_config: None,
};
```

### Builder Pattern (Recommended)

```rust
let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("my_client")
    .build();
// All other fields use sensible defaults
```

---

## Benefits of Builder Pattern

1. **Sensible Defaults**: Most fields have reasonable default values
2. **Fluent API**: Chain method calls for clean, readable code
3. **Type Safety**: Compile-time checks for all parameters
4. **Flexibility**: Mix and match only the options you need
5. **Backward Compatible**: Traditional struct initialization still works
6. **Self-Documenting**: Method names clearly indicate what's being configured
7. **Easy Testing**: Simple to create test configurations with minimal setup

---

## Migration Guide

Existing code using struct initialization continues to work without changes. To migrate to the builder pattern:

### Before: Verbose Struct Initialization

```rust
let options = MqttClientOptions {
    peer: "localhost:1883".to_string(),
    client_id: "test".to_string(),
    clean_start: true,
    keep_alive: 60,
    username: Some("user".to_string()),
    password: Some(b"pass".to_vec()),
    will: None,
    reconnect: true,
    sessionless: false,
    subscription_topics: Vec::new(),
    auto_ack: true,
    session_expiry_interval: Some(3600),
    maximum_packet_size: None,
    request_response_information: None,
    request_problem_information: None,
    #[cfg(feature = "tls")]
    tls_config: None,
};
```

### After: Clean Builder Pattern

```rust
let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("test")
    .username("user")
    .password(b"pass".to_vec())
    .reconnect(true)
    .session_expiry_interval(3600)
    .build();
```

Much cleaner and more maintainable!

---

## Best Practices

### 1. Use Builders for New Code

Always prefer the builder pattern for new code:

```rust
// Good ✅
let options = MqttClientOptions::builder()
    .peer("mqtt://broker.example.com:1883")
    .client_id("my_client")
    .build();

// Avoid ❌ (unless you need to update existing code)
let mut options = MqttClientOptions::default();
options.peer = "mqtt://broker.example.com:1883".to_string();
options.client_id = "my_client".to_string();
```

### 2. Chain Related Options Together

Group related configuration for clarity:

```rust
let config = TokioAsyncClientConfig::builder()
    // Reconnection settings
    .auto_reconnect(true)
    .reconnect_delay_ms(1000)
    .max_reconnect_delay_ms(30000)
    .max_reconnect_attempts(10)
    
    // Timeout settings
    .connect_timeout_ms(Some(30000))
    .publish_ack_timeout_ms(Some(10000))
    .ping_timeout_ms(Some(5000))
    
    // Network settings
    .tcp_nodelay(true)
    .send_buffer_size(2000)
    .recv_buffer_size(2000)
    
    .build();
```

### 3. Extract Complex Configurations

For complex setups, extract configuration into functions:

```rust
fn production_config() -> TokioAsyncClientConfig {
    TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .reconnect_delay_ms(1000)
        .max_reconnect_delay_ms(30000)
        .tcp_nodelay(true)
        .connect_timeout_ms(Some(30000))
        .build()
}

fn quic_production_config(ca_path: &str, cert_path: &str, key_path: &str) 
    -> Result<TokioAsyncClientConfig, std::io::Error> 
{
    let ca_pem = std::fs::read_to_string(ca_path)?;
    let cert_pem = std::fs::read_to_string(cert_path)?;
    let key_pem = std::fs::read_to_string(key_path)?;
    
    Ok(TokioAsyncClientConfig::builder()
        .auto_reconnect(true)
        .quic_custom_root_ca_pem(ca_pem)
        .quic_client_cert_pem(cert_pem)
        .quic_client_key_pem(key_pem)
        .build())
}
```

### 4. Document Security-Sensitive Options

Always document when using security-sensitive options:

```rust
let config = TokioAsyncClientConfig::builder()
    .quic_insecure_skip_verify(true)  // ⚠️ TESTING ONLY - NEVER USE IN PRODUCTION!
    .build();
```

---

## Feature-Specific Configuration

### TLS Configuration (requires `tls` feature)

```rust
#[cfg(feature = "tls")]
{
    use flowsdk::mqtt_client::transport::tls::TlsConfig;
    
    let tls_config = TlsConfig::builder()
        .enable_sni(true)
        .build();
    
    let options = MqttClientOptions::builder()
        .peer("mqtts://broker.example.com:8883")
        .client_id("tls_client")
        .tls_config(tls_config)
        .build();
}
```

### QUIC Configuration (requires `quic` feature)

```rust
#[cfg(feature = "quic")]
{
    // Testing configuration
    let test_config = TokioAsyncClientConfig::builder()
        .quic_insecure_skip_verify(true)  // ⚠️ Testing only!
        .quic_enable_0rtt(false)
        .build();
    
    // Production configuration with custom CA
    let ca_pem = std::fs::read_to_string("ca.pem")?;
    let prod_config = TokioAsyncClientConfig::builder()
        .quic_custom_root_ca_pem(ca_pem)
        .quic_insecure_skip_verify(false)  // Enable verification
        .quic_enable_0rtt(false)            // Disable 0-RTT (safer)
        .build();
    
    // Production with mTLS
    let cert_pem = std::fs::read_to_string("client.pem")?;
    let key_pem = std::fs::read_to_string("client.key")?;
    let mtls_config = TokioAsyncClientConfig::builder()
        .quic_custom_root_ca_pem(ca_pem)
        .quic_client_cert_pem(cert_pem)
        .quic_client_key_pem(key_pem)
        .build();
}
```

---

## 3. PublishCommand Builder

The `PublishCommand` builder creates MQTT PUBLISH packets with full control over QoS, retain flags, and MQTT v5 properties.

### Default Values

When using `PublishCommand::builder()`:

```rust
PublishCommandBuilder {
    topic_name: None,       // ❗ Required - must be set
    payload: Vec::new(),    // Empty payload
    qos: 0,                 // At most once delivery
    retain: false,          // Don't retain message
    dup: false,             // Not a duplicate
    packet_id: None,        // Auto-assigned by client
    properties: Vec::new(), // No MQTT v5 properties
}
```

### Usage Examples

#### 1. Simple Publish

```rust
use flowsdk::mqtt_client::tokio_async_client::PublishCommand;

let cmd = PublishCommand::builder()
    .topic("sensors/temp")
    .payload(b"23.5")
    .build()
    .unwrap();
```

#### 2. Publish with QoS and Retain

```rust
let cmd = PublishCommand::builder()
    .topic("sensors/status")
    .payload(b"online")
    .qos(1)           // At least once delivery
    .retain(true)     // Store for new subscribers
    .build()
    .unwrap();
```

#### 3. Publish with Content Type (MQTT v5)

```rust
let json_data = br#"{"temp": 23.5, "humidity": 60}"#;

let cmd = PublishCommand::builder()
    .topic("sensors/data")
    .payload(json_data)
    .qos(1)
    .with_content_type("application/json")
    .build()
    .unwrap();
```

#### 4. Request/Response Pattern (MQTT v5)

```rust
let cmd = PublishCommand::builder()
    .topic("requests/temperature")
    .payload(b"get_current")
    .qos(1)
    .with_response_topic("responses/temperature/client1")
    .with_correlation_data(b"req-12345")
    .with_message_expiry_interval(30)  // Expire after 30 seconds
    .build()
    .unwrap();
```

#### 5. Topic Alias for Bandwidth Optimization (MQTT v5)

```rust
// First publish: send full topic and establish alias
let cmd1 = PublishCommand::builder()
    .topic("sensors/temperature/room1/sensor3")
    .payload(b"22.5")
    .with_topic_alias(42)  // Server remembers topic as alias 42
    .build()
    .unwrap();

// Subsequent publishes: use alias (saves bandwidth)
let cmd2 = PublishCommand::builder()
    .topic("")  // Empty topic when using alias
    .payload(b"23.0")
    .with_topic_alias(42)  // Reference established alias
    .build()
    .unwrap();
```

#### 6. User Properties for Metadata (MQTT v5)

```rust
let cmd = PublishCommand::builder()
    .topic("sensors/temp")
    .payload(b"23.5")
    .qos(1)
    .with_user_property("sensor_id", "sensor-42")
    .with_user_property("location", "room1")
    .with_user_property("calibration", "2025-01-15")
    .build()
    .unwrap();
```

### Method Reference

| Method | Parameters | Description | Default |
|--------|-----------|-------------|---------|
| `builder()` | - | Create new builder | - |
| `topic()` | `impl Into<String>` | **Required**: Set topic name | None |
| `payload()` | `impl Into<Vec<u8>>` | Set message payload | `[]` |
| `qos()` | `u8` (0-2) | Set Quality of Service | `0` |
| `retain()` | `bool` | Set retain flag | `false` |
| `dup()` | `bool` | Set duplicate flag | `false` |
| `with_packet_id()` | `u16` | Set packet ID (usually auto) | Auto |
| `add_property()` | `Property` | Add custom MQTT v5 property | - |
| `with_message_expiry_interval()` | `u32` | Message lifetime in seconds | None |
| `with_content_type()` | `impl Into<String>` | MIME type of payload | None |
| `with_response_topic()` | `impl Into<String>` | Response destination | None |
| `with_correlation_data()` | `impl Into<Vec<u8>>` | Request/response correlation | None |
| `with_topic_alias()` | `u16` | Numeric topic alias | None |
| `with_user_property()` | `key: String, value: String` | Custom metadata | None |
| `build()` | - | Build PublishCommand | - |

### Error Handling

The `build()` method returns `Result<PublishCommand, PublishBuilderError>`:

```rust
pub enum PublishBuilderError {
    NoTopic,  // topic() was not called
}
```

**Example**:

```rust
let result = PublishCommand::builder()
    .payload(b"data")
    .build();

match result {
    Ok(cmd) => println!("Command created successfully"),
    Err(PublishBuilderError::NoTopic) => {
        eprintln!("Error: Topic name is required");
    }
}
```

### MQTT v5 Properties Overview

| Property | Method | Use Case |
|----------|--------|----------|
| Message Expiry Interval | `with_message_expiry_interval(seconds)` | Time-sensitive messages (alerts, temporary data) |
| Content Type | `with_content_type(mime_type)` | Indicate payload format (e.g., "application/json") |
| Response Topic | `with_response_topic(topic)` | Request/response patterns |
| Correlation Data | `with_correlation_data(bytes)` | Match requests with responses |
| Topic Alias | `with_topic_alias(alias)` | Reduce bandwidth for long topics |
| User Properties | `with_user_property(key, value)` | Application-specific metadata |

### Best Practices

1. **Always Set Topic**: The `topic()` method is required. `build()` will fail without it.

2. **Choose Appropriate QoS**:
   - **QoS 0**: Fire-and-forget (sensor data, telemetry)
   - **QoS 1**: At least once (commands, important events)
   - **QoS 2**: Exactly once (financial transactions, critical state changes)

3. **Use Retain Wisely**:
   - Set `retain(true)` for status messages (device online/offline)
   - Use for configuration or "last known value" scenarios
   - Retained messages persist on broker until replaced

4. **Content Type for Interoperability**:
   ```rust
   .with_content_type("application/json")  // JSON data
   .with_content_type("text/plain")        // Plain text
   .with_content_type("application/octet-stream")  // Binary
   ```

5. **Message Expiry for Time-Sensitive Data**:
   ```rust
   // Alert expires after 5 minutes
   .with_message_expiry_interval(300)
   ```

6. **Topic Aliases for High-Frequency Publishing**:
   - Use aliases for topics published frequently
   - Reduces per-message overhead significantly
   - Especially useful for long topic names

7. **User Properties for Observability**:
   ```rust
   .with_user_property("trace_id", "xyz-123")
   .with_user_property("source", "sensor-network-a")
   ```

---

## 4. SubscribeCommand Builder

The `SubscribeCommand` builder creates MQTT SUBSCRIBE packets with full control over subscription options and MQTT v5 features.

### Default Values

When using `SubscribeCommand::builder()`:

```rust
SubscribeCommandBuilder {
    topics: Vec::new(),     // ❗ Required - must add at least one topic
    properties: Vec::new(), // No MQTT v5 properties
    packet_id: None,        // Auto-assigned by client
}
```

**Default Subscription Options** (per topic):
```rust
TopicSubscription {
    topic_filter: String,        // Topic pattern with wildcards
    qos: u8,                      // Set via add_topic()
    no_local: false,              // Receive own messages
    retain_as_published: false,   // Retain flag set to 0
    retain_handling: 0,           // Send retained messages
}
```

### Usage Examples

#### 1. Simple Subscription

```rust
use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;

let cmd = SubscribeCommand::builder()
    .add_topic("sensors/temp", 1)
    .build()
    .unwrap();
```

#### 2. Multiple Topics with Different QoS

```rust
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/temp", 1)      // QoS 1
    .add_topic("sensors/humidity", 0)  // QoS 0 (faster)
    .add_topic("alerts/#", 2)          // QoS 2 (all alerts)
    .build()
    .unwrap();
```

#### 3. No Local Option (Don't Receive Own Messages)

```rust
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/+/temp", 1)
    .with_no_local(true)  // Won't receive messages this client publishes
    .build()
    .unwrap();
```

#### 4. Retain Handling Options

```rust
// Don't send retained messages at subscribe time
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/status", 1)
    .with_retain_handling(2)  // 2 = Don't send retained
    .build()
    .unwrap();

// Send retained only if subscription is new
let cmd2 = SubscribeCommand::builder()
    .add_topic("config/#", 1)
    .with_retain_handling(1)  // 1 = Send only on new subscription
    .build()
    .unwrap();
```

#### 5. Retain As Published

```rust
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/data", 1)
    .with_retain_as_published(true)  // Keep original retain flag
    .build()
    .unwrap();
```

#### 6. Subscription Identifier (MQTT v5)

```rust
// Track which subscription matched the message
let cmd = SubscribeCommand::builder()
    .add_topic("sensors/#", 1)
    .with_subscription_id(42)  // ID included in PUBLISH packets
    .build()
    .unwrap();
```

#### 7. Multiple Topics with Different Options

```rust
let cmd = SubscribeCommand::builder()
    // First topic: standard subscription
    .add_topic("sensors/temp", 1)
    
    // Second topic: no local, don't send retained
    .add_topic("devices/+/status", 1)
    .with_no_local(true)
    .with_retain_handling(2)
    
    // Third topic: retain as published
    .add_topic("config/#", 2)
    .with_retain_as_published(true)
    
    .with_subscription_id(100)  // Applies to all topics
    .build()
    .unwrap();
```

#### 8. Full Options Control

```rust
let cmd = SubscribeCommand::builder()
    .add_topic_with_options(
        "sensors/+/data",
        1,      // QoS
        true,   // no_local
        false,  // retain_as_published
        2       // retain_handling (don't send retained)
    )
    .with_subscription_id(42)
    .build()
    .unwrap();
```

### Method Reference

| Method | Parameters | Description | Default |
|--------|-----------|-------------|---------|
| `builder()` | - | Create new builder | - |
| `add_topic()` | `topic: String, qos: u8` | **Required**: Add topic with QoS | - |
| `add_topic_with_options()` | `topic, qos, no_local, rap, rh` | Add topic with all options | - |
| `with_no_local()` | `bool` | Set No Local for last topic | `false` |
| `with_retain_as_published()` | `bool` | Set Retain As Published for last topic | `false` |
| `with_retain_handling()` | `u8` (0-2) | Set Retain Handling for last topic | `0` |
| `with_subscription_id()` | `u32` | Set Subscription Identifier (MQTT v5) | None |
| `add_property()` | `Property` | Add custom MQTT v5 property | - |
| `with_packet_id()` | `u16` | Set packet ID (usually auto) | Auto |
| `build()` | - | Build SubscribeCommand | - |

### Subscription Options Explained

#### No Local (`no_local`)
- **`false` (default)**: Client receives messages it publishes to matching topics
- **`true`**: Client does NOT receive its own messages

**Use Case**: Prevent message loops when a client publishes and subscribes to the same topic.

```rust
.add_topic("sensors/my_sensor", 1)
.with_no_local(true)  // Don't echo back my own sensor data
```

#### Retain As Published (`retain_as_published`)
- **`false` (default)**: Broker clears retain flag (always 0) in forwarded messages
- **`true`**: Broker keeps original retain flag from publisher

**Use Case**: Distinguish between retained messages and live messages.

```rust
.add_topic("status/#", 1)
.with_retain_as_published(true)  // Know if message was retained
```

#### Retain Handling (`retain_handling`)
- **`0` (default)**: Send retained messages at subscribe time
- **`1`**: Send retained messages only if subscription doesn't exist yet
- **`2`**: Don't send retained messages at all

**Use Cases**:
- `0`: Get last known value immediately (status, configuration)
- `1`: Avoid duplicate retained messages on reconnect
- `2`: Only want live messages (real-time data streams)

```rust
// Live data stream only
.add_topic("sensors/stream", 1)
.with_retain_handling(2)  // Ignore retained messages

// Avoid duplicates on reconnect
.add_topic("config/#", 1)
.with_retain_handling(1)  // Only if new subscription
```

#### Subscription Identifier (`subscription_id`)
Numeric ID included in PUBLISH packets to indicate which subscription(s) matched.

**Use Case**: When subscribing to overlapping topics, know which subscription triggered the message.

```rust
let cmd1 = SubscribeCommand::builder()
    .add_topic("sensors/temp", 1)
    .with_subscription_id(1)
    .build()
    .unwrap();

let cmd2 = SubscribeCommand::builder()
    .add_topic("sensors/#", 0)
    .with_subscription_id(2)
    .build()
    .unwrap();

// Message to "sensors/temp" will include both IDs: [1, 2]
```

### Error Handling

The `build()` method returns `Result<SubscribeCommand, SubscribeBuilderError>`:

```rust
pub enum SubscribeBuilderError {
    NoTopics,  // No topics added
}
```

**Example**:

```rust
let result = SubscribeCommand::builder()
    .with_subscription_id(42)
    .build();

match result {
    Ok(cmd) => println!("Subscription created successfully"),
    Err(SubscribeBuilderError::NoTopics) => {
        eprintln!("Error: At least one topic is required");
    }
}
```

### Topic Wildcards

MQTT supports two wildcard characters in topic filters:

| Wildcard | Description | Example |
|----------|-------------|---------|
| `+` | Single-level wildcard | `sensors/+/temp` matches `sensors/room1/temp`, `sensors/room2/temp` |
| `#` | Multi-level wildcard | `sensors/#` matches `sensors/temp`, `sensors/room1/temp`, `sensors/room1/humidity` |

**Rules**:
- `+` matches exactly one topic level
- `#` must be the last character and matches zero or more levels
- `#` alone matches all topics

**Examples**:

```rust
// All temperature sensors
.add_topic("sensors/+/temp", 1)

// All sensors in a specific room
.add_topic("sensors/room1/#", 1)

// All topics (use with caution!)
.add_topic("#", 0)
```

### Best Practices

1. **Always Add Topics**: At least one `add_topic()` call is required.

2. **Choose Appropriate QoS per Topic**:
   ```rust
   .add_topic("telemetry/#", 0)    // High-frequency, loss acceptable
   .add_topic("commands/#", 1)     // Important commands
   .add_topic("transactions/#", 2) // Critical data
   ```

3. **Use No Local for Command Topics**:
   ```rust
   // Avoid processing own commands
   .add_topic("devices/+/commands", 1)
   .with_no_local(true)
   ```

4. **Subscription Identifiers for Overlapping Subscriptions**:
   ```rust
   // General monitoring (ID 1)
   client.subscribe(SubscribeCommand::builder()
       .add_topic("#", 0)
       .with_subscription_id(1)
       .build()?).await?;
   
   // Critical alerts (ID 2)
   client.subscribe(SubscribeCommand::builder()
       .add_topic("alerts/#", 2)
       .with_subscription_id(2)
       .build()?).await?;
   
   // Alert messages will have both IDs [1, 2]
   ```

5. **Retain Handling on Reconnect**:
   ```rust
   // Avoid getting retained messages again after reconnect
   .add_topic("status/#", 1)
   .with_retain_handling(1)  // Only send if new subscription
   ```

6. **Careful with Wildcards**:
   - `#` alone subscribes to ALL topics (high load)
   - Prefer specific patterns: `sensors/#` instead of `#`
   - Use `+` for controlled single-level matching

7. **Subscription Options Apply to Last Added Topic**:
   ```rust
   .add_topic("topic1", 1)
   .with_no_local(true)      // Applies to topic1
   .add_topic("topic2", 1)
   .with_retain_handling(2)  // Applies to topic2 only
   ```

8. **Validate Wildcard Usage**:
   ```rust
   // ✅ Valid wildcards
   "sensors/+/temp"
   "sensors/#"
   "sensors/+/data/#"
   
   // ❌ Invalid wildcards
   "sensors/+room/temp"  // + must be alone in level
   "sensors/#/temp"      // # must be last
   ```

---

## Reference

### Related Documentation

- **TokioAsyncClient API Guide**: `docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md`


### Version Information

- **Document Version**: 2.0
- **Last Updated**: October 16, 2025
- **Applies to**: flowsdk v0.1+
