// SPDX-License-Identifier: MPL-2.0
//! `MQTTAsync_connect`, `MQTTAsync_disconnect`, `MQTTAsync_isConnected`

use libc::c_int;
use std::sync::Arc;

use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_serde::mqttv5::willv5::Will;

use crate::common::async_structs::{
    MQTTAsync, MQTTAsync_connectOptions, MQTTAsync_disconnectOptions,
};
use crate::common::return_codes::*;
use crate::common::uri_parser;
use crate::inner::client_state::{
    AsyncResponse, PahoClientInner, PahoCommand, SharedState,
};
use crate::inner::io_worker::IoWorker;

/// Build an `AsyncResponse` from a set of callback pointers + context.
fn make_response(
    context: *mut libc::c_void,
    on_success: crate::common::async_structs::MQTTAsync_onSuccess,
    on_failure: crate::common::async_structs::MQTTAsync_onFailure,
    on_success5: crate::common::async_structs::MQTTAsync_onSuccess5,
    on_failure5: crate::common::async_structs::MQTTAsync_onFailure5,
) -> AsyncResponse {
    AsyncResponse {
        context,
        on_success,
        on_failure,
        on_success5,
        on_failure5,
        token: 0,
    }
}

/// Connect to an MQTT broker asynchronously. Returns immediately; the result is
/// delivered via `options->onSuccess` / `options->onFailure`.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_connect(MQTTAsync handle, const MQTTAsync_connectOptions* options);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_connect(
    handle: MQTTAsync,
    options: *mut MQTTAsync_connectOptions,
) -> c_int {
    if handle.is_null() || options.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }

    let inner = &mut *(handle as *mut PahoClientInner);
    let opts = &*options;

    if !opts.validate() {
        return MQTTASYNC_BAD_STRUCTURE;
    }

    let mqtt_version = opts.effective_mqtt_version();
    let keep_alive = opts.effective_keep_alive();
    let clean_start = opts.effective_clean_start();
    let connect_timeout_secs = opts.effective_connect_timeout_secs();

    if mqtt_version != 3 && mqtt_version != 4 && mqtt_version != 5 {
        return MQTTASYNC_BAD_MQTT_VERSION;
    }

    let parsed_uri = match uri_parser::parse_server_uri(&inner.server_uri) {
        Some(uri) => uri,
        None => return MQTTASYNC_FAILURE,
    };

    // Username / password
    let username = if opts.username.is_null() {
        None
    } else {
        Some(
            std::ffi::CStr::from_ptr(opts.username)
                .to_string_lossy()
                .into_owned(),
        )
    };
    let password = if !opts.binarypwd.data.is_null() && opts.binarypwd.len > 0 {
        Some(
            std::slice::from_raw_parts(opts.binarypwd.data as *const u8, opts.binarypwd.len as usize)
                .to_vec(),
        )
    } else if !opts.password.is_null() {
        Some(std::ffi::CStr::from_ptr(opts.password).to_bytes().to_vec())
    } else {
        None
    };

    // Will
    let will = if opts.will.is_null() {
        None
    } else {
        let will_opts = &*opts.will;
        if !will_opts.validate() {
            return MQTTASYNC_BAD_STRUCTURE;
        }
        if will_opts.topicName.is_null() {
            return MQTTASYNC_0_LEN_WILL_TOPIC;
        }
        let topic = std::ffi::CStr::from_ptr(will_opts.topicName)
            .to_string_lossy()
            .into_owned();
        if topic.is_empty() {
            return MQTTASYNC_0_LEN_WILL_TOPIC;
        }
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
            std::ffi::CStr::from_ptr(will_opts.message).to_bytes().to_vec()
        } else {
            Vec::new()
        };
        let qos = will_opts.qos as u8;
        if qos > 2 {
            return MQTTASYNC_BAD_QOS;
        }
        Some(Will::new(topic, payload, qos, will_opts.retained != 0))
    };

    // Build new engine options
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

    // Shut down the existing I/O worker and start a fresh one.
    let _ = inner.command_tx.send(PahoCommand::Shutdown);
    if let Some(h) = inner.io_thread.take() {
        let _ = h.join();
    }

    let (command_tx, command_rx) = crossbeam_channel::unbounded();
    let (message_tx, message_rx) = crossbeam_channel::bounded(1000);

    // Preserve the user's callbacks across the engine rebuild.
    let old_callbacks = {
        let mut cb = inner.shared.callbacks.lock();
        std::mem::take(&mut *cb)
    };
    let old_async_callbacks = {
        let mut cb = inner.shared.async_callbacks.lock();
        std::mem::take(&mut *cb)
    };

    let shared = Arc::new(SharedState::new(message_tx));
    *shared.callbacks.lock() = old_callbacks;
    *shared.async_callbacks.lock() = old_async_callbacks;

    let shared_clone = shared.clone();
    let client_id_str = inner.client_id.clone();
    let io_thread = std::thread::Builder::new()
        .name(format!("paho-async-io-{}", client_id_str))
        .spawn(move || {
            let worker = IoWorker::new(new_options, command_rx, shared_clone);
            worker.run();
        });
    let io_thread = match io_thread {
        Ok(h) => h,
        Err(_) => return MQTTASYNC_FAILURE,
    };

    inner.command_tx = command_tx;
    inner.io_thread = Some(io_thread);
    inner.shared = shared;
    inner.message_rx = message_rx;

    let response = make_response(
        opts.context,
        opts.onSuccess,
        opts.onFailure,
        opts.onSuccess5,
        opts.onFailure5,
    );

    if inner
        .send_command(PahoCommand::ConnectAsync {
            uri: parsed_uri,
            connect_timeout_secs,
            response,
        })
        .is_err()
    {
        return MQTTASYNC_FAILURE;
    }

    MQTTASYNC_SUCCESS
}

/// Disconnect asynchronously.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_disconnect(MQTTAsync handle, const MQTTAsync_disconnectOptions* options);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_disconnect(
    handle: MQTTAsync,
    options: *mut MQTTAsync_disconnectOptions,
) -> c_int {
    if handle.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);

    if !inner.is_connected() {
        return MQTTASYNC_DISCONNECTED;
    }

    let response = if options.is_null() {
        make_response(std::ptr::null_mut(), None, None, None, None)
    } else {
        let opts = &*options;
        if !opts.validate() {
            return MQTTASYNC_BAD_STRUCTURE;
        }
        make_response(
            opts.context,
            opts.onSuccess,
            opts.onFailure,
            opts.onSuccess5,
            opts.onFailure5,
        )
    };

    if inner
        .send_command(PahoCommand::DisconnectAsync { response })
        .is_err()
    {
        return MQTTASYNC_FAILURE;
    }

    MQTTASYNC_SUCCESS
}

/// Check whether the async client is currently connected.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_isConnected(MQTTAsync handle);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_isConnected(handle: MQTTAsync) -> c_int {
    if handle.is_null() {
        return 0;
    }
    let inner = &*(handle as *mut PahoClientInner);
    inner.is_connected() as c_int
}
