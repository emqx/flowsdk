# Checked C API (version 1)

Include `include/flowsdk.h` and link the matching `libflowsdk_ffi`.
`MqttOptionsC` layouts and event tags 1–18 are unchanged. QUIC symbols require the
`quic` feature; disabled TLS constructors return `MQTT_UNSUPPORTED`.

JSON support is optional and disabled by default. Build with
`cargo build -p flowsdk_ffi --features json` and define `FLOWSDK_JSON` before
including the header to use `*_new_v1`, `*_command_v1`, `*_take_events` (JSON
strings), or `mqtt_event_list_get_json`. These symbols and serialization of FFI
records require the `json` feature, which enables the FFI's optional `serde` and
`serde_json` dependencies. `durable-session` enables `json` automatically;
`FLOWSDK_DURABLE_SESSION` likewise implies `FLOWSDK_JSON` in the C header.

Typed C calls, `*_take_events_list` and event accessors work without JSON or
UniFFI. Typed UniFFI bindings also work without JSON. The core `flowsdk` crate
still uses Serde internally; this feature controls the FFI's direct dependencies.

The additive constructors and command calls accept UTF-8 JSON and its byte length.
JSON avoids changing existing C struct layouts when optional settings are added.
Use the typed UniFFI records in Python, Kotlin and Swift.

```json
{
  "version": 1,
  "connect": {
    "options": {
      "client_id": "device-17",
      "mqtt_version": 5,
      "clean_start": false,
      "keep_alive": 30
    },
    "properties": [{"SessionExpiryInterval": {"value": 3600}}],
    "will": {
      "topic": "device-17/status", "payload": [111, 102, 102],
      "qos": 1, "retain": true, "properties": []
    }
  },
  "runtime": {
    "peer": "tcp://broker.example:1883",
    "operation_timeouts": {"connect_ms": 30000, "publish_ms": 10000},
    "incoming_receive_maximum": 32,
    "max_incoming_packet_size": 65536,
    "max_incoming_buffer_bytes": 131072,
    "max_outgoing_buffer_bytes": 131072,
    "reconnect": false
  }
}
```

Pass this to `mqtt_engine_new_v1`, `mqtt_tls_engine_new_v1`, or
`mqtt_quic_engine_new_v1`. TLS additionally takes top-level `server_name` and
`tls` (CA/client certificate paths, `insecure_skip_verify`, `alpn_protocols`,
`enable_key_log`). QUIC takes TLS/endpoint settings at CONNECT instead.
`connect.binary_password` accepts a byte array instead of `options.password`.
`connect.engine_options` has the same fields as `MqttEngineOptionsFFI`.
Byte payloads are arrays of integers in 0–255; properties use the enum names in
`engine/properties.rs`, for example `{"UserProperty":{"key":"k","value":"v"}}`.

Missing `options` fields use `MqttOptionsFFI::default()` (MQTT 5, clean start,
60-second keepalive, no credentials). Missing properties are empty; missing
optional settings are null. Deadlines are disabled by default. Zero is an
immediate deadline, not a disabled deadline. `max_inflight` limits outgoing
exchanges; `incoming_receive_maximum` advertises the distinct inbound quota.
CONNECT singletons with equal values deduplicate; conflicting values fail.

`*_command_v1` returns a status and optional packet ID (0 means no ID). Commands:

| JSON command             | Additional fields                                                                    | Transport   |
|--------------------------|--------------------------------------------------------------------------------------|-------------|
| `connect`                | none                                                                                 | TCP/TLS     |
| `reset_transport`        | none                                                                                 | TCP/TLS     |
| `set_reconnect`          | `enabled`                                                                            | all         |
| `schedule_reconnect`     | `now_ms`                                                                             | TCP         |
| `ping`                   | none                                                                                 | all         |
| `auth`                   | `reason_code`, `properties`                                                          | all, MQTT 5 |
| `disconnect`             | `options`: reason_code, properties                                                   | all         |
| `publish`                | `topic`, `payload`, `options`: qos, retain, priority, properties                     | all         |
| `subscribe`              | `options`: subscriptions, properties                                                 | all         |
| `unsubscribe`            | `options`: topics, properties                                                        | all         |
| `acknowledge`            | `kind`: `PubAck`/`PubRec`/`PubComp`, `packet_id`, reason_code, properties, stream_id | all         |
| `connect_quic`           | `server_addr`, `server_name`, `tls`, `now_ms`, optional `zero_rtt`                   | QUIC        |
| `reconnect`              | `now_ms`                                                                             | QUIC        |
| `subscribe_on_control`   | same options as subscribe                                                            | QUIC        |
| `unsubscribe_on_control` | same options as unsubscribe                                                          | QUIC        |

