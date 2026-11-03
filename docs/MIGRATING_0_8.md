# Migrating Rust publish payloads to 0.8

FlowSDK 0.8 is a source-breaking Rust release. `PublishCommand::payload` and
the MQTT 3.1.1/5 `MqttPublish::payload` fields now hold `bytes::Bytes`. This also
affects `MqttMessage`, the MQTT 5 publish type used by receive callbacks.
The fields are immutable shared buffers. Cloning commands, packets, inflight
entries, or in-memory session checkpoints shares their payload allocation.

## Publish without a replacement payload copy

```rust
use bytes::Bytes;
use flowsdk::mqtt_client::PublishCommand;

let retry_payload = Bytes::from(vec![0u8; 4096]);
let command = PublishCommand::from_bytes("telemetry", retry_payload.clone(), 1, false);
assert_eq!(command.payload.as_ptr(), retry_payload.as_ptr());
// Submit command through publish_with_command, publish_with_command_sync,
// publish_with_command_admitted, or the sans-I/O engine's publish method.
```

`from_bytes` moves the supplied handle. It preserves unique, shared, sliced,
static, and owner-backed buffers. This guarantee extends through the command
channels, MQTT packet, priority queue, inflight state, and reconnect replay.
It excludes wire serialization, transport buffers, decoding, persistence, and
conversion to external representations that require their own byte vectors.
Sharing may allocate reference-count metadata; it does not allocate or copy a
replacement payload. Topic names and properties still have their own allocations.

`PublishCommand::new`, `simple`, and `with_priority`, and the PUBLISH packet
constructors accept `impl Into<Bytes>`. Passing an existing `Vec<u8>` moves its
allocation, including vectors with spare capacity. Passing `Bytes` retains its
storage. A redundant `.into()` at a generic constructor call may now need to be
removed, or an unconstrained `.collect()` annotated as `.collect::<Vec<u8>>()`.

The builder has `.payload_bytes(Bytes)` for shared storage. Its existing
`.payload(impl Into<Vec<u8>>)` method still accepts strings, arrays and borrowed
slices; pass a `Vec` there to transfer ownership. Passing shared `Bytes` to that
legacy method can copy, so use `.payload_bytes` for the shared-buffer guarantee.
The high-level helpers accepting `&[u8]` continue to copy borrowed inputs into
owned storage; use a command to pass an existing shared handle.

## Update field access

* Struct literals and assignments: use `payload: data.into()` for a `Vec`.
* Read bytes with `&packet.payload` or `packet.payload.as_ref()`.
* Replace `payload.as_slice()` with `payload.as_ref()`.
* Compare against byte arrays using `payload.as_ref() == b"value"`.
* For mutation, construct a new payload or use `BytesMut` and `.freeze()`.
  `payload.to_vec()` explicitly copies when independent mutable storage is needed.
* To move into an API requiring a vector, use `Vec::<u8>::from(payload)`.
  This may reclaim uniquely owned storage; shared, sliced, static, or externally
  owned storage can require copying or moving bytes.

The borrowed `to_mqtt_publish` / `to_mqttv3_publish` conversions still clone topic
and property metadata but now share the payload. Consuming conversions move all
fields. The legacy `MqttControlPacket::payload()` accessor still returns an owned
`Vec<u8>` and therefore copies; the optimized wire encoder reads the `Bytes`
directly without calling that accessor.

## Lifetime and memory accounting

The SDK releases its handles when a publication no longer needs its payload.
QoS 0 can release after encoding; QoS 1 retains through PUBACK; QoS 2 can release
after PUBREC replaces PUBLISH with PUBREL. Deferred publishes and session replay
keep handles while outstanding. Caller-held checkpoints, packets, or retry
buffers can keep the allocation alive after the SDK completes or shuts down.
Rejected commands release their handles, and worker shutdown drops pending
command/admission channels and engine state.

A 4 KiB slice of a 64 KiB allocation keeps the 64 KiB backing allocation alive.
Queue byte limits continue to count the encoded packet's visible bytes, not
backing allocation capacity or memory also retained by the application. This
preserves existing admission semantics. Use `Bytes::copy_from_slice(&slice)`
explicitly when compacting a long-lived slice is worth a copy.

## Portability, persistence, and bindings

* `Bytes` was already an alloc-only dependency. The change adds no `std`
  requirement. Targets without pointer atomics still use `portable-atomics` and
  an application-provided critical-section implementation, as before.
* Payload serde adapters preserve the former `Vec<u8>` sequence representation,
  including numeric byte arrays in JSON. The durable checkpoint schema remains
  version 1. Serialization writes payload bytes; deserialization creates new
  storage. In-memory snapshots share handles with the engine.
* Fuzzing derives generate a `Vec<u8>` and transfer it into `Bytes`, retaining
  the arbitrary-byte input coverage without requiring `Bytes: Arbitrary`.
* C and UniFFI records retain their byte-vector interfaces. Swift, Kotlin, and
  Python callers do not change. Core, FFI, and Python package versions advance
  together to 0.8.0 to match the existing release workflow. Conversion to FFI
  events consumes `Bytes` into the required vector. C input pointers still
  require an owned copy at the language boundary.
* Protobuf schemas remain unchanged; the proxy converts between protobuf vectors
  and MQTT shared payloads at its boundary.

See [shared payload benchmarks](BENCHMARKS.md#shared-publish-payloads) for allocation,
copy-volume, elapsed-time, and retained-memory comparisons.
