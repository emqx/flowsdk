# TokioAsyncMqttClient API Guide

**Version**: 1.1
**Last Updated**: September 22, 2026

## Overview

`TokioAsyncMqttClient` runs the shared MQTT engine on Tokio for MQTT 3.1.1 and
5.0. All its operations are asynchronous. Methods named `*_sync` await an
acknowledgement or local completion; they do not block a runtime thread.

### Operation completion and errors

| API | What successful completion means |
| --- | --- |
| `connect`, `subscribe`, `publish`, `unsubscribe`, `ping`, `auth`, `send_packet` | The command entered the worker queue; subsequent errors go to `on_error` |
| `connect_sync` | An actual CONNACK arrived; concurrent callers share its result and calls while connected return its cached metadata |
| `subscribe_sync`, `unsubscribe_sync`, QoS 1/2 `publish_sync`, `ping_sync` | The corresponding broker response arrived |
| QoS 0 `publish_sync` | The engine validated and accepted the publication into its output queue |
| `publish_with_command_admitted` (all QoS levels) | The engine validated and admitted this publication; no transport write or broker acknowledgement is promised |
| `try_publish_tracked` | A command entered the worker queue and returned an operation receipt without yielding |
| `PublishOperation::wait` | Engine admission for QoS 0; PUBACK/PUBCOMP or a broker rejection for QoS 1/2 |
| `PublishDrain::drain_through` | This operation's initial transport write completed locally; broker delivery is not implied |
| `puback`, `pubrec`, `pubcomp` | The engine validated the receive stage, queued the acknowledgement, and updated its state |
| `disconnect`, `disconnect_with` | A disconnect command entered the worker queue |
| `disconnect_sync`, `disconnect_with_sync` | DISCONNECT was written and the local transport closed |
| `shutdown(self)` | The client attempted graceful disconnect and the worker terminated |

Acknowledgement-waiting methods return local validation, operation-deadline,
session-loss, and terminal connection errors directly as `MqttClientError`.
Broker acknowledgement reason codes and properties remain in their result
objects: inspect `is_success()` when a broker can reject an operation.

An acknowledgement timeout or cancelled waiting future does not release the
engine's packet ID or cancel an already accepted MQTT exchange. A late valid
acknowledgement can still complete it and produce a callback. Engine deadlines
from `MqttClientOptions::operation_timeouts` are honored even if the wrapper's
corresponding timeout is disabled.

Callbacks execute on the worker. Keep them short and send work to another task
before awaiting acknowledgement/completion methods or `shutdown`. Awaiting
those methods inside a callback would wait for the same worker that is running
the callback. Local `publish_with_command_admitted` is the exception: its
dedicated bounded queue is serviced while a callback is waiting, without socket
I/O or recursive event delivery.

### Tracked publication, cancellation, and local drain

`try_publish_tracked(command, deadline)` returns a `PublishOperation` immediately
when the bounded command queue has space. A full queue returns `BufferFull`
without accepting the publication. The deadline is an absolute
`tokio::time::Instant` shared by queue admission and confirmation waiting.

```rust
use flowsdk::mqtt_client::commands::PublishCommand;
use tokio::time::{Duration, Instant};

let deadline = Instant::now() + Duration::from_secs(5);
let mut operation = client.try_publish_tracked(
    PublishCommand::simple("sensors/temp", b"22.5".to_vec(), 0, false),
    deadline,
)?;
let drain = operation.drain_receipt();
match operation.wait().await {
    Ok(result) if result.is_success() => {
        drain.drain_through(deadline).await?;
    }
    Ok(result) => eprintln!("Broker rejected publication: {:?}", result.reason_code),
    Err(error) => eprintln!("Publish failed (admitted={}): {}", error.admitted, error.error),
}
```

Keep the operation handle when cancelling its `wait()` future, then resume the
same wait to retain its identity and original deadline. Confirmations recorded
before the deadline remain available when the wait resumes afterward; confirmations
recorded at or after the deadline produce `OperationTimeout`, even without an active
waiter. Dropping the handle or expiring its deadline before engine admission prevents
later publication; cancellation and admission are arbitrated under the same lock.
After admission, dropping the handle or timing out a wait does not retract the MQTT exchange.
`TrackedPublishError::admitted` distinguishes these cases. A missing confirmation
after admission leaves delivery uncertain, and retrying may duplicate output.
Broker rejection codes remain in `PublishResult`; inspect `is_success()`.

The receipt identity is independent of a reusable MQTT packet ID. QoS 1/2 still
use the engine's session recovery. The tracked path requires a connected engine
and bypasses offline buffering and priority scheduling. Tracked QoS 0 is never
replayed after transport loss.

