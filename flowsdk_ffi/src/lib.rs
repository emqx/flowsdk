// SPDX-License-Identifier: MPL-2.0

// Alias the fork's quinn-proto to the standard name when building with the
// OpenSSL backend, so engine.rs code using quinn_proto:: works transparently.
#[cfg(feature = "quic-openssl")]
extern crate quinn_proto_openssl as quinn_proto;

pub mod engine;

#[cfg(feature = "uniffi-bindings")]
uniffi::setup_scaffolding!("flowsdk_ffi");
