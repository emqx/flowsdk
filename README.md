# FlowSDK

FlowSDK is a safety-first, realistic, behavior-predictable messaging SDK.

With FlowSDK, you can build messaging-based [micro-middleware](#micro-middleware-functions) that run in your app and communicate with other apps on the same host or remotely.

FlowSDK leverages multiple protocols (MQTT and gRPC for now) across different layers and makes the best use of each for real-world scenarios.

## What is Flow?

The definition of flow changes from iteration to iteration. 

- 0.1:  `Flow` stands for  **[Everything will flow](https://flowmq.io/blog/everything-will-flow)**

Technically, Flow is the projection of data streaming from many sources with the help of [micro-middleware functions](#micro-middleware-functions).

## AI / LLM-friendly

FlowSDK is designed to be AI-friendly. The public APIs and documentation are written with explicit, consistent naming, examples, and structured sections so they can be easily consumed by large language models and other automated tools for code generation, testing, and analysis. This makes it straightforward to integrate Flow into AI-driven workflows, generate example code, or use LLMs to assist with SDK integration. SEE [Doc](#Documentation)

## Micro-middleware functions

- Messaging client
- Pub/Sub broker
- Filter
- Proxy
- Protocol relay
- Queue
- Table
- Relay

## Be realistic

Messages are not created equal. Messaging has costs, resources are limited, and lower network/transport layers are not always reliable. FlowSDK intentionally surfaces errors and resource constraints instead of hiding them; this helps you identify trade-offs early and design a resilient system, reduces surprises in production.

For example, with FlowSDK, user should look for **acceptable** latency instead of **lowest** latency with all the tunable parts of timeout, QoS, priority, reconnect/backoff policies. 

## Communication models

- Req/Resp
- Pub/Sub
- Stream
- Reliable delivery
- Unreliable delivery

## Protocols

- MQTT
- gRPC
- ...

## Current status

Work in progress

## Project structure

This project is organized as a Cargo workspace with two main components:

### 📚 Core Library (`flowsdk`)
- **MQTT Protocol**: Complete serialization/deserialization implementation
- **MQTT Client Library**: TokioAsyncMqttClient with async/await support
- **Shared Conversions**: gRPC ↔ MQTT conversion utilities
- **Example Applications**: Simple client/server demos

### 🔗 Proxy Workspace (`mqtt_grpc_duality/`)  
- **Dedicated Proxy Applications**: Client-facing (`r-proxy`) and server-side (`s-proxy`) proxies
- **Self-contained**: Own protobuf definitions and shared conversion logic
- **Production Ready**: Optimized for deployment scenarios

## Quick Start

### Build Everything
```bash
# Build both main library and proxy workspace
cargo build --workspace
```

### Build Individual Components
```bash
# Main library and examples only
cargo build

# Proxy applications only  
cd mqtt_grpc_duality && cargo build
```

### Run Proxy Applications
```bash
# Start server-side proxy (connects to MQTT broker)
cd mqtt_grpc_duality && cargo run --bin s-proxy

# In another terminal, start client-facing proxy
cd mqtt_grpc_duality && cargo run --bin r-proxy
```

## Architecture

### Component Details

#### Core Library Components
- **`mqtt_serde`**: MQTT protocol Encoder and Decoder, serialization and deserialization of MQTT packets
- **`mqtt_client`**: TokioAsyncMqttClient - Production-ready async MQTT v5.0 client


#### Proxy Components (in `mqtt_grpc_duality/` workspace)
see [mqtt_grpc_duality README.md](mqtt_grpc_duality/README.md)

## Features

### MQTT v5.0 Client
- ✅ **Full Async/Await Support** - Built on Tokio runtime
- ✅ **Dual API Design** - Fire-and-forget async + wait-for-ACK sync operations
- ✅ **Advanced Subscriptions** - Builder pattern with No Local, Retain Handling, Retain As Published
- ✅ **Flow Control** - Receive Maximum, Topic Alias Maximum
- ✅ **Configurable Timeouts** - Network-specific presets (local/internet/satellite)
- ✅ **Auto Reconnection** - Exponential backoff with message buffering
- ✅ **Event-Driven** - Comprehensive callback system for all MQTT events
- ✅ **Thread-Safe** - Clone-friendly, safe for concurrent use

### Transport Layer
- ✅ **TCP Transport** - Traditional TCP connections
- ✅ **TLS Transport** - Secure TLS/SSL connections (feature-gated)
- ✅ **QUIC Transport** - Modern QUIC protocol with built-in encryption (feature-gated)

### MQTT v5.0 Protocol Support
- ✅ All control packet types (Connect, Publish, Subscribe, etc.)
- ✅ QoS 0, 1, and 2 message flows
- ✅ Properties support for enhanced metadata
- ✅ Authentication and session management
- ✅ Protocol compliance validation
- ✅ Shared subscriptions support
- ✅ Client session

### Protocol Testing Infrastructure ⚠️ (Feature-Gated)
- ✅ **Raw Packet API** - Low-level packet manipulation for testing
- ✅ **Malformed Packet Generator** - 20+ pre-built protocol violations
- ✅ **Raw Test Client** - Direct TCP access bypassing MQTT protocol
- 📋 **Protocol Compliance Tests** - Infrastructure ready, 84% coverage achievable (0/185 implemented)
- ⚠️ **Test-Only** - Behind `protocol-testing` feature flag for safety

### Performance & Reliability
- ✅ Zero-copy deserialization where possible
- ✅ Concurrent connection handling with `tokio`
- ✅ Memory-efficient streaming
- ✅ Comprehensive error handling

### Feature Flags (Compiling)

**Standard Features** (always available):
- Default MQTT v5.0 client functionality
- All standard operations and APIs

**Optional Features**:
- `quic` - Enables QUIC transport support (requires `quinn`, `rustls`, `rustls-native-certs`, `rustls-pki-types`)
  - QuicTransport - QUIC-based transport implementation
  - QuicConfig - Configuration with ALPN, 0-RTT, custom roots, mTLS support
  - PEM file loading helpers for certificates and keys
  - **Use case**: High-performance, low-latency connections with built-in encryption for mobile network.

- `protocol-testing` - ⚠️ **DANGEROUS** - Enables raw packet API for protocol compliance testing
  - RawPacketBuilder - packet manipulation
  - RawTestClient - direct TCP access
  - MalformedPacketGenerator - protocol violation generators
  - **WARNING**: Creates malformed packets, test-only, never use in production

```toml
# Enable QUIC transport
[dependencies]
flowsdk = { version = "0.1", features = ["quic"] }

# Enable protocol testing features
[dependencies]
flowsdk = { version = "0.1", features = ["protocol-testing"] }
```

### Fuzz Testing

The project includes continuous fuzz testing using [ClusterFuzzLite](https://google.github.io/clusterfuzzlite/) integrated with GitHub Actions.

**Fuzz Targets:**
- `fuzz_parser_funs`: Tests MQTT packet parsing functions
- `fuzz_mqtt_packet_symmetric`: Tests packet serialization/deserialization symmetry

**Running Locally:**
```bash
# Install cargo-fuzz (requires nightly Rust)
cargo install cargo-fuzz

# Run a specific fuzz target
cd fuzz
cargo fuzz run fuzz_parser_funs -- -max_total_time=60

# Run with coverage
cargo fuzz coverage fuzz_parser_funs
```

**Automated Fuzzing:**
- **PR Fuzzing**: Runs on pull requests for 5 minutes per target
- **Batch Fuzzing**: Runs on main branch pushes for 10 minutes per target
- **Continuous Fuzzing**: Runs daily for 1 hour per target
- **Sanitizers**: Tests with AddressSanitizer, UndefinedBehaviorSanitizer, and MemorySanitizer

Fuzzing results are automatically uploaded as GitHub Security Alerts (SARIF format).

## Usage Examples

### MQTT Client - Quick Start

```rust
use flowsdk::mqtt_client::{
    MqttClientOptions, 
    TokioAsyncClientConfig, 
    TokioAsyncMqttClient,
    TokioMqttEventHandler,
};

// Create client options
let options = MqttClientOptions::builder()
    .peer("mqtt.example.com:1883")
    .client_id("my_client")
    .clean_start(true)
    .keep_alive(60)
    .build();

// Create event handler
let handler = Box::new(MyEventHandler::new());

// Create client with config
let config = TokioAsyncClientConfig::builder()
    .auto_reconnect(true)
    .internet_timeouts()
    .receive_maximum(100)
    .build();

let client = TokioAsyncMqttClient::new(options, handler, config).await?;

// Connect and subscribe
client.connect_sync().await?;
client.subscribe_sync("sensors/#", 1).await?;

// Publish message
client.publish_sync("sensors/temp", b"23.5", 1, false).await?;
```

### Advanced Subscription with Builder

```rust
use flowsdk::mqtt_client::tokio_async_client::SubscribeCommand;

let cmd = SubscribeCommand::builder()
    .add_topic("sensors/+/temp", 1)
    .with_no_local(true)           // Don't receive own messages
    .with_retain_handling(2)       // Don't send retained messages
    .with_subscription_id(42)      // Track which subscription matched
    .build()?;

client.subscribe_with_command_sync(cmd).await?;
```

See [docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md) for complete API documentation.

## Examples (in `examples/`)

This repository includes several runnable examples demonstrating client usage patterns and transports. Build and run them with `cargo run --example <name>` from the repository root (or `cargo run --bin <name>` if they are provided as binaries).

- `mqtt_client_v5` — Synchronous-style example using the legacy `MqttClient` API. Demonstrates connect/subscribe/publish/ping/unsubscribe and receive loops. Run:

```bash
cargo run --example mqtt_client_v5
```

- `mqtt_client_builder_example` — Shows the `MqttClientOptions` builder pattern across multiple configurations (minimal, auth, session expiry, full). Also demonstrates creating a `TokioAsyncMqttClient` and attempting a real connection if a broker is available. Run:

```bash
cargo run --example mqtt_client_builder_example
```

- `tokio_async_mqtt_client_example` — Full Tokio async client example with an event handler. Demonstrates subscriptions, multiple publish styles (including MQTT v5 properties), ping, unsubscribe, reconnection behavior, and graceful shutdown. Run:

```bash
cargo run --example tokio_async_mqtt_client_example
```

- `tokio_async_mqtt_quic_example` / `quic_client_example` — Examples showing QUIC transport usage (feature-gated). Enable the `quic` feature and run; requires QUIC dependencies and a compatible broker or proxy. Example run:

```bash
cargo run --example tokio_async_mqtt_quic_example --features quic
```

- `tokio_async_mqtt_auth_example` — Demonstrates authentication options and how to pass username/password or other auth-related properties to the client. Run:

```bash
cargo run --example tokio_async_mqtt_auth_example
```

- `tls_client` — TLS transport example that shows how to configure the client for secure connections (feature-gated). Provide valid certificates or use a broker with TLS. Run:

```bash
cargo run --example tls_client
```

- `tokio_async_mqtt_all_sync_operations` — Demonstrates the "all sync operations" convenience API for the Tokio client (wait-for-ACK style operations). Run:

```bash
cargo run --example tokio_async_mqtt_all_sync_operations
```

Notes:
- Examples assume a local broker at `localhost:1883` unless otherwise configured. Adjust the code or pass environment variables as needed.
- Some examples require feature flags (`quic`, `tls`) and extra dependencies. See the Feature Flags section above for details.


## Documentation

### Client API & Usage
- **[TOKIO_ASYNC_CLIENT_API_GUIDE.md](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md)** - Complete API reference with examples
- **[ASYNC_CLIENT.md](docs/ASYNC_CLIENT.md)** - Async client architecture and design
- **[BUILDER_PATTERN.md](docs/BUILDER_PATTERN.md)** - Builder pattern implementation details

### Documentation index (in `docs/`)

- **[TOKIO_ASYNC_CLIENT_API_GUIDE.md](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md)** — Complete API guide for `TokioAsyncMqttClient`: initialization, configuration profiles, async vs sync APIs, and full examples.
- **[ASYNC_CLIENT.md](docs/ASYNC_CLIENT.md)** — Background-thread async client: event-driven callbacks, usage examples, and configuration for non-Tokio environments.
- **[BUILDER_PATTERN.md](docs/BUILDER_PATTERN.md)** — Builder pattern for `MqttClientOptions` with examples (auto-subscribe, session expiry, auth).
- **[PROTOCOL_COMPLIANCE.md](docs/PROTOCOL_COMPLIANCE.md)** — Protocol compliance and validation rules, error messages, and feature flags for strict validation.
- **[MQTT_SESSION.md](docs/MQTT_SESSION.md)** — MQTT v5 session model: client/server session states, inflight buffers, and session expiry semantics.
- **[DEV.md](docs/DEV.md)** — Developer guide: build, test, and development workflow for contributors.
- **[TEST.md](docs/TEST.md)** — Testing infrastructure and guidance (fuzzing, integration tests, raw packet testing).
- **[CONTRIBUTING.md](docs/CONTRIBUTING.md)** — Contribution guidelines and project expectations.
- **[TODO.md](docs/TODO.md)** — Current roadmap and outstanding tasks for the project.

For detailed information, open the corresponding file in the `docs/` directory. These documents provide design notes, examples, and developer guidance for using and contributing to FlowSDK.


## License

See [LICENSE](LICENSE)
