// SPDX-License-Identifier: MPL-2.0
//! Paho C-compatible struct definitions.
//!
//! These `#[repr(C)]` structs match the exact layout of the corresponding Paho C structs,
//! including the `struct_id` magic bytes and `struct_version` field used for ABI versioning.

use libc::{c_char, c_int, c_void};
use std::ffi::CStr;

use flowsdk::mqtt_serde::mqttv5::common::properties::Property;

use super::properties::{self, MQTTProperties};

// ─── Magic byte constants ────────────────────────────────────────────────

pub const MQTTCLIENT_MESSAGE_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'T' as c_char,
    b'M' as c_char,
];
pub const MQTTCLIENT_CONNECT_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'T' as c_char,
    b'C' as c_char,
];
pub const MQTTCLIENT_WILL_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'T' as c_char,
    b'W' as c_char,
];
pub const MQTTCLIENT_SSL_STRUCT_ID: [c_char; 4] = [
    b'M' as c_char,
    b'Q' as c_char,
    b'T' as c_char,
    b'S' as c_char,
];

// ─── MQTTClient_message ─────────────────────────────────────────────────

/// Matches the Paho `MQTTClient_message` struct layout.
///
/// struct_id = "MQTM", struct_version = 1
///
/// `MQTTAsync_message` has the identical layout, so the async API aliases this type.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTClient_message {
    /// Must be "MQTM"
    pub struct_id: [c_char; 4],
    /// Must be 1
    pub struct_version: c_int,
    /// Length of the payload in bytes
    pub payloadlen: c_int,
    /// Pointer to the payload data
    pub payload: *mut c_void,
    /// QoS level (0, 1, or 2)
    pub qos: c_int,
    /// Whether this is a retained message
    pub retained: c_int,
    /// Whether this is a duplicate delivery
    pub dup: c_int,
    /// MQTT message/packet identifier
    pub msgid: c_int,
    /// MQTT v5 properties (struct_version >= 1). Empty for v3.x messages.
    pub properties: MQTTProperties,
}

impl MQTTClient_message {
    /// Validate the struct_id and struct_version fields.
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTCLIENT_MESSAGE_STRUCT_ID
            && self.struct_version >= 0
            && self.struct_version <= 1
    }
}

// ─── MQTTClient_willOptions ─────────────────────────────────────────────

/// Matches the Paho `MQTTClient_willOptions` struct layout.
///
/// struct_id = "MQTW", struct_version = 1
#[repr(C)]
pub struct MQTTClient_willOptions {
    /// Must be "MQTW"
    pub struct_id: [c_char; 4],
    /// Must be 1
    pub struct_version: c_int,
    /// The topic to publish the will message to
    pub topicName: *const c_char,
    /// The will message (null-terminated string for version 0)
    pub message: *const c_char,
    /// Whether the will message should be retained
    pub retained: c_int,
    /// QoS for the will message
    pub qos: c_int,
    // struct_version >= 1: binary will payload
    /// Binary will payload (struct_version >= 1)
    pub payload: MQTTClient_willPayload,
}

/// Binary payload for will messages (struct_version >= 1).
#[repr(C)]
pub struct MQTTClient_willPayload {
    /// Length of the binary payload
    pub len: c_int,
    /// Pointer to the binary payload data
    pub data: *const c_void,
}

impl MQTTClient_willOptions {
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTCLIENT_WILL_STRUCT_ID
            && self.struct_version >= 0
            && self.struct_version <= 1
    }
}

// ─── MQTTClient_SSLOptions ──────────────────────────────────────────────

/// Matches the Paho `MQTTClient_SSLOptions` struct layout.
///
/// struct_id = "MQTS"
#[repr(C)]
pub struct MQTTClient_SSLOptions {
    /// Must be "MQTS"
    pub struct_id: [c_char; 4],
    /// Struct version
    pub struct_version: c_int,
    /// Path to a file containing trusted CA certificates (PEM format)
    pub trustStore: *const c_char,
    /// Path to a file containing the client's certificate chain (PEM format)
    pub keyStore: *const c_char,
    /// Path to a file containing the client's private key (PEM format)
    pub privateKey: *const c_char,
    /// Password to decrypt the private key
    pub privateKeyPassword: *const c_char,
    /// List of SSL/TLS ciphers to enable (colon-separated)
    pub enabledCipherSuites: *const c_char,
    /// Whether to verify the server certificate (1 = yes, 0 = no)
    pub enableServerCertAuth: c_int,
    /// SSL/TLS protocol version
    pub sslVersion: c_int,
    /// Whether to carry out post-connect checks (struct_version >= 1)
    pub verify: c_int,
    /// Path to a directory containing CA certificates (struct_version >= 2)
    pub CApath: *const c_char,
}

