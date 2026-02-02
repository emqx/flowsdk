# FlowSDK Python Bindings

Python bindings for [FlowSDK](https://github.com/emqx/flowsdk), providing both low-level FFI access and high-level async MQTT client.

## Installation

```bash
pip install flowsdk
```

## Quick Start

### Async Client (Recommended)

The high-level async client provides a simple asyncio-based API:

```python
import asyncio
from flowsdk import FlowMqttClient

async def main():
    # Create client with message callback
    client = FlowMqttClient(
        "my_client_id",
        on_message=lambda topic, payload, qos: print(f"{topic}: {payload}")
    )
    
    # Connect and use
    await client.connect("broker.emqx.io", 1883)
    await client.subscribe("test/topic", qos=1)
    await client.publish("test/topic", b"Hello World!", qos=1)
    
    await asyncio.sleep(2)  # Wait for messages
    await client.disconnect()

asyncio.run(main())
```

### Low-Level FFI

For advanced use cases requiring manual control:

```python
import flowsdk

# Create engine
engine = flowsdk.MqttEngineFfi("client_id", mqtt_version=5)
engine.connect()

# Manual network I/O required (see examples/)
```

## Features

- ✅ MQTT 3.1.1 and 5.0 support
- ✅ Async/await API with asyncio
- ✅ TLS/SSL support
- ✅ QUIC transport support
- ✅ QoS 0, 1, 2 support
- ✅ Clean session and persistent sessions
- ✅ Last Will and Testament
- ✅ Automatic reconnection

## Examples

See the [examples directory](../../examples/) for more usage patterns:

- `simple_async_usage.py` - Minimal async client example
- `proper_async_client.py` - Full-featured async client
- `select_example.py` - Select-based non-blocking I/O
- `async_example.py` - Manual asyncio with low-level API
- `test_binding.py` - FFI binding tests

## API Reference

### FlowMqttClient

High-level async MQTT client.

**Constructor:**
```python
FlowMqttClient(
    client_id: str,
    mqtt_version: int = 5,          # 3 or 5
    clean_start: bool = True,
    keep_alive: int = 30,           # seconds
    username: Optional[str] = None,
    password: Optional[str] = None,
    on_message: Optional[Callable[[str, bytes, int], None]] = None
)
```

**Methods:**
- `async connect(host: str, port: int)` - Connect to broker
- `async subscribe(topic: str, qos: int = 0) -> int` - Subscribe to topic
- `async unsubscribe(topic: str) -> int` - Unsubscribe from topic
- `async publish(topic: str, payload: bytes, qos: int = 0, retain: bool = None) -> int` - Publish message
- `async disconnect()` - Disconnect gracefully

## License

Mozilla Public License 2.0
