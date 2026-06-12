// SPDX-License-Identifier: MPL-2.0
//! Paho C-compatible struct and callback definitions for the asynchronous API
//! (`MQTTAsync_*`).
//!
//! These `#[repr(C)]` structs mirror the layout of the corresponding structs in
//! Eclipse Paho's `MQTTAsync.h`. `MQTTAsync_message` is layout-identical to
//! `MQTTClient_message`, so it is aliased rather than redefined.
//!
//! NOTE: MQTT v5 property containers (`MQTTProperties`) are defined here for ABI
//! layout purposes only; populating/parsing individual properties is Phase 4.

use libc::{c_char, c_int, c_void};

use super::structs::{MQTTClient_SSLOptions, MQTTClient_message, MQTTClient_willOptions};

// ─── Handle / token / message aliases ────────────────────────────────────

/// Opaque async client handle. Points to a `PahoClientInner` (shared with the sync API).
pub type MQTTAsync = *mut c_void;

/// Async delivery / operation token (same as a packet id).
pub type MQTTAsync_token = c_int;

/// Async message — identical layout to `MQTTClient_message`.
pub type MQTTAsync_message = MQTTClient_message;

/// Async will options — identical layout to `MQTTClient_willOptions`.
pub type MQTTAsync_willOptions = MQTTClient_willOptions;

/// Async SSL options — identical layout to `MQTTClient_SSLOptions`.
pub type MQTTAsync_SSLOptions = MQTTClient_SSLOptions;

// ─── Magic byte constants ────────────────────────────────────────────────

pub const MQTTASYNC_RESPONSE_OPTIONS_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'T' as c_char,
    b'R' as c_char,
];
pub const MQTTASYNC_CONNECT_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'T' as c_char,
    b'C' as c_char,
];
pub const MQTTASYNC_DISCONNECT_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'T' as c_char,
    b'D' as c_char,
];
pub const MQTTASYNC_CREATE_OPTIONS_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'C' as c_char,
    b'O' as c_char,
];

// ─── MQTT v5 property containers ─────────────────────────────────────────

// The real `MQTTProperty` / `MQTTProperties` layouts and their C API live in
// `common::properties`; re-export them so existing `async_structs::` paths work.
pub use super::properties::{MQTTProperties, MQTTProperty};

/// Per-subscription options (MQTT v5). Matches Paho's `MQTTSubscribe_options`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTSubscribe_options {
    pub struct_id: [c_char; 4],
    pub struct_version: c_int,
    pub noLocal: libc::c_uchar,
    pub retainAsPublished: libc::c_uchar,
    pub retainHandling: libc::c_uchar,
}

// ─── Success / failure data ──────────────────────────────────────────────

/// `pub` arm of the `MQTTAsync_successData.alt` union.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTAsync_successData_pub {
    pub message: MQTTAsync_message,
    pub destinationName: *mut c_char,
}

/// `connect` arm of the `MQTTAsync_successData.alt` union.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTAsync_successData_connect {
    pub serverURI: *mut c_char,
    pub MQTTVersion: c_int,
    pub sessionPresent: c_int,
}

/// Union alternative for `MQTTAsync_successData`.
#[repr(C)]
pub union MQTTAsync_successData_alt {
    pub pub_: MQTTAsync_successData_pub,
    pub connect: MQTTAsync_successData_connect,
    pub qos: c_int,
    pub qosList: *mut c_int,
}

/// Data passed to `MQTTAsync_onSuccess`. Matches Paho's `MQTTAsync_successData`.
#[repr(C)]
pub struct MQTTAsync_successData {
    pub token: MQTTAsync_token,
    pub alt: MQTTAsync_successData_alt,
}

/// Data passed to `MQTTAsync_onFailure`. Matches Paho's `MQTTAsync_failureData`.
#[repr(C)]
pub struct MQTTAsync_failureData {
    pub token: MQTTAsync_token,
    pub code: c_int,
    pub message: *const c_char,
}

/// Data passed to `MQTTAsync_onSuccess5` (MQTT v5). Layout per Paho.
#[repr(C)]
pub struct MQTTAsync_successData5 {
    pub token: MQTTAsync_token,
    pub reasonCode: c_int,
    pub properties: MQTTProperties,
    pub alt: MQTTAsync_successData_alt,
}

