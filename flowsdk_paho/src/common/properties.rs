// SPDX-License-Identifier: MPL-2.0
//! MQTT v5 property containers (`MQTTProperty` / `MQTTProperties`) and the
//! Paho-compatible C property API.
//!
//! The `#[repr(C)]` layouts here match Eclipse Paho's `MQTTProperties.h` exactly
//! (including the tagged-union `MQTTProperty.value`), so C programs that read or
//! manipulate v5 properties through these structs and functions behave identically.
//!
//! Two private helpers bridge to the engine's `Property` enum:
//! - [`props_to_c`]   — `&[Property]` → heap `MQTTProperties` (malloc-owned)
//! - [`props_from_c`] — `&MQTTProperties` → `Vec<Property>`

use libc::{c_char, c_int, c_void};

use flowsdk::mqtt_serde::mqttv5::common::properties::Property;

// ─── Property identifier codes (MQTT 5.0 §2.2.2) ─────────────────────────

pub const MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR: c_int = 1;
pub const MQTTPROPERTY_CODE_MESSAGE_EXPIRY_INTERVAL: c_int = 2;
pub const MQTTPROPERTY_CODE_CONTENT_TYPE: c_int = 3;
pub const MQTTPROPERTY_CODE_RESPONSE_TOPIC: c_int = 8;
pub const MQTTPROPERTY_CODE_CORRELATION_DATA: c_int = 9;
pub const MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIER: c_int = 11;
pub const MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL: c_int = 17;
pub const MQTTPROPERTY_CODE_ASSIGNED_CLIENT_IDENTIFER: c_int = 18;
pub const MQTTPROPERTY_CODE_SERVER_KEEP_ALIVE: c_int = 19;
pub const MQTTPROPERTY_CODE_AUTHENTICATION_METHOD: c_int = 21;
pub const MQTTPROPERTY_CODE_AUTHENTICATION_DATA: c_int = 22;
pub const MQTTPROPERTY_CODE_REQUEST_PROBLEM_INFORMATION: c_int = 23;
pub const MQTTPROPERTY_CODE_WILL_DELAY_INTERVAL: c_int = 24;
pub const MQTTPROPERTY_CODE_REQUEST_RESPONSE_INFORMATION: c_int = 25;
pub const MQTTPROPERTY_CODE_RESPONSE_INFORMATION: c_int = 26;
pub const MQTTPROPERTY_CODE_SERVER_REFERENCE: c_int = 28;
pub const MQTTPROPERTY_CODE_REASON_STRING: c_int = 31;
pub const MQTTPROPERTY_CODE_RECEIVE_MAXIMUM: c_int = 33;
pub const MQTTPROPERTY_CODE_TOPIC_ALIAS_MAXIMUM: c_int = 34;
pub const MQTTPROPERTY_CODE_TOPIC_ALIAS: c_int = 35;
pub const MQTTPROPERTY_CODE_MAXIMUM_QOS: c_int = 36;
pub const MQTTPROPERTY_CODE_RETAIN_AVAILABLE: c_int = 37;
pub const MQTTPROPERTY_CODE_USER_PROPERTY: c_int = 38;
pub const MQTTPROPERTY_CODE_MAXIMUM_PACKET_SIZE: c_int = 39;
pub const MQTTPROPERTY_CODE_WILDCARD_SUBSCRIPTION_AVAILABLE: c_int = 40;
pub const MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIERS_AVAILABLE: c_int = 41;
pub const MQTTPROPERTY_CODE_SHARED_SUBSCRIPTION_AVAILABLE: c_int = 42;

// ─── Property value types (Paho `MQTTPropertyTypes`) ─────────────────────

pub const MQTTPROPERTY_TYPE_BYTE: c_int = 0;
pub const MQTTPROPERTY_TYPE_TWO_BYTE_INTEGER: c_int = 1;
pub const MQTTPROPERTY_TYPE_FOUR_BYTE_INTEGER: c_int = 2;
pub const MQTTPROPERTY_TYPE_VARIABLE_BYTE_INTEGER: c_int = 3;
pub const MQTTPROPERTY_TYPE_BINARY_DATA: c_int = 4;
pub const MQTTPROPERTY_TYPE_UTF_8_ENCODED_STRING: c_int = 5;
pub const MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR: c_int = 6;

// ─── C struct layouts (match Paho MQTTProperties.h) ──────────────────────

/// `MQTTLenString` — a length-prefixed (not null-terminated) string/blob.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTLenString {
    pub len: c_int,
    pub data: *mut c_char,
}

impl Default for MQTTLenString {
    fn default() -> Self {
        Self {
            len: 0,
            data: std::ptr::null_mut(),
        }
    }
}

/// The `{ data; value; }` arm of `MQTTProperty.value` (string and string-pair
/// properties). For a single string only `data` is used; for a user property
/// `data` is the key and `value` is the value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTLenStringPair {
    pub data: MQTTLenString,
    pub value: MQTTLenString,
}