A `PublishDrain` waits for the operation's initial local transport write and flush
without disconnecting the client. Both share one transport timeout budget.
Partial writes, flush failures, and transport timeouts fail the fence; a completed
fence remains successful after later transport loss. `same_connection()` compares
the connection generations in which two operations were admitted, and
`is_written()` reports successful local write and flush completion. Retain an older
failed fence when accumulating progress across reconnects so later writes do not
hide that failure. QoS 1/2 confirmation remains the operation receipt's responsibility.

Tracked commands use the normal worker queue. A blocked callback can delay their
admission and transport progress; do not await tracked completion inside a
callback. The existing callback-safe admission API retains its dedicated queue.

### Configuration precedence and reconnect

- Async `receive_maximum` configures the incoming limit advertised in CONNECT.
  Core `receive_maximum` still limits outgoing publications independently.
- Async `topic_alias_maximum` merges into CONNECT properties. Missing async
  values preserve core settings; equal values are deduplicated and conflicting
  explicit settings are rejected.
- Async `auto_reconnect` controls automatic reconnect after connection failure
  or loss, overriding `MqttClientOptions::reconnect`. It defaults to `true`,
  including with `with_default_config`. To disable retries, use
  `TokioAsyncClientConfig::builder().auto_reconnect(false)`; setting only the
  core reconnect flag to `false` no longer disables them. Async retry count and
  maximum delay also configure the engine; the base delay comes from
  `MqttClientOptions`.
- `set_auto_reconnect(false)` cancels scheduled retries. It does not cancel
  operations on an active connection. An intentional disconnect suppresses
  retries until an explicit new `connect`/`connect_sync` call.
- `buffer_messages(false)` rejects publications accepted while disconnected.
  `max_buffer_size` bounds queued offline publications. With priorities enabled,
  the packet budget is the smaller of `priority_queue_limit` and core
  `max_outgoing_packet_count`. Full queues reject new publications without
  evicting accepted exchanges. Disabling priorities gives queued publications
  equal priority and FIFO ordering.
- `default_operation_timeout_ms` bounds transport writes, closes, and manual
  acknowledgement/completion waits. It and `command_queue_size` must be nonzero.
  `no_timeouts()` disables broker-response waits; transport cleanup stays bounded.
- Native TLS and rustls use the TLS configuration supplied in `MqttClientOptions`,
  including custom trust roots and client credentials.
- `MqttClientOptions::retransmit_on_timer` defaults to `true`. Set it to `false`
  to disable MQTT 3 periodic retransmission and its timer wakeups while preserving
  QoS 1/2 replay during session recovery on reconnect. MQTT 5 already replays only
  on reconnect.

### Manual acknowledgements and disconnect metadata

Set `MqttClientOptions::auto_ack(false)` and use:

```rust
client.puback(packet_id, 0, vec![]).await?; // incoming QoS 1
client.pubrec(packet_id, 0, vec![]).await?; // accept incoming QoS 2
// After on_pubrel_received(packet_id):
client.pubcomp(packet_id, 0, vec![]).await?;
```

MQTT 3.1.1 uses reason zero and no properties. Raw PUBACK/PUBREC/PUBCOMP commands
submitted through `send_packet` use these same state transitions; their errors
are delivered through `on_error`.

`disconnect_with` and `disconnect_with_sync` accept a reason code and MQTT 5
properties. A broker DISCONNECT invokes the new default
`on_disconnect_received(reason_code, properties)` callback, followed by the
existing `on_disconnected` callback. Local disconnect reports `on_disconnected`
once after closing the transport; a later shutdown does not repeat it.

This wrapper uses a full MQTT parser and one byte-stream transport, including
its single-stream QUIC transport. Parser-depth controls and detailed QUIC
stream/0-RTT events are available through the lower-level engine clients;
QUIC stream routing and rebinding are available through `TokioQuicMqttClient`.

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
- ✅ Shareable between tasks through `Arc`
- ✅ Raw packet commands; malformed-packet testing helpers require `protocol-testing`

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
pub async fn shutdown(self) -> io::Result<()>
```

**Description**: Attempts graceful disconnect, closes the transport, and waits for the worker to terminate.
**Use case**: Final cleanup before dropping client

**Example**:
```rust
client.disconnect_sync().await?;
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
- QoS 0: Returns after engine validation and enqueueing (no ACK)
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

#### `publish_with_command_admitted()` - Correlated Local Admission

```rust
pub async fn publish_with_command_admitted(
    &self,
    command: PublishCommand,
) -> Result<Option<u16>, MqttClientError>
```