impl MQTTClient_SSLOptions {
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTCLIENT_SSL_STRUCT_ID && self.struct_version >= 0
    }
}

// ─── MQTTClient_connectOptions ──────────────────────────────────────────

/// Matches the Paho `MQTTClient_connectOptions` struct layout.
///
/// struct_id = "MQTC"
#[repr(C)]
pub struct MQTTClient_connectOptions {
    /// Must be "MQTC"
    pub struct_id: [c_char; 4],
    /// Struct version (0-8)
    pub struct_version: c_int,
    /// Keep alive interval in seconds
    pub keepAliveInterval: c_int,
    /// Clean session flag (MQTT v3.1.1)
    pub cleansession: c_int,
    /// Reliable publishing (unused in most implementations)
    pub reliable: c_int,
    /// Pointer to will options (NULL if no will)
    pub will: *mut MQTTClient_willOptions,
    /// Username for authentication
    pub username: *const c_char,
    /// Password for authentication (null-terminated string)
    pub password: *const c_char,
    /// Connect timeout in seconds
    pub connectTimeout: c_int,
    /// Retry interval in seconds
    pub retryInterval: c_int,
    /// Pointer to SSL/TLS options (NULL if no TLS)
    pub ssl: *mut MQTTClient_SSLOptions,
    /// Number of server URIs in the serverURIs array
    pub serverURIcount: c_int,
    /// Array of server URIs for failover
    pub serverURIs: *const *const c_char,
    /// MQTT protocol version (0=default, 3=3.1, 4=3.1.1, 5=5.0)
    pub MQTTVersion: c_int,
    // struct_version >= 3: returned connection values
    /// Returned data from the connect (struct_version >= 3)
    pub returned: MQTTClient_connectReturned,
    // struct_version >= 4: binary password
    /// Binary password (struct_version >= 4)
    pub binarypwd: MQTTClient_binaryPassword,
    // struct_version >= 5: maxInflightMessages
    /// Maximum number of inflight messages (struct_version >= 5, 0 = default)
    pub maxInflightMessages: c_int,
    // struct_version >= 6: MQTT v5 clean start
    /// Clean start flag for MQTT v5 (struct_version >= 6)
    pub cleanstart: c_int,
    // struct_version >= 7: HTTP headers, HTTP proxy, HTTPS proxy
    /// HTTP headers for websocket connections (struct_version >= 7)
    pub httpHeaders: *const MQTTClient_nameValue,
    /// HTTP proxy URL (struct_version >= 7)
    pub httpProxy: *const c_char,
    /// HTTPS proxy URL (struct_version >= 7)
    pub httpsProxy: *const c_char,
}

/// Values returned from a connect operation (Paho struct_version >= 3).
#[repr(C)]
pub struct MQTTClient_connectReturned {
    /// The server URI connected to
    pub serverURI: *const c_char,
    /// The MQTT version used
    pub MQTTVersion: c_int,
    /// Whether a session was present on the server
    pub sessionPresent: c_int,
}

/// Binary password support (Paho struct_version >= 4).
#[repr(C)]
pub struct MQTTClient_binaryPassword {
    /// Length of the binary password
    pub len: c_int,
    /// Pointer to the binary password data
    pub data: *const c_void,
}

/// Name-value pair for HTTP headers.
#[repr(C)]
pub struct MQTTClient_nameValue {
    pub name: *const c_char,
    pub value: *const c_char,
}

impl MQTTClient_connectOptions {
    /// Validate the struct_id magic bytes.
    pub fn validate(&self) -> bool {
        self.struct_id == MQTTCLIENT_CONNECT_STRUCT_ID && self.struct_version >= 0
    }