/// Tagged union for `MQTTProperty.value`.
#[repr(C)]
#[derive(Clone, Copy)]
pub union MQTTPropertyValue {
    pub byte: libc::c_uchar,
    pub integer2: libc::c_ushort,
    pub integer4: libc::c_uint,
    pub strings: MQTTLenStringPair,
}

/// A single MQTT v5 property. Layout matches Paho's `MQTTProperty`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTProperty {
    /// The MQTT v5 property identifier (`MQTTPROPERTY_CODE_*`).
    pub identifier: c_int,
    /// The property value, interpreted according to `MQTTProperty_getType`.
    pub value: MQTTPropertyValue,
}

/// A collection of MQTT v5 properties. Layout matches Paho's `MQTTProperties`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MQTTProperties {
    /// Number of property entries in `array`.
    pub count: c_int,
    /// Capacity of `array` (entries).
    pub max_count: c_int,
    /// Encoded byte length of all properties (informational).
    pub length: c_int,
    /// Heap array of `count` properties (malloc-owned), or null.
    pub array: *mut MQTTProperty,
}

impl Default for MQTTProperties {
    fn default() -> Self {
        Self {
            count: 0,
            max_count: 0,
            length: 0,
            array: std::ptr::null_mut(),
        }
    }
}

// ─── Type classification ─────────────────────────────────────────────────

/// Classify a property identifier into its value type (`MQTTPROPERTY_TYPE_*`).
pub fn property_type(identifier: c_int) -> c_int {
    match identifier {
        MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR
        | MQTTPROPERTY_CODE_REQUEST_PROBLEM_INFORMATION
        | MQTTPROPERTY_CODE_REQUEST_RESPONSE_INFORMATION
        | MQTTPROPERTY_CODE_MAXIMUM_QOS
        | MQTTPROPERTY_CODE_RETAIN_AVAILABLE
        | MQTTPROPERTY_CODE_WILDCARD_SUBSCRIPTION_AVAILABLE
        | MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIERS_AVAILABLE
        | MQTTPROPERTY_CODE_SHARED_SUBSCRIPTION_AVAILABLE => MQTTPROPERTY_TYPE_BYTE,

        MQTTPROPERTY_CODE_SERVER_KEEP_ALIVE
        | MQTTPROPERTY_CODE_RECEIVE_MAXIMUM
        | MQTTPROPERTY_CODE_TOPIC_ALIAS_MAXIMUM
        | MQTTPROPERTY_CODE_TOPIC_ALIAS => MQTTPROPERTY_TYPE_TWO_BYTE_INTEGER,

        MQTTPROPERTY_CODE_MESSAGE_EXPIRY_INTERVAL
        | MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL
        | MQTTPROPERTY_CODE_WILL_DELAY_INTERVAL
        | MQTTPROPERTY_CODE_MAXIMUM_PACKET_SIZE => MQTTPROPERTY_TYPE_FOUR_BYTE_INTEGER,

        MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIER => MQTTPROPERTY_TYPE_VARIABLE_BYTE_INTEGER,

        MQTTPROPERTY_CODE_CORRELATION_DATA | MQTTPROPERTY_CODE_AUTHENTICATION_DATA => {
            MQTTPROPERTY_TYPE_BINARY_DATA
        }

        MQTTPROPERTY_CODE_USER_PROPERTY => MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR,

        // Everything else that remains is a single UTF-8 string.
        MQTTPROPERTY_CODE_CONTENT_TYPE
        | MQTTPROPERTY_CODE_RESPONSE_TOPIC
        | MQTTPROPERTY_CODE_ASSIGNED_CLIENT_IDENTIFER
        | MQTTPROPERTY_CODE_AUTHENTICATION_METHOD
        | MQTTPROPERTY_CODE_RESPONSE_INFORMATION
        | MQTTPROPERTY_CODE_SERVER_REFERENCE
        | MQTTPROPERTY_CODE_REASON_STRING => MQTTPROPERTY_TYPE_UTF_8_ENCODED_STRING,

        _ => -1,
    }
}