Use this method when an application needs the result of local validation and
capacity checks without waiting for PUBACK or PUBCOMP. It returns `None` for
QoS 0 and the assigned packet identifier for QoS 1/2. Offline buffering still
follows the client configuration. Local rejection is returned to this caller
without a duplicate generic error callback; it does not itself mark the
connection as lost. Later protocol/transport outcomes remain event-handler
notifications. Packet identifiers may be reused after an exchange completes.

The existing `publish_with_command` remains enqueue-only. The existing
`publish_with_command_sync` still waits for broker acknowledgement at QoS 1/2.
Admission adds a worker round trip, so it is a correctness choice rather than
a promise of lower per-call latency. For concurrent publishing, bound the number
of outstanding application operations instead of spawning an unbounded task
per message.

Admission uses a separate queue with `command_queue_size` slots, in addition to
the existing command queue. It remains serviced while an event callback waits
on downstream capacity, so a shared source/sink can obtain local admission or
a precise capacity rejection without first draining that callback. Callback
completion has priority; no additional socket reads, writes, or recursive
callbacks occur while it waits. Output admitted during a callback is flushed
promptly after the callback completes, without waiting for a new input or timer.

The admission queue is FIFO internally, but it has no enqueue-order guarantee
with the legacy command queue. Await `connect_sync` when connection completion
must precede admission; enqueueing `connect` alone is not a sequencing barrier.
During saturation, admission can return `BufferFull`. This is bounded progress
of the local request, not a guarantee of successful delivery under arbitrary
backpressure or indefinite application retries. Event consumption must resume
for transport output and acknowledgements to progress.

There is no implicit acknowledgement timeout for admission. If the application
times out or drops the future after enqueueing, the publish can still proceed;
do not retry on the assumption that cancellation retracted it. Shutdown closes
the admission queue and rejects requests that have not yet been admitted.

#### `publish_batch_admitted()` - Bounded Local Admission

```rust
pub async fn publish_batch_admitted(
    &self,
    commands: Vec<PublishCommand>,
) -> Result<PublishBatchAdmission, PublishBatchAdmissionError>;
```

This submits independent MQTT PUBLISH commands in one admission request and
receives one report. It does not merge payloads or application records. Each
publication still undergoes the normal engine validation, encoding, queue,
packet-identifier and negotiated-capability checks.

Enable the API with explicit budgets before creating the client:

```rust
use flowsdk::mqtt_client::{PublishAdmissionConfig, TokioAsyncClientConfig};

let config = TokioAsyncClientConfig::builder()
    .publish_admission(PublishAdmissionConfig {
        max_batch_messages: 32,
        max_batch_bytes: 256 * 1024,
        max_pending_messages: 128,
        max_pending_bytes: 4 * 1024 * 1024,
    })
    .build();
```

All limits must be nonzero, and each maximum batch must fit the corresponding
pending limit. Invalid configuration fails client creation before spawning its
worker. `publish_admission: None` (the default) preserves the existing single
admission behavior and rejects nonempty batches before submission. Empty batches
always return an empty report locally.

```rust
use flowsdk::mqtt_client::{
    PublishCommand, PublishBatchAdmissionError, TokioAsyncMqttClient,
};

async fn submit(client: &TokioAsyncMqttClient) -> Result<(), PublishBatchAdmissionError> {
    let report = client.publish_batch_admitted(vec![
        PublishCommand::simple("telemetry/a", vec![1, 2], 0, false),
        PublishCommand::simple("telemetry/b", vec![3, 4], 1, false),
    ]).await?;

    for (index, result) in report.results.into_iter().enumerate() {
        match result {
            Ok(id) => println!("item {index} admitted, packet ID {id:?}"),
            Err(error) => println!("item {index} not admitted: {error}"),
        }
    }
    Ok(())
}
```

`PublishBatchAdmission::results` has exactly one entry per input command, in
input order. `Ok(None)` admits QoS 0 and `Ok(Some(id))` admits QoS 1/2. An inner
error means that item was not admitted. The outer `Ok(report)` can contain any
mix of accepted and rejected items, including only rejections. Local rejections
are not duplicated as generic error callbacks.

Processing is sequential and nontransactional. Earlier successes can consume
capacity, reserve IDs, or establish topic aliases used by later items. An error
neither rolls back earlier items nor skips later ones. There is no wait for
socket I/O, broker ACKs or engine capacity between items; capacity failures are
reported per item. Consequently a batch can encounter capacity failures that
individual calls separated by socket progress would avoid. Acceptance is still
local admission, not a write or broker-delivery guarantee.

