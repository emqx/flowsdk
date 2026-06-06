// SPDX-License-Identifier: MPL-2.0
//! `MQTTAsync_setCallbacks`, `MQTTAsync_setConnected`, `MQTTAsync_setDisconnected`

use libc::{c_int, c_void};

use crate::common::async_structs::{
    MQTTAsync, MQTTAsync_connected, MQTTAsync_connectionLost, MQTTAsync_deliveryComplete,
    MQTTAsync_disconnected, MQTTAsync_messageArrived,
};
use crate::common::return_codes::*;
use crate::inner::client_state::PahoClientInner;
use crate::inner::event_dispatch::CallbackState;

/// Set the connection-lost, message-arrived, and delivery-complete callbacks.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_setCallbacks(MQTTAsync handle, void* context,
///                            MQTTAsync_connectionLost* cl,
///                            MQTTAsync_messageArrived* ma,
///                            MQTTAsync_deliveryComplete* dc);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_setCallbacks(
    handle: MQTTAsync,
    context: *mut c_void,
    cl: MQTTAsync_connectionLost,
    ma: MQTTAsync_messageArrived,
    dc: MQTTAsync_deliveryComplete,
) -> c_int {
    if handle.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    let inner = &*(handle as *mut PahoClientInner);

    // MQTTAsync_message is layout-identical to MQTTClient_message, so the async
    // callback pointer types are the same as the sync CallbackState fields.
    let mut callbacks = inner.shared.callbacks.lock();
    *callbacks = CallbackState {
        context,
        connection_lost: cl,
        message_arrived: ma,
        delivery_complete: dc,
    };

    MQTTASYNC_SUCCESS
}

/// Set the callback invoked when the client connects or reconnects.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_setConnected(MQTTAsync handle, void* context, MQTTAsync_connected* co);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_setConnected(
    handle: MQTTAsync,
    context: *mut c_void,
    co: MQTTAsync_connected,
) -> c_int {
    if handle.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    let inner = &*(handle as *mut PahoClientInner);
    let mut acb = inner.shared.async_callbacks.lock();
    acb.connected = co;
    acb.connected_context = context;
    MQTTASYNC_SUCCESS
}

/// Set the callback invoked when the client is disconnected by the server.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_setDisconnected(MQTTAsync handle, void* context, MQTTAsync_disconnected* co);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_setDisconnected(
    handle: MQTTAsync,
    context: *mut c_void,
    co: MQTTAsync_disconnected,
) -> c_int {
    if handle.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    let inner = &*(handle as *mut PahoClientInner);
    let mut acb = inner.shared.async_callbacks.lock();
    acb.disconnected = co;
    acb.disconnected_context = context;
    MQTTASYNC_SUCCESS
}