/// Human-readable name for a property identifier (matches Paho's `MQTTPropertyName`).
pub fn property_name(identifier: c_int) -> &'static str {
    match identifier {
        MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR => "PAYLOAD_FORMAT_INDICATOR",
        MQTTPROPERTY_CODE_MESSAGE_EXPIRY_INTERVAL => "MESSAGE_EXPIRY_INTERVAL",
        MQTTPROPERTY_CODE_CONTENT_TYPE => "CONTENT_TYPE",
        MQTTPROPERTY_CODE_RESPONSE_TOPIC => "RESPONSE_TOPIC",
        MQTTPROPERTY_CODE_CORRELATION_DATA => "CORRELATION_DATA",
        MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIER => "SUBSCRIPTION_IDENTIFIER",
        MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL => "SESSION_EXPIRY_INTERVAL",
        MQTTPROPERTY_CODE_ASSIGNED_CLIENT_IDENTIFER => "ASSIGNED_CLIENT_IDENTIFIER",
        MQTTPROPERTY_CODE_SERVER_KEEP_ALIVE => "SERVER_KEEP_ALIVE",
        MQTTPROPERTY_CODE_AUTHENTICATION_METHOD => "AUTHENTICATION_METHOD",
        MQTTPROPERTY_CODE_AUTHENTICATION_DATA => "AUTHENTICATION_DATA",
        MQTTPROPERTY_CODE_REQUEST_PROBLEM_INFORMATION => "REQUEST_PROBLEM_INFORMATION",
        MQTTPROPERTY_CODE_WILL_DELAY_INTERVAL => "WILL_DELAY_INTERVAL",
        MQTTPROPERTY_CODE_REQUEST_RESPONSE_INFORMATION => "REQUEST_RESPONSE_INFORMATION",
        MQTTPROPERTY_CODE_RESPONSE_INFORMATION => "RESPONSE_INFORMATION",
        MQTTPROPERTY_CODE_SERVER_REFERENCE => "SERVER_REFERENCE",
        MQTTPROPERTY_CODE_REASON_STRING => "REASON_STRING",
        MQTTPROPERTY_CODE_RECEIVE_MAXIMUM => "RECEIVE_MAXIMUM",
        MQTTPROPERTY_CODE_TOPIC_ALIAS_MAXIMUM => "TOPIC_ALIAS_MAXIMUM",
        MQTTPROPERTY_CODE_TOPIC_ALIAS => "TOPIC_ALIAS",
        MQTTPROPERTY_CODE_MAXIMUM_QOS => "MAXIMUM_QOS",
        MQTTPROPERTY_CODE_RETAIN_AVAILABLE => "RETAIN_AVAILABLE",
        MQTTPROPERTY_CODE_USER_PROPERTY => "USER_PROPERTY",
        MQTTPROPERTY_CODE_MAXIMUM_PACKET_SIZE => "MAXIMUM_PACKET_SIZE",
        MQTTPROPERTY_CODE_WILDCARD_SUBSCRIPTION_AVAILABLE => "WILDCARD_SUBSCRIPTION_AVAILABLE",
        MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIERS_AVAILABLE => {
            "SUBSCRIPTION_IDENTIFIERS_AVAILABLE"
        }
        MQTTPROPERTY_CODE_SHARED_SUBSCRIPTION_AVAILABLE => "SHARED_SUBSCRIPTION_AVAILABLE",
        _ => "UNKNOWN_PROPERTY",
    }
}

// ─── Low-level allocation helpers ────────────────────────────────────────

/// Copy `bytes` into a fresh `libc::malloc` allocation. Returns (len, ptr).
/// An empty slice yields `(0, null)`.
unsafe fn malloc_bytes(bytes: &[u8]) -> MQTTLenString {
    if bytes.is_empty() {
        return MQTTLenString::default();
    }
    let ptr = libc::malloc(bytes.len()) as *mut c_char;
    if ptr.is_null() {
        return MQTTLenString::default();
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    MQTTLenString {
        len: bytes.len() as c_int,
        data: ptr,
    }
}

/// Read a `MQTTLenString` back into an owned `Vec<u8>`.
unsafe fn read_len_string(s: &MQTTLenString) -> Vec<u8> {
    if s.data.is_null() || s.len <= 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(s.data as *const u8, s.len as usize).to_vec()
    }
}

// ─── Engine <-> C conversions ────────────────────────────────────────────

