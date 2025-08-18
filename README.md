# flowSDK

The developer toolkit for modern 

## Current Status

Developing in private REPO.

## Project Structure

This project is organized as a Cargo workspace with two main components:

### 📚 Core Library (`flowsdk`)
- **MQTT v5.0 Protocol**: Complete serialization/deserialization implementation
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

```
gRPC Clients → r-proxy → gRPC → s-proxy → MQTT Broker
                ↑                  ↑
           Client-facing     Server-side
           (Port 50516)      (Port 50515)
```

### Component Details

#### Core Library Components
- **`mqtt_serde`**: "Encoder and Decoder, serialization and deserialization of MQTT packets"


#### Proxy Components (in `mqtt_grpc_duality/` workspace)
see [mqtt_grpc_duality README.md](mqtt_grpc_duality/README.md)

## Features

### MQTT v5.0 Support
- ✅ All control packet types (Connect, Publish, Subscribe, etc.)
- ✅ QoS 0, 1, and 2 message flows
- ✅ Properties support for enhanced metadata
- ✅ Authentication and session management
- ✅ Protocol compliance validation


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
cargo build                            # Main library only
cd mqtt_grpc_duality && cargo build    # Proxy workspace only

# Run tests
cargo test --workspace                 # All tests
cargo test                             # Main library tests
cd mqtt_grpc_duality && cargo test     # Proxy tests only

# Clean build artifacts
cargo clean --workspace                # Everything
cargo clean                            # Main library
cd mqtt_grpc_duality && cargo clean    # Proxy workspace
```

### Testing
```bash
# Run all tests in both workspaces
cargo test --workspace

# Or individually
cargo test                             # Main library tests
cd mqtt_grpc_duality && cargo test     # Proxy workspace tests
```

## Usage Examples


## Protocol Compliance

This implementation follows the MQTT v5.0 specification with:
- ✅ Strict packet format validation
- ✅ Proper QoS flow handling  
- ✅ Session state management
- ✅ Properties support
- ✅ Error code compliance

## Contributing

1. **Core MQTT Protocol**: Changes go in `src/mqtt_serde/`
2. **gRPC Conversions**: Shared logic in `src/grpc_conversions.rs` and `mqtt_grpc_duality/src/lib.rs`
3. **Proxy Applications**: New features in `mqtt_grpc_duality/src/bin/`
4. **Examples**: Simple demos in `src/bin/`

## License

[Add your license information here]
