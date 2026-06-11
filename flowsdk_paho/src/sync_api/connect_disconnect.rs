// SPDX-License-Identifier: MPL-2.0
//! `MQTTClient_connect`, `MQTTClient_disconnect`, `MQTTClient_isConnected`

use libc::c_int;
use std::time::Duration;

use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_serde::mqttv5::willv5::Will;

use crate::common::error_mapping;
use crate::common::return_codes::*;
use crate::common::structs::MQTTClient_connectOptions;
use crate::common::uri_parser;
use crate::inner::client_state::SharedState;
use crate::inner::client_state::{PahoClientInner, PahoCommand};
use crate::inner::io_worker::IoWorker;

use super::create_destroy::MQTTClient;

/// Connect to an MQTT broker (blocking).
///
/// This function blocks until the connection is established or the timeout expires.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_connect(MQTTClient handle, MQTTClient_connectOptions* options);
/// ```
///
/// # Safety
/// - `handle` must be a valid `MQTTClient` from `MQTTClient_create`.
/// - `options` must be a valid pointer to an initialized `MQTTClient_connectOptions`.
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_connect(
    handle: MQTTClient,
    options: *mut MQTTClient_connectOptions,
) -> c_int {
    if handle.is_null() || options.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &mut *(handle as *mut PahoClientInner);
    let opts = &*options;

    // Validate struct_id
    if !opts.validate() {
        return MQTTCLIENT_BAD_STRUCTURE;
    }

    let mqtt_version = opts.effective_mqtt_version();
    let keep_alive = opts.effective_keep_alive();
    let clean_start = opts.effective_clean_start();
    let connect_timeout_secs = opts.effective_connect_timeout_secs();

    // Validate MQTT version
    if mqtt_version != 3 && mqtt_version != 4 && mqtt_version != 5 {
        return MQTTCLIENT_BAD_MQTT_VERSION;
    }

    // Parse the server URI
    let parsed_uri = match uri_parser::parse_server_uri(&inner.server_uri) {
        Some(uri) => uri,
        None => return MQTTCLIENT_FAILURE,
    };

    // Build a TLS connector from any SSL options before tearing down the worker.
    let tls_handle = match crate::inner::transport::tls_handle_from_options(opts.ssl) {
        Ok(h) => h,
        Err(crate::inner::transport::TlsSetupError::NotSupported) => {
            return MQTTCLIENT_SSL_NOT_SUPPORTED
        }
        Err(crate::inner::transport::TlsSetupError::BadStructure) => {
            return MQTTCLIENT_BAD_STRUCTURE
        }
        Err(crate::inner::transport::TlsSetupError::Failed(_)) => return MQTTCLIENT_FAILURE,
    };

    // Read optional fields
    let username = opts.get_username();
    let password = opts.get_password();

    // Parse will options
    let will = if !opts.will.is_null() {
        let will_opts = &*opts.will;
        if !will_opts.validate() {
            return MQTTCLIENT_BAD_STRUCTURE;
        }

        let topic = if will_opts.topicName.is_null() {
            return MQTTCLIENT_0_LEN_WILL_TOPIC;
        } else {
            let s = std::ffi::CStr::from_ptr(will_opts.topicName)
                .to_string_lossy()
                .into_owned();
            if s.is_empty() {
                return MQTTCLIENT_0_LEN_WILL_TOPIC;
            }
            s
        };

        // Get will payload: prefer binary payload (struct_version >= 1), else string
        let payload = if will_opts.struct_version >= 1
            && !will_opts.payload.data.is_null()
            && will_opts.payload.len > 0
        {
            std::slice::from_raw_parts(
                will_opts.payload.data as *const u8,
                will_opts.payload.len as usize,
            )
            .to_vec()
        } else if !will_opts.message.is_null() {
            std::ffi::CStr::from_ptr(will_opts.message)
                .to_bytes()
                .to_vec()
        } else {
            Vec::new()
        };

        let qos = will_opts.qos as u8;
        if qos > 2 {
            return MQTTCLIENT_BAD_QOS;
        }

        Some(Will::new(topic, payload, qos, will_opts.retained != 0))
    } else {
        None
    };

    // Build new MqttClientOptions with the connect parameters.
    // We need to recreate the engine with these options.
    let mut builder = MqttClientOptions::builder()
        .peer(&parsed_uri.addr)
        .client_id(&inner.client_id)
        .mqtt_version(mqtt_version)
        .keep_alive(keep_alive)
        .clean_start(clean_start);

    if let Some(ref u) = username {
        builder = builder.username(u.as_str());
    }
    if let Some(ref p) = password {
        builder = builder.password(p.clone());
    }
    if let Some(w) = will {
        builder = builder.will(w);
    }

    let new_options = builder.build();
    inner.mqtt_version = mqtt_version;

    // We need to recreate the I/O worker with the new options.
    // Shut down the current one and start a new one.
    let _ = inner.command_tx.send(PahoCommand::Shutdown);
    if let Some(handle) = inner.io_thread.take() {
        let _ = handle.join();
    }

    // Create new channels
    let (command_tx, command_rx) = crossbeam_channel::unbounded();
    let (message_tx, message_rx) = crossbeam_channel::bounded(1000);

    // Create new shared state (preserving callbacks from the old one)
    let old_callbacks = {
        let mut cb = inner.shared.callbacks.lock();
        std::mem::take(&mut *cb)
    };
    let shared = std::sync::Arc::new(SharedState::new(message_tx));
    {
        let mut cb = shared.callbacks.lock();
        *cb = old_callbacks;
    }
    *shared.tls_connector.lock() = tls_handle;

    // Set up synchronous connect: create a channel to wait for CONNACK
    let (response_tx, response_rx) = std::sync::mpsc::channel();

    // Store the response sender in shared state before starting the worker
    *shared.connect_waiter.lock() = Some(response_tx.clone());

    let shared_clone = shared.clone();
    let client_id_str = inner.client_id.clone();

    // Spawn new I/O worker
    let io_thread = std::thread::Builder::new()
        .name(format!("paho-io-{}", client_id_str))
        .spawn(move || {
            let worker = IoWorker::new(new_options, command_rx, shared_clone);
            worker.run();
        });

    let io_thread = match io_thread {
        Ok(h) => h,
        Err(_) => return MQTTCLIENT_FAILURE,
    };

    // Update inner state
    inner.command_tx = command_tx;
    inner.io_thread = Some(io_thread);
    inner.shared = shared;
    inner.message_rx = message_rx;

    // Send connect command
    let _ = inner.send_command(PahoCommand::ConnectSync {
        uri: parsed_uri,
        connect_timeout_secs,
        response_tx,
    });

    // Block waiting for the connection result
    let timeout = Duration::from_secs(connect_timeout_secs);
    match response_rx.recv_timeout(timeout) {
        Ok(Ok(result)) => {
            if result.is_success() {
                MQTTCLIENT_SUCCESS
            } else {
                error_mapping::map_connack_reason_code(result.reason_code)
            }
        }
        Ok(Err(e)) => error_mapping::map_error(&e),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => MQTTCLIENT_FAILURE,
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => MQTTCLIENT_FAILURE,
    }
}

/// Disconnect from the MQTT broker (blocking).
///
/// # Paho C signature
/// ```c
/// int MQTTClient_disconnect(MQTTClient handle, int timeout);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_disconnect(handle: MQTTClient, timeout: c_int) -> c_int {
    if handle.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);

    if !inner.is_connected() {
        return MQTTCLIENT_DISCONNECTED;
    }

    // Send disconnect command
    if inner.send_command(PahoCommand::Disconnect).is_err() {
        return MQTTCLIENT_FAILURE;
    }

    // Wait briefly for the disconnect to take effect
    let wait = Duration::from_millis(if timeout <= 0 { 100 } else { timeout as u64 });
    let start = std::time::Instant::now();
    while inner.is_connected() && start.elapsed() < wait {
        std::thread::sleep(Duration::from_millis(10));
    }

    MQTTCLIENT_SUCCESS
}

/// Check if the client is currently connected.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_isConnected(MQTTClient handle);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_isConnected(handle: MQTTClient) -> c_int {
    if handle.is_null() {
        return 0;
    }

    let inner = &*(handle as *mut PahoClientInner);
    inner.is_connected() as c_int
}
