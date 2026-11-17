# FlowSDK binding generator

Build native scaffolding with `flowsdk_ffi`'s `uniffi-bindings` feature, then run:

```sh
cargo run -p flowsdk-bindgen --bin uniffi-bindgen -- generate \
  --library target/debug/libflowsdk_ffi.so --language python --out-dir /tmp/bindings
```

The `scripts/build_{python,kotlin,swift}_bindings.sh` scripts build and package
the matching native library and generated bindings. This tool replaces the
former `flowsdk_ffi` binary target; it is not published to crates.io.

Keep UniFFI's exact version in this manifest aligned with `flowsdk_ffi/Cargo.toml`.
The textwrap constraint preserves the generator's Rust 1.85 minimum.
The runtime crate intentionally disables generator features so published SDK
consumers do not depend on Askama. The repository tool uses upstream Askama;
`deny.toml` documents the accepted `basic-toml` maintenance advisory until
the upstream generator dependencies replace it.
