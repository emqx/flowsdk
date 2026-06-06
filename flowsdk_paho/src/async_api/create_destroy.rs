// SPDX-License-Identifier: MPL-2.0
//! `MQTTAsync_create`, `MQTTAsync_createWithOptions`, `MQTTAsync_destroy`

use libc::{c_char, c_int, c_void};
use std::ffi::CStr;
use std::sync::Arc;

use flowsdk::mqtt_client::opts::MqttClientOptions;

use crate::common::async_structs::{MQTTAsync, MQTTAsync_createOptions};
use crate::common::return_codes::*;
use crate::common::uri_parser;
use crate::inner::client_state::{PahoClientInner, SharedState};
use crate::inner::io_worker::IoWorker;

/// Build a `PahoClientInner` and spawn its I/O thread. Shared by both create entry points.
unsafe fn create_inner(
    server_uri: *const c_char,
    client_id: *const c_char,
    mqtt_version: u8,
) -> Result<*mut PahoClientInner, c_int> {
    if server_uri.is_null() || client_id.is_null() {
        return Err(MQTTASYNC_NULL_PARAMETER);
    }

    let server_uri_str = match CStr::from_ptr(server_uri).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return Err(MQTTASYNC_BAD_UTF8_STRING),
    };
    let client_id_str = match CStr::from_ptr(client_id).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return Err(MQTTASYNC_BAD_UTF8_STRING),
    };

    let parsed = match uri_parser::parse_server_uri(&server_uri_str) {
        Some(p) => p,
        None => return Err(MQTTASYNC_FAILURE),
    };

    let options = MqttClientOptions::builder()
        .peer(&parsed.addr)
        .client_id(&client_id_str)
        .mqtt_version(mqtt_version)
        .keep_alive(60)
        .clean_start(true)
        .build();

    let (command_tx, command_rx) = crossbeam_channel::unbounded();
    let (message_tx, message_rx) = crossbeam_channel::bounded(1000);

    let shared = Arc::new(SharedState::new(message_tx));
    let shared_clone = shared.clone();

    let io_thread = std::thread::Builder::new()
        .name(format!("paho-async-io-{}", client_id_str))
        .spawn(move || {
            let worker = IoWorker::new(options, command_rx, shared_clone);
            worker.run();
        });

    let io_thread = match io_thread {
        Ok(h) => h,
        Err(_) => return Err(MQTTASYNC_FAILURE),
    };

    let inner = Box::new(PahoClientInner {
        command_tx,
        io_thread: Some(io_thread),
        shared,
        message_rx,
        server_uri: server_uri_str,
        client_id: client_id_str,
        mqtt_version,
    });

    Ok(Box::into_raw(inner))
}

/// Create an async MQTT client.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_create(MQTTAsync* handle, const char* serverURI, const char* clientId,
///                      int persistence_type, void* persistence_context);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_create(
    handle: *mut MQTTAsync,
    server_uri: *const c_char,
    client_id: *const c_char,
    _persistence_type: c_int,
    _persistence_context: *mut c_void,
) -> c_int {
    if handle.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    match create_inner(server_uri, client_id, 4) {
        Ok(ptr) => {
            *handle = ptr as *mut c_void;
            MQTTASYNC_SUCCESS
        }
        Err(rc) => rc,
    }
}

/// Create an async MQTT client with extended options.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_createWithOptions(MQTTAsync* handle, const char* serverURI,
///                                 const char* clientId, int persistence_type,
///                                 void* persistence_context,
///                                 MQTTAsync_createOptions* options);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_createWithOptions(
    handle: *mut MQTTAsync,
    server_uri: *const c_char,
    client_id: *const c_char,
    _persistence_type: c_int,
    _persistence_context: *mut c_void,
    options: *mut MQTTAsync_createOptions,
) -> c_int {
    if handle.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }

    let mqtt_version = if options.is_null() {
        4
    } else {
        let opts = &*options;
        if !opts.validate() {
            return MQTTASYNC_BAD_STRUCTURE;
        }
        match opts.MQTTVersion {
            0 | 4 => 4,
            3 => 3,
            5 => 5,
            _ => 4,
        }
    };

    match create_inner(server_uri, client_id, mqtt_version) {
        Ok(ptr) => {
            *handle = ptr as *mut c_void;
            MQTTASYNC_SUCCESS
        }
        Err(rc) => rc,
    }
}

/// Destroy an async MQTT client and free all resources.
///
/// # Paho C signature
/// ```c
/// void MQTTAsync_destroy(MQTTAsync* handle);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_destroy(handle: *mut MQTTAsync) {
    if handle.is_null() || (*handle).is_null() {
        return;
    }
    let inner = Box::from_raw(*handle as *mut PahoClientInner);
    drop(inner);
    *handle = std::ptr::null_mut();
}