/// Data passed to `MQTTAsync_onFailure5` (MQTT v5). Layout per Paho.
#[repr(C)]
pub struct MQTTAsync_failureData5 {
    pub token: MQTTAsync_token,
    pub reasonCode: c_int,
    pub properties: MQTTProperties,
    pub code: c_int,
    pub message: *const c_char,
    pub packet_type: c_int,
}

// ─── Callback function pointer typedefs ──────────────────────────────────

/// `typedef void MQTTAsync_onSuccess(void* context, MQTTAsync_successData* response)`
pub type MQTTAsync_onSuccess =
    Option<unsafe extern "C" fn(context: *mut c_void, response: *mut MQTTAsync_successData)>;

/// `typedef void MQTTAsync_onFailure(void* context, MQTTAsync_failureData* response)`
pub type MQTTAsync_onFailure =
    Option<unsafe extern "C" fn(context: *mut c_void, response: *mut MQTTAsync_failureData)>;

/// `typedef void MQTTAsync_onSuccess5(void* context, MQTTAsync_successData5* response)`
pub type MQTTAsync_onSuccess5 =
    Option<unsafe extern "C" fn(context: *mut c_void, response: *mut MQTTAsync_successData5)>;

/// `typedef void MQTTAsync_onFailure5(void* context, MQTTAsync_failureData5* response)`
pub type MQTTAsync_onFailure5 =
    Option<unsafe extern "C" fn(context: *mut c_void, response: *mut MQTTAsync_failureData5)>;

/// `typedef void MQTTAsync_connectionLost(void* context, char* cause)`
pub type MQTTAsync_connectionLost =
    Option<unsafe extern "C" fn(context: *mut c_void, cause: *mut c_char)>;

/// `typedef int MQTTAsync_messageArrived(void* context, char* topicName, int topicLen, MQTTAsync_message* message)`
pub type MQTTAsync_messageArrived = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        topicName: *mut c_char,
        topicLen: c_int,
        message: *mut MQTTAsync_message,
    ) -> c_int,
>;

/// `typedef void MQTTAsync_deliveryComplete(void* context, MQTTAsync_token token)`
pub type MQTTAsync_deliveryComplete =
    Option<unsafe extern "C" fn(context: *mut c_void, token: MQTTAsync_token)>;

/// `typedef void MQTTAsync_connected(void* context, char* cause)`
pub type MQTTAsync_connected =
    Option<unsafe extern "C" fn(context: *mut c_void, cause: *mut c_char)>;

/// `typedef void MQTTAsync_disconnected(void* context, MQTTProperties* properties, enum MQTTReasonCodes reasonCode)`
pub type MQTTAsync_disconnected = Option<
    unsafe extern "C" fn(context: *mut c_void, properties: *mut MQTTProperties, reasonCode: c_int),
>;

// ─── MQTTAsync_responseOptions ───────────────────────────────────────────

/// Matches Paho's `MQTTAsync_responseOptions`. struct_id = "MQTR".
#[repr(C)]
pub struct MQTTAsync_responseOptions {
    pub struct_id: [c_char; 4],
    pub struct_version: c_int,
    pub onSuccess: MQTTAsync_onSuccess,
    pub onFailure: MQTTAsync_onFailure,
    pub context: *mut c_void,
    pub token: MQTTAsync_token,
    pub onSuccess5: MQTTAsync_onSuccess5,
    pub onFailure5: MQTTAsync_onFailure5,
    pub properties: MQTTProperties,
    pub subscribeOptions: MQTTSubscribe_options,
    pub subscribeOptionsCount: c_int,
    pub subscribeOptionsList: *mut MQTTSubscribe_options,
}

impl MQTTAsync_responseOptions {
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTASYNC_RESPONSE_OPTIONS_STRUCT_ID && self.struct_version >= 0
    }
}

// ─── MQTTAsync_connectOptions ────────────────────────────────────────────

/// Returned-connection sub-struct of `MQTTAsync_connectOptions`.
#[repr(C)]
pub struct MQTTAsync_connectReturned {
    pub serverURI: *const c_char,
    pub MQTTVersion: c_int,
    pub sessionPresent: c_int,
}

/// Binary password sub-struct of `MQTTAsync_connectOptions`.
#[repr(C)]
pub struct MQTTAsync_binaryPwd {
    pub len: c_int,
    pub data: *const c_void,
}

