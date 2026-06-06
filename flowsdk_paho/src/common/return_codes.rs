// SPDX-License-Identifier: MPL-2.0
//! Paho C return code constants.
//!
//! These match the values defined in `MQTTClient.h` / `MQTTAsync.h`.

use libc::{c_char, c_int};

pub const MQTTCLIENT_SUCCESS: c_int = 0;
pub const MQTTCLIENT_FAILURE: c_int = -1;
pub const MQTTCLIENT_DISCONNECTED: c_int = -3;
pub const MQTTCLIENT_MAX_MESSAGES_INFLIGHT: c_int = -4;
pub const MQTTCLIENT_BAD_UTF8_STRING: c_int = -5;
pub const MQTTCLIENT_NULL_PARAMETER: c_int = -6;
pub const MQTTCLIENT_TOPICNAME_TRUNCATED: c_int = -7;
pub const MQTTCLIENT_BAD_STRUCTURE: c_int = -8;
pub const MQTTCLIENT_BAD_QOS: c_int = -9;
pub const MQTTCLIENT_SSL_NOT_SUPPORTED: c_int = -10;
pub const MQTTCLIENT_BAD_MQTT_VERSION: c_int = -11;
pub const MQTTCLIENT_BAD_PROTOCOL: c_int = -14;
pub const MQTTCLIENT_BAD_MQTT_OPTION: c_int = -15;
pub const MQTTCLIENT_WRONG_MQTT_VERSION: c_int = -16;
pub const MQTTCLIENT_0_LEN_WILL_TOPIC: c_int = -17;

// Aliases for the async API (same values, different names in Paho headers)
pub const MQTTASYNC_SUCCESS: c_int = MQTTCLIENT_SUCCESS;
pub const MQTTASYNC_FAILURE: c_int = MQTTCLIENT_FAILURE;
pub const MQTTASYNC_DISCONNECTED: c_int = MQTTCLIENT_DISCONNECTED;
pub const MQTTASYNC_MAX_MESSAGES_INFLIGHT: c_int = MQTTCLIENT_MAX_MESSAGES_INFLIGHT;
pub const MQTTASYNC_BAD_UTF8_STRING: c_int = MQTTCLIENT_BAD_UTF8_STRING;
pub const MQTTASYNC_NULL_PARAMETER: c_int = MQTTCLIENT_NULL_PARAMETER;
pub const MQTTASYNC_BAD_STRUCTURE: c_int = MQTTCLIENT_BAD_STRUCTURE;
pub const MQTTASYNC_BAD_QOS: c_int = MQTTCLIENT_BAD_QOS;
pub const MQTTASYNC_SSL_NOT_SUPPORTED: c_int = MQTTCLIENT_SSL_NOT_SUPPORTED;
pub const MQTTASYNC_BAD_MQTT_VERSION: c_int = MQTTCLIENT_BAD_MQTT_VERSION;
pub const MQTTASYNC_BAD_PROTOCOL: c_int = MQTTCLIENT_BAD_PROTOCOL;
pub const MQTTASYNC_BAD_MQTT_OPTION: c_int = MQTTCLIENT_BAD_MQTT_OPTION;
pub const MQTTASYNC_WRONG_MQTT_VERSION: c_int = MQTTCLIENT_WRONG_MQTT_VERSION;
pub const MQTTASYNC_0_LEN_WILL_TOPIC: c_int = MQTTCLIENT_0_LEN_WILL_TOPIC;

// MQTT protocol version constants (matching Paho)
pub const MQTTVERSION_DEFAULT: c_int = 0;
pub const MQTTVERSION_3_1: c_int = 3;
pub const MQTTVERSION_3_1_1: c_int = 4;
pub const MQTTVERSION_5: c_int = 5;

// Persistence types
pub const MQTTCLIENT_PERSISTENCE_NONE: c_int = 1;
pub const MQTTCLIENT_PERSISTENCE_DEFAULT: c_int = 0;
pub const MQTTCLIENT_PERSISTENCE_USER: c_int = 2;

// CONNACK reason codes (MQTT v5) - also used as Paho return codes
pub const MQTT_BAD_SUBSCRIBE: c_int = 0x80;

/// Get a human-readable error message for a Paho return code.
///
/// Returns a pointer to a static, null-terminated C string. The caller must NOT free it.
pub fn strerror(rc: c_int) -> *const c_char {
    // Each literal is a null-terminated C string (the `\0` is explicit).
    let s: &'static str = match rc {
        MQTTCLIENT_SUCCESS => "Success\0",
        MQTTCLIENT_FAILURE => "Failure\0",
        MQTTCLIENT_DISCONNECTED => "Disconnected\0",
        MQTTCLIENT_MAX_MESSAGES_INFLIGHT => "Maximum messages in flight\0",
        MQTTCLIENT_BAD_UTF8_STRING => "Bad UTF-8 string\0",
        MQTTCLIENT_NULL_PARAMETER => "Null parameter\0",
        MQTTCLIENT_TOPICNAME_TRUNCATED => "Topic name truncated\0",
        MQTTCLIENT_BAD_STRUCTURE => "Bad structure\0",
        MQTTCLIENT_BAD_QOS => "Bad QoS value\0",
        MQTTCLIENT_SSL_NOT_SUPPORTED => "SSL not supported\0",
        MQTTCLIENT_BAD_MQTT_VERSION => "Bad MQTT version\0",
        MQTTCLIENT_BAD_PROTOCOL => "Bad protocol\0",
        MQTTCLIENT_BAD_MQTT_OPTION => "Bad MQTT option\0",
        MQTTCLIENT_WRONG_MQTT_VERSION => "Wrong MQTT version\0",
        MQTTCLIENT_0_LEN_WILL_TOPIC => "Zero-length will topic\0",
        _ => "Unknown error\0",
    };
    s.as_ptr() as *const c_char
}
