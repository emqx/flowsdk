// SPDX-License-Identifier: MPL-2.0
pub use super::properties::MqttPropertyFFI;

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttPublishOptionsFFI {
    pub qos: u8,
    pub retain: bool,
    #[cfg_attr(feature = "json", serde(default))]
    pub priority: Option<u8>,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttSubscriptionFFI {
    pub topic_filter: String,
    pub qos: u8,
    pub no_local: bool,
    pub retain_as_published: bool,
    pub retain_handling: u8,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttSubscribeOptionsFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub subscriptions: Vec<MqttSubscriptionFFI>,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttUnsubscribeOptionsFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub topics: Vec<String>,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttDisconnectOptionsFFI {
    pub reason_code: u8,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct MqttWillFFI {
    pub topic: String,
    #[cfg_attr(feature = "json", serde(default))]
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttEngineOptionsFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub retransmission_timeout_ms: Option<u64>,
    #[cfg_attr(feature = "json", serde(default))]
    pub ping_timeout_multiplier: Option<u32>,
    #[cfg_attr(feature = "json", serde(default))]
    pub max_outgoing_packet_count: Option<u32>,
    #[cfg_attr(feature = "json", serde(default))]
    pub max_event_count: Option<u32>,
    #[cfg_attr(feature = "json", serde(default))]
    pub parser_buffer_size: Option<u32>,
    #[cfg_attr(feature = "json", serde(default))]
    pub max_inflight: Option<u16>,
    #[cfg_attr(feature = "json", serde(default))]
    pub auto_keepalive: Option<bool>,
    #[cfg_attr(feature = "json", serde(default))]
    pub auto_ack: Option<bool>,
    #[cfg_attr(feature = "json", serde(default))]
    pub sessionless: Option<bool>,
    #[cfg_attr(feature = "json", serde(default))]
    pub subscriptions: Vec<MqttSubscriptionFFI>,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct MqttConnectOptionsFFI {
    pub options: MqttOptionsFFI,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
    #[cfg_attr(feature = "json", serde(default))]
    pub will: Option<MqttWillFFI>,
    #[cfg_attr(feature = "json", serde(default))]
    pub binary_password: Option<Vec<u8>>,
    #[cfg_attr(feature = "json", serde(default))]
    pub engine_options: Option<MqttEngineOptionsFFI>,
}

impl From<MqttOptionsFFI> for MqttConnectOptionsFFI {
    fn from(options: MqttOptionsFFI) -> Self {
        Self {
            options,
            properties: vec![],
            will: None,
            binary_password: None,
            engine_options: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub enum MqttErrorFFI {
    #[error("Invalid configuration: {detail}")]
    Configuration { detail: String },
    #[error("Invalid argument: {detail}")]
    InvalidArgument { detail: String },
    #[error("MQTT engine error: {detail}")]
    Engine { detail: String },
    #[error("Unsupported operation: {detail}")]
    Unsupported { detail: String },
}

impl From<flowsdk::mqtt_client::error::MqttClientError> for MqttErrorFFI {
    fn from(error: flowsdk::mqtt_client::error::MqttClientError) -> Self {
        Self::Engine {
            detail: error.to_string(),
        }
    }
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct MqttMessageFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub stream_id: Option<u64>,
    pub topic: String,
    #[cfg_attr(feature = "json", serde(default))]
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    pub dup: bool,
    #[cfg_attr(feature = "json", serde(default))]
    pub packet_id: Option<u16>,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct ConnectionResultFFI {
    pub reason_code: u8,
    pub session_present: bool,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct AuthResultFFI {
    pub reason_code: u8,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct PublishResultFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub packet_id: Option<u16>,
    #[cfg_attr(feature = "json", serde(default))]
    pub reason_code: Option<u8>,
    pub qos: u8,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct SubscribeResultFFI {
    pub packet_id: u16,
    #[cfg_attr(feature = "json", serde(default))]
    pub reason_codes: Vec<u8>,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct UnsubscribeResultFFI {
    pub packet_id: u16,
    #[cfg_attr(feature = "json", serde(default))]
    pub reason_codes: Vec<u8>,
    #[cfg_attr(feature = "json", serde(default))]
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
#[derive(Clone)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub enum MqttEventFFI {
    TransportClosed {
        reason: String,
        by_peer: bool,
        error_code: Option<u64>,
    },
    ZeroRttStatusChanged {
        status: QuicZeroRttStatusFFI,
    },
    PublishReceived {
        packet_id: Option<u16>,
        stream_id: Option<u64>,
    },
    PubRelReceived {
        packet_id: u16,
        stream_id: Option<u64>,
    },
    Connected(ConnectionResultFFI),
    AuthReceived(AuthResultFFI),
    Disconnected {
        reason_code: Option<u8>,
        properties: Vec<MqttPropertyFFI>,
    },
    MessageReceived(MqttMessageFFI),
    Published(PublishResultFFI),
    Subscribed(SubscribeResultFFI),
    Unsubscribed(UnsubscribeResultFFI),
    PingResponse {
        success: bool,
    },
    Error {
        message: String,
    },
    StreamClosed {
        stream_id: u64,
        reason: String,
        by_peer: bool,
    },
    StreamReset {
        stream_id: u64,
        error_code: u64,
    },
    StreamStopped {
        stream_id: u64,
        error_code: u64,
    },
    OperationFailed {
        operation: MqttOperationKindFFI,
        packet_id: Option<u16>,
        kind: MqttFailureKindFFI,
        detail: String,
        timeout_ms: Option<u64>,
    },
    ReconnectNeeded,
    ReconnectScheduled {
        attempt: u32,
        delay_ms: u64,
    },
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttOptionsFFI {
    pub client_id: String,
    pub mqtt_version: u8,
    pub clean_start: bool,
    pub keep_alive: u16,
    #[cfg_attr(feature = "json", serde(default))]
    pub username: Option<String>,
    #[cfg_attr(feature = "json", serde(default))]
    pub password: Option<String>,
    pub reconnect_base_delay_ms: u64,
    pub reconnect_max_delay_ms: u64,
    pub max_reconnect_attempts: u32,
}

impl From<MqttOptionsFFI> for flowsdk::mqtt_client::opts::MqttClientOptions {
    fn from(opts: MqttOptionsFFI) -> Self {
        let mut builder = Self::builder()
            .client_id(opts.client_id)
            .mqtt_version(opts.mqtt_version)
            .clean_start(opts.clean_start)
            .keep_alive(opts.keep_alive)
            .reconnect_base_delay_ms(opts.reconnect_base_delay_ms)
            .reconnect_max_delay_ms(opts.reconnect_max_delay_ms)
            .max_reconnect_attempts(opts.max_reconnect_attempts);

        if let Some(username) = opts.username {
            builder = builder.username(username);
        }
        if let Some(password) = opts.password {
            builder = builder.password(password);
        }
        builder.build()
    }
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
pub struct MqttTlsOptionsFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub ca_cert_file: Option<String>,
    #[cfg_attr(feature = "json", serde(default))]
    pub client_cert_file: Option<String>,
    #[cfg_attr(feature = "json", serde(default))]
    pub client_key_file: Option<String>,
    pub insecure_skip_verify: bool,
    #[cfg_attr(feature = "json", serde(default))]
    pub alpn_protocols: Vec<String>,
    pub enable_key_log: bool,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct MqttDatagramFFI {
    pub addr: String,
    #[cfg_attr(feature = "json", serde(default))]
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub enum MqttParseLevelFFI {
    Full,
    HeadersParsed,
    TypeOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub enum MqttAcknowledgementFFI {
    PubAck,
    PubRec,
    PubComp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub enum QuicZeroRttStatusFFI {
    Disabled,
    Unavailable,
    Attempted,
    Accepted,
    Rejected,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct QuicZeroRttOptionsFFI {
    pub session_cache_size: u32,
    pub replay_on_reject: bool,
}

/// Engine protocol deadlines. None disables a deadline; zero expires immediately.
#[derive(Clone, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttOperationTimeoutsFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub connect_ms: Option<u64>,
    #[cfg_attr(feature = "json", serde(default))]
    pub publish_ms: Option<u64>,
    #[cfg_attr(feature = "json", serde(default))]
    pub subscribe_ms: Option<u64>,
    #[cfg_attr(feature = "json", serde(default))]
    pub unsubscribe_ms: Option<u64>,
}

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
pub fn mqtt_cloud_timeouts() -> MqttOperationTimeoutsFFI {
    MqttOperationTimeoutsFFI {
        connect_ms: Some(30_000),
        publish_ms: Some(10_000),
        subscribe_ms: Some(10_000),
        unsubscribe_ms: Some(10_000),
    }
}

/// Additive configuration shared by all transports. `peer` is a stable broker
/// identity, not necessarily its resolved socket address. Required for persistence.
#[derive(Clone, Default)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(default, deny_unknown_fields))]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttRuntimeOptionsFFI {
    #[cfg_attr(feature = "json", serde(default))]
    pub peer: Option<String>,
    #[cfg_attr(feature = "json", serde(default))]
    pub operation_timeouts: Option<MqttOperationTimeoutsFFI>,
    #[cfg_attr(feature = "json", serde(default))]
    pub incoming_receive_maximum: Option<u16>,
    #[cfg_attr(feature = "json", serde(default))]
    pub max_incoming_packet_size: Option<u64>,
    #[cfg_attr(feature = "json", serde(default))]
    pub max_incoming_buffer_bytes: Option<u64>,
    #[cfg_attr(feature = "json", serde(default))]
    pub max_outgoing_buffer_bytes: Option<u64>,
    #[cfg_attr(feature = "json", serde(default))]
    pub reconnect: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum MqttOperationKindFFI {
    Connect,
    Publish,
    Subscribe,
    Unsubscribe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum MqttFailureKindFFI {
    Timeout,
    SessionExpired,
    Cancelled,
    Other,
}

#[cfg(feature = "durable-session")]
#[derive(Clone, Debug)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttSessionInfoFFI {
    pub version: u32,
    pub peer: String,
    pub client_id: String,
    pub mqtt_version: u8,
    pub has_session: bool,
    pub session_expiry_interval: u32,
}

impl Default for MqttOptionsFFI {
    fn default() -> Self {
        Self {
            client_id: "mqtt_client".into(),
            mqtt_version: 5,
            clean_start: true,
            keep_alive: 60,
            username: None,
            password: None,
            reconnect_base_delay_ms: 1000,
            reconnect_max_delay_ms: 60_000,
            max_reconnect_attempts: 0,
        }
    }
}