For example `{"command":"subscribe_on_control","options":{"subscriptions":[{"topic_filter":"a/+","qos":1}]}}`.
Default QUIC subscriptions use a data stream. Explicit control subscriptions
produce the same ACK and timeout events. `zero_rtt` contains
`session_cache_size` and `replay_on_reject`.

CONNECT is checked and does not discard an active handshake or queued events.
Call reset only when replacing TCP/TLS transport; TLS reset recreates its TLS
state. Restore a checkpoint **before** reset, CONNECT or any transport I/O.
Time arguments are milliseconds relative to the engine's creation, as for legacy
ticks. Keep using existing input/tick/output APIs to drive the engine.

Status values: 0 success, 1 invalid argument/JSON, 2 configuration, 3 engine
failure, 4 unsupported transport/build. Nonnull error outputs are owned strings
freed with `mqtt_engine_free_string`. Output pointers/lengths are initialized on
failure. Error and packet-ID output pointers may be null. Other outputs are
required. Input pointers may be null only with zero length; all nonnull pointers
must reference valid memory for their lengths. Outputs must not alias inputs.
Do not free handles during a concurrent call.

Checkpoint APIs require building `flowsdk_ffi` with `--features durable-session`
(disabled by default). Define `FLOWSDK_DURABLE_SESSION` before including
`flowsdk.h` to declare those optional symbols. Ordinary reconnect and in-memory
session resumption remain available without this feature.

`*_snapshot_session` returns opaque bytes; free them with
`mqtt_engine_free_bytes(ptr, len)`. `*_restore_session_state` borrows the input
only for the call. `mqtt_session_inspect` returns owned metadata JSON, including
the client ID assigned by the broker. Inspection does not prove the broker still
has the session. Restoration requires the same explicit peer/client/version,
`clean_start=false`, session tracking, and a fresh engine. Failed restoration
leaves the engine usable. Supply credentials, Will and transport configuration
again; they are not in the checkpoint.

New event tag **19** is `OperationFailed`. `mqtt_event_list_get_json` returns owned
event JSON, including operation, nullable packet ID, kind (`Timeout`,
`SessionExpired`, `Cancelled`, `Other`), detail and nullable timeout milliseconds.
`*_take_events` also returns event JSON without requiring UniFFI. Poll only once:
the Rust/UniFFI input and tick methods return event copies which are also queued.
A timed-out operation may still complete its MQTT exchange; do not free/reuse its
packet ID in a host retry queue. Ordinary `Error` events remain connection errors.

Use an application store with create/resume/update/delete. Creation must fail if
present, resume must not remove the record, update must atomically replace an
existing record, and delete is idempotent. Use one writer per key, propagate
storage failures, and include account identity in the key where needed. Commit
the checkpoint before starting a replacement client. Checkpoints alone do not
provide transactional application processing or automatic crash persistence.

`tests/c_restart.c` is a compiled ownership/restart example with a synthetic wire
peer. Run `cargo build -p flowsdk_ffi --no-default-features --features durable-session`
then `bash scripts/test_c_bindings.sh --durable-session`. Use `--features json`
and `bash scripts/test_c_bindings.sh --json` to exercise in-memory reconnect
through the JSON API without checkpoint support. Build with
`--no-default-features` and run `bash scripts/test_c_bindings.sh` to test the
typed C API (`tests/c_basic.c`) without either feature.
