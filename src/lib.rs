// SPDX-License-Identifier: MPL-2.0

// When building with the OpenSSL QUIC backend (fork), alias the renamed crates
// back to the standard names so all downstream code using `quinn::` and
// `quinn_proto::` works transparently.  LTO strips the unused mainstream quinn
// symbols from the release-small binary.
#[cfg(feature = "quic-openssl")]
extern crate quinn_openssl as quinn;
#[cfg(feature = "quic-proto-openssl")]
extern crate quinn_proto_openssl as quinn_proto;

pub mod mqtt_client;
pub mod mqtt_serde;
pub mod mqtt_session;
pub mod priority_queue;
