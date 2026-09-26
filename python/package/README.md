# FlowSDK Python Bindings

Python 3.8 or newer is required. Generate bindings with
`./scripts/build_python_bindings.sh --test` before building a wheel; the generated
module and native library must come from the same build. Missing bindings produce
an import error. Wheels contain a platform-specific native library and are tagged
accordingly.

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
    subscription = await client.subscribe("test/topic", qos=1)
    subscription.raise_for_status()
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

See the [examples directory](../examples/) for more usage patterns:

- `simple_async_usage.py` - Minimal async client example
- `asyncio_tcp_client_example.py` - Async client lifecycle
- `select_example.py` - Select-based non-blocking I/O
- `async_example.py` - Manual asyncio with low-level API
- `test_binding.py` - FFI binding tests
- `mqtt5_options.py` - Properties, retained publishing, Will, and session options

## API Reference

### FlowMqttClient

High-level async MQTT client.

**Constructor:**
```python
FlowMqttClient(
    client_id, transport=TransportType.TCP, mqtt_version=5,
    clean_start=True, keep_alive=30, username=None, password=None,
    reconnect_base_delay_ms=1000, reconnect_max_delay_ms=30000,
    max_reconnect_attempts=0, on_message=None, ca_cert_file=None,
    client_cert_file=None, client_key_file=None, insecure_skip_verify=False,
    alpn_protocols=None, server_name=None, enable_key_log=False,
    *, on_message_full=None, on_event=None, connect_properties=None,
    will=None, engine_options=None, auto_reconnect=False, quic_zero_rtt=None,
)
```

**Methods:**

| Method | Result / options |
| --- | --- |
| `await connect(host, port, server_name=None, timeout=10, *, return_result=False)` | Optional `ConnectionResult` |
| `await publish(topic, payload, qos=0, retain=None, *, timeout=10, properties=None, priority=None, return_result=False, stream_id=None, early_data=False)` | Packet ID or `PublishResult` |
| `await subscribe(topic, qos=0, *, timeout=10, no_local=False, retain_as_published=False, retain_handling=0, properties=None)` | `SubscribeResult` |
| `await subscribe_many(subscriptions, *, properties=None, timeout=10, stream_id=None, early_data=False)` | `SubscribeResult` in input order |
| `await unsubscribe(topic, *, timeout=10, properties=None)` | `UnsubscribeResult` |
| `await unsubscribe_many(topics, *, properties=None, timeout=10, stream_id=None, early_data=False)` | `UnsubscribeResult` in input order |
| `await auth(reason_code=0x18, *, properties=None)` | Submit an enhanced-authentication response |
| `await ping(*, timeout=10)` | Await MQTT PINGRESP |
| `await disconnect(reason_code=0, *, properties=None, timeout=5)` | Flush DISCONNECT and close |
| `await acknowledge(packet_id, qos, *, stream_id=None, reason_code=0, properties=None)` | Manual PUBACK / PUBREC |
| `await complete_qos2(packet_id, *, stream_id=None, reason_code=0, properties=None)` | Manual PUBCOMP |
| `set_parse_level(level)` | Configure receive parsing after connecting |
| `pump()` | Drive outgoing I/O and dispatch queued events |

`is_connected`, `mqtt_version`, `connection_result`, `reconnect_error`, and `quic`
provide connection state and advanced QUIC controls. Callbacks run synchronously;
use `asyncio.create_task(...)` to start an async response from `on_event`.

`SubscribeResult` and `UnsubscribeResult` are exported from `flowsdk`. Both contain
`packet_id: int` and `reason_codes: List[int]`, preserving broker reason codes in
topic order, including failures. They expose `is_success` and `is_failure` boolean
properties. A result with any failed topic has `is_failure=True`.

Broker rejection returns normally from these methods. Applications can inspect
the result or explicitly call `raise_for_status()`, which raises `MqttAckError`
on failure. The exception's `result` attribute contains the complete result,
including successful entries in a mixed acknowledgement.

