# FlowSDK Python Examples

This directory contains examples demonstrating various usage patterns of the FlowSDK Python bindings.

## Quick Start Examples

### 1. Simple Async Usage (`simple_async_usage.py`)
**Minimal async MQTT client example - Start here!**

```bash
PYTHONPATH=../package python3 simple_async_usage.py
```

The simplest way to use FlowSDK with asyncio. Shows connect, subscribe, publish, and disconnect in just a few lines.

### 2. Proper Async Client (`proper_async_client.py`)
**Full-featured async client with message callbacks**

```bash
PYTHONPATH=../package python3 proper_async_client.py
```

Demonstrates the complete FlowMqttClient API with proper error handling and message callbacks.

## Transport Examples

### 3. QUIC Transport (`async_quic_client_example.py`)
**MQTT over QUIC for modern UDP-based transport**

```bash
PYTHONPATH=../package python3 async_quic_client_example.py
```

Shows how to use MQTT over QUIC protocol, which provides:
- Built-in encryption (TLS 1.3)
- Connection migration
- Lower latency than TCP
- No head-of-line blocking

**Requirements**: QUIC-enabled MQTT broker (e.g., EMQX 5.0+ with QUIC listener)

## Low-Level Examples

### 4. Select-Based Client (`select_example.py`)
**Non-blocking I/O using select()**

```bash
PYTHONPATH=../package python3 select_example.py
```

Demonstrates manual socket management with the select() system call. Good for:
- Understanding low-level I/O
- Integration with non-asyncio event loops
- Single-threaded blocking I/O patterns

### 5. Async with Manual I/O (`async_example.py`)
**Asyncio with manual socket operations**

```bash
PYTHONPATH=../package python3 async_example.py
```

Shows how to manually integrate the MQTT engine with asyncio event loop using socket I/O.

### 6. Async Interop (`async_interop_example.py`)
**Asyncio Protocol integration pattern**

```bash
PYTHONPATH=../package python3 async_interop_example.py
```

Demonstrates using asyncio's Protocol pattern with add_reader/add_writer for event loop integration.

## Testing

### 7. Binding Tests (`test_binding.py`)
**Low-level FFI binding verification**

```bash
PYTHONPATH=../package python3 test_binding.py
```

Tests the FFI bindings for:
- Standard MQTT engine (MqttEngineFfi)
- TLS MQTT engine (TlsMqttEngineFfi)
- QUIC MQTT engine (QuicMqttEngineFfi)

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
- Full control over networking
- Custom transport implementation
- Integration with existing event loops

## MQTT Broker Setup

### For TCP Examples (most examples)
Any MQTT broker works:
```bash
# Using mosquitto
mosquitto -p 1883

# Or use public broker
# broker.emqx.io:1883 (already configured in examples)
```

### For QUIC Example
Requires QUIC-enabled broker (EMQX 5.0+):
```bash
# EMQX with QUIC enabled
docker run -d --name emqx \
  -p 1883:1883 \
  -p 14567:14567/udp \
  emqx/emqx:latest

# Configure QUIC listener in emqx.conf:
# listeners.quic.default {
#   bind = "0.0.0.0:14567"
#   max_connections = 1024000
# }
```

## Common Issues

### Import Error
```
ModuleNotFoundError: No module named 'flowsdk'
```
**Solution**: Set PYTHONPATH to the package directory:
```bash
export PYTHONPATH=/path/to/python/package
```

### Connection Refused
```
ConnectionRefusedError: [Errno 61] Connection refused
```
**Solution**: 
- Ensure MQTT broker is running
- Check broker address and port
- For public brokers, check your internet connection

### QUIC Connection Failed
```
QUIC: Connection lost: None
```
**Solution**:
- Verify QUIC listener is enabled on broker
- Check UDP port 14567 is open
- Try with `insecure_skip_verify=True` for testing

## Learning Path

1. **Start here**: `simple_async_usage.py` - Learn the basics
2. **Next**: `proper_async_client.py` - See full features
3. **Advanced**: `select_example.py` - Understand low-level I/O
4. **Modern**: `async_quic_client_example.py` - Try QUIC transport

## More Information

- [Package README](../package/README.md) - API reference
- [FlowSDK Docs](../../../docs/) - Protocol details
- [MQTT Specification](https://mqtt.org/mqtt-specification/) - MQTT protocol

## License

Mozilla Public License 2.0
