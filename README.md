# FlowSDK

[![CI](https://github.com/emqx/flowsdk/actions/workflows/ci.yml/badge.svg)](https://github.com/emqx/flowsdk/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/emqx/flowsdk/branch/main/graph/badge.svg?token=7VJHFC3JSE)](https://codecov.io/gh/emqx/flowsdk)
[![License: MPL 2.0](https://img.shields.io/badge/License-MPL_2.0-brightgreen.svg)](https://opensource.org/licenses/MPL-2.0)

**FlowSDK** is a safety-first, behavior-predictable messaging SDK designed for modern distributed systems.

It allows you to build **messaging-based micro-middleware** that runs within your application, communicating efficiently with other apps locally or remotely using protocols like MQTT (v5.0) and gRPC.

> **Philosophy**: Messaging has costs and networks can be unreliable. FlowSDK is honest about constraints like latency and resource limits, enabling you to build resilient systems that handle failures gracefully rather than hiding them.

---

## Key Features

*   **Robust MQTT v5.0 Client**: Full support for MQTT v5.0 features (shared subscriptions, request/response, properties).
*   **Async & Sync Dual API**: Built on `tokio` for high-performance async I/O, with convenience wrappers for synchronous-style operations.
*   **Multi-Transport Support**:
    *   **TCP**: Standard reliable transport.
    *   **TLS**: Secure encrypted communication (via `rustls` or `native-tls`).
    *   **QUIC**: Next-gen low-latency, encrypted transport (via `quinn`).
*   **AI/LLM Friendly**: Codebase and docs are structured for easy consumption by AI tools, making it a great target for agentic coding.
*   **Cross-Platform & FFI**: Native Rust implementation with C bindings (`flowsdk_ffi`) for integration with other languages.
*   **Production Ready**: Includes advanced flow control, priority queuing, and comprehensive event handling.

---

## Project Structure

This workspace consists of three main components:

| Component | Directory | Description |
| :--- | :--- | :--- |
| **Core Library** | [`/`](./) | The main `flowsdk` crate. Contains the MQTT client, protocol logic, and examples. |
| **FFI Bindings** | [`flowsdk_ffi/`](flowsdk_ffi/) | C/C++ bindings generated via UniFFI. Allows using FlowSDK from other languages. |
| **Proxy Workspace** | [`mqtt_grpc_duality/`](mqtt_grpc_duality/) | A specialized workspace for `r-proxy` (client-facing) and `s-proxy` (server-side) applications. |

---

## Quick Start

### 1. Installation

Add `flowsdk` to your `Cargo.toml`.

```toml
[dependencies]
flowsdk = { version = "0.3.1", features = ["tls"] }
# For QUIC support:
# flowsdk = { version = "0.3.1", features = ["quic"] }
```

### 2. Basic Usage

Here is a minimal example of connecting, subscribing, and publishing.

```rust
use flowsdk::mqtt_client::{MqttClientOptions, TokioAsyncMqttClient, TokioAsyncClientConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Configure the Client
    let options = MqttClientOptions::builder()
        .peer("broker.emqx.io:1883")
        .client_id("flowsdk-example")
        .keep_alive(60)
        .build();

    // 2. Create the Client
    let client = TokioAsyncMqttClient::new(
        options, 
        Box::new(|ev| { println!("Event: {:?}", ev); Box::pin(async {}) }), 
        TokioAsyncClientConfig::default()
    ).await?;

    // 3. Connect
    client.connect_sync().await?;
    println!("Connected!");

    // 4. Subscribe
    client.subscribe_sync("hello/flowsdk", 1).await?;

    // 5. Publish
    client.publish_sync("hello/flowsdk", b"Hello from FlowSDK!", 1, false).await?;

    Ok(())
}
```

---

## Documentation Index

Detailed documentation is available in the [`docs/`](docs/) directory.

| Document | Description |
| :--- | :--- |
| [Client API Guide](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md) | **Start Here**. Complete reference for the `TokioAsyncMqttClient` API. |
| [Async Client Architecture](docs/ASYNC_CLIENT.md) | Deep dive into the event-driven design and callback system. |
| [Builder Pattern](docs/BUILDER_PATTERN.md) | Guide to using the configuration builders for options and commands. |
| [MQTT Session](docs/MQTT_SESSION.md) | Explanation of session state, inflight buffers, and persistence. |
| [Protocol Compliance](docs/PROTOCOL_COMPLIANCE.md) | Details on MQTT v5.0 compliance, validation, and strict mode. |
| [Testing & Fuzzing](docs/TEST.md) | How to run the test suite, fuzzers, and compliance tests. |
| [Development](docs/DEV.md) | Guide for contributors: build setup, workflow, and CI. |

---

## Examples

Check the [`examples/`](examples/) directory for runnable code.

*   `mqtt_client_v5`: Traditional synchronous loop style.
*   `tokio_async_mqtt_client_example`: Full featured async client.
*   `tls_client`: Secure connection example.
*   `tokio_async_mqtt_quic_example`: **QUIC** transport example.
*   `c_ffi_example`: How to use the C bindings.

Run an example:
```bash
cargo run --example tokio_async_mqtt_client_example
```

---

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](docs/CONTRIBUTING.md) for guidelines.

## License

This project is licensed under the [MPL 2.0 License](LICENSE).
