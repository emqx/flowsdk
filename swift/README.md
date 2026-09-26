# Swift Examples

This folder contains Swift Package Manager examples for FlowSDK FFI.

## Prerequisites

- macOS with Swift toolchain
- Rust toolchain

## 1) Build bindings and native library

From repository root:

./scripts/build_swift_bindings.sh

This generates Swift bindings and copies libflowsdk_ffi into swift/lib.

## 2) Build the Swift package

From repository root:

LIBRARY_PATH="$PWD/swift/lib" swift build --package-path swift

## 3) Run TCP example

From repository root:

LIBRARY_PATH="$PWD/swift/lib" swift run --package-path swift TcpClientExample

Optional broker override:

LIBRARY_PATH="$PWD/swift/lib" swift run --package-path swift TcpClientExample broker.emqx.io 1883

## 4) Run QUIC example

From repository root:

LIBRARY_PATH="$PWD/swift/lib" swift run --package-path swift QuicClientExample

Optional broker override:

LIBRARY_PATH="$PWD/swift/lib" swift run --package-path swift QuicClientExample broker.emqx.io 14567

## Optional: TLS key logging for QUIC (Wireshark)

SSLKEYLOGFILE=~/tmp/sslkeylog.txt LIBRARY_PATH="$PWD/swift/lib" swift run --package-path swift QuicClientExample

### Runtime options and session restarts

Regenerate and run the portable regression harness with
`scripts/build_swift_bindings.sh --test`. The harness in
`Tests/FlowSDKTests/SessionTests.swift` runs with Command Line Tools as well as Xcode.
It is also a disk-round-trip example using a synthetic peer. Its temporary-file
write tests serialization only; production stores must durably commit using a
transactional database or fsync and atomic replacement.

Use `newWithRuntimeOptions` with `MqttRuntimeOptionsFfi` for explicit broker identity,
operation deadlines, receive quota, byte limits and reconnect policy. Existing
constructors remain compatible. Checkpoint APIs are disabled by default; generate
them with `bash scripts/build_swift_bindings.sh --durable-session --test`.
Runtime options, reconnect and in-memory session resumption work without this
feature. `snapshotSession()` returns opaque Data;
`restoreSessionState(state:)` requires a fresh engine before any transport I/O,
cleanStart=false and matching peer/client/version. `inspectSessionState(state:)`
returns metadata including a broker-assigned client ID. Supply credentials/Will
in new options. Stop driving the old engine and commit storage before replacement.

Application stores provide create/resume/update/delete: create fails if present,
resume never removes, update atomically replaces an existing key, delete is
idempotent. Use one writer per key and propagate errors without falling back to
a fresh session. Checkpoints alone do not provide crash-safe application processing.

OperationFailed events distinguish per-operation timeout/session loss from a
connection error. Do not release a live packet ID on timeout. Use checked CONNECT,
reset TCP/TLS explicitly for a new transport, and poll `takeEvents()` only once
instead of also handling returned copies. QUIC adds subscribeOnControl and
unsubscribeOnControl. Ship generated Swift and its native library together.