```python
from flowsdk import MqttAckError

subscription = await client.subscribe("test/topic", qos=1)
if subscription.is_failure:
    print(f"Subscription rejected: {subscription.reason_codes}")
else:
    print(f"Subscribed with packet ID {subscription.packet_id}")

unsubscription = await client.unsubscribe("test/topic")
try:
    unsubscription.raise_for_status()  # Opt in to exceptions for broker rejection.
except MqttAckError as exc:
    print(exc.result.packet_id, exc.result.reason_codes)
```

For subscriptions, granted QoS codes `0`, `1`, and `2` are successful. For
unsubscriptions, `0` and `0x11` ("No subscription existed") are successful;
MQTT 3.1.1's empty UNSUBACK reason-code list also indicates success.
`raise_for_status()` returns `None` for successful results.

**Migration:** subscribe and unsubscribe previously returned an integer packet
ID. Callers that use the return value must now read `result.packet_id`:

```python
result = await client.subscribe("test/topic", qos=1)
packet_id = result.packet_id
```

Calling publish, subscribe, or unsubscribe without an active MQTT connection
raises `ConnectionError`.
Local request failures raise an exception immediately, since no acknowledgement
can arrive. Broker-rejected connections raise `ConnectionError`;
rejected publishes raise `RuntimeError`. Successful publish calls return their
packet IDs, including "No matching subscribers" outcomes.

Subscribe, unsubscribe, and QoS 1/2 publish wait up to 10 seconds for their
acknowledgement. Set `timeout` per call, or use `timeout=None` to disable this
deadline. Expiry raises `asyncio.TimeoutError`; task cancellation propagates
`asyncio.CancelledError`. Both remove the Python waiter, and late acknowledgements
are ignored. The queued MQTT operation may still complete in the engine or at
the broker. QoS 0 returns after local submission and does not wait for an ACK.

Transport loss, engine errors, and disconnect finish pending calls with
`ConnectionError`. Failed, timed-out, or cancelled connection attempts close the
transport and cancel its timer. Calling `connect()` while already connected or
connecting raises `ConnectionError`.

Disconnect drives TLS/QUIC output before closing, with a five-second flushing
deadline. It closes the socket on timeout or cancellation too. Outstanding calls
fail immediately; disconnect does not wait for broker acknowledgements.

Retain and MQTT 5 publish properties work on TCP, TLS, and QUIC. Properties may be
an ordered list of `MqttPropertyFfi` variants or a `PublishProperties` record:

```python
from flowsdk import PublishProperties

await client.publish("sensors/temperature", b"23.5", qos=1, retain=True,
    properties=PublishProperties(content_type="text/plain",
        correlation_data=b"\x00\xff", user_properties=[("source", "sensor"), ("source", "gateway")]))
```

All transports accept `priority=0..255`; higher values have higher priority.
TCP/TLS schedule queued MQTT packets. QUIC uses a separate automatic publish
stream for each priority and prioritizes both local flushing and QUIC transmission.
FIFO order is preserved within each priority. The MQTT control stream has the
highest transport priority. Stream limits can reject opening another priority
stream. On an explicitly selected stream, publish priority changes that stream
and all its queued packets. `client.quic.set_stream_priority(id, priority)` changes
the priority directly. Priorities survive rejected 0-RTT replay.
Invalid properties, duplicate singleton properties, invalid QoS/topic values,
and MQTT 5 properties used with MQTT 3.1.1 raise `MqttErrorFfi` before queueing.

`connect(return_result=True)` returns `ConnectionResult` with reason code,
session-present flag, and properties; the result is also in `connection_result`.
`publish(return_result=True)` returns `PublishResult`, including broker rejections,
with `is_success`, `is_failure`, and `raise_for_status()`. Its properties and all
subscription-result properties are preserved. The default publish API still
returns a packet ID and raises on rejection. QoS 0 details describe local submission.

