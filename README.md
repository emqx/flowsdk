# MQTT-gRPC Duality

A high-performance MQTT v5.0 protocol implementation with bidirectional gRPC streaming support for modern distributed systems.

## Project Structure

This project is organized as a Cargo workspace with two main components:

### 📚 Core Library (`mqtt-grpc-duality`)
- **MQTT v5.0 Protocol**: Complete serialization/deserialization implementation
- **Shared Conversions**: gRPC ↔ MQTT conversion utilities
- **Example Applications**: Simple client/server demos

### 🔗 Proxy Workspace (`proxy/`)  
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
cd proxy && cargo build
```

### Run Proxy Applications
```bash
# Start server-side proxy (connects to MQTT broker)
cd proxy && cargo run --bin s-proxy

# In another terminal, start client-facing proxy
cd proxy && cargo run --bin r-proxy
```

## Architecture

```
gRPC Clients → r-proxy → gRPC → s-proxy → MQTT Broker
                ↑                  ↑
           Client-facing     Server-side
           (Port 50516)      (Port 50515)
```

### Component Details

#### Core Library Components
- **`simple-server`**: Basic gRPC server implementation
- **`simple-client`**: Basic gRPC client example  
- **`s-client`**: Streaming client example

#### Proxy Components (in `proxy/` workspace)
- **`r-proxy`**: Client-facing proxy - receives gRPC calls and forwards to s-proxy
- **`s-proxy`**: Server-side proxy - connects to MQTT broker and handles bidirectional communication

## Features

### MQTT v5.0 Support
- ✅ All control packet types (Connect, Publish, Subscribe, etc.)
- ✅ QoS 0, 1, and 2 message flows
- ✅ Properties support for enhanced metadata
- ✅ Authentication and session management
- ✅ Protocol compliance validation

### gRPC Integration
- ✅ Bidirectional streaming for real-time communication
- ✅ Efficient protobuf serialization
- ✅ Connection pooling and session management
- ✅ Error handling and status reporting

### Performance & Reliability
- ✅ Zero-copy deserialization where possible
- ✅ Concurrent connection handling with `tokio`
- ✅ Memory-efficient streaming
- ✅ Comprehensive error handling

## Development

### Dependencies Management
The project uses minimal dependencies:

**Core Library**: 
- Essential: `serde`, `hex`, `bytes`, `tokio`
- gRPC: `tonic`, `prost` (for conversions module)
- Testing: `serde_json`, `arbitrary`

**Proxy Workspace**:
- Additional: `dashmap`, `tracing`, `tokio-stream`
- Self-contained with own protobuf compilation

### Building and Testing
```bash
# Build everything
cargo build --workspace

# Build individual workspaces
cargo build                    # Main library only
cd proxy && cargo build        # Proxy workspace only

# Run tests
cargo test --workspace         # All tests
cargo test                     # Main library tests
cd proxy && cargo test         # Proxy tests only

# Clean build artifacts
cargo clean --workspace        # Everything
cargo clean                    # Main library
cd proxy && cargo clean        # Proxy workspace
```

### Testing
```bash
# Run all tests in both workspaces
cargo test --workspace

# Or individually
cargo test                     # Main library tests
cd proxy && cargo test         # Proxy workspace tests
```

## Usage Examples

### Basic gRPC Client
```rust
use mqttv5pb::mqtt_relay_service_client::MqttRelayServiceClient;
use mqttv5pb::Connect;

let mut client = MqttRelayServiceClient::connect("http://[::1]:50516").await?;

let connect = Connect {
    client_id: "test-client".to_string(),
    username: "user".to_string(), 
    // ... other fields
};

let response = client.mqtt_connect(connect).await?;
```

### Running the Full Stack
```bash
# 1. Start MQTT broker (e.g., mosquitto on port 1883)
mosquitto -p 1883

# 2. Start the proxy stack
# Terminal 1:
cd proxy && cargo run --bin s-proxy

# Terminal 2: 
cd proxy && cargo run --bin r-proxy

# 3. Run a client application
cargo run --bin simple-client
```

## Protocol Compliance

This implementation follows the MQTT v5.0 specification with:
- ✅ Strict packet format validation
- ✅ Proper QoS flow handling  
- ✅ Session state management
- ✅ Properties support
- ✅ Error code compliance

## Contributing

1. **Core MQTT Protocol**: Changes go in `src/mqtt_serde/`
2. **gRPC Conversions**: Shared logic in `src/grpc_conversions.rs` and `proxy/src/lib.rs`
3. **Proxy Applications**: New features in `proxy/src/bin/`
4. **Examples**: Simple demos in `src/bin/`

## License

[Add your license information here]
