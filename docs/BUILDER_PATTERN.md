# MqttClientOptions Builder Pattern

## Overview

The `MqttClientOptions` struct now supports both traditional struct initialization and a convenient builder pattern. The builder pattern provides a fluent, chainable API for configuring MQTT client options.

## Default Values

When using `MqttClientOptions::default()` or `MqttClientOptions::builder()`, you get:

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
}
```

## Usage Examples

### 1. Simple Builder Usage

```rust
use flowsdk::mqtt_client::MqttClientOptions;

let options = MqttClientOptions::builder()
    .peer("mqtt.example.com:1883")
    .client_id("my_awesome_client")
    .build();
```

### 2. Full Configuration

```rust
let options = MqttClientOptions::builder()
    .peer("mqtt.example.com:8883")
    .client_id("secure_client")
    .username("mqtt_user")
    .password(b"secret_password".to_vec())
    .clean_start(false)
    .keep_alive(120)
    .reconnect(true)
    .auto_ack(true)
    .session_expiry_interval(3600)
    .build();
```

### 3. With Authentication

```rust
let options = MqttClientOptions::builder()
    .peer("broker.emqx.io:1883")
    .client_id("authenticated_client")
    .username("my_username")
    .password(b"my_password".to_vec())
    .build();
```

### 4. With Session Expiry

```rust
// Session expires after 1 hour
let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("persistent_client")
    .clean_start(false)
    .session_expiry_interval(3600)
    .build();

// Session never expires
let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("forever_client")
    .session_expiry_interval(0xFFFFFFFF)
    .build();
```

### 5. With Auto-Subscribe Topics

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
        topic("sensors/humidity".to_string(),
        qos: 1,
        no_local: false,
        retain_as_published: false,
        retain_handling: 0,
    })
    .build();
```

### 6. Using Default with Overrides

```rust
// Start with defaults
let mut options = MqttClientOptions::default();
options.peer = "custom.broker.com:1883".to_string();
options.client_id = "custom_id".to_string();
```

### 7. Complete Example with TokioAsyncMqttClient

```rust
use std::sync::Arc;
use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};
use flowsdk::mqtt_client::event_handler::EventHandler;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Build options using the builder pattern
    let options = MqttClientOptions::builder()
        .peer("localhost:1883")
        .client_id("builder_example_client")
        .clean_start(true)
        .keep_alive(60)
        .reconnect(true)
        .auto_ack(true)
        .session_expiry_interval(7200) // 2 hours
        .build();

    // Create event handler
    let handler = Box::new(MyEventHandler);

    // Create client configuration
    let config = TokioAsyncClientConfig::default();

    // Create the client
    let client = Arc::new(
        TokioAsyncMqttClient::new(options, handler, config).await?
    );

    // Connect and use the client
    let result = client.connect_sync().await?;
    println!("Connected: {:?}", result);

    Ok(())
}
```

## Builder Methods Reference

| Method | Parameter Type | Description |
|--------|---------------|-------------|
| `builder()` | - | Create a new builder with default values |
| `peer()` | `impl Into<String>` | Set broker address (host:port) |
| `client_id()` | `impl Into<String>` | Set MQTT client identifier |
| `clean_start()` | `bool` | Set clean start flag |
| `keep_alive()` | `u16` | Set keep-alive interval in seconds |
| `username()` | `impl Into<String>` | Set authentication username |
| `password()` | `impl Into<Vec<u8>>` | Set authentication password |
| `will()` | `Will` | Set Last Will and Testament |
| `reconnect()` | `bool` | Enable/disable auto-reconnect |
| `sessionless()` | `bool` | Enable/disable sessionless mode |
| `subscription_topics()` | `Vec<TopicSubscription>` | Set all auto-subscribe topics |
| `add_subscription_topic()` | `TopicSubscription` | Add single auto-subscribe topic |
| `auto_ack()` | `bool` | Enable/disable automatic message acknowledgment |
| `session_expiry_interval()` | `u32` | Set session expiry in seconds |
| `build()` | - | Consume builder and return options |

## Comparison: Traditional vs Builder

### Traditional Struct Initialization

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
};
```

### Builder Pattern

```rust
let options = MqttClientOptions::builder()
    .peer("localhost:1883")
    .client_id("my_client")
    .build();
// All other fields use sensible defaults
```

## Benefits

1. **Sensible Defaults**: Most fields have reasonable default values
2. **Fluent API**: Chain method calls for clean, readable code
3. **Type Safety**: Compile-time checks for all parameters
4. **Flexibility**: Mix and match only the options you need
5. **Backward Compatible**: Traditional struct initialization still works

## Migration Guide

Existing code using struct initialization continues to work without changes. To migrate to the builder pattern:

**Before:**
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
};
```

**After:**
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