/// Name/value pair (HTTP headers).
#[repr(C)]
pub struct MQTTAsync_nameValue {
    pub name: *const c_char,
    pub value: *const c_char,
}

/// Matches Paho's `MQTTAsync_connectOptions`. struct_id = "MQTC".
#[repr(C)]
pub struct MQTTAsync_connectOptions {
    pub struct_id: [c_char; 4],
    pub struct_version: c_int,
    pub keepAliveInterval: c_int,
    pub cleansession: c_int,
    pub maxInflight: c_int,
    pub will: *mut MQTTAsync_willOptions,
    pub username: *const c_char,
    pub password: *const c_char,
    pub connectTimeout: c_int,
    pub retryInterval: c_int,
    pub ssl: *mut MQTTAsync_SSLOptions,
    pub onSuccess: MQTTAsync_onSuccess,
    pub onFailure: MQTTAsync_onFailure,
    pub context: *mut c_void,
    pub serverURIcount: c_int,
    pub serverURIs: *const *const c_char,
    pub MQTTVersion: c_int,
    pub returned: MQTTAsync_connectReturned,
    pub binarypwd: MQTTAsync_binaryPwd,
    pub cleanstart: c_int,
    pub connectProperties: *mut MQTTProperties,
    pub willProperties: *mut MQTTProperties,
    pub onSuccess5: MQTTAsync_onSuccess5,
    pub onFailure5: MQTTAsync_onFailure5,
    pub automaticReconnect: c_int,
    pub minRetryInterval: c_int,
    pub maxRetryInterval: c_int,
    pub createOptions: *mut MQTTAsync_createOptions,
    pub httpHeaders: *const MQTTAsync_nameValue,
    pub httpProxy: *const c_char,
    pub httpsProxy: *const c_char,
}

impl MQTTAsync_connectOptions {
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTASYNC_CONNECT_STRUCT_ID && self.struct_version >= 0
    }

    pub fn effective_mqtt_version(&self) -> u8 {
        match self.MQTTVersion {
            0 | 4 => 4,
            3 => 3,
            5 => 5,
            _ => 4,
        }
    }

    pub fn effective_keep_alive(&self) -> u16 {
        if self.keepAliveInterval <= 0 {
            60
        } else {
            self.keepAliveInterval as u16
        }
    }

    pub fn effective_clean_start(&self) -> bool {
        if self.effective_mqtt_version() == 5 {
            self.cleanstart != 0
        } else {
            self.cleansession != 0
        }
    }

    pub fn effective_connect_timeout_secs(&self) -> u64 {
        if self.connectTimeout <= 0 {
            30
        } else {
            self.connectTimeout as u64
        }
    }
}

// ─── MQTTAsync_disconnectOptions ─────────────────────────────────────────

/// Matches Paho's `MQTTAsync_disconnectOptions`. struct_id = "MQTD".
#[repr(C)]
pub struct MQTTAsync_disconnectOptions {
    pub struct_id: [c_char; 4],
    pub struct_version: c_int,
    pub timeout: c_int,
    pub onSuccess: MQTTAsync_onSuccess,
    pub onFailure: MQTTAsync_onFailure,
    pub context: *mut c_void,
    pub properties: MQTTProperties,
    pub reasonCode: c_int,
    pub onSuccess5: MQTTAsync_onSuccess5,
    pub onFailure5: MQTTAsync_onFailure5,
}

impl MQTTAsync_disconnectOptions {
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTASYNC_DISCONNECT_STRUCT_ID && self.struct_version >= 0
    }
}

// ─── MQTTAsync_createOptions ─────────────────────────────────────────────

/// Matches Paho's `MQTTAsync_createOptions`. struct_id = "MQCO".
#[repr(C)]
pub struct MQTTAsync_createOptions {
    pub struct_id: [c_char; 4],
    pub struct_version: c_int,
    pub sendWhileDisconnected: c_int,
    pub maxBufferedMessages: c_int,
    pub MQTTVersion: c_int,
    pub allowDisconnectedSendAtAnyTime: c_int,
    pub deleteOldestMessages: c_int,
    pub restoreMessages: c_int,
    pub persistQoS0: c_int,
}

impl MQTTAsync_createOptions {
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTASYNC_CREATE_OPTIONS_STRUCT_ID && self.struct_version >= 0
    }
}
