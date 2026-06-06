// SPDX-License-Identifier: MPL-2.0
//! Routes `MqttEvent` from the engine to Paho C callbacks and completion tracking.

use flowsdk::mqtt_client::client::{
    ConnectionResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::engine::{MqttEvent, MqttMessage};
use libc::{c_char, c_int, c_void};
use std::sync::Arc;

use super::client_state::{AsyncResponse, SharedState};
use super::token_tracker::CompletionResult;
use crate::common::async_structs::{
    MQTTAsync_failureData, MQTTAsync_successData, MQTTAsync_successData_alt,
    MQTTAsync_successData_connect, MQTTAsync_successData_pub,
};
use crate::common::return_codes::*;
use crate::common::structs;

// ─── Paho C callback type definitions ────────────────────────────────────

/// `typedef void MQTTClient_connectionLost(void* context, char* cause)`
pub type ConnectionLostCallback =
    Option<unsafe extern "C" fn(context: *mut c_void, cause: *mut c_char)>;

/// `typedef int MQTTClient_messageArrived(void* context, char* topicName, int topicLen, MQTTClient_message* message)`
pub type MessageArrivedCallback = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        topic_name: *mut c_char,
        topic_len: c_int,
        message: *mut structs::MQTTClient_message,
    ) -> c_int,
>;

/// `typedef void MQTTClient_deliveryComplete(void* context, MQTTClient_deliveryToken dt)`
pub type DeliveryCompleteCallback = Option<unsafe extern "C" fn(context: *mut c_void, dt: c_int)>;

/// Stored callback state.
#[derive(Default)]
pub struct CallbackState {
    pub context: *mut c_void,
    pub connection_lost: ConnectionLostCallback,
    pub message_arrived: MessageArrivedCallback,
    pub delivery_complete: DeliveryCompleteCallback,
}

// SAFETY: The context pointer and callbacks are provided by the C caller
// and are only invoked from the I/O thread. The C caller is responsible
// for ensuring the context and function pointers are valid.
unsafe impl Send for CallbackState {}
unsafe impl Sync for CallbackState {}

/// Dispatch a batch of `MqttEvent`s to the shared state (callbacks, tokens, message queue).
///
/// Called from the I/O thread after `engine.handle_incoming()` or `engine.handle_tick()`.
pub fn dispatch_events(events: Vec<MqttEvent>, shared: &Arc<SharedState>) {
    for event in events {
        dispatch_event(event, shared);
    }
}

