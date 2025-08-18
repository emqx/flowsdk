# MQTT-gRPC Proxy System

This directory contains the MQTT-gRPC proxy system, consisting of two complementary proxy applications that enable seamless integration between MQTT clients/brokers and gRPC services using bidirectional streaming.

## Architecture Overview

The proxy system consists of two main components that work together to bridge MQTT and gRPC protocols:

```
[MQTT Client] <--TCP--> [r-proxy] <--gRPC Stream--> [s-proxy] <--TCP--> [MQTT Broker]
```

- **r-proxy** (Receive Proxy): Acts as an MQTT broker interface for MQTT clients
- **s-proxy** (Send Proxy): Acts as an MQTT client interface for MQTT brokers

## Components

### r-proxy (Receive Proxy)

**Purpose**: Receives MQTT connections from clients and forwards them via gRPC streaming to s-proxy.

**Key Features**:
- Listens on TCP port for incoming MQTT client connections
- Parses MQTT packets using the FlowSDK MQTT parser
- Converts MQTT packets to protobuf messages
- Establishes bidirectional gRPC streaming connections with s-proxy
- Manages session state and connection lifecycle
- Handles all MQTT packet types (CONNECT, PUBLISH, SUBSCRIBE, etc.)

**Architecture**:
```
┌─────────────┐    TCP/MQTT    ┌─────────────┐    gRPC Stream  ┌─────────────┐
│ MQTT Client │ ─────────────► │   r-proxy   │ ──────────────► │   s-proxy   │
│             │                │             │                 │             │
│             │ ◄───────────── │             │ ◄────────────── │             │
└─────────────┘                └─────────────┘                 └─────────────┘
```

**Main Functions**:
- `run_proxy()`: Main entry point that listens for MQTT client connections
- `handle_streaming_client()`: Manages individual MQTT client sessions
- `streaming_client_loop()`: Bidirectional message processing loop
- `wait_for_mqtt_connect()`: Initial MQTT CONNECT packet handling

### s-proxy (Send Proxy)

**Purpose**: Receives gRPC streaming requests from r-proxy and forwards them to MQTT brokers.

**Key Features**:
- Implements `MqttRelayService` gRPC server
- Establishes TCP connections to MQTT brokers
- Converts protobuf messages back to MQTT packets
- Manages MQTT client sessions on behalf of r-proxy
- Handles bidirectional streaming for real-time message relay
- Supports all MQTT 5.0 features including properties and QoS levels

**Architecture**:
```
┌─────────────┐    gRPC Stream ┌─────────────┐    TCP/MQTT     ┌─────────────┐
│   r-proxy   │ ─────────────► │   s-proxy   │ ──────────────► │ MQTT Broker │
│             │                │             │                 │             │
│             │ ◄───────────── │             │ ◄────────────── │             │
└─────────────┘                └─────────────┘                 └─────────────┘
```

**Main Functions**:
- `mqtt_connect()`: Handles MQTT connection establishment
- `mqtt_publish_qos1()`: Handles QoS 1 publish operations
- `stream_mqtt_messages()`: Bidirectional streaming implementation
- `mqtt_client_loop()`: Message relay loop between gRPC and MQTT broker

## Protocol Flow

### 1. Connection Establishment

1. **MQTT Client connects to r-proxy**:
   - Client opens TCP connection to r-proxy (default port: 1883)
   - Client sends MQTT CONNECT packet

2. **r-proxy processes CONNECT**:
   - Parses MQTT CONNECT packet
   - Generates unique proxy session ID
   - Establishes gRPC streaming connection to s-proxy
   - Sends session establishment message

3. **s-proxy forwards to MQTT Broker**:
   - Receives gRPC stream from r-proxy
   - Establishes TCP connection to MQTT broker
   - Forwards CONNECT packet to broker
   - Receives CONNACK from broker

4. **CONNACK flows back**:
   - s-proxy sends CONNACK via gRPC stream to r-proxy
   - r-proxy forwards CONNACK to MQTT client
   - Connection is fully established

### 2. Message Flow

**Client-to-Broker (Publish, Subscribe, etc.)**:
```
Client → r-proxy → gRPC Stream → s-proxy → Broker
```

**Broker-to-Client (Publish delivery, SUBACK, etc.)**:
```
Broker → s-proxy → gRPC Stream → r-proxy → Client
```

### 3. Session Management

- Each MQTT client gets a unique session ID
- Sessions are maintained throughout the connection lifecycle
- Connection state is shared between r-proxy and s-proxy via gRPC metadata
- Graceful cleanup on disconnection

## Message Types Supported

The system supports all MQTT 5.0 control packets:

