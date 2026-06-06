// SPDX-License-Identifier: MPL-2.0
//! Routes `MqttEvent` from the engine to Paho C callbacks and completion tracking.

use flowsdk::mqtt_client::client::{
    ConnectionResult, PublishResult, SubscribeResult, UnsubscribeResult,
};
use flowsdk::mqtt_client::engine::{MqttEvent, MqttMessage};
use libc::{c_char, c_int, c_void};
use std::sync::Arc;

use super::client_state::SharedState;
use super::token_tracker::CompletionResult;
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
    shared
        .connected
        .store(true, std::sync::atomic::Ordering::SeqCst);

    // Signal any waiting connect_sync call
    if let Some(tx) = shared.connect_waiter.lock().take() {
        let _ = tx.send(Ok(result));
    }
}

fn handle_disconnected(reason: Option<u8>, shared: &Arc<SharedState>) {
    shared
        .connected
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Invoke connectionLost callback
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
