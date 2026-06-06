// SPDX-License-Identifier: MPL-2.0
//! Paho C/C++ compatible API for FlowSDK MQTT client.
//!
//! This crate provides a drop-in replacement for the Eclipse Paho C MQTT client library,
//! wrapping FlowSDK's `MqttEngine` (sans-I/O) with a blocking I/O thread that manages
//! TCP/TLS connections using `std::net`.
//!
//! Two APIs are provided:
//! - **Synchronous** (`MQTTClient_*`): Blocking operations, matching `libpaho-mqtt3c`
//! - **Asynchronous** (`MQTTAsync_*`): Callback-based operations, matching `libpaho-mqtt3a`

// Paho's C API uses camelCase function/field names and snake_case type names
// (e.g. `MQTTAsync_onSuccess`). We mirror them exactly for ABI/source
// compatibility, so the nonstandard-style lints are intentionally off.
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]

pub mod async_api;
pub mod common;
pub mod inner;
pub mod sync_api;
