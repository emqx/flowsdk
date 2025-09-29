# Async MQTT Client

A thread-safe, event-driven MQTT client that runs in a background thread and communicates via callbacks.

## Features

- **Thread-Safe**: Can be used from multiple threads safely
- **Event-Driven**: All MQTT operations trigger callbacks
- **Non-Blocking**: API calls return immediately, events come via callbacks
- **Auto-Reconnection**: Automatic reconnection with exponential backoff
- **Message Buffering**: Buffer messages during disconnection
- **Clean Shutdown**: Graceful thread termination

## Usage

### 1. Implement Event Handler

```rust
use flowsdk::mqtt_client::{MqttEventHandler, AsyncMqttClient, MqttClientOptions, AsyncClientConfig};
use flowsdk::mqtt_client::client::{ConnectionResult, PublishResult, SubscribeResult};
use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
use std::io;

struct MyHandler {
    name: String,
}

impl MqttEventHandler for MyHandler {
    fn on_connected(&mut self, result: &ConnectionResult) {
        if result.is_success() {
            println!("Connected! Session present: {}", result.session_present);
        } else {
            println!("Connection failed: {}", result.reason_description());
        }
    }

    fn on_message_received(&mut self, publish: &MqttPublish) {
        println!("Message on '{}': {:?}", 
            publish.topic_name, 
            String::from_utf8_lossy(&publish.payload)
        );
    }

    fn on_error(&mut self, error: &io::Error) {
        println!("Error: {}", error);
    }
    
    // ... implement other callbacks as needed
}
```

### 2. Create and Configure Client

```rust
// MQTT configuration
let mqtt_options = MqttClientOptions {
    client_id: "my_async_client".to_string(),
    clean_start: true,
    keep_alive: 60,
    username: None,
    password: None,
    will: None,
    reconnect: true,
    sessionless: false,
    subscription_topics: vec![],
    auto_ack: false,
};

// Async client configuration
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
let handler = Box::new(MyHandler { name: "Client".to_string() });

// Create async client
let client = AsyncMqttClient::new(mqtt_options, handler, async_config)?;
```

### 3. Use the Client

```rust
// Connect (non-blocking)
client.connect()?;

// Subscribe (non-blocking)
client.subscribe("sensor/+/temperature", 1)?;
client.subscribe("alerts/#", 2)?;

// Publish (non-blocking)
client.publish("sensor/1/temperature", b"23.5", 1, false)?;
client.publish("alerts/high_temp", b"Temperature too high!", 2, true)?;

// Ping (non-blocking)
client.ping()?;

// Unsubscribe (non-blocking)
client.unsubscribe(vec!["sensor/+/temperature"])?;

// Disconnect (non-blocking)
client.disconnect()?;

// Shutdown and wait for worker thread
client.shutdown()?;
```

## Event Types

The `MqttEvent` enum contains all possible events:

```rust
pub enum MqttEvent {
    Connected(ConnectionResult),           // Connection established
    Disconnected(Option<u8>),             // Disconnected (reason code)
    Published(PublishResult),              // Message published
    Subscribed(SubscribeResult),           // Subscription completed
    Unsubscribed(UnsubscribeResult),       // Unsubscription completed
    MessageReceived(MqttPublish),          // Incoming message
    PingResponse(PingResult),              // Ping response
    Error(io::Error),                      // Error occurred
    PeerCertReceived(Vec<u8>),            // TLS certificate (for validation)
    ConnectionLost,                        // Connection lost unexpectedly
    ReconnectAttempt(u32),                // Reconnection attempt (number)
    PendingOperationsCleared,             // Pending ops cleared on reconnect
}
```

## Configuration Options

### AsyncClientConfig

- `auto_reconnect`: Enable automatic reconnection (default: true)
- `reconnect_delay_ms`: Initial reconnect delay (default: 1000ms)
- `max_reconnect_delay_ms`: Maximum reconnect delay (default: 30000ms)
- `max_reconnect_attempts`: Max reconnect attempts, 0 = infinite (default: 0)
- `command_queue_size`: Command queue size (default: 100)
- `buffer_messages`: Buffer messages during disconnection (default: true)
- `max_buffer_size`: Maximum message buffer size (default: 1000)

## Thread Safety

The `AsyncMqttClient` is thread-safe and can be shared between threads:

```rust
use std::sync::Arc;

let client = Arc::new(AsyncMqttClient::new(options, handler, config)?);

// Clone for use in different threads
let client_clone = Arc::clone(&client);
std::thread::spawn(move || {
    client_clone.publish("thread/message", b"Hello from thread!", 1, false).unwrap();
});
```

## Error Handling

All errors are reported via the `on_error` callback. The client automatically handles:

- Connection failures (with reconnection if enabled)
- Network timeouts
- Parse errors
- Protocol violations

## Reconnection

When `auto_reconnect` is enabled:

1. Connection loss triggers `on_connection_lost()`
2. Reconnection attempts start with exponential backoff
3. Each attempt triggers `on_reconnect_attempt(attempt_number)`
4. On successful reconnection:
   - Pending subscriptions are restored
   - Buffered messages are sent
   - `on_connected()` is called

## Message Buffering

When `buffer_messages` is enabled:

- Messages published during disconnection are buffered
- Buffer has configurable maximum size
- Oldest messages are dropped when buffer is full
- All buffered messages are sent on reconnection

## Shutdown

Always call `shutdown()` to cleanly terminate:

```rust
// Graceful shutdown
client.shutdown()?;
```

This will:
1. Send disconnect packet
2. Stop the worker thread
3. Clean up resources

## Examples

See `bin/examples/async_mqtt_client_example.rs` for a complete working example.

## Running the Example

```bash
cargo run --bin async_mqtt_client_example
```