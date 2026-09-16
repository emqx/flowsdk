// SPDX-License-Identifier: MPL-2.0
pub use super::properties::MqttPropertyFFI;

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttPublishOptionsFFI {
    pub qos: u8,
    pub retain: bool,
    pub priority: Option<u8>,
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttSubscriptionFFI {
    pub topic_filter: String,
    pub qos: u8,
    pub no_local: bool,
    pub retain_as_published: bool,
    pub retain_handling: u8,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttSubscribeOptionsFFI {
    pub subscriptions: Vec<MqttSubscriptionFFI>,
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttUnsubscribeOptionsFFI {
    pub topics: Vec<String>,
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttDisconnectOptionsFFI {
    pub reason_code: u8,
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttWillFFI {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone, Default)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttEngineOptionsFFI {
    pub retransmission_timeout_ms: Option<u64>,
    pub ping_timeout_multiplier: Option<u32>,
    pub max_outgoing_packet_count: Option<u32>,
    pub max_event_count: Option<u32>,
    pub parser_buffer_size: Option<u32>,
    pub max_inflight: Option<u16>,
    pub auto_keepalive: Option<bool>,
    pub auto_ack: Option<bool>,
    pub sessionless: Option<bool>,
    pub subscriptions: Vec<MqttSubscriptionFFI>,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttConnectOptionsFFI {
    pub options: MqttOptionsFFI,
    pub properties: Vec<MqttPropertyFFI>,
    pub will: Option<MqttWillFFI>,
    pub binary_password: Option<Vec<u8>>,
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

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record, serde::Serialize))]
#[derive(Clone)]
pub struct MqttMessageFFI {
    pub stream_id: Option<u64>,
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    pub dup: bool,
    pub packet_id: Option<u16>,
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record, serde::Serialize))]
#[derive(Clone)]
pub struct ConnectionResultFFI {
    pub reason_code: u8,
    pub session_present: bool,
    pub properties: Vec<MqttPropertyFFI>,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record, serde::Serialize))]
pub struct AuthResultFFI {
    pub reason_code: u8,
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record, serde::Serialize))]
#[derive(Clone)]
pub struct PublishResultFFI {
    pub packet_id: Option<u16>,
    pub reason_code: Option<u8>,
    pub qos: u8,
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record, serde::Serialize))]
#[derive(Clone)]
pub struct SubscribeResultFFI {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record, serde::Serialize))]
#[derive(Clone)]
pub struct UnsubscribeResultFFI {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<MqttPropertyFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum, serde::Serialize))]
#[derive(Clone)]
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
    ReconnectNeeded,
    ReconnectScheduled {
        attempt: u32,
        delay_ms: u64,
    },
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
#[derive(Clone)]
pub struct MqttOptionsFFI {
    pub client_id: String,
    pub mqtt_version: u8,
    pub clean_start: bool,
    pub keep_alive: u16,
    pub username: Option<String>,
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
pub struct MqttTlsOptionsFFI {
    pub ca_cert_file: Option<String>,
    pub client_cert_file: Option<String>,
    pub client_key_file: Option<String>,
    pub insecure_skip_verify: bool,
    pub alpn_protocols: Vec<String>,
    pub enable_key_log: bool,
}

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct MqttDatagramFFI {
    pub addr: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum MqttParseLevelFFI {
    Full,
    HeadersParsed,
    TypeOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum))]
pub enum MqttAcknowledgementFFI {
    PubAck,
    PubRec,
    PubComp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum, serde::Serialize))]
pub enum QuicZeroRttStatusFFI {
    Disabled,
    Unavailable,
    Attempted,
    Accepted,
    Rejected,
}

#[derive(Clone)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Record))]
pub struct QuicZeroRttOptionsFFI {
    pub session_cache_size: u32,
    pub replay_on_reject: bool,
}
