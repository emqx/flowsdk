# FlowSDK

[![CI](https://github.com/emqx/flowsdk/actions/workflows/ci.yml/badge.svg)](https://github.com/emqx/flowsdk/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/emqx/flowsdk/branch/main/graph/badge.svg?token=7VJHFC3JSE)](https://codecov.io/gh/emqx/flowsdk)
[![License: MPL 2.0](https://img.shields.io/badge/License-MPL_2.0-brightgreen.svg)](https://opensource.org/licenses/MPL-2.0)

**FlowSDK** is a Rust messaging SDK for MQTT 3.1.1 and 5.0, built around a reusable protocol engine. It gives applications control over how messaging fits into their runtime, how much work they accept, and when an operation is complete.

**Development status:** FlowSDK is under active development; its API is unstable and may change between releases.

## Why FlowSDK

- **Your runtime, your I/O.** The core engine handles MQTT state without opening sockets or running an event loop. Use the Tokio client for managed networking, or drive the same engine from your own I/O and timers.
- **Clear completion semantics.** Accepting a command, receiving a broker acknowledgement, and finishing shutdown are distinct events. Applications can wait for the result they need and handle rejection, timeout, or connection loss explicitly.
- **Control under load and failure.** Configurable queue and byte limits, backpressure, deadlines, and reconnect policies let you decide how much work to retain and how to recover. Session recovery is currently in memory.
- **Acknowledgements on your terms.** Automatic acknowledgements cover common uses; manual acknowledgements let your application decide when it has accepted an incoming message, including after storing it.
- **Same Rust core, same protocol behavior across languages.** Native Rust APIs and foreign function interface (FFI) bindings share the MQTT implementation for validation, QoS, acknowledgements, and session recovery. TCP, TLS, and QUIC use that same core, with lower-level QUIC APIs for stream control.
- **Test protocol behavior without a network.** Feed bytes and advance time to exercise fragmented input, backpressure, deadlines, and recovery without a live broker. The protocol engine is available independently of the networking client.

The same building blocks also support the workspace's [MQTT/gRPC proxies](mqtt_grpc_duality/) and [io_uring benchmark](mqtt_ring_bench/). FlowSDK is designed to fit messaging into the application architecture you choose.

## Supported languages

The FFI bindings call the Rust protocol engine directly, keeping protocol behavior consistent across languages. Each binding adapts the shared core to its language's types and runtime.

| Language | Integration | Getting started |
| --- | --- | --- |
| Rust | Native `flowsdk` crate with Tokio and sans-I/O clients. | [Tokio guide](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md), [sans-I/O guide](docs/NO_IO_CLIENT.md) |
| C / C++ | C ABI provided by `flowsdk_ffi`. | [C examples](examples/c_ffi_example/) |
| Python | Generated UniFFI bindings and an `asyncio` client wrapper. | [Python guide](python/package/README.md) |
| Swift | Generated UniFFI bindings with Swift Package Manager examples. | [Swift guide](swift/README.md) |
| Kotlin | Generated UniFFI bindings for the JVM. | [Build bindings](scripts/build_kotlin_bindings.sh), [Kotlin example](kotlin/examples/quic_client/README.md) |

## Try it

With a local MQTT broker running, try either compact publish/subscribe example:

| Example | Who drives I/O? |
| --- | --- |
| [async_pubsub.rs](examples/async_pubsub.rs) | The Tokio client manages networking and timers. |
| [no_io_pubsub.rs](examples/no_io_pubsub.rs) | Your application drives TCP I/O and timers around the MQTT engine. |

```bash
cargo run --example async_pubsub -- localhost:1883
cargo run --example no_io_pubsub -- localhost:1883
```

Both demonstrate QoS 1 and 2 delivery and graceful disconnect. Append `1` or `2` to select one QoS. More transport and integration examples are in [examples/](examples/).

## Learn more

- [Tokio client guide](docs/TOKIO_ASYNC_CLIENT_API_GUIDE.md) — managed I/O, operation completion, and lifecycle.
- [Sans-I/O guide](docs/NO_IO_CLIENT.md) — integrate the engine with your own networking and event loop.
- [Python guide](python/package/README.md) — async clients and direct engine access.
- [Protocol validation](docs/PROTOCOL_COMPLIANCE.md) and [remaining work](docs/TODO.md) — current scope and limitations.
- [Contributing](docs/CONTRIBUTING.md) and [testing](docs/TEST.md).

## License

FlowSDK is licensed under the [Mozilla Public License 2.0](LICENSE).