/// Convert one engine `Property` into a heap-backed C `MQTTProperty`.
/// String/binary payloads are deep-copied with `libc::malloc`.
unsafe fn property_to_c(p: &Property) -> MQTTProperty {
    let mut out = MQTTProperty {
        identifier: 0,
        value: MQTTPropertyValue { integer4: 0 },
    };
    match p {
        Property::PayloadFormatIndicator(v) => {
            out.identifier = MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR;
            out.value.byte = *v;
        }
        Property::MessageExpiryInterval(v) => {
            out.identifier = MQTTPROPERTY_CODE_MESSAGE_EXPIRY_INTERVAL;
            out.value.integer4 = *v;
        }
        Property::ContentType(s) => {
            out.identifier = MQTTPROPERTY_CODE_CONTENT_TYPE;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(s.as_bytes()),
                value: MQTTLenString::default(),
            };
        }
        Property::ResponseTopic(s) => {
            out.identifier = MQTTPROPERTY_CODE_RESPONSE_TOPIC;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(s.as_bytes()),
                value: MQTTLenString::default(),
            };
        }
        Property::CorrelationData(d) => {
            out.identifier = MQTTPROPERTY_CODE_CORRELATION_DATA;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(d),
                value: MQTTLenString::default(),
            };
        }
        Property::SubscriptionIdentifier(v) => {
            out.identifier = MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIER;
            out.value.integer4 = *v;
        }
        Property::SessionExpiryInterval(v) => {
            out.identifier = MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL;
            out.value.integer4 = *v;
        }
        Property::AssignedClientIdentifier(s) => {
            out.identifier = MQTTPROPERTY_CODE_ASSIGNED_CLIENT_IDENTIFER;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(s.as_bytes()),
                value: MQTTLenString::default(),
            };
        }
        Property::ServerKeepAlive(v) => {
            out.identifier = MQTTPROPERTY_CODE_SERVER_KEEP_ALIVE;
            out.value.integer2 = *v;
        }
        Property::AuthenticationMethod(s) => {
            out.identifier = MQTTPROPERTY_CODE_AUTHENTICATION_METHOD;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(s.as_bytes()),
                value: MQTTLenString::default(),
            };
        }
        Property::AuthenticationData(d) => {
            out.identifier = MQTTPROPERTY_CODE_AUTHENTICATION_DATA;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(d),
                value: MQTTLenString::default(),
            };
        }
        Property::RequestProblemInformation(v) => {
            out.identifier = MQTTPROPERTY_CODE_REQUEST_PROBLEM_INFORMATION;
            out.value.byte = *v;
        }
        Property::WillDelayInterval(v) => {
            out.identifier = MQTTPROPERTY_CODE_WILL_DELAY_INTERVAL;
            out.value.integer4 = *v;
        }
        Property::RequestResponseInformation(v) => {
            out.identifier = MQTTPROPERTY_CODE_REQUEST_RESPONSE_INFORMATION;
            out.value.byte = *v;
        }
        Property::ResponseInformation(s) => {
            out.identifier = MQTTPROPERTY_CODE_RESPONSE_INFORMATION;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(s.as_bytes()),
                value: MQTTLenString::default(),
            };
        }
        Property::ServerReference(s) => {
            out.identifier = MQTTPROPERTY_CODE_SERVER_REFERENCE;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(s.as_bytes()),
                value: MQTTLenString::default(),
            };
        }
        Property::ReasonString(s) => {
            out.identifier = MQTTPROPERTY_CODE_REASON_STRING;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(s.as_bytes()),
                value: MQTTLenString::default(),
            };
        }
        Property::ReceiveMaximum(v) => {
            out.identifier = MQTTPROPERTY_CODE_RECEIVE_MAXIMUM;
            out.value.integer2 = *v;
        }
        Property::TopicAliasMaximum(v) => {
            out.identifier = MQTTPROPERTY_CODE_TOPIC_ALIAS_MAXIMUM;
            out.value.integer2 = *v;
        }
        Property::TopicAlias(v) => {
            out.identifier = MQTTPROPERTY_CODE_TOPIC_ALIAS;
            out.value.integer2 = *v;
        }
        Property::MaximumQoS(v) => {
            out.identifier = MQTTPROPERTY_CODE_MAXIMUM_QOS;
            out.value.byte = *v;
        }
        Property::RetainAvailable(v) => {
            out.identifier = MQTTPROPERTY_CODE_RETAIN_AVAILABLE;
            out.value.byte = *v;
        }
        Property::UserProperty(k, v) => {
            out.identifier = MQTTPROPERTY_CODE_USER_PROPERTY;
            out.value.strings = MQTTLenStringPair {
                data: malloc_bytes(k.as_bytes()),
                value: malloc_bytes(v.as_bytes()),
            };
        }
        Property::MaximumPacketSize(v) => {
            out.identifier = MQTTPROPERTY_CODE_MAXIMUM_PACKET_SIZE;
            out.value.integer4 = *v;
        }
        Property::WildcardSubscriptionAvailable(v) => {
            out.identifier = MQTTPROPERTY_CODE_WILDCARD_SUBSCRIPTION_AVAILABLE;
            out.value.byte = *v;
        }
        Property::SubscriptionIdentifierAvailable(v) => {
            out.identifier = MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIERS_AVAILABLE;
            out.value.byte = *v;
        }
        Property::SharedSubscriptionAvailable(v) => {
            out.identifier = MQTTPROPERTY_CODE_SHARED_SUBSCRIPTION_AVAILABLE;
            out.value.byte = *v;
        }
    }
    out
}