Use `on_message_full(message)` to receive retain, DUP, packet ID, and properties
alongside topic, payload, and QoS. `on_event(event)` receives every exposed native
event. The existing three-argument `on_message` callback remains supported.
Callback exceptions are logged without interrupting acknowledgement processing.
`client.is_connected` and `client.mqtt_version` expose current connection state
and the configured protocol version.

`subscribe()` also accepts `no_local`, `retain_as_published`, `retain_handling`,
and `properties`. Use `subscribe_many()` and `unsubscribe_many()` to send batches:

```python
from flowsdk import Subscription, MqttPropertyFfi

result = await client.subscribe_many([
    Subscription("sensors/+", qos=1, no_local=True),
    Subscription("alerts/#", qos=2, retain_handling=2),
], properties=[MqttPropertyFfi.SUBSCRIPTION_IDENTIFIER(value=7)])
await client.unsubscribe_many(["sensors/+", "alerts/#"])
```

Batch results preserve topic order and mixed reason codes. Subscription flags and
properties require MQTT 5; default batches also work with MQTT 3.1.1. Empty batches,
invalid filters, and No Local on a shared subscription are rejected.

Connection properties and Will messages are configured at client construction:

```python
from flowsdk import ConnectProperties, Will

client = FlowMqttClient("sensor", username="device", password=b"binary-password",
    clean_start=False,
    connect_properties=ConnectProperties(session_expiry_interval=3600,
        receive_maximum=32, maximum_packet_size=65536),
    will=Will("sensor/status", b"offline", qos=1, retain=True))
```

`connect_properties` also accepts ordered native properties, including enhanced
authentication fields. `Will.properties` accepts MQTT 5 Will properties. String
passwords remain supported; bytes preserve arbitrary binary credentials. MQTT
3.1.1 supports Will messages and binary passwords, but rejects MQTT 5 properties.

Enhanced authentication is supported on all three transports. Configure
`ConnectProperties(authentication_method=..., authentication_data=...)`, handle
`event.is_auth_received()` through `on_event`, and send a response with
`await client.auth(0x18, properties=[...])`. AUTH can run before CONNACK or during
reauthentication; the authentication method must remain the one used in CONNECT.
Client reason codes are `0x18` (continue) and `0x19` (reauthenticate after
CONNACK). Responses automatically include the configured authentication method;
server challenges must retain that method.
`auth()` reports local submission, while subsequent challenges/results arrive as
events. Application-specific authentication computations belong in the callback
or in a task it schedules.

`await client.ping(timeout=10.0)` sends MQTT PINGREQ on any transport and returns
`True` on PINGRESP. Only one explicit ping can wait at a time. Ping uses the same
timeout, cancellation, and connection-loss cleanup as other operations.

