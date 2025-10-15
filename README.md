# FlowSDK

FlowSDK is a safty-first, realistic, behavior predictable messaging SDK.

With FlowSDK, You could build messaging based [micro middleware functions](#Micro-middleware-functions) that runs in your app.

FlowSDK leverage multiple protocols in different layers and make best use of them for real world scenarios.


## What is Flow

Flow is the projection of data streaming from many sources with the help of [micro middleware functions](#Micro-middleware-functions)

## Micro middleware functions

- Messaging client
- Pub/Sub broker
- Filter
- Proxy
- Protocol relay
- Queue
- K/V store

## Be realistic

Message is not created equal.  Messaging comes with costs, resources are not unlimited.

## Communication models

- **Req/Resp**
- **PUB/SUB**
- **Stream**
- **Reliable Delivery**
- **Unreliable Delivery**

## Protocols

- **MQTT**
- **gRPC**
- ...

## Current Status

** Working in progress ** 

## Project Structure

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

### Protocol Testing (Feature-Gated)

```rust
#[cfg(feature = "protocol-testing")]
use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;

// Test server rejection of malformed packets
let mut client = RawTestClient::connect("localhost:1883").await?;
let malformed = MalformedPacketGenerator::connect_reserved_flag()?;
client.send_expect_disconnect(malformed, 5000).await?;
```

See [docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md) for complete API documentation.

## Documentation

### Client API & Usage
- **[TOKIO_ASYNC_CLIENT_API_GUIDE.md](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md)** - Complete API reference with examples
- **[ASYNC_CLIENT.md](docs/ASYNC_CLIENT.md)** - Async client architecture and design
- **[BUILDER_PATTERN.md](docs/BUILDER_PATTERN.md)** - Builder pattern implementation details

## Protocol Compliance

This implementation follows the MQTT v5.0 specification with:
- ✅ Strict packet format validation
- ✅ Proper QoS flow handling  
- ✅ Session state management
- ✅ Properties support
- ✅ Error code compliance
- 📋 Protocol test coverage: 84% **achievable** (0/185 implemented)
  - Infrastructure complete with raw packet API
  - 106 tests possible with standard API
  - 79 tests possible with raw packet API
  - 30 tests require server-side testing (not applicable to client library)

## Performance & Quality

### Test Metrics (October 2025)
- ✅ **400+ unit testing**
- ✅ **Fuzz testing infrastructure** 
- ✅ **Zero compiler warnings**
- ✅ **Zero unsafe code** in core client
- ✅ **Feature-gated dangerous APIs** (protocol-testing)

### Code Quality
- Comprehensive error handling with custom error types
- Property-based testing support with `arbitrary`
- Strict protocol compliance mode (feature-gated)
- Memory-efficient streaming with zero-copy where possible

## Contributing

### Areas for Contribution

1. **Core MQTT Protocol**: Changes go in `src/mqtt_serde/`
2. **MQTT Client**: Enhancements to `src/mqtt_client/`
3. **gRPC Conversions**: Shared logic in `src/grpc_conversions.rs` and `mqtt_grpc_duality/src/lib.rs`
4. **Proxy Applications**: New features in `mqtt_grpc_duality/src/bin/`
5. **Examples**: Simple demos in `src/bin/`
6. **Protocol Tests**: Add compliance tests in `tests/protocol_compliance_tests.rs`
7. **Documentation**: Update docs in `docs/` directory

### Development Workflow

1. Fork the repository
2. Create a feature branch
3. Make changes with tests
4. Ensure all tests pass: `cargo test --workspace`
5. Run formatter: `cargo fmt --all`
6. Run clippy: `cargo clippy --workspace -- -D warnings`
7. Submit pull request

### Testing Guidelines

- Add unit tests for new functionality
- Add integration tests for client operations
- Use `#[ignore]` for tests requiring live broker
- Document protocol violations in raw packet tests
- Keep test coverage above 80%

## Roadmap

### Completed ✅
- [x] MQTT v3.1.1, v5.0 packet serialization/deserialization
- [x] TokioAsyncMqttClient (v5.0) with dual API (async/sync)
- [x] Builder pattern
- [x] MQTT v5 flow control (Receive Maximum, Topic Alias Maximum)
- [x] Raw Packet API for protocol testing
- [x] Comprehensive documentation
- [x] Protocol testing infrastructure (84% coverage achievable)
- [x] TLS/SSL support
- [x] QUIC support (single stream)

### In Progress 🚧
- [ ] Authentication method support
- [ ] More TLS configs
- [ ] Protocol compliance test implementation (0/185 tests)
  - [ ] Phase 1: Foundation tests with current API (30 tests)
  - [ ] Phase 2: MQTT v5 feature tests (40 tests)  
  - [ ] Phase 3-4: Raw packet malformed tests (40 tests)
- [ ] Enhanced event handler properties (subscription IDs, MQTT v5 properties)

### Planned 📋
- [ ] Client support MQTT v3
- [ ] QUIC support (multi stream)
- [ ] WebSocket transport support
- [ ] Packet inspection utilities
- [ ] Multi-broker failover
- [ ] Message persistence
- [ ] Metrics and observability

## License

see [LICENSE](LICENSE)
