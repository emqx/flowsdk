# Migrating Rust publish payloads to 0.8

FlowSDK 0.8 makes shared payload storage opt-in. `mqtt_serde::Payload`, used by
`PublishCommand::payload` and both MQTT PUBLISH packet types, is `Vec<u8>` by
default and `bytes::Bytes` with `shared-payloads`. This also affects `MqttMessage`,
the MQTT 5 publish type used by receive callbacks. Shared mode clones handles
through commands, packets, inflight state, and in-memory session checkpoints;
owned mode clones independent vectors.

## Optional batch admission configuration

The Tokio client adds `publish_batch_admitted`, `PublishAdmissionConfig`,
`PublishBatchAdmission`, and `PublishBatchAdmissionError`, re-exported from
`flowsdk::mqtt_client` when `async-client` is enabled. Configure explicit message
and logical-byte limits using `TokioAsyncClientConfig::builder().publish_admission(...)`.
See the [batch admission contract](TOKIO_ASYNC_CLIENT_API_GUIDE.md#publish_batch_admitted---bounded-local-admission)
for partial results, cancellation, ordering, and memory-accounting limits.

`TokioAsyncClientConfig` has a new public field:

```rust
pub publish_admission: Option<PublishAdmissionConfig>
```

Its default is `None`, preserving single-publication admission behavior and
disabling nonempty batches. Exhaustive config literals must add
`publish_admission: None`; literals using `..Default::default()` and the builder
need no compatibility update. This is a Rust source-compatibility change for
exhaustive literals, not a new requirement on the sans-I/O/no_std core.

## Select the storage configuration

An owned-only `no_std + alloc` application can use:

```toml
[dependencies]
flowsdk = { version = "0.8", default-features = false, features = ["alloc"] }
```

This configuration does not depend on `bytes`, `portable-atomic`, or
`critical-section`. It builds on `thumbv6m-none-eabi` (Cortex-M0, no pointer
atomics), including with `strict-protocol-compliance` and `durable-session`.
The application supplies its normal allocator, clock, and I/O; no atomic
emulation or critical-section provider is required by the SDK.

Applications such as VeloFlux opt into sharing separately:

```toml
[dependencies]
flowsdk = { version = "0.8", features = ["shared-payloads"] }
bytes = "1.12.1"
```

For shared storage without `std`, disable defaults and enable `alloc,shared-payloads`
on a target with pointer atomics. `portable-atomics` remains a separate optional
choice for shared storage on targets without them; that choice requires an
application-supplied critical-section provider. Neither `std` nor the default
features implicitly enables `shared-payloads`. Cargo features are additive, so
another dependency enabling it changes the storage type for the whole build.

The public parser buffers follow the same selection: leveled parser input,
`RawBodyPacket::remaining`, and `HeadersParsedPacket::raw_payload` use `Payload`.
`MqttParser::buffer_mut()` returns `ParserBuffer` (`Vec<u8>` by default,
`BytesMut` with sharing). Owned parsing compacts vectors; shared parsing retains
buffer views. Code that previously passed `Bytes` into leveled parsers must enable
`shared-payloads`, or pass vectors instead. These parser API changes, the generic
publish constructors, and the shared-mode field changes are versioned in 0.8.

The remainder of this guide describes **shared mode**.

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
constructors accept `impl Into<Payload>` (`Bytes` in shared mode). Passing an
existing `Vec<u8>` moves its allocation, including vectors with spare capacity. Passing `Bytes` retains its
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
* Read bytes with `&packet.payload[..]`, which works in both modes.
* Replace `payload.as_slice()` with `&payload[..]` for code supporting both modes.
* Compare against byte arrays using `&payload[..] == b"value"`.
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

* `shared-payloads` adds no `std` requirement. The owned-only core has no
  atomics dependency; see the configurations above.
* Payload serde adapters preserve the former `Vec<u8>` sequence representation,
  including numeric byte arrays in JSON. The durable checkpoint schema remains
  version 1. Serialization writes payload bytes; deserialization creates new
  storage. In-memory snapshots share handles with the engine in shared mode and
  clone vectors in owned mode; both modes use the same checkpoint schema.
* Fuzzing derives generate a `Vec<u8>` and transfer it into `Bytes`, retaining
  the arbitrary-byte input coverage without requiring `Bytes: Arbitrary`.
* C and UniFFI records retain their byte-vector interfaces. Swift, Kotlin, and
  Python callers do not change. Core, FFI, and Python package versions advance
  together to 0.8.0 to match the existing release workflow. Conversion to FFI
  events consumes the payload into the required vector in either mode. C input pointers still
  require an owned copy at the language boundary.
* Protobuf schemas remain unchanged; the proxy converts between protobuf vectors
  and MQTT shared payloads at its boundary.

See [shared payload benchmarks](BENCHMARKS.md#shared-publish-payloads) for allocation,
copy-volume, elapsed-time, and retained-memory comparisons.
