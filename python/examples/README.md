# FlowSDK Python Examples

This directory contains examples demonstrating various usage patterns of the FlowSDK Python bindings.

## Quick Start Examples

### 1. Simple Async Usage (`simple_async_usage.py`)
**Minimal async MQTT client example - Start here!**

```bash
PYTHONPATH=../package python3 simple_async_usage.py
```

The simplest way to use FlowSDK with asyncio. Shows connect, subscribe, publish, and disconnect in just a few lines.

### 2. Asyncio TCP Client (`asyncio_tcp_client_example.py`)
**Standard async client using TCP**

```bash
PYTHONPATH=../package python3 asyncio_tcp_client_example.py
```

Demonstrates the high-level `FlowMqttClient` API using standard TCP transport with message callbacks and proper error handling.

## Secure Transport Examples

### 3. TLS Transport (`async_tls_client_example.py`)
**MQTT over TLS for secure communication**

```bash
PYTHONPATH=../package python3 async_tls_client_example.py
```

Shows how to use MQTT over TLS (port 8883) with the unified `FlowMqttClient`. Includes:
- Explicit TLS transport selection
- Certificate verification (default)
- SNI support via `server_name`

### 4. QUIC Transport (`asyncio_quic_client_example.py`)
**MQTT over QUIC for modern UDP-based transport**

```bash
PYTHONPATH=../package python3 asyncio_quic_client_example.py
```

Shows how to use MQTT over QUIC protocol with the unified `FlowMqttClient`. QUIC provides:
- Built-in encryption (TLS 1.3)
- Connection migration
- Lower latency than TCP
- No head-of-line blocking

**Requirements**: QUIC-enabled MQTT broker (e.g., EMQX 5.0+ with QUIC listener on port 14567)

## Low-Level Examples

### 5. Select-Based Client (`select_example.py`)
**Non-blocking I/O using select()**

```bash
PYTHONPATH=../package python3 select_example.py
```

Demonstrates manual socket management with the select() system call.

### 6. Async with Manual I/O (`async_example.py`)
**Asyncio with manual socket operations**

```bash
PYTHONPATH=../package python3 async_example.py
```

Shows how to manually integrate the MQTT engine with asyncio event loop using socket I/O.

## Testing

### 8. Binding Tests (`test_binding.py`)
**Low-level FFI binding verification**

```bash
PYTHONPATH=../package python3 test_binding.py
```

## Architecture Overview

### High-Level API (Recommended)
```
FlowMqttClient → FlowMqttProtocol → MqttEngineFfi → Rust FFI
```
- Simple async/await API
- Automatic network handling
- Built-in reconnection support

### Low-Level API (Advanced)
```
Your Code → MqttEngineFfi → Manual I/O → Network
```

## Transport Selection

FlowSDK supports multiple transport protocols via the unified `FlowMqttClient`:

### TCP
```python
client = FlowMqttClient("my_client", transport=TransportType.TCP)
await client.connect("broker.emqx.io", 1883)
```

### TLS
```python
client = FlowMqttClient(
    "my_client", 
    transport=TransportType.TLS,
    server_name="broker.emqx.io"
)
await client.connect("broker.emqx.io", 8883)
```

### QUIC
```python
client = FlowMqttClient(
    "my_client",
    transport=TransportType.QUIC,
    server_name="broker.emqx.io"
)
await client.connect("broker.emqx.io", 14567, server_name="broker.emqx.io")
```

## Learning Path

1. **Start here**: `simple_async_usage.py` - Learn the basics
2. **Next**: `asyncio_tcp_client_example.py` - See full features
3. **Secure**: `async_tls_client_example.py` - Try TLS transport
4. **Modern**: `asyncio_quic_client_example.py` - Try QUIC transport

## License

Mozilla Public License 2.0
