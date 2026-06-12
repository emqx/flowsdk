// SPDX-License-Identifier: MPL-2.0
//! `MQTTClient_receive`, `MQTTClient_setCallbacks`, `MQTTClient_waitForCompletion`,
//! `MQTTClient_getPendingDeliveryTokens`, `MQTTClient_yield`

use libc::{c_char, c_int, c_void};
use std::time::Duration;

use crate::common::return_codes::*;
use crate::common::structs;
use crate::inner::client_state::PahoClientInner;
use crate::inner::event_dispatch::{
    CallbackState, ConnectionLostCallback, DeliveryCompleteCallback, MessageArrivedCallback,
};

use super::create_destroy::MQTTClient;

/// Receive a message (blocking with timeout).
///
/// # Paho C signature
/// ```c
/// int MQTTClient_receive(MQTTClient handle, char** topicName, int* topicLen,
///                         MQTTClient_message** message, unsigned long timeout);
/// ```
///
/// Returns `MQTTCLIENT_SUCCESS` if a message was received,
/// `MQTTCLIENT_TOPICNAME_TRUNCATED` if no message arrived before timeout (Paho convention).
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_receive(
    handle: MQTTClient,
    topic_name: *mut *mut c_char,
    topic_len: *mut c_int,
    message: *mut *mut structs::MQTTClient_message,
    timeout: libc::c_ulong,
) -> c_int {
    if handle.is_null() || topic_name.is_null() || topic_len.is_null() || message.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);

    let duration = Duration::from_millis(timeout);

    match inner.message_rx.recv_timeout(duration) {
        Ok(msg) => {
            // Allocate topic string
            *topic_name = structs::alloc_c_string(&msg.topic);
            *topic_len = msg.topic.len() as c_int;

            // Allocate message (queued messages carry no v5 properties)
            *message = structs::alloc_paho_message(
                &msg.payload,
                msg.qos,
                msg.retained,
                msg.dup,
                msg.msgid,
                &[],
            );

            MQTTCLIENT_SUCCESS
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            *topic_name = std::ptr::null_mut();
            *topic_len = 0;
            *message = std::ptr::null_mut();
            // Paho returns SUCCESS with null message on timeout
            MQTTCLIENT_SUCCESS
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            *topic_name = std::ptr::null_mut();
            *topic_len = 0;
            *message = std::ptr::null_mut();
            MQTTCLIENT_DISCONNECTED
        }
    }
}

/// Set callback functions for connection lost, message arrived, and delivery complete.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_setCallbacks(MQTTClient handle, void* context,
///                              MQTTClient_connectionLost* cl,
///                              MQTTClient_messageArrived* ma,
///                              MQTTClient_deliveryComplete* dc);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_setCallbacks(
    handle: MQTTClient,
    context: *mut c_void,
    cl: ConnectionLostCallback,
    ma: MessageArrivedCallback,
    dc: DeliveryCompleteCallback,
) -> c_int {
    if handle.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);

    let mut callbacks = inner.shared.callbacks.lock();
    *callbacks = CallbackState {
        context,
        connection_lost: cl,
        message_arrived: ma,
        delivery_complete: dc,
    };

    MQTTCLIENT_SUCCESS
}

/// Wait for a published message to be delivered.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_waitForCompletion(MQTTClient handle,
///                                   MQTTClient_deliveryToken dt,
///                                   unsigned long timeout);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_waitForCompletion(
    handle: MQTTClient,
    dt: c_int,
    timeout: libc::c_ulong,
) -> c_int {
    if handle.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);
    let duration = Duration::from_millis(timeout);

    match inner.shared.token_tracker.wait_for_completion(dt, duration) {
        Some(result) => result.rc,
        None => MQTTCLIENT_FAILURE, // Timeout or unknown token
    }
}

/// Get the list of pending delivery tokens.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_getPendingDeliveryTokens(MQTTClient handle,
///                                          MQTTClient_deliveryToken** tokens);
/// ```
///
/// The returned token array is allocated with `malloc` and terminated with -1.
/// The caller must free it with `MQTTClient_free`.
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_getPendingDeliveryTokens(
    handle: MQTTClient,
    tokens: *mut *mut c_int,
) -> c_int {
    if handle.is_null() || tokens.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);
    let pending = inner.shared.token_tracker.get_pending_tokens();

    if pending.is_empty() {
        *tokens = std::ptr::null_mut();
        return MQTTCLIENT_SUCCESS;
    }

    // Allocate array with space for tokens + sentinel (-1)
    let array_size = (pending.len() + 1) * std::mem::size_of::<c_int>();
    let array = libc::malloc(array_size) as *mut c_int;
    if array.is_null() {
        return MQTTCLIENT_FAILURE;
    }

    for (i, token) in pending.iter().enumerate() {
        *array.add(i) = *token;
    }
    *array.add(pending.len()) = -1; // Sentinel

    *tokens = array;
    MQTTCLIENT_SUCCESS
}

/// Yield processing time to the library.
///
/// In Paho C, this allows the library to process incoming messages
/// when not using callbacks. In our implementation, the I/O thread handles
/// this automatically, so this is a no-op.
///
/// # Paho C signature
/// ```c
/// void MQTTClient_yield(void);
/// ```
#[no_mangle]
pub extern "C" fn MQTTClient_yield() {
    // No-op: the I/O thread runs continuously. Yield is only meaningful in
    // Paho's single-threaded mode where the caller drives I/O.
}
