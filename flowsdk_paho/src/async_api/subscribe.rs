// SPDX-License-Identifier: MPL-2.0
//! `MQTTAsync_subscribe`, `MQTTAsync_subscribeMany`, `MQTTAsync_unsubscribe`,
//! `MQTTAsync_unsubscribeMany`

use libc::{c_char, c_int};
use std::ffi::CStr;
use std::time::Duration;

use flowsdk::mqtt_client::commands::{
    SubscribeCommand, SubscribeCommandBuilder, UnsubscribeCommand,
};

use crate::common::async_structs::{MQTTAsync, MQTTAsync_responseOptions};
use crate::common::properties;
use crate::common::return_codes::*;
use crate::inner::client_state::{PahoClientInner, PahoCommand};

use super::send::build_response;

const TOKEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve the per-topic subscribe options (noLocal, retainAsPublished,
/// retainHandling) for topic `index` from a `MQTTAsync_responseOptions`.
///
/// If a `subscribeOptionsList` is present it is indexed per-topic; otherwise the
/// single `subscribeOptions` applies to every topic.
unsafe fn subscribe_options(
    response: *const MQTTAsync_responseOptions,
    index: usize,
) -> (bool, bool, u8) {
    if response.is_null() {
        return (false, false, 0);
    }
    let opts = &*response;
    let so =
        if !opts.subscribeOptionsList.is_null() && (index as c_int) < opts.subscribeOptionsCount {
            &*opts.subscribeOptionsList.add(index)
        } else {
            &opts.subscribeOptions
        };
    (
        so.noLocal != 0,
        so.retainAsPublished != 0,
        so.retainHandling,
    )
}

/// Fold any MQTT v5 properties from a `MQTTAsync_responseOptions` into the builder.
unsafe fn apply_response_properties(
    mut builder: SubscribeCommandBuilder,
    response: *const MQTTAsync_responseOptions,
) -> SubscribeCommandBuilder {
    if !response.is_null() {
        for prop in properties::props_from_c(&(*response).properties) {
            builder = builder.add_property(prop);
        }
    }
    builder
}

/// Subscribe to a single topic asynchronously.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_subscribe(MQTTAsync handle, const char* topic, int qos,
///                         MQTTAsync_responseOptions* response);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_subscribe(
    handle: MQTTAsync,
    topic: *const c_char,
    qos: c_int,
    response: *mut MQTTAsync_responseOptions,
) -> c_int {
    if handle.is_null() || topic.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    if !(0..=2).contains(&qos) {
        return MQTTASYNC_BAD_QOS;
    }

    let inner = &*(handle as *mut PahoClientInner);
    if !inner.is_connected() {
        return MQTTASYNC_DISCONNECTED;
    }

    let topic_str = match CStr::from_ptr(topic).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTASYNC_BAD_UTF8_STRING,
    };

    let (no_local, rap, rh) = subscribe_options(response, 0);
    let mut builder = SubscribeCommand::builder()
        .add_topic_with_options(&topic_str, qos as u8, no_local, rap, rh);
    builder = apply_response_properties(builder, response);
    let cmd = match builder.build() {
        Ok(cmd) => cmd,
        Err(_) => return MQTTASYNC_FAILURE,
    };

    let async_response = build_response(response);
    let (token_tx, token_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::SubscribeAsync {
            command: cmd,
            response: async_response,
            token_tx,
        })
        .is_err()
    {
        return MQTTASYNC_FAILURE;
    }

    let token = token_rx.recv_timeout(TOKEN_TIMEOUT).unwrap_or(0);
    if !response.is_null() {
        (*response).token = token;
    }
    MQTTASYNC_SUCCESS
}

