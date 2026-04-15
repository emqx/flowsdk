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
