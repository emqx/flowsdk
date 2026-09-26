# Kotlin bindings

### Session checkpoints and runtime options

Checkpoint APIs are opt-in and absent from default builds. Enable them with
`bash scripts/build_kotlin_bindings.sh --durable-session`. This also includes the
disk restart tests; plain builds test in-memory session resumption. Runtime
options and ordinary reconnect work without the feature.

Regenerate with `scripts/build_kotlin_bindings.sh` using JDK 17. The package now
tracks the Rust/Python version, 0.6.1. `MqttRuntimeOptionsFfi` and
`newWithRuntimeOptions` expose deadlines, byte limits, reconnect policy and an
explicit peer identity. Existing constructors remain available.

Each engine exports `snapshotSession()` / `restoreSessionState(bytes)`;
`inspectSessionState(bytes)` returns metadata including a broker-assigned client
ID. Restore only into a fresh engine before transport I/O, using cleanStart=false,
matching peer/client/version and new credentials/Will. Storage is application-owned:
create fails if present, resume reads without removing, update atomically commits
an existing key, delete is idempotent. Propagate storage failures and use one writer
per key. Stop driving the old transport and commit bytes before replacement.

`package/src/test/kotlin/SessionTest.kt` is an executable disk-round-trip example
with a synthetic wire peer. Its temporary file is a test fixture, not a production
durability backend; use a transactional database or fsync plus atomic replacement.
Checkpoints do not implement automatic crash-safe application processing.

`MqttEventFfi.OperationFailed` retains operation, packet ID, failure kind and timeout.
Handle it independently from connection-level Error; late ACK events may follow.
Use checked CONNECT and explicit transport reset. Poll `takeEvents()` OR consume
returned event copies, never both. QUIC adds subscribeOnControl/unsubscribeOnControl.