/// Convert one C `MQTTProperty` into an engine `Property`, if recognised.
unsafe fn property_from_c(p: &MQTTProperty) -> Option<Property> {
    let s = |ls: &MQTTLenString| String::from_utf8_lossy(&read_len_string(ls)).into_owned();
    Some(match p.identifier {
        MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR => {
            Property::PayloadFormatIndicator(p.value.byte)
        }
        MQTTPROPERTY_CODE_MESSAGE_EXPIRY_INTERVAL => {
            Property::MessageExpiryInterval(p.value.integer4)
        }
        MQTTPROPERTY_CODE_CONTENT_TYPE => Property::ContentType(s(&p.value.strings.data)),
        MQTTPROPERTY_CODE_RESPONSE_TOPIC => Property::ResponseTopic(s(&p.value.strings.data)),
        MQTTPROPERTY_CODE_CORRELATION_DATA => {
            Property::CorrelationData(read_len_string(&p.value.strings.data))
        }
        MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIER => {
            Property::SubscriptionIdentifier(p.value.integer4)
        }
        MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL => {
            Property::SessionExpiryInterval(p.value.integer4)
        }
        MQTTPROPERTY_CODE_ASSIGNED_CLIENT_IDENTIFER => {
            Property::AssignedClientIdentifier(s(&p.value.strings.data))
        }
        MQTTPROPERTY_CODE_SERVER_KEEP_ALIVE => Property::ServerKeepAlive(p.value.integer2),
        MQTTPROPERTY_CODE_AUTHENTICATION_METHOD => {
            Property::AuthenticationMethod(s(&p.value.strings.data))
        }
        MQTTPROPERTY_CODE_AUTHENTICATION_DATA => {
            Property::AuthenticationData(read_len_string(&p.value.strings.data))
        }
        MQTTPROPERTY_CODE_REQUEST_PROBLEM_INFORMATION => {
            Property::RequestProblemInformation(p.value.byte)
        }
        MQTTPROPERTY_CODE_WILL_DELAY_INTERVAL => Property::WillDelayInterval(p.value.integer4),
        MQTTPROPERTY_CODE_REQUEST_RESPONSE_INFORMATION => {
            Property::RequestResponseInformation(p.value.byte)
        }
        MQTTPROPERTY_CODE_RESPONSE_INFORMATION => {
            Property::ResponseInformation(s(&p.value.strings.data))
        }
        MQTTPROPERTY_CODE_SERVER_REFERENCE => Property::ServerReference(s(&p.value.strings.data)),
        MQTTPROPERTY_CODE_REASON_STRING => Property::ReasonString(s(&p.value.strings.data)),
        MQTTPROPERTY_CODE_RECEIVE_MAXIMUM => Property::ReceiveMaximum(p.value.integer2),
        MQTTPROPERTY_CODE_TOPIC_ALIAS_MAXIMUM => Property::TopicAliasMaximum(p.value.integer2),
        MQTTPROPERTY_CODE_TOPIC_ALIAS => Property::TopicAlias(p.value.integer2),
        MQTTPROPERTY_CODE_MAXIMUM_QOS => Property::MaximumQoS(p.value.byte),
        MQTTPROPERTY_CODE_RETAIN_AVAILABLE => Property::RetainAvailable(p.value.byte),
        MQTTPROPERTY_CODE_USER_PROPERTY => {
            Property::UserProperty(s(&p.value.strings.data), s(&p.value.strings.value))
        }
        MQTTPROPERTY_CODE_MAXIMUM_PACKET_SIZE => Property::MaximumPacketSize(p.value.integer4),
        MQTTPROPERTY_CODE_WILDCARD_SUBSCRIPTION_AVAILABLE => {
            Property::WildcardSubscriptionAvailable(p.value.byte)
        }
        MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIERS_AVAILABLE => {
            Property::SubscriptionIdentifierAvailable(p.value.byte)
        }
        MQTTPROPERTY_CODE_SHARED_SUBSCRIPTION_AVAILABLE => {
            Property::SharedSubscriptionAvailable(p.value.byte)
        }
        _ => return None,
    })
}

/// Best-effort encoded length of a property (identifier byte + value bytes).
fn property_encoded_len(identifier: c_int, p: &MQTTProperty) -> c_int {
    // All property identifiers in the supported set encode in a single byte.
    let id_len = 1;
    let val = match property_type(identifier) {
        MQTTPROPERTY_TYPE_BYTE => 1,
        MQTTPROPERTY_TYPE_TWO_BYTE_INTEGER => 2,
        MQTTPROPERTY_TYPE_FOUR_BYTE_INTEGER => 4,
        MQTTPROPERTY_TYPE_VARIABLE_BYTE_INTEGER => 4, // upper bound
        MQTTPROPERTY_TYPE_BINARY_DATA | MQTTPROPERTY_TYPE_UTF_8_ENCODED_STRING => {
            2 + unsafe { p.value.strings.data.len.max(0) }
        }
        MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR => unsafe {
            4 + p.value.strings.data.len.max(0) + p.value.strings.value.len.max(0)
        },
        _ => 0,
    };
    id_len + val
}

