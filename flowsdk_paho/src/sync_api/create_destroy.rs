// SPDX-License-Identifier: MPL-2.0
//! `MQTTClient_create` and `MQTTClient_destroy`

use libc::{c_char, c_int, c_void};
use std::ffi::CStr;
use std::sync::Arc;

use flowsdk::mqtt_client::opts::MqttClientOptions;

use crate::common::return_codes::*;
use crate::common::uri_parser;
use crate::inner::client_state::{PahoClientInner, SharedState};
use crate::inner::io_worker::IoWorker;

/// Paho-compatible `MQTTClient` handle type (opaque pointer).
pub type MQTTClient = *mut c_void;

/// Create an MQTT client.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_create(MQTTClient* handle, const char* serverURI,
///                        const char* clientId, int persistence_type,
///                        void* persistence_context);
/// ```
///
/// # Safety
/// - `handle`, `server_uri`, and `client_id` must be valid non-null pointers.
/// - `handle` must point to writable memory for an `MQTTClient`.
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_create(
    handle: *mut MQTTClient,
    server_uri: *const c_char,
    client_id: *const c_char,
    persistence_type: c_int,
    _persistence_context: *mut c_void,
) -> c_int {
    // Validate parameters
    if handle.is_null() || server_uri.is_null() || client_id.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    // Parse strings
    let server_uri_str = match CStr::from_ptr(server_uri).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTCLIENT_BAD_UTF8_STRING,
    };

    let client_id_str = match CStr::from_ptr(client_id).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTCLIENT_BAD_UTF8_STRING,
    };

    // Validate server URI is parseable
    if uri_parser::parse_server_uri(&server_uri_str).is_none() {
        return MQTTCLIENT_FAILURE;
    }

    // Only PERSISTENCE_NONE is supported in Phase 1
    if persistence_type != MQTTCLIENT_PERSISTENCE_NONE
        && persistence_type != MQTTCLIENT_PERSISTENCE_DEFAULT
    {
        // Silently accept — graceful degradation (we just don't persist)
    }

    // Determine peer address for MqttClientOptions
    // We'll re-parse at connect time, but need something for the engine
    let parsed = uri_parser::parse_server_uri(&server_uri_str).unwrap();

    // Build MqttClientOptions with defaults — connect options will override most of these
    let options = MqttClientOptions::builder()
        .peer(&parsed.addr)
        .client_id(&client_id_str)
        .mqtt_version(4) // Default to MQTT 3.1.1, overridden at connect time
        .keep_alive(60)
        .clean_start(true)
        .build();

    // Create communication channels
    let (command_tx, command_rx) = crossbeam_channel::unbounded();
    let (message_tx, message_rx) = crossbeam_channel::bounded(1000);

    // Create shared state
    let shared = Arc::new(SharedState::new(message_tx));
    let shared_clone = shared.clone();

    // Spawn I/O worker thread
    let io_thread = std::thread::Builder::new()
        .name(format!("paho-io-{}", client_id_str))
        .spawn(move || {
            let worker = IoWorker::new(options, command_rx, shared_clone);
            worker.run();
        });

    let io_thread = match io_thread {
        Ok(handle) => handle,
        Err(_) => return MQTTCLIENT_FAILURE,
    };

    // Allocate the opaque handle
    let inner = Box::new(PahoClientInner {
        command_tx,
        io_thread: Some(io_thread),
        shared,
        message_rx,
        server_uri: server_uri_str,
        client_id: client_id_str,
        mqtt_version: 4,
    });

    *handle = Box::into_raw(inner) as *mut c_void;

    MQTTCLIENT_SUCCESS
}

/// Destroy an MQTT client and free all associated resources.
///
/// # Paho C signature
/// ```c
/// void MQTTClient_destroy(MQTTClient* handle);
/// ```
///
/// # Safety
/// - `handle` must be a valid pointer to an `MQTTClient` created by `MQTTClient_create`.
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_destroy(handle: *mut MQTTClient) {
    if handle.is_null() || (*handle).is_null() {
        return;
    }

    // Recover the Box and drop it (triggers PahoClientInner::drop → shutdown + join)
    let inner = Box::from_raw(*handle as *mut PahoClientInner);
    drop(inner);

    *handle = std::ptr::null_mut();
}