/// Subscribe to multiple topics asynchronously.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_subscribeMany(MQTTAsync handle, int count, char* const* topic,
///                             int* qos, MQTTAsync_responseOptions* response);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_subscribeMany(
    handle: MQTTAsync,
    count: c_int,
    topic: *const *const c_char,
    qos: *mut c_int,
    response: *mut MQTTAsync_responseOptions,
) -> c_int {
    if handle.is_null() || topic.is_null() || qos.is_null() || count <= 0 {
        return MQTTASYNC_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);
    if !inner.is_connected() {
        return MQTTASYNC_DISCONNECTED;
    }

    let mut builder = SubscribeCommand::builder();
    for i in 0..count as usize {
        let t = *topic.add(i);
        if t.is_null() {
            return MQTTASYNC_NULL_PARAMETER;
        }
        let topic_str = match CStr::from_ptr(t).to_str() {
            Ok(s) => s,
            Err(_) => return MQTTASYNC_BAD_UTF8_STRING,
        };
        let q = *qos.add(i);
        if !(0..=2).contains(&q) {
            return MQTTASYNC_BAD_QOS;
        }
        let (no_local, rap, rh) = subscribe_options(response, i);
        builder = builder.add_topic_with_options(topic_str, q as u8, no_local, rap, rh);
    }
    builder = apply_response_properties(builder, response);

    let cmd = match builder.build() {
        Ok(cmd) => cmd,
        Err(_) => return MQTTASYNC_FAILURE,
    };

    let async_response = build_response(response);
    let (token_tx, token_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::SubscribeAsync {
            command: cmd,
            response: async_response,
            token_tx,
        })
        .is_err()
    {
        return MQTTASYNC_FAILURE;
    }

    let token = token_rx.recv_timeout(TOKEN_TIMEOUT).unwrap_or(0);
    if !response.is_null() {
        (*response).token = token;
    }
    MQTTASYNC_SUCCESS
}

/// Unsubscribe from a single topic asynchronously.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_unsubscribe(MQTTAsync handle, const char* topic,
///                           MQTTAsync_responseOptions* response);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_unsubscribe(
    handle: MQTTAsync,
    topic: *const c_char,
    response: *mut MQTTAsync_responseOptions,
) -> c_int {
    if handle.is_null() || topic.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);
    if !inner.is_connected() {
        return MQTTASYNC_DISCONNECTED;
    }

    let topic_str = match CStr::from_ptr(topic).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTASYNC_BAD_UTF8_STRING,
    };

    let cmd = UnsubscribeCommand::from_topics(vec![topic_str]);
    let async_response = build_response(response);
    let (token_tx, token_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::UnsubscribeAsync {
            command: cmd,
            response: async_response,
            token_tx,
        })
        .is_err()
    {
        return MQTTASYNC_FAILURE;
    }

    let token = token_rx.recv_timeout(TOKEN_TIMEOUT).unwrap_or(0);
    if !response.is_null() {
        (*response).token = token;
    }
    MQTTASYNC_SUCCESS
}

/// Unsubscribe from multiple topics asynchronously.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_unsubscribeMany(MQTTAsync handle, int count, char* const* topic,
///                               MQTTAsync_responseOptions* response);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_unsubscribeMany(
    handle: MQTTAsync,
    count: c_int,
    topic: *const *const c_char,
    response: *mut MQTTAsync_responseOptions,
) -> c_int {
    if handle.is_null() || topic.is_null() || count <= 0 {
        return MQTTASYNC_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);
    if !inner.is_connected() {
        return MQTTASYNC_DISCONNECTED;
    }

    let mut topics = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let t = *topic.add(i);
        if t.is_null() {
            return MQTTASYNC_NULL_PARAMETER;
        }
        match CStr::from_ptr(t).to_str() {
            Ok(s) => topics.push(s.to_string()),
            Err(_) => return MQTTASYNC_BAD_UTF8_STRING,
        }
    }

    let cmd = UnsubscribeCommand::from_topics(topics);
    let async_response = build_response(response);
    let (token_tx, token_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::UnsubscribeAsync {
            command: cmd,
            response: async_response,
            token_tx,
        })
        .is_err()
    {
        return MQTTASYNC_FAILURE;
    }

    let token = token_rx.recv_timeout(TOKEN_TIMEOUT).unwrap_or(0);
    if !response.is_null() {
        (*response).token = token;
    }
    MQTTASYNC_SUCCESS
}
