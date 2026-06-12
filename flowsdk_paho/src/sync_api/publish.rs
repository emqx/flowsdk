// SPDX-License-Identifier: MPL-2.0
//! `MQTTClient_publish` and `MQTTClient_publishMessage`

use libc::{c_char, c_int, c_void};
use std::ffi::CStr;

use flowsdk::mqtt_client::commands::PublishCommand;

use crate::common::return_codes::*;
use crate::common::structs::MQTTClient_message;
use crate::inner::client_state::{PahoClientInner, PahoCommand};

use super::create_destroy::MQTTClient;

/// Publish a message to a topic.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_publish(MQTTClient handle, const char* topicName,
///                         int payloadlen, const void* payload,
///                         int qos, int retained,
///                         MQTTClient_deliveryToken* dt);
/// ```
///
/// Returns as soon as the message is queued and `dt` is populated with the
/// delivery token (packet ID). For QoS 1/2, call `MQTTClient_waitForCompletion`
/// with `dt` to block until the broker acknowledges.
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_publish(
    handle: MQTTClient,
    topic_name: *const c_char,
    payloadlen: c_int,
    payload: *const c_void,
    qos: c_int,
    retained: c_int,
    dt: *mut c_int,
) -> c_int {
    if handle.is_null() || topic_name.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    if !(0..=2).contains(&qos) {
        return MQTTCLIENT_BAD_QOS;
    }

    let inner = &*(handle as *mut PahoClientInner);

    if !inner.is_connected() {
        return MQTTCLIENT_DISCONNECTED;
    }

    // Parse topic
    let topic = match CStr::from_ptr(topic_name).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return MQTTCLIENT_BAD_UTF8_STRING,
    };

    // Build payload
    let payload_bytes = if payloadlen > 0 && !payload.is_null() {
        std::slice::from_raw_parts(payload as *const u8, payloadlen as usize).to_vec()
    } else {
        Vec::new()
    };

    // Build the publish command
    let cmd = match PublishCommand::builder()
        .topic(topic)
        .payload(payload_bytes)
        .qos(qos as u8)
        .retain(retained != 0)
        .build()
    {
        Ok(cmd) => cmd,
        Err(_) => return MQTTCLIENT_FAILURE,
    };

    // Send the publish command to the I/O thread and get the assigned packet ID.
    let (response_tx, response_rx) = std::sync::mpsc::channel();
    if inner
        .send_command(PahoCommand::PublishSync {
            command: cmd,
            response_tx,
        })
        .is_err()
    {
        return MQTTCLIENT_FAILURE;
    }

    // Block briefly to get the packet ID back from the I/O thread.
    // This is not waiting for the broker ACK — that's MQTTClient_waitForCompletion.
    let packet_id = match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(pid)) => pid,
        Ok(Err(_)) => return MQTTCLIENT_FAILURE,
        Err(_) => return MQTTCLIENT_FAILURE,
    };

    if !dt.is_null() {
        *dt = packet_id.map(|p| p as c_int).unwrap_or(0);
    }

    MQTTCLIENT_SUCCESS
}

/// Publish a message using a `MQTTClient_message` struct.
///
/// # Paho C signature
/// ```c
/// int MQTTClient_publishMessage(MQTTClient handle, const char* topicName,
///                                MQTTClient_message* msg,
///                                MQTTClient_deliveryToken* dt);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_publishMessage(
    handle: MQTTClient,
    topic_name: *const c_char,
    msg: *mut MQTTClient_message,
    dt: *mut c_int,
) -> c_int {
    if handle.is_null() || topic_name.is_null() || msg.is_null() {
        return MQTTCLIENT_NULL_PARAMETER;
    }

    let message = &*msg;
    if !message.validate() {
        return MQTTCLIENT_BAD_STRUCTURE;
    }

    MQTTClient_publish(
        handle,
        topic_name,
        message.payloadlen,
        message.payload,
        message.qos,
        message.retained,
        dt,
    )
}
