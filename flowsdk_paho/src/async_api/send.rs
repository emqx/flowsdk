// SPDX-License-Identifier: MPL-2.0
//! `MQTTAsync_send`, `MQTTAsync_sendMessage`

use libc::{c_char, c_int, c_void};
use std::ffi::CStr;
use std::time::Duration;

use flowsdk::mqtt_client::commands::PublishCommand;

use crate::common::async_structs::{MQTTAsync, MQTTAsync_message, MQTTAsync_responseOptions};
use crate::common::return_codes::*;
use crate::inner::client_state::{AsyncResponse, PahoClientInner, PahoCommand};

/// Maximum time to wait for the I/O thread to assign a token. This is a local
/// round-trip (not a broker ACK), so it completes in microseconds.
const TOKEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Publish a message asynchronously.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_send(MQTTAsync handle, const char* destinationName, int payloadlen,
///                    const void* payload, int qos, int retained,
///                    MQTTAsync_responseOptions* response);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_send(
    handle: MQTTAsync,
    destination_name: *const c_char,
    payloadlen: c_int,
    payload: *const c_void,
    qos: c_int,
    retained: c_int,
    response: *mut MQTTAsync_responseOptions,
) -> c_int {
    if handle.is_null() || destination_name.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    if qos < 0 || qos > 2 {
        return MQTTASYNC_BAD_QOS;
    }

    let inner = &*(handle as *mut PahoClientInner);
    if !inner.is_connected() {
        return MQTTASYNC_DISCONNECTED;
    }

    let topic = match CStr::from_ptr(destination_name).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTASYNC_BAD_UTF8_STRING,
    };

    let payload_bytes = if payloadlen > 0 && !payload.is_null() {
        std::slice::from_raw_parts(payload as *const u8, payloadlen as usize).to_vec()
    } else {
        Vec::new()
    };

    let cmd = match PublishCommand::builder()
        .topic(topic)
        .payload(payload_bytes)
        .qos(qos as u8)
        .retain(retained != 0)
        .build()
    {
        Ok(cmd) => cmd,
        Err(_) => return MQTTASYNC_FAILURE,
    };

    let async_response = build_response(response);

    let (token_tx, token_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::PublishAsync {
            command: cmd,
            response: async_response,
            token_tx,
        })
        .is_err()
    {
        return MQTTASYNC_FAILURE;
    }

    // Retrieve the assigned token and write it back into the caller's struct.
    let token = token_rx.recv_timeout(TOKEN_TIMEOUT).unwrap_or(0);
    if !response.is_null() {
        (*response).token = token;
    }

    MQTTASYNC_SUCCESS
}

/// Publish a message described by a `MQTTAsync_message` struct.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_sendMessage(MQTTAsync handle, const char* destinationName,
///                           const MQTTAsync_message* message,
///                           MQTTAsync_responseOptions* response);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_sendMessage(
    handle: MQTTAsync,
    destination_name: *const c_char,
    message: *mut MQTTAsync_message,
    response: *mut MQTTAsync_responseOptions,
) -> c_int {
    if handle.is_null() || destination_name.is_null() || message.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    let msg = &*message;
    if !msg.validate() {
        return MQTTASYNC_BAD_STRUCTURE;
    }
    MQTTAsync_send(
        handle,
        destination_name,
        msg.payloadlen,
        msg.payload,
        msg.qos,
        msg.retained,
        response,
    )
}

/// Construct an `AsyncResponse` from an optional `MQTTAsync_responseOptions` pointer.
pub(crate) unsafe fn build_response(response: *mut MQTTAsync_responseOptions) -> AsyncResponse {
    if response.is_null() {
        AsyncResponse {
            context: std::ptr::null_mut(),
            on_success: None,
            on_failure: None,
            on_success5: None,
            on_failure5: None,
            token: 0,
        }
    } else {
        let opts = &*response;
        AsyncResponse {
            context: opts.context,
            on_success: opts.onSuccess,
            on_failure: opts.onFailure,
            on_success5: opts.onSuccess5,
            on_failure5: opts.onFailure5,
            token: 0,
        }
    }
}