/// Convert a slice of engine properties into a heap `MQTTProperties`.
///
/// The returned struct owns a `libc::malloc`'d array (and deep-copied string
/// data). Free it with [`MQTTProperties_free`].
pub unsafe fn props_to_c(props: &[Property]) -> MQTTProperties {
    if props.is_empty() {
        return MQTTProperties::default();
    }
    let count = props.len();
    let array = libc::malloc(count * std::mem::size_of::<MQTTProperty>()) as *mut MQTTProperty;
    if array.is_null() {
        return MQTTProperties::default();
    }
    let mut length = 0;
    for (i, p) in props.iter().enumerate() {
        let cp = property_to_c(p);
        length += property_encoded_len(cp.identifier, &cp);
        std::ptr::write(array.add(i), cp);
    }
    MQTTProperties {
        count: count as c_int,
        max_count: count as c_int,
        length,
        array,
    }
}

/// Convert a C `MQTTProperties` into a `Vec<Property>` (unrecognised entries skipped).
pub unsafe fn props_from_c(props: *const MQTTProperties) -> Vec<Property> {
    if props.is_null() {
        return Vec::new();
    }
    let props = &*props;
    if props.array.is_null() || props.count <= 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(props.count as usize);
    for i in 0..props.count as usize {
        let entry = &*props.array.add(i);
        if let Some(p) = property_from_c(entry) {
            out.push(p);
        }
    }
    out
}

// ─── Paho-compatible C API ───────────────────────────────────────────────

/// `int MQTTProperty_getType(int identifier)` — the value type, or -1 if unknown.
#[no_mangle]
pub extern "C" fn MQTTProperty_getType(identifier: c_int) -> c_int {
    property_type(identifier)
}

/// `int MQTTProperties_len(MQTTProperties* props)` — encoded byte length.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_len(props: *mut MQTTProperties) -> c_int {
    if props.is_null() {
        0
    } else {
        (*props).length
    }
}

/// `int MQTTProperties_add(MQTTProperties* props, const MQTTProperty* prop)`.
///
/// Deep-copies `prop` (including string/binary data) into a growing array.
/// Returns 0 on success, non-zero on failure.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_add(
    props: *mut MQTTProperties,
    prop: *const MQTTProperty,
) -> c_int {
    if props.is_null() || prop.is_null() {
        return 1;
    }
    let props = &mut *props;
    let prop = &*prop;

    // Grow the array if necessary (double capacity, min 4).
    if props.count >= props.max_count {
        let new_cap = if props.max_count <= 0 {
            4
        } else {
            props.max_count * 2
        };
        let new_size = new_cap as usize * std::mem::size_of::<MQTTProperty>();
        let new_array = libc::realloc(props.array as *mut c_void, new_size) as *mut MQTTProperty;
        if new_array.is_null() {
            return 1;
        }
        props.array = new_array;
        props.max_count = new_cap;
    }

    // Deep-copy the property (duplicate any heap-backed string/binary data).
    let mut copy = *prop;
    match property_type(prop.identifier) {
        MQTTPROPERTY_TYPE_BINARY_DATA | MQTTPROPERTY_TYPE_UTF_8_ENCODED_STRING => {
            let src = &prop.value.strings.data;
            copy.value.strings.data = malloc_bytes(&read_len_string(src));
            copy.value.strings.value = MQTTLenString::default();
        }
        MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR => {
            let k = read_len_string(&prop.value.strings.data);
            let v = read_len_string(&prop.value.strings.value);
            copy.value.strings.data = malloc_bytes(&k);
            copy.value.strings.value = malloc_bytes(&v);
        }
        _ => {}
    }

    std::ptr::write(props.array.add(props.count as usize), copy);
    props.count += 1;
    props.length += property_encoded_len(copy.identifier, &copy);
    0
}

/// `void MQTTProperties_free(MQTTProperties* props)` — free the array and any
/// heap-backed string/binary data. Resets count/length but not the container.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_free(props: *mut MQTTProperties) {
    if props.is_null() {
        return;
    }
    let props = &mut *props;
    if !props.array.is_null() {
        for i in 0..props.count as usize {
            let entry = &*props.array.add(i);
            match property_type(entry.identifier) {
                MQTTPROPERTY_TYPE_BINARY_DATA | MQTTPROPERTY_TYPE_UTF_8_ENCODED_STRING => {
                    if !entry.value.strings.data.data.is_null() {
                        libc::free(entry.value.strings.data.data as *mut c_void);
                    }
                }
                MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR => {
                    if !entry.value.strings.data.data.is_null() {
                        libc::free(entry.value.strings.data.data as *mut c_void);
                    }
                    if !entry.value.strings.value.data.is_null() {
                        libc::free(entry.value.strings.value.data as *mut c_void);
                    }
                }
                _ => {}
            }
        }
        libc::free(props.array as *mut c_void);
    }
    props.array = std::ptr::null_mut();
    props.count = 0;
    props.max_count = 0;
    props.length = 0;
}