    /// Get the effective MQTT version. Version 0 means "try 3.1.1 first, fall back to 3.1".
    pub fn effective_mqtt_version(&self) -> u8 {
        match self.MQTTVersion {
            0 | 4 => 4, // Default and 3.1.1
            3 => 3,     // 3.1
            5 => 5,     // v5.0
            _ => 4,     // Fallback to 3.1.1
        }
    }

    /// Get the keep alive interval, defaulting to 60 seconds.
    pub fn effective_keep_alive(&self) -> u16 {
        if self.keepAliveInterval <= 0 {
            60
        } else {
            self.keepAliveInterval as u16
        }
    }

    /// Get clean session/clean start flag based on MQTT version.
    pub fn effective_clean_start(&self) -> bool {
        if self.effective_mqtt_version() == 5 && self.struct_version >= 6 {
            self.cleanstart != 0
        } else {
            self.cleansession != 0
        }
    }

    /// Get the connect timeout in seconds, defaulting to 30.
    pub fn effective_connect_timeout_secs(&self) -> u64 {
        if self.connectTimeout <= 0 {
            30
        } else {
            self.connectTimeout as u64
        }
    }

    /// Safely read the username, if non-null.
    pub unsafe fn get_username(&self) -> Option<String> {
        if self.username.is_null() {
            None
        } else {
            Some(CStr::from_ptr(self.username).to_string_lossy().into_owned())
        }
    }

    /// Safely read the password.
    /// Prefers binary password (struct_version >= 4) if available, falls back to string.
    pub unsafe fn get_password(&self) -> Option<Vec<u8>> {
        if self.struct_version >= 4 && !self.binarypwd.data.is_null() && self.binarypwd.len > 0 {
            let slice = std::slice::from_raw_parts(
                self.binarypwd.data as *const u8,
                self.binarypwd.len as usize,
            );
            Some(slice.to_vec())
        } else if !self.password.is_null() {
            let cstr = CStr::from_ptr(self.password);
            Some(cstr.to_bytes().to_vec())
        } else {
            None
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Allocate a `MQTTClient_message` on the heap with `libc::malloc` and return a raw pointer.
/// The caller must free it with `MQTTClient_freeMessage`.
pub unsafe fn alloc_paho_message(
    topic_payload: &[u8],
    qos: i32,
    retained: bool,
    dup: bool,
    msgid: i32,
    props: &[Property],
) -> *mut MQTTClient_message {
    let msg_ptr =
        libc::malloc(std::mem::size_of::<MQTTClient_message>()) as *mut MQTTClient_message;
    if msg_ptr.is_null() {
        return std::ptr::null_mut();
    }

    // Allocate and copy payload
    let payload_ptr = if topic_payload.is_empty() {
        std::ptr::null_mut()
    } else {
        let p = libc::malloc(topic_payload.len());
        if p.is_null() {
            libc::free(msg_ptr as *mut c_void);
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(topic_payload.as_ptr(), p as *mut u8, topic_payload.len());
        p
    };

    *msg_ptr = MQTTClient_message {
        struct_id: MQTTCLIENT_MESSAGE_STRUCT_ID,
        struct_version: 1,
        payloadlen: topic_payload.len() as c_int,
        payload: payload_ptr,
        qos: qos as c_int,
        retained: retained as c_int,
        dup: dup as c_int,
        msgid: msgid as c_int,
        properties: properties::props_to_c(props),
    };

    msg_ptr
}

/// Free a `MQTTClient_message` allocated by `alloc_paho_message`.
pub unsafe fn free_paho_message(msg: *mut *mut MQTTClient_message) {
    if msg.is_null() || (*msg).is_null() {
        return;
    }
    let m = *msg;
    if !(*m).payload.is_null() {
        libc::free((*m).payload);
    }
    properties::MQTTProperties_free(&mut (*m).properties);
    libc::free(m as *mut c_void);
    *msg = std::ptr::null_mut();
}

/// Allocate a C string with `libc::malloc` from a Rust `&str`.
pub unsafe fn alloc_c_string(s: &str) -> *mut c_char {
    let len = s.len() + 1; // +1 for null terminator
    let ptr = libc::malloc(len) as *mut c_char;
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    std::ptr::copy_nonoverlapping(s.as_ptr(), ptr as *mut u8, s.len());
    *ptr.add(s.len()) = 0; // null terminator
    ptr
}