Single and batch requests share one FIFO admission lane, beginning at channel
enqueue. Callers competing for budget credits have no FIFO reservation guarantee.
A batch is one bounded, synchronous worker turn, so another admission or shutdown cannot interleave with
its items. The worker returns to its scheduling loop after that request. This
adds no ordering guarantee with the legacy command channel or over MQTT priority
and replay scheduling. An accepted prefix of a batch is not rolled back.

When configured, single and batch requests share pending count/byte credits.
A batch reserves both dimensions together before obtaining a channel slot.
Temporary exhaustion waits asynchronously; a request larger than its applicable
limit fails before enqueueing. Cancellation releases any reservation. Channel
closure wakes callers waiting for credits. A single publication is subject to
pending limits, not the smaller per-batch limits. `command_queue_size` continues
to bound request envelopes independently.

Logical bytes are `size_of::<PublishCommand>()` per command, visible payload and
topic bytes, `size_of::<Property>()` per property, and each property's dynamic
string/binary contents. Accounting uses checked arithmetic and does not encode
or copy payloads. Credits cover requests that have reserved capacity, including
those waiting for a channel slot, queued requests, and the active request. They
are released when processing finishes or a request is dropped. They do not bound
caller-owned futures still waiting to reserve credits, result storage, spare
allocation capacity, or the backing allocation retained by a small `Bytes` slice.
Engine queues, inflight payloads and extracted transport buffers need their own
limits; the admission budget is not a total-process-memory bound.

`PublishBatchAdmissionError` distinguishes:

* `NotSubmitted { error, commands }`: no item entered the admission channel; all
  original commands are returned unchanged, including their payload ownership.
* `OutcomeUnknown { error }`: the request entered the channel, but its report was
  lost. Any subset may have been accepted. Retrying the entire batch can duplicate
  publications.

There is no implicit timeout. Dropping the caller future after enqueueing does
not retract its publications, and a caller timeout cannot establish that nothing
was submitted. Graceful shutdown rejects queued unprocessed batches with one
`OperationCancelled` result per command. Closing drains outstanding channel
reservations as well as already-enqueued requests, bounded by
`default_operation_timeout_ms`. A stalled reservation can time out; transport
cleanup still runs, but reports lost on timeout or abrupt termination have unknown
outcomes.
Per-item engine errors consume their commands, matching single admission; retain
application retry state when needed. The `shared-payloads` feature makes retaining
payload handles inexpensive without changing wire-encoding copies.

The same batch path remains available while an event callback is backpressured.
It performs no socket I/O or recursive callbacks there. A known failed transport
rejects admission, and newly admitted output waits for callback completion before
normal draining. Batch admission alone does not remove that transport constraint.

The API and its budgets are gated by `async-client`; owned-only `no_std + alloc`
continues to use the sans-I/O engine without Tokio or atomics. No sans-I/O batch
helper is needed: call its synchronous `publish()` repeatedly before draining
output. Applications such as VeloFlux must collect multiple pending deliveries to
benefit; calling this method with one item preserves the per-request roundtrip.

#### Sharing Existing Payload Buffers (0.8)

Enable `flowsdk`'s `shared-payloads` Cargo feature to use this API. Without it,
payloads use owned `Vec<u8>` storage, including in the alloc-only core on targets
without pointer atomics. See the [0.8 configuration guide](MIGRATING_0_8.md).

`PublishCommand::from_bytes(topic, payload, qos, retain)` accepts an owned
`bytes::Bytes` and keeps its backing storage through command queues, packets,
inflight state, and reconnect replay. Shared, sliced, static, and uniquely owned
buffers do not need a replacement payload allocation, even when the application
retains another handle for retries. Encoding still writes the payload into a wire
buffer. A small slice retains its entire backing allocation.

```rust
use bytes::Bytes;
use flowsdk::mqtt_client::{MqttClientError, PublishCommand, TokioAsyncMqttClient};

async fn submit(
    client: &TokioAsyncMqttClient,
    payload: Bytes,
) -> Result<Option<u16>, MqttClientError> {
    // A separate task must continue draining the client's events.
    let command = PublishCommand::from_bytes("sensors/temp", payload, 1, false);
    client.publish_with_command_admitted(command).await
}
```

The engine consumes commands using `into_mqtt_publish` / `into_mqttv3_publish`,
moving the topic, payload, and MQTT 5 properties. The borrowed `to_*` conversions
remain available: they clone metadata and share the payload. Owned receive batches
let an adapter move `message.topic_name` and `message.payload` into its downstream
channel. `PublishCommand::builder().payload_bytes(payload)` also preserves a shared
buffer; the existing `.payload(...)` builder method accepts inputs convertible to
`Vec<u8>` and may copy borrowed or shared input.

