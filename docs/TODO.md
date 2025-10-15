# Project Improvement TODO List

This document tracks the tasks for improving the `flowsdk` project.

# Messaging system goal

Messages can be communicated within processes, between processes, over LAN, and across wide area networks. It supports client-server req/resp, pub/sub mode, or point-to-point mode. The network transport layer supports traditional TCP-based or UDP protocols, as well as security-focused TCP/TLS, and even the latest QUIC protocol.

## Code Quality 

- [ ] Improve error handling by replacing `unwrap()` with proper error handling and using `?` to propagate errors.
- [ ] Add configuration options for the broker and gRPC server addresses.
- [ ] Improve logging by replacing `println!` with a logging framework like `tracing`.
- [ ] Refactor duplicated code between `client.rs` and `s-client.rs`.

## Performance and Optimization

- [ ] Implement connection state cleanup to prevent memory leaks.

## Architecture

- [ ] Implement bidirectional message flow using server-streaming gRPC for subscriptions.

## Security

- [ ] Add authentication to the gRPC server.
- [ ] Add input validation to the gRPC server.

## bin/r-proxy.rs Specific Improvements

- [ ] Fix the infinite `loop {}` in spawned tasks in `run_proxy` to allow proper task termination and resource release.
- [ ] Implement full MQTT proxy logic to handle all MQTT control packets (PUBLISH, SUBSCRIBE, PINGREQ, DISCONNECT, etc.) beyond CONNECT/CONNACK.
- [ ] Utilize the `mpsc::Sender` and implement the corresponding `Receiver` to handle incoming messages from the gRPC server (e.g., PUBLISH messages).
- [ ] Improve error handling in `r-proxy.rs` to provide more specific error types and context for easier debugging.
- [ ] Replace `eprintln!` with structured logging (e.g., `tracing`) for better observability in production environments.
- [ ] Implement robust connection state management to ensure proper handling of client connections throughout their lifecycle.
- [ ] Should use gRPC stream mode

## Missing MQTT 5.0 Mandatory Normative Statements

*Analysis of MQTT 5.0 specification Appendix B reveals the following missing mandatory validations:*

### HIGH Priority (Protocol Correctness)
- [ ] **Property Value Ranges**: Validate that `ReceiveMaximum` > 0 [MQTT-3.1.2-18]
- [ ] **Property Value Ranges**: Validate that boolean properties (Request Problem Information, Request Response Information, etc.) are only 0 or 1 [MQTT-3.1.2-14]
- [ ] **Duplicate Properties**: Detect and reject duplicate properties in property sets [MQTT-2.2.2-2]
- [ ] **MaximumPacketSize**: Enforce `MaximumPacketSize` limits during packet parsing/encoding [MQTT-3.1.2-24]
- [ ] **Property Context Validation**: Validate property usage context (e.g., Will properties only when Will Flag set) [MQTT-3.1.2-11]

### MEDIUM Priority (Compliance Enhancement)
- [ ] **Will Topic Validation**: Validate Will Topic syntax when Will Flag is set [MQTT-3.1.2-9]
- [ ] **Session Expiry**: Validate Session Expiry Interval behavior and limits [MQTT-3.1.2-23]
- [ ] **Server Keep Alive**: Implement Server Keep Alive override validation [MQTT-3.1.4-13]

### LOW Priority (Recommendation Level)
- [ ] **Client ID Characters**: Validate Client ID contains only recommended characters (SHOULD requirement) [MQTT-3.1.3-5]

### Implementation Notes
- Property value range validation should be added to `PropertyType::decode()` methods
- Duplicate property detection requires tracking during property parsing
- MaximumPacketSize enforcement needs integration with buffer size management
- Boolean property validation can be added as feature-gated checks in property decoding
- Property context validation requires cross-referencing with packet flags during validation

## Session Management
- [ ] Implement MQTT topic matching with wildcards (+ and #) for message routing in the server session.
