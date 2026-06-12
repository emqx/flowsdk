// SPDX-License-Identifier: MPL-2.0
//! `MQTTClient_subscribe`, `MQTTClient_subscribeMany`, `MQTTClient_unsubscribe`, `MQTTClient_unsubscribeMany`

use libc::{c_char, c_int};
use std::ffi::CStr;
use std::time::Duration;

use flowsdk::mqtt_client::commands::{SubscribeCommand, UnsubscribeCommand};

use crate::common::return_codes::*;
use crate::inner::client_state::{PahoClientInner, PahoCommand};

const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Subscribe to a single topic (blocking until SUBACK).
///
/// # Paho C signature
/// ```c
/// int MQTTClient_subscribe(MQTTClient handle, const char* topic, int qos);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_subscribe(
    handle: *mut libc::c_void,
    topic: *const c_char,
    qos: c_int,
) -> c_int {
    if handle.is_null() || topic.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    if !(0..=2).contains(&qos) {
        return MQTTCLIENT_BAD_QOS;
    }

    let inner = &*(handle as *mut PahoClientInner);

    if !inner.is_connected() {
        return MQTTCLIENT_DISCONNECTED;
    }

    let topic_str = match CStr::from_ptr(topic).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTCLIENT_BAD_UTF8_STRING,
    };

    let cmd = match SubscribeCommand::builder()
        .add_topic(&topic_str, qos as u8)
        .build()
    {
        Ok(cmd) => cmd,
        Err(_) => return MQTTCLIENT_FAILURE,
    };

    let (response_tx, response_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::SubscribeSync {
            command: cmd,
            response_tx,
        })
        .is_err()
    {
        return MQTTCLIENT_FAILURE;
    }

    match response_rx.recv_timeout(SUBSCRIBE_TIMEOUT) {
        Ok(Ok(())) => MQTTCLIENT_SUCCESS,
        Ok(Err(_)) => MQTTCLIENT_FAILURE,
        Err(_) => MQTTCLIENT_FAILURE,
    }
}

/// Subscribe to multiple topics (blocking until SUBACK).
///
/// # Paho C signature
/// ```c
/// int MQTTClient_subscribeMany(MQTTClient handle, int count,
///                               char* const* topic, int* qos);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_subscribeMany(
    handle: *mut libc::c_void,
    count: c_int,
    topic: *const *const c_char,
    qos: *mut c_int,
) -> c_int {
    if handle.is_null() || topic.is_null() || qos.is_null() || count <= 0 {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);

    if !inner.is_connected() {
        return MQTTCLIENT_DISCONNECTED;
    }

    let mut builder = SubscribeCommand::builder();

    for i in 0..count as usize {
        let t = *topic.add(i);
        if t.is_null() {
            return MQTTCLIENT_NULL_PARAMETER;
        }
        let topic_str = match CStr::from_ptr(t).to_str() {
            Ok(s) => s,
            Err(_) => return MQTTCLIENT_BAD_UTF8_STRING,
        };
        let q = *qos.add(i);
        if !(0..=2).contains(&q) {
            return MQTTCLIENT_BAD_QOS;
        }
        builder = builder.add_topic(topic_str, q as u8);
    }

    let cmd = match builder.build() {
        Ok(cmd) => cmd,
        Err(_) => return MQTTCLIENT_FAILURE,
    };

    let (response_tx, response_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::SubscribeSync {
            command: cmd,
            response_tx,
        })
        .is_err()
    {
        return MQTTCLIENT_FAILURE;
    }

    match response_rx.recv_timeout(SUBSCRIBE_TIMEOUT) {
        Ok(Ok(())) => MQTTCLIENT_SUCCESS,
        Ok(Err(_)) => MQTTCLIENT_FAILURE,
        Err(_) => MQTTCLIENT_FAILURE,
    }
}

/// Unsubscribe from a single topic (blocking until UNSUBACK).
///
/// # Paho C signature
/// ```c
/// int MQTTClient_unsubscribe(MQTTClient handle, const char* topic);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_unsubscribe(
    handle: *mut libc::c_void,
    topic: *const c_char,
) -> c_int {
    if handle.is_null() || topic.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);

    if !inner.is_connected() {
        return MQTTCLIENT_DISCONNECTED;
    }

    let topic_str = match CStr::from_ptr(topic).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTCLIENT_BAD_UTF8_STRING,
    };

    let cmd = UnsubscribeCommand::from_topics(vec![topic_str]);

    let (response_tx, response_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::UnsubscribeSync {
            command: cmd,
            response_tx,
        })
        .is_err()
    {
        return MQTTCLIENT_FAILURE;
    }

    match response_rx.recv_timeout(SUBSCRIBE_TIMEOUT) {
        Ok(Ok(())) => MQTTCLIENT_SUCCESS,
        Ok(Err(_)) => MQTTCLIENT_FAILURE,
        Err(_) => MQTTCLIENT_FAILURE,
    }
}

/// Unsubscribe from multiple topics (blocking until UNSUBACK).
///
/// # Paho C signature
/// ```c
/// int MQTTClient_unsubscribeMany(MQTTClient handle, int count, char* const* topic);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_unsubscribeMany(
    handle: *mut libc::c_void,
    count: c_int,
    topic: *const *const c_char,
) -> c_int {
    if handle.is_null() || topic.is_null() || count <= 0 {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let inner = &*(handle as *mut PahoClientInner);

    if !inner.is_connected() {
        return MQTTCLIENT_DISCONNECTED;
    }

    let mut topics = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let t = *topic.add(i);
        if t.is_null() {
            return MQTTCLIENT_NULL_PARAMETER;
        }
        match CStr::from_ptr(t).to_str() {
            Ok(s) => topics.push(s.to_string()),
            Err(_) => return MQTTCLIENT_BAD_UTF8_STRING,
        }
    }

    let cmd = UnsubscribeCommand::from_topics(topics);

    let (response_tx, response_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::UnsubscribeSync {
            command: cmd,
            response_tx,
        })
        .is_err()
    {
        return MQTTCLIENT_FAILURE;
    }

    match response_rx.recv_timeout(SUBSCRIBE_TIMEOUT) {
        Ok(Ok(())) => MQTTCLIENT_SUCCESS,
        Ok(Err(_)) => MQTTCLIENT_FAILURE,
        Err(_) => MQTTCLIENT_FAILURE,
    }
}