`disconnect(reason_code=0, properties=[...])` supports MQTT 5 reason codes and
properties. Incoming disconnect events retain the broker's properties. Outgoing
codes are checked against the client entries in the
[MQTT 5 DISCONNECT table](https://docs.oasis-open.org/mqtt/mqtt/v5.0/os/mqtt-v5.0-os.html).
MQTT 3.1.1 accepts the default disconnect only. Rust event consumers should handle
`MqttEvent::DisconnectReceived` for complete MQTT 5 broker-disconnect metadata;
transport-loss notifications remain `MqttEvent::Disconnected`.

Run the broker-free asyncio wrapper regression tests from the repository root:

```bash
python3 -m unittest discover -s python/tests -v
```

### Reconnection

Set `auto_reconnect=True` to reopen a lost connection using exponential backoff
from `reconnect_base_delay_ms`, capped by `reconnect_max_delay_ms`.
`max_reconnect_attempts=0` means unlimited retries. Automatic retry starts after
a successful connection; initial connection errors are returned to the caller.
The default is disabled. `on_event` receives retry scheduling events, and
`reconnect_error` retains the last failed attempt if retries are exhausted.

Pending operations fail on loss. The engine preserves MQTT inflight state for
broker-confirmed session recovery; Python does not resubmit requests. Use a
stable client ID, `clean_start=False`, and an appropriate MQTT 5 session expiry
for persistent sessions. `disconnect()` cancels retries and connection setup,
including DNS resolution. A later `connect()` starts a fresh transport and TLS
handshake using the same engine clock.

QUIC resolves IPv4 and IPv6 addresses and uses an unconnected UDP socket. Each
outgoing datagram uses the destination selected by the QUIC engine, including
IPv6 scope IDs; receiving a packet does not change the configured peer.

### Engine settings

Pass `engine_options=EngineOptions(...)` for retransmission timeout, ping timeout
multiplier, outgoing queue/event limits, initial parser capacity, `max_inflight`,
automatic keepalive, automatic acknowledgements, or configured subscriptions.
Omitted values retain the native defaults; numeric limits must be positive.
`max_inflight` limits outgoing QoS 1/2 messages and also respects the broker limit.
The CONNECT `receive_maximum` property separately advertises the incoming limit.

`EngineOptions(subscriptions=[Subscription("status/#", qos=1)])` subscribes after
a successful CONNACK when the broker has no previous session. SUBACK results are
available through `on_event`. `sessionless=True` discards local session state on
transport replacement and requires `clean_start=True` with zero session expiry.
A broker response indicating a new session also clears obsolete inflight state.

### Manual acknowledgements and parser depth

Use `EngineOptions(auto_ack=False)` to acknowledge received QoS 1/2 messages
yourself. Full messages include `packet_id` and `stream_id`. Call
`await client.acknowledge(message.packet_id, message.qos, stream_id=message.stream_id)`
to send PUBACK or PUBREC. For QoS 2, wait for `event.is_pub_rel_received()` and
then call `await client.complete_qos2(event.packet_id, stream_id=event.stream_id)`.
Both methods accept MQTT 5 reason codes and properties. Outgoing publish
handshakes remain managed by the engine. These methods report local submission.

`PublishReceived` metadata precedes each full message; `PubRelReceived` retains
the packet and stream ID. QUIC manual acknowledgements use that same stream and
respect its send-buffer limit. TCP/TLS reject explicit stream IDs.

After connecting with no outstanding operations, `client.set_parse_level(...)`
accepts `MqttParseLevelFfi.FULL`, `.HEADERS_PARSED`, or `.TYPE_ONLY`. Reduced modes
report receive metadata without full messages. They are intended for receive
counting; outgoing operations that require MQTT acknowledgements are rejected.
Type-only mode discards packet IDs and therefore only supports incoming QoS 0.
Reconnect resets the parser to Full so CONNECT can complete.

### QUIC controls

`client.quic` exposes `open_stream()`, `finish_stream(id)`,
`reset_stream(id, error_code=0)`, `stop_stream(id, error_code=0)`,
`control_stream_id`, and `data_stream_count`. Use
`await client.quic.publish(id, topic, payload, qos=1, properties=...)`,
`await client.quic.subscribe(id, [Subscription(...)])`, or
`await client.quic.unsubscribe(id, [topic])` to select a data stream.
Normal client methods continue to use the engine's default streams.

`client.quic.ping()` queues a QUIC PING. `notify_local_address_changed()` notifies
the engine after an external address change; it does not rebind the UDP socket.
`close_transport(error_code=0, reason=b"", silent=False)` closes QUIC directly
and disables retries. Use `await client.disconnect()` for MQTT DISCONNECT.
Transport close details and stream close/reset/stop events reach `on_event`.

Enable the in-memory TLS ticket cache with
`quic_zero_rtt=QuicZeroRttOptions(session_cache_size=256, replay_on_reject=True)`.
Read `client.quic.zero_rtt_status`, observe `ZeroRttStatusChanged` events, or
call `client.quic.clear_session_cache()`. The first connection normally reports
Unavailable; subsequent connections may report Attempted, Accepted, or Rejected.
To submit before CONNACK, explicitly use `early_data=True` on publish or batch
subscription methods while the status is Attempted. Early data can be replayed,
so applications must choose operations that tolerate duplicates. With
`replay_on_reject=True`, the engine resends rejected early bytes after the QUIC
handshake; with False, it leaves recovery to the caller.

## Verification and rebuilding

Run `./scripts/build_python_bindings.sh --test` from any directory. The suite
checks generated bindings as well as the asyncio wrapper. Live-broker tests are
opt-in; their TCP/TLS/QUIC setup is documented in [the examples guide](../examples/README.md#live-broker-regression-tests).

Regenerate Python, Swift, and Kotlin bindings whenever the native FFI changes.
Existing C layouts and signatures are retained; UniFFI records and events have
expanded and require matching generated bindings. Python package and native crate
versions are currently `0.6.1`. Release wheels build their native library inside
the target build environment and run the regression suite against the installed
wheel before upload.

## License

Mozilla Public License 2.0

### Runtime limits and durable sessions

Disk checkpoint/restore is disabled by default. Build bindings with
`bash scripts/build_python_bindings.sh --durable-session --test` to enable it.
Without it, `session_state=` and `checkpoint_for_restart()` raise
`MqttErrorFfi.Unsupported`; the generated native checkpoint APIs are absent.
Ordinary reconnect and in-memory session resumption work with either build.

`RuntimeOptions` adds protocol `OperationTimeouts`, incoming receive quota and
incoming/outgoing byte limits. These deadlines are separate from Python's
`timeout=` argument; `OperationTimeouts.cloud()` selects 30 seconds for CONNECT
and 10 seconds for publish/subscribe/unsubscribe. Defaults remain disabled.
`OperationFailedError` identifies the affected operation and packet ID without
closing unrelated healthy work. A late ACK is still delivered through `on_event`.

For planned restarts, construct `FlowMqttClient` with `clean_start=False`, a
nonzero MQTT 5 `ConnectProperties(session_expiry_interval=...)`, and
`RuntimeOptions(peer="tcp://broker.example:1883")`. The peer must match the
transport and the original hostname/port passed to `connect` (lowercase host,
bracketed IPv6). Low-level FFI engines allow application-defined peer identities.
Call `await client.checkpoint_for_restart()` to stop commands, timers and retries,
retire the transport abruptly, and return stable opaque bytes. This may trigger
the Will. The old client cannot connect again. Pending Python futures fail; they
are not serialized. Persist the bytes before constructing a fresh client with
`session_state=saved_bytes` and the same options. Supply credentials and Will again.
An empty client ID can be recovered from saved metadata; explicit mismatches fail.

`ClientSessionStore` defines application-owned create/resume/update/delete.
`SqliteSessionStore` implements transactional commits with SQLite FULL synchronous
mode and propagates native storage errors. Its methods are synchronous and must
run outside engine callbacks. Use one writer per key and include account identity
in keys as needed. `python/examples/durable_session.py` shows the lifecycle.
A snapshot is not automatic crash persistence: persist-before-send/event ordering
and a transactional application inbox remain the application's responsibility.

Python owns one opt-in reconnect scheduler (`auto_reconnect=False` by default).
During an actual retry it keeps ACK waiters and lets the engine replay existing
exchanges; disabling/exhausting retries fails them. `set_auto_reconnect(False)`
cancels retries. Offline publishing remains unsupported by the high-level client;
there is no additional Python replay queue. Canceling an await removes only its
waiter. `disconnect()` waits for protocol output and the socket close, bounded by
five seconds by default (also when timeout=None); cancellation aborts the socket.

`client.quic.subscribe_on_control(...)` and `unsubscribe_on_control(...)` expose
explicit control-stream commands. Default subscriptions still use a data stream.
Keep generated bindings and their native library from the same build together.