fn dispatch_event(event: MqttEvent, shared: &Arc<SharedState>) {
    match event {
        MqttEvent::Connected(result) => handle_connected(result, shared),
        MqttEvent::Disconnected(reason) => handle_disconnected(reason, shared),
        MqttEvent::Published(result) => handle_published(result, shared),
        MqttEvent::Subscribed(result) => handle_subscribed(result, shared),
        MqttEvent::Unsubscribed(result) => handle_unsubscribed(result, shared),
        MqttEvent::MessageReceived(msg) => handle_message_received(msg, shared),
        MqttEvent::PingResponse(_) => { /* No Paho callback for pings */ }
        MqttEvent::Error(_err) => { /* Errors are handled via return codes, not callbacks */ }
        MqttEvent::ReconnectNeeded => {
            // Signal the I/O worker to reconnect — handled in io_worker.rs
            shared
                .reconnect_needed
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        MqttEvent::ReconnectScheduled { .. } => { /* Informational only */ }
    }
}

fn handle_connected(result: ConnectionResult, shared: &Arc<SharedState>) {
    let success = result.is_success();
    let session_present = result.session_present;

    if success {
        shared
            .connected
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    // Async: fire the connect response callback.
    if let Some(resp) = shared.async_connect_response.lock().take() {
        if success {
            fire_success_connect(&resp, session_present);
        } else {
            fire_failure(&resp, MQTTCLIENT_FAILURE, "Connection refused");
        }
    }

    // Async: fire the persistent `connected` callback on a successful connect.
    if success {
        let cb = shared.async_callbacks.lock();
        if let Some(connected) = cb.connected {
            let ctx = cb.connected_context;
            drop(cb);
            let cause = unsafe { structs::alloc_c_string("connect") };
            unsafe { connected(ctx, cause) };
            unsafe { libc::free(cause as *mut c_void) };
        }
    }

    // Signal any waiting sync connect call.
    if let Some(tx) = shared.connect_waiter.lock().take() {
        let _ = tx.send(Ok(result));
    }
}

fn handle_disconnected(reason: Option<u8>, shared: &Arc<SharedState>) {
    shared
        .connected
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Async: fail any outstanding operations and connect attempt.
    fail_pending_async(shared, "Disconnected by server");

    // Invoke connectionLost callback (shared by sync + async cl callbacks)
    let callbacks = shared.callbacks.lock();
    if let Some(cb) = callbacks.connection_lost {
        let cause = reason
            .map(|r| format!("Disconnect reason code: {}", r))
            .unwrap_or_else(|| "Connection lost".to_string());

        let cause_cstr = unsafe { crate::common::structs::alloc_c_string(&cause) };
        let ctx = callbacks.context;
        drop(callbacks); // Release lock before calling into C code

        unsafe { cb(ctx, cause_cstr) };
        // Note: Paho's contract says the library frees the cause string
        unsafe { libc::free(cause_cstr as *mut c_void) };
    } else {
        drop(callbacks);
    }

    // Async: fire the persistent `disconnected` (v5) callback if set.
    let acb = shared.async_callbacks.lock();
    if let Some(disconnected) = acb.disconnected {
        let ctx = acb.disconnected_context;
        drop(acb);
        unsafe { disconnected(ctx, std::ptr::null_mut(), reason.unwrap_or(0) as c_int) };
    }
}

/// Fire `onFailure` for every outstanding async operation (connect + ops).
fn fail_pending_async(shared: &Arc<SharedState>, reason: &str) {
    if let Some(resp) = shared.async_connect_response.lock().take() {
        fire_failure(&resp, MQTTCLIENT_FAILURE, reason);
    }
    let pending: Vec<AsyncResponse> =
        shared.async_responses.lock().drain().map(|(_, r)| r).collect();
    for resp in pending {
        fire_failure(&resp, MQTTCLIENT_FAILURE, reason);
    }
}

fn handle_published(result: PublishResult, shared: &Arc<SharedState>) {
    if let Some(pid) = result.packet_id {
        let token = pid as i32;

        // Mark token as completed
        shared.token_tracker.mark_completed(CompletionResult {
            token,
            success: true,
            rc: MQTTCLIENT_SUCCESS,
        });

        // Invoke deliveryComplete callback
        let callbacks = shared.callbacks.lock();
        if let Some(cb) = callbacks.delivery_complete {
            let ctx = callbacks.context;
            drop(callbacks);
            unsafe { cb(ctx, token) };
        }

        // Async: fire the per-token onSuccess for this publish.
        if let Some(resp) = shared.async_responses.lock().remove(&token) {
            fire_success_publish(&resp, "");
        }
    }
}

fn handle_subscribed(result: SubscribeResult, shared: &Arc<SharedState>) {
    let token = result.packet_id as i32;
    shared.token_tracker.mark_completed(CompletionResult {
        token,
        success: true,
        rc: MQTTCLIENT_SUCCESS,
    });

    // Signal any MQTTClient_subscribe caller waiting for SUBACK
    if let Some(tx) = shared
        .subscribe_waiters
        .lock()
        .remove(&(result.packet_id as u16))
    {
        let _ = tx.send(Ok(()));
    }

    // Async: fire the per-token onSuccess/onFailure for this subscribe.
    if let Some(resp) = shared.async_responses.lock().remove(&token) {
        if result.is_success() {
            let granted_qos = result.reason_codes.first().copied().unwrap_or(0) as c_int;
            fire_success_subscribe(&resp, granted_qos);
        } else {
            fire_failure(&resp, MQTTCLIENT_FAILURE, "Subscription rejected");
        }
    }
}

fn handle_unsubscribed(result: UnsubscribeResult, shared: &Arc<SharedState>) {
    let token = result.packet_id as i32;
    shared.token_tracker.mark_completed(CompletionResult {
        token,
        success: true,
        rc: MQTTCLIENT_SUCCESS,
    });

    // Signal any MQTTClient_unsubscribe caller waiting for UNSUBACK
    if let Some(tx) = shared
        .unsubscribe_waiters
        .lock()
        .remove(&(result.packet_id as u16))
    {
        let _ = tx.send(Ok(()));
    }

    // Async: fire the per-token onSuccess for this unsubscribe.
    if let Some(resp) = shared.async_responses.lock().remove(&token) {
        fire_success_simple(&resp);
    }
}

fn handle_message_received(msg: MqttMessage, shared: &Arc<SharedState>) {
    let callbacks = shared.callbacks.lock();

    if let Some(cb) = callbacks.message_arrived {
        // Allocate Paho-compatible message and topic
        let paho_msg = unsafe {
            structs::alloc_paho_message(
                &msg.payload,
                msg.qos as i32,
                msg.retain,
                msg.dup,
                msg.packet_id.unwrap_or(0) as i32,
            )
        };

        let topic_cstr = unsafe { structs::alloc_c_string(&msg.topic_name) };
        let topic_len = msg.topic_name.len() as c_int;
        let ctx = callbacks.context;
        drop(callbacks); // Release lock before C callback

        let handled = unsafe { cb(ctx, topic_cstr, topic_len, paho_msg) };

        if handled == 0 {
            // Callback declined the message — queue it for MQTTClient_receive.
            // The callback did not free; we own the memory, so free it here
            // after extracting the payload into the queue.
            // (Re-delivery via MQTTClient_receive is a rare path; for now drop it.)
            unsafe {
                libc::free(topic_cstr as *mut c_void);
                let mut msg_ptr = paho_msg;
                structs::free_paho_message(&mut msg_ptr);
            }
        }
        // handled == 1: callback consumed the message and called MQTTClient_freeMessage
        // + MQTTClient_free itself. Do NOT free here — that would be a double-free.
    } else {
        // No callback set → queue for MQTTClient_receive
        let _ = shared.message_tx.try_send(ReceivedMessage {
            topic: msg.topic_name.clone(),
            payload: msg.payload.clone(),
            qos: msg.qos as i32,
            retained: msg.retain,
            dup: msg.dup,
            msgid: msg.packet_id.unwrap_or(0) as i32,
        });
    }
}

/// A received MQTT message stored in the message queue for `MQTTClient_receive`.
#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: i32,
    pub retained: bool,
    pub dup: bool,
    pub msgid: i32,
}

// ─── Async callback firing helpers ───────────────────────────────────────

/// Invoke `onFailure` for an async operation. `message` is copied into a
/// freshly-allocated C string that is freed once the callback returns.
pub fn fire_failure(resp: &AsyncResponse, code: c_int, message: &str) {
    if let Some(on_failure) = resp.on_failure {
        let msg_cstr = unsafe { structs::alloc_c_string(message) };
        let mut data = MQTTAsync_failureData {
            token: resp.token,
            code,
            message: msg_cstr as *const c_char,
        };
        unsafe { on_failure(resp.context, &mut data) };
        unsafe { libc::free(msg_cstr as *mut c_void) };
    }
}

/// Invoke `onSuccess` with no operation-specific data (e.g. disconnect, unsubscribe).
pub fn fire_success_simple(resp: &AsyncResponse) {
    if let Some(on_success) = resp.on_success {
        let mut data = MQTTAsync_successData {
            token: resp.token,
            alt: unsafe { std::mem::zeroed() },
        };
        unsafe { on_success(resp.context, &mut data) };
    }
}

/// Invoke `onSuccess` for a completed publish.
pub fn fire_success_publish(resp: &AsyncResponse, destination: &str) {
    if let Some(on_success) = resp.on_success {
        let dest_cstr = if destination.is_empty() {
            std::ptr::null_mut()
        } else {
            unsafe { structs::alloc_c_string(destination) }
        };
        let mut data = MQTTAsync_successData {
            token: resp.token,
            alt: MQTTAsync_successData_alt {
                pub_: MQTTAsync_successData_pub {
                    message: unsafe { std::mem::zeroed() },
                    destinationName: dest_cstr,
                },
            },
        };
        unsafe { on_success(resp.context, &mut data) };
        if !dest_cstr.is_null() {
            unsafe { libc::free(dest_cstr as *mut c_void) };
        }
    }
}

/// Invoke `onSuccess` for a completed subscribe (granted QoS in `alt.qos`).
pub fn fire_success_subscribe(resp: &AsyncResponse, granted_qos: c_int) {
    if let Some(on_success) = resp.on_success {
        let mut data = MQTTAsync_successData {
            token: resp.token,
            alt: MQTTAsync_successData_alt { qos: granted_qos },
        };
        unsafe { on_success(resp.context, &mut data) };
    }
}

/// Invoke `onSuccess` for a completed connect.
pub fn fire_success_connect(resp: &AsyncResponse, session_present: bool) {
    if let Some(on_success) = resp.on_success {
        let mut data = MQTTAsync_successData {
            token: resp.token,
            alt: MQTTAsync_successData_alt {
                connect: MQTTAsync_successData_connect {
                    serverURI: std::ptr::null_mut(),
                    MQTTVersion: 0,
                    sessionPresent: session_present as c_int,
                },
            },
        };
        unsafe { on_success(resp.context, &mut data) };
    }
}