- **CONNECT/CONNACK**: Connection establishment
- **PUBLISH/PUBACK/PUBREC/PUBREL/PUBCOMP**: Message publishing with QoS 0, 1, 2
- **SUBSCRIBE/SUBACK**: Topic subscriptions
- **UNSUBSCRIBE/UNSUBACK**: Topic unsubscriptions
- **PINGREQ/PINGRESP**: Keep-alive mechanism
- **DISCONNECT**: Clean disconnection
- **AUTH**: Authentication exchange (MQTT 5.0)

## Configuration

### r-proxy Configuration

Default configuration:
- Listen port: 1883 (MQTT standard)
- s-proxy endpoint: `http://[::1]:50516`
- Buffer size: 4096 bytes
- gRPC channel capacity: 1000 messages

### s-proxy Configuration

Default configuration:
- gRPC server port: 50516
- MQTT broker endpoint: Configurable via command line
- Connection pool management
- Stream capacity: 32 messages per connection

## Running the Proxies

### Starting s-proxy

```bash
# Start s-proxy (gRPC server)
cargo run --bin s-proxy
```

s-proxy will:
- Start gRPC server on `[::]:50516`
- Wait for streaming connections from r-proxy
- Forward MQTT traffic to the configured broker

### Starting r-proxy

```bash
# Start r-proxy with broker destination
cargo run --bin r-proxy <broker_host:port>

# Example:
cargo run --bin r-proxy 127.0.0.1:1883
```

r-proxy will:
- Listen on `0.0.0.0:1883` for MQTT clients
- Connect to s-proxy via gRPC
- Forward client traffic through the proxy chain

## Use Cases

### 1. Cloud-Native MQTT Integration

Deploy s-proxy in cloud environments where direct MQTT broker access is restricted:

```
[Edge Clients] → [r-proxy] → [Internet/gRPC] → [s-proxy] → [Cloud MQTT Broker]
```

### 2. Protocol Translation

Use the proxy system to add gRPC capabilities to existing MQTT infrastructures:

```
[gRPC Services] → [r-proxy API] → [gRPC Stream] → [s-proxy] → [Legacy MQTT Broker]
```

### 3. Load Balancing and Scaling

Deploy multiple s-proxy instances behind load balancers:

```
[Clients] → [r-proxy] → [Load Balancer] → [s-proxy-1, s-proxy-2, s-proxy-N] → [MQTT Brokers]
```

## Implementation Details

### Shared Conversion Library

Both proxies use the shared `mqtt_grpc_proxy` library which provides:

- **Bidirectional Conversions**: MQTT ↔ Protobuf type conversions
- **Property Handling**: Full MQTT 5.0 properties support
- **QoS Management**: Proper QoS flow handling
- **Session State**: Shared session management utilities

### Error Handling

- Connection failures are gracefully handled with proper cleanup
- gRPC stream errors trigger connection termination
- MQTT protocol violations are reported via gRPC status codes
- Automatic retry mechanisms for transient failures

### Performance Considerations

- **Streaming**: Uses bidirectional gRPC streaming for low-latency message relay
- **Connection Pooling**: s-proxy maintains connection pools to MQTT brokers
- **Memory Management**: Efficient buffer management for high-throughput scenarios
- **Async Processing**: Full async/await implementation for scalability

## Monitoring and Debugging

### Logging

Both proxies provide structured logging:

```rust
tracing::info!("Client connected: {}", client_id);
tracing::error!("Failed to relay message: {}", error);
```

### Metrics

Key metrics to monitor:
- Active connections count
- Message throughput (messages/second)
- gRPC stream health
- Connection establishment latency
- Error rates by type

### Debugging

Enable debug logging:

```bash
RUST_LOG=debug cargo run --bin r-proxy
RUST_LOG=debug cargo run --bin s-proxy
```

## Security Considerations

- **TLS Support**: gRPC connections support TLS encryption
- **Authentication**: MQTT authentication is preserved through the proxy chain
- **Authorization**: Client authorization is handled by the target MQTT broker
- **Network Isolation**: Proxies can operate across network boundaries securely

## Future Enhancements

- **HA Support**: High availability with multiple proxy instances
- **Metrics Export**: Prometheus metrics integration
- **Dynamic Routing**: Route messages based on topics or client properties
- **Protocol Extensions**: Support for custom MQTT extensions
- **Admin API**: REST API for proxy management and monitoring

## Contributing

When contributing to the proxy system:

1. Ensure both r-proxy and s-proxy remain compatible
2. Update protobuf definitions when adding new message types
3. Maintain backward compatibility for existing deployments
4. Add comprehensive tests for new functionality
5. Update this README with any architectural changes

## Dependencies

- **FlowSDK**: Core MQTT parsing and serialization
- **Tonic**: gRPC client/server implementation
- **Tokio**: Async runtime and networking
- **DashMap**: Concurrent hash map for connection management
- **Tracing**: Structured logging and instrumentation

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

- **flowsdk**: Parent crate providing MQTT v5.0 serialization/parsing
- **tonic**: gRPC framework
- **tokio**: Async runtime
- **dashmap**: Concurrent hashmap for session management
