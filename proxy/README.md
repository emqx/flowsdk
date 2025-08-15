# MQTT-gRPC Proxy Workspace

This workspace contains the proxy applications for the MQTT-gRPC duality project.

## Components

### r-proxy (Client-facing proxy)
- **Binary**: `r-proxy`
- **Purpose**: Acts as a gRPC server that clients connect to, forwarding MQTT messages to the broker
- **Port**: `[::1]:50516` (gRPC server)
- **Architecture**: Receives gRPC calls from clients and translates them to MQTT packets sent to broker

### s-proxy (Server-side proxy)  
- **Binary**: `s-proxy`
- **Purpose**: Acts as a gRPC server that handles broker-side operations, connects to MQTT broker
- **Port**: `[::1]:50515` (gRPC server)
- **Architecture**: Establishes MQTT connections to broker and forwards broker responses via gRPC to r-proxy

## Shared Components

- **lib.rs**: Contains shared conversion implementations between MQTT v5.0 and Protocol Buffers
- **proto/mqttv5.proto**: Protocol Buffer definitions for MQTT v5.0 messages and gRPC services
- **Conversions**: Bidirectional `From`/`TryFrom` trait implementations for all MQTT packet types

## Building

```bash
cargo build
```

This will build both `r-proxy` and `s-proxy` binaries.

## Running

Start both proxies:

```bash
# Terminal 1 - Start s-proxy (connects to MQTT broker)
./target/debug/s-proxy

# Terminal 2 - Start r-proxy (serves gRPC clients)  
./target/debug/r-proxy
```

## Dependencies

- **mqtt-grpc-duality**: Parent crate providing MQTT v5.0 serialization/parsing
- **tonic**: gRPC framework
- **tokio**: Async runtime
- **dashmap**: Concurrent hashmap for session management

## Architecture

```
gRPC Client -> r-proxy -> s-proxy -> MQTT Broker
                   ^         |
                   |         v
               gRPC Calls  MQTT TCP
```

The proxy architecture eliminates code duplication by:
1. **Shared Conversions**: Common conversion logic in `lib.rs` used by both proxies
2. **Dedicated Workspace**: Self-contained proxy applications with their own protobuf definitions
3. **Clean Separation**: r-proxy handles client connections, s-proxy handles broker connections
4. **Streaming Support**: Bidirectional gRPC streaming for real-time message forwarding