/// `int MQTTProperties_hasProperty(MQTTProperties* props, int propid)`.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_hasProperty(
    props: *mut MQTTProperties,
    propid: c_int,
) -> c_int {
    (MQTTProperties_propertyCount(props, propid) > 0) as c_int
}

/// `int MQTTProperties_propertyCount(MQTTProperties* props, int propid)`.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_propertyCount(
    props: *mut MQTTProperties,
    propid: c_int,
) -> c_int {
    if props.is_null() || (*props).array.is_null() {
        return 0;
    }
    let props = &*props;
    let mut n = 0;
    for i in 0..props.count as usize {
        if (*props.array.add(i)).identifier == propid {
            n += 1;
        }
    }
    n
}

/// `MQTTProperty* MQTTProperties_getPropertyAt(MQTTProperties* props, int propid, int index)`.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_getPropertyAt(
    props: *mut MQTTProperties,
    propid: c_int,
    index: c_int,
) -> *mut MQTTProperty {
    if props.is_null() || (*props).array.is_null() || index < 0 {
        return std::ptr::null_mut();
    }
    let props = &*props;
    let mut seen = 0;
    for i in 0..props.count as usize {
        let entry = props.array.add(i);
        if (*entry).identifier == propid {
            if seen == index {
                return entry;
            }
            seen += 1;
        }
    }
    std::ptr::null_mut()
}

/// `MQTTProperty* MQTTProperties_getProperty(MQTTProperties* props, int propid)`.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_getProperty(
    props: *mut MQTTProperties,
    propid: c_int,
) -> *mut MQTTProperty {
    MQTTProperties_getPropertyAt(props, propid, 0)
}

/// `int MQTTProperties_getNumericValueAt(MQTTProperties* props, int propid, int index)`.
/// Returns the numeric value, or -9999999 if not present (matching Paho).
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_getNumericValueAt(
    props: *mut MQTTProperties,
    propid: c_int,
    index: c_int,
) -> c_int {
    let p = MQTTProperties_getPropertyAt(props, propid, index);
    if p.is_null() {
        return -9999999;
    }
    let p = &*p;
    match property_type(propid) {
        MQTTPROPERTY_TYPE_BYTE => p.value.byte as c_int,
        MQTTPROPERTY_TYPE_TWO_BYTE_INTEGER => p.value.integer2 as c_int,
        MQTTPROPERTY_TYPE_FOUR_BYTE_INTEGER | MQTTPROPERTY_TYPE_VARIABLE_BYTE_INTEGER => {
            p.value.integer4 as c_int
        }
        _ => -9999999,
    }
}

/// `int MQTTProperties_getNumericValue(MQTTProperties* props, int propid)`.
#[no_mangle]
pub unsafe extern "C" fn MQTTProperties_getNumericValue(
    props: *mut MQTTProperties,
    propid: c_int,
) -> c_int {
    MQTTProperties_getNumericValueAt(props, propid, 0)
}