The public command and PUBLISH packet `payload` fields changed from `Vec<u8>` to
`Bytes` in 0.8. See [the migration guide](MIGRATING_0_8.md) for struct literals,
mutation, serialization, memory accounting, and binding compatibility.

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

### Owned ordered event batches

Override `on_event_batch(&mut self, events: Vec<TokioMqttEvent>)` to receive one
owned vector per non-empty engine dispatch. The default implementation calls
the existing individual callbacks in order, so existing handlers need no changes.
Overriding it replaces those callbacks; handle all relevant event variants in
the batch consumer. `TokioMqttEvent` is now `#[non_exhaustive]`: downstream matches
must include a wildcard arm, even if they handle every currently known variant.
Adding this attribute and the `DisconnectReceived`, `PubRelReceived`, and
`EngineEvent` variants is a breaking API change for existing exhaustive matches.
The wildcard arm allows future variants to be added without another such break;
treat unknown variants as ordering boundaries just like other non-message events.

Each read, timer tick, and outgoing drain has its own dispatch boundary. The
worker does not read more data, drain a callback channel, or wait to fill a batch.
Worker-generated errors and lifecycle notifications also use this callback.
Transport failures deliver `Error`, `Disconnected`, and `ConnectionLost` in one
batch, in that order; the last two are omitted if already disconnected.
These boundaries depend on local transport reads and parsing, so they are not
guaranteed to match the broker's writes.

Consecutive `MessageReceived` events can be processed together. Stop at the
first other event or the end of the vector. For example,
`[MessageReceived(A), MessageReceived(B), PingResponse, MessageReceived(C)]`
contains message runs `[A, B]` and `[C]`, with the ping handled between them.
Do not filter out other events before grouping messages or combine runs across
vectors. The adapter folds redundant PUBLISH metadata into a fully parsed
message. Other engine events without a dedicated high-level callback remain
visible as `EngineEvent`, including metadata-only PUBLISH events. This also
includes `Disconnected` when the worker is already disconnected, every
`ReconnectNeeded`, and `ReconnectScheduled` when automatic reconnection is
disabled or intentionally stopped. The default callback adapter ignores these
wrapped events; batch consumers must preserve their ordering boundaries.

```rust
use async_trait::async_trait;
use flowsdk::mqtt_client::{TokioMqttEvent, TokioMqttEventHandler};
use tokio::sync::mpsc;

struct BatchForwarder {
    tx: mpsc::Sender<Vec<TokioMqttEvent>>,
}

#[async_trait]
impl TokioMqttEventHandler for BatchForwarder {
    async fn on_event_batch(&mut self, events: Vec<TokioMqttEvent>) {
        // One channel send per batch, moving ownership of every message.
        let _ = self.tx.send(events).await;
    }
}

async fn consume_batches(mut rx: mpsc::Receiver<Vec<TokioMqttEvent>>) {
    while let Some(batch) = rx.recv().await {
        let mut events = batch.into_iter().peekable();
        while let Some(event) = events.next() {
            match event {
                TokioMqttEvent::MessageReceived(first) => {
                    let mut messages = vec![first];
                    while matches!(events.peek(), Some(TokioMqttEvent::MessageReceived(_))) {
                        if let Some(TokioMqttEvent::MessageReceived(message)) = events.next() {
                            messages.push(message);
                        }
                    }
                    // Replace with application batch processing.
                    println!("Received {} consecutive messages", messages.len());
                }
                other => println!("Event: {other:?}"),
            }
        }
    }
}
```

Pass `Box::new(BatchForwarder { tx })` to `TokioAsyncMqttClient::new` and run the
consumer in another task. Callbacks execute on the transport worker. A bounded
channel applies backpressure to I/O. Local publish admission is serviced while
the callback waits, but completion/acknowledgement operations still require the
callback to return. Keep the consumer progressing and delegate those operations
to another task. A full local output queue produces a correlated admission
rejection; it does not justify unbounded buffering or indefinite retries.
For engine batches, protocol state and pending operation results are updated
before delivery, including connect, publish, subscribe, unsubscribe, and ping completion. Manual
acknowledgement calls still belong in a separate task; `PubRelReceived` marks
when a QoS 2 exchange is ready for PUBCOMP.

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
    async fn on_message_received(&mut self, publish: &MqttMessage) {}
    
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
    
    async fn on_message_received(&mut self, publish: &MqttMessage) {
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

// 2. Flush DISCONNECT and close the transport
client.disconnect_sync().await?;

// 3. Wait for the client worker to terminate
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

