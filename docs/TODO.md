# FlowSDK TODO

This document tracks completed capabilities and remaining work in the current codebase.
Last reviewed: 2026-09-19.

## Messaging system goal

Support messaging within processes, between processes, over LANs, and across wide area networks, with request/response, publish/subscribe, and point-to-point communication over TCP, TLS, and QUIC.

## Completed ✅

### Clients, transports, and bindings

- [x] MQTT 3.1.1 and 5.0 packet serialization/deserialization.
- [x] Sans-I/O client support for MQTT 3.1.1 and 5.0, with an [integration and migration guide](NO_IO_CLIENT.md).
- [x] `TokioAsyncMqttClient` with async operations and acknowledgment-waiting convenience APIs.
- [x] Builder APIs for client options and commands.
- [x] TLS support through native-tls and rustls, including custom trust roots and mutual TLS configuration.
- [x] QUIC single-stream and multi-stream support, stream lifecycle APIs, and optional 0-RTT.
- [x] Rust QUIC source bind/rebind APIs and Sans-I/O local-address-change notification.
- [x] C FFI and UniFFI bindings for TCP, TLS, and QUIC engines with native events.
- [x] MQTT 5 properties, subscription identifiers, acknowledgment metadata, and AUTH events exposed through engine and FFI event APIs.
- [x] Python APIs for MQTT properties, Will configuration, manual acknowledgments, enhanced authentication, and QUIC stream control.
- [x] QUIC local-address-change notification exposed through FFI and Python; socket bind/rebind remains separate work below.
- [x] Packet inspection primitives with full, headers-only, raw-body, and type-only parsing.

### Protocol validation and client reliability

- [x] Raw packet API and malformed-packet generators behind `protocol-testing`.
- [x] Shared strict validation for complete MQTT 3.1.1 and 5.0 packet encoding and decoding: packet identifiers, required payloads, topics, flags, reason codes, and MQTT strings.
- [x] MQTT 5 property value ranges, singleton uniqueness, packet/Will contexts, and Will/Response Topic validation. Repeated User Properties and PUBLISH Subscription Identifiers remain supported.
- [x] CONNECT property merging with conflicting singleton and invalid configuration rejection.
- [x] Negotiated Receive Maximum, Maximum Packet Size, maximum QoS, retain/subscription capabilities, and Server Keep Alive handling.
- [x] Incoming packet-size enforcement and optional input/output byte limits, with output capacity reserved for protocol responses and session replay.
- [x] Topic Alias resolution and connection-scoped alias reset.
- [x] Collision-safe packet identifiers, incoming QoS 2 duplicate suppression, and checked manual acknowledgment stages.
- [x] In-memory reconnect recovery, including MQTT 3.1.1 persistent outbound replay when the broker reports no session and MQTT 5 session-loss handling.
- [x] Replay pending PUBREL before PUBLISH without consuming the new connection's PUBLISH quota; reset receive quota independently of retained QoS 2 state.
- [x] Optional operation deadlines and explicit failure events without prematurely releasing outstanding packet identifiers.
- [x] Terminal connection-state handling, CONNACK state checks, and SUBACK result-count validation.
- [x] Enhanced-authentication continuation and client-initiated re-authentication state/method validation in the client engine.
- [x] Negotiate Session Expiry Interval and reject a client DISCONNECT that extends a zero interval to a nonzero value.

See [protocol compliance](PROTOCOL_COMPLIANCE.md) for validation scope. Regression suites include [codec validation](../tests/codec_validation.rs), [Sans-I/O reliability](../tests/no_io_reliability.rs), and [client lifecycle](../tests/client_engine_lifecycle.rs). These cover implemented behavior; full specification conformance still needs a tracked audit.

### Server session helpers

- [x] Match non-shared subscriptions with `+` and `#`, including the `$` topic rule.
- [x] Track incoming QoS 2 duplicates separately from outgoing acknowledgment/quota state.
- [x] Apply subscription QoS and Retain As Published, with in-memory retained replay and Retain Handling options.
- [x] Forward subscription identifiers on live messages, retained replay, and retransmission; preserve per-subscription QoS and identifiers for overlapping subscriptions.
- [x] Replace or remove identifiers when subscriptions are replaced or removed.

See [subscription identifier regressions](../tests/server_subscription_identifiers.rs). These are local `ServerSession` capabilities; broker-wide storage and routing remain open below.

### MQTT/gRPC proxy and testing infrastructure

- [x] Bidirectional gRPC streaming between `r-proxy` and `s-proxy`, including broker-to-client message delivery.
- [x] MQTT 3.1.1 and 5.0 packet conversion/forwarding beyond CONNECT/CONNACK.
- [x] Command-line configuration for proxy ports, gRPC destination, and broker address.
- [x] Structured proxy logging through `tracing`.
- [x] CI coverage gates requiring at least 80% Rust workspace and Python coverage via [check_coverage.sh](../scripts/check_coverage.sh).

## Remaining work 📋

### Protocol compliance and interoperability

- [ ] Expand broker interoperability and malformed-input coverage beyond the current codec, engine, and local transport regression suites.
- [ ] Audit remaining session-dependent rules, including Session Expiry lifetime behavior, separately from packet-field validation.
- [ ] Add authentication-method examples and broker interoperability tests beyond the implemented AUTH exchange/state APIs.
- [ ] Expand TLS configuration documentation and transport-parity tests for custom CAs, client certificates, and ALPN.

### Transports and language wrappers

- [ ] Expose QUIC source socket bind/rebind through language client wrappers and test migration end to end; address-change notification alone does not rebind UDP sockets.
- [ ] Add Unix domain socket transport.
- [ ] Add WebSocket transport.
- [ ] Build user-facing packet inspection utilities on the existing parser APIs.

### Session management and persistence

- [ ] Provide broker-wide retained storage beyond the local `ServerSession` helper.
- [ ] Implement broker-wide message expiry and shared-subscription routing.
- [ ] Add durable message/session persistence across process restarts; current client recovery is in memory.

### MQTT/gRPC proxy hardening

- [ ] Add authentication to the gRPC server.
- [ ] Validate gRPC metadata, session-control messages, and converted packet fields before use.
- [ ] Replace remaining fallible-path `unwrap()` calls in the proxies with contextual errors, including metadata conversion and packet serialization.
- [ ] Ensure connection-map entries, broker tasks, and channels are cleaned up on every EOF, stream error, cancellation, and shutdown path.
- [ ] Add a timeout while waiting for the initial MQTT CONNECT in `r-proxy`.
- [ ] Improve error propagation for malformed initial packets and failed MQTT/protobuf conversion.

### Code quality and observability

- [ ] Review production `unwrap()` and ad hoc logging paths outside the proxies; propagate recoverable errors and use structured diagnostics.
- [ ] Consolidate duplicated client/transport logic where the shared engine can provide the same behavior.
- [ ] Add library metrics and observability beyond the existing proxy logging and benchmark counters.