/// `const char* MQTTPropertyName(int identifier)`.
#[no_mangle]
pub extern "C" fn MQTTPropertyName(identifier: c_int) -> *const c_char {
    // Static NUL-terminated names keyed by identifier.
    macro_rules! cstr {
        ($s:literal) => {
            concat!($s, "\0").as_ptr() as *const c_char
        };
    }
    match identifier {
        MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR => cstr!("PAYLOAD_FORMAT_INDICATOR"),
        MQTTPROPERTY_CODE_MESSAGE_EXPIRY_INTERVAL => cstr!("MESSAGE_EXPIRY_INTERVAL"),
        MQTTPROPERTY_CODE_CONTENT_TYPE => cstr!("CONTENT_TYPE"),
        MQTTPROPERTY_CODE_RESPONSE_TOPIC => cstr!("RESPONSE_TOPIC"),
        MQTTPROPERTY_CODE_CORRELATION_DATA => cstr!("CORRELATION_DATA"),
        MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIER => cstr!("SUBSCRIPTION_IDENTIFIER"),
        MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL => cstr!("SESSION_EXPIRY_INTERVAL"),
        MQTTPROPERTY_CODE_ASSIGNED_CLIENT_IDENTIFER => cstr!("ASSIGNED_CLIENT_IDENTIFIER"),
        MQTTPROPERTY_CODE_SERVER_KEEP_ALIVE => cstr!("SERVER_KEEP_ALIVE"),
        MQTTPROPERTY_CODE_AUTHENTICATION_METHOD => cstr!("AUTHENTICATION_METHOD"),
        MQTTPROPERTY_CODE_AUTHENTICATION_DATA => cstr!("AUTHENTICATION_DATA"),
        MQTTPROPERTY_CODE_REQUEST_PROBLEM_INFORMATION => cstr!("REQUEST_PROBLEM_INFORMATION"),
        MQTTPROPERTY_CODE_WILL_DELAY_INTERVAL => cstr!("WILL_DELAY_INTERVAL"),
        MQTTPROPERTY_CODE_REQUEST_RESPONSE_INFORMATION => cstr!("REQUEST_RESPONSE_INFORMATION"),
        MQTTPROPERTY_CODE_RESPONSE_INFORMATION => cstr!("RESPONSE_INFORMATION"),
        MQTTPROPERTY_CODE_SERVER_REFERENCE => cstr!("SERVER_REFERENCE"),
        MQTTPROPERTY_CODE_REASON_STRING => cstr!("REASON_STRING"),
        MQTTPROPERTY_CODE_RECEIVE_MAXIMUM => cstr!("RECEIVE_MAXIMUM"),
        MQTTPROPERTY_CODE_TOPIC_ALIAS_MAXIMUM => cstr!("TOPIC_ALIAS_MAXIMUM"),
        MQTTPROPERTY_CODE_TOPIC_ALIAS => cstr!("TOPIC_ALIAS"),
        MQTTPROPERTY_CODE_MAXIMUM_QOS => cstr!("MAXIMUM_QOS"),
        MQTTPROPERTY_CODE_RETAIN_AVAILABLE => cstr!("RETAIN_AVAILABLE"),
        MQTTPROPERTY_CODE_USER_PROPERTY => cstr!("USER_PROPERTY"),
        MQTTPROPERTY_CODE_MAXIMUM_PACKET_SIZE => cstr!("MAXIMUM_PACKET_SIZE"),
        MQTTPROPERTY_CODE_WILDCARD_SUBSCRIPTION_AVAILABLE => {
            cstr!("WILDCARD_SUBSCRIPTION_AVAILABLE")
        }
        MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIERS_AVAILABLE => {
            cstr!("SUBSCRIPTION_IDENTIFIERS_AVAILABLE")
        }
        MQTTPROPERTY_CODE_SHARED_SUBSCRIPTION_AVAILABLE => cstr!("SHARED_SUBSCRIPTION_AVAILABLE"),
        _ => cstr!("UNKNOWN_PROPERTY"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_classification() {
        assert_eq!(
            property_type(MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR),
            MQTTPROPERTY_TYPE_BYTE
        );
        assert_eq!(
            property_type(MQTTPROPERTY_CODE_RECEIVE_MAXIMUM),
            MQTTPROPERTY_TYPE_TWO_BYTE_INTEGER
        );
        assert_eq!(
            property_type(MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL),
            MQTTPROPERTY_TYPE_FOUR_BYTE_INTEGER
        );
        assert_eq!(
            property_type(MQTTPROPERTY_CODE_USER_PROPERTY),
            MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR
        );
        assert_eq!(
            property_type(MQTTPROPERTY_CODE_CONTENT_TYPE),
            MQTTPROPERTY_TYPE_UTF_8_ENCODED_STRING
        );
        assert_eq!(
            property_type(MQTTPROPERTY_CODE_CORRELATION_DATA),
            MQTTPROPERTY_TYPE_BINARY_DATA
        );
    }

    #[test]
    fn test_roundtrip_engine_props() {
        let input = vec![
            Property::MessageExpiryInterval(3600),
            Property::ContentType("application/json".to_string()),
            Property::UserProperty("k".to_string(), "v".to_string()),
            Property::CorrelationData(vec![1, 2, 3, 4]),
            Property::TopicAlias(42),
            Property::PayloadFormatIndicator(1),
        ];
        unsafe {
            let mut c = props_to_c(&input);
            assert_eq!(c.count, 6);
            let back = props_from_c(&c as *const MQTTProperties);
            assert_eq!(back, input);
            MQTTProperties_free(&mut c);
            assert_eq!(c.count, 0);
            assert!(c.array.is_null());
        }
    }

    #[test]
    fn test_c_api_add_and_query() {
        unsafe {
            let mut props = MQTTProperties::default();

            let up = MQTTProperty {
                identifier: MQTTPROPERTY_CODE_USER_PROPERTY,
                value: MQTTPropertyValue {
                    strings: MQTTLenStringPair {
                        data: MQTTLenString {
                            len: 3,
                            data: c"key".as_ptr() as *mut c_char,
                        },
                        value: MQTTLenString {
                            len: 3,
                            data: c"val".as_ptr() as *mut c_char,
                        },
                    },
                },
            };
            assert_eq!(MQTTProperties_add(&mut props, &up), 0);

            let sei = MQTTProperty {
                identifier: MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL,
                value: MQTTPropertyValue { integer4: 120 },
            };
            assert_eq!(MQTTProperties_add(&mut props, &sei), 0);

            assert_eq!(props.count, 2);
            assert_eq!(
                MQTTProperties_hasProperty(&mut props, MQTTPROPERTY_CODE_USER_PROPERTY),
                1
            );
            assert_eq!(
                MQTTProperties_getNumericValue(
                    &mut props,
                    MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL
                ),
                120
            );

            MQTTProperties_free(&mut props);
        }
    }
}
