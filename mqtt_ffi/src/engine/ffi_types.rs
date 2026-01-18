// SPDX-License-Identifier: MPL-2.0

#[derive(uniffi::Record, Clone, serde::Serialize)]
pub struct MqttMessageFFI {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
}

#[derive(uniffi::Record, Clone, serde::Serialize)]
pub struct ConnectionResultFFI {
    pub reason_code: u8,
    pub session_present: bool,
}

#[derive(uniffi::Record, Clone, serde::Serialize)]
pub struct PublishResultFFI {
    pub packet_id: Option<u16>,
    pub reason_code: Option<u8>,
    pub qos: u8,
}

#[derive(uniffi::Record, Clone, serde::Serialize)]
pub struct SubscribeResultFFI {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
}

#[derive(uniffi::Record, Clone, serde::Serialize)]
pub struct UnsubscribeResultFFI {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
}

#[derive(uniffi::Enum, Clone, serde::Serialize)]
pub enum MqttEventFFI {
    Connected(ConnectionResultFFI),
    Disconnected { reason_code: Option<u8> },
    MessageReceived(MqttMessageFFI),
    Published(PublishResultFFI),
    Subscribed(SubscribeResultFFI),
    Unsubscribed(UnsubscribeResultFFI),
    PingResponse { success: bool },
    Error { message: String },
    ReconnectNeeded,
    ReconnectScheduled { attempt: u32, delay_ms: u64 },
}

#[derive(uniffi::Record)]
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

#[derive(uniffi::Record)]
pub struct MqttTlsOptionsFFI {
    pub ca_cert_file: Option<String>,
    pub client_cert_file: Option<String>,
    pub client_key_file: Option<String>,
    pub insecure_skip_verify: bool,
    pub alpn_protocols: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct MqttDatagramFFI {
    pub addr: String,
    pub data: Vec<u8>,
}
