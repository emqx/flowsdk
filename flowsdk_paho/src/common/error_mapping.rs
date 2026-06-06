// SPDX-License-Identifier: MPL-2.0
//! Map FlowSDK errors to Paho C return codes.

use flowsdk::mqtt_client::error::MqttClientError;
use libc::c_int;

use super::return_codes::*;

/// Map a FlowSDK `MqttClientError` to the closest Paho C return code.
pub fn map_error(err: &MqttClientError) -> c_int {
    match err {
        MqttClientError::NotConnected => MQTTCLIENT_DISCONNECTED,
        MqttClientError::ConnectionLost { .. } => MQTTCLIENT_DISCONNECTED,
        MqttClientError::ConnectionRefused { .. } => MQTTCLIENT_FAILURE,
        MqttClientError::NetworkError { .. } => MQTTCLIENT_FAILURE,
        MqttClientError::OperationTimeout { .. } => MQTTCLIENT_FAILURE,
        MqttClientError::BufferFull { .. } => MQTTCLIENT_MAX_MESSAGES_INFLIGHT,
        MqttClientError::PacketIdExhausted => MQTTCLIENT_MAX_MESSAGES_INFLIGHT,
        MqttClientError::InvalidConfiguration { field, .. } => {
            if field == "qos" {
                MQTTCLIENT_BAD_QOS
            } else if field == "mqtt_version" {
                MQTTCLIENT_BAD_MQTT_VERSION
            } else {
                MQTTCLIENT_BAD_MQTT_OPTION
            }
        }
        MqttClientError::ProtocolViolation { .. } => MQTTCLIENT_BAD_PROTOCOL,
        MqttClientError::InvalidState { .. } => MQTTCLIENT_FAILURE,
        MqttClientError::AlreadyConnected => MQTTCLIENT_FAILURE,
        MqttClientError::AuthenticationFailed { .. } => MQTTCLIENT_FAILURE,
        _ => MQTTCLIENT_FAILURE,
    }
}

/// Map a CONNACK reason code to a Paho-compatible return code.
/// For MQTT v3.1.1 reason codes 1-5, Paho returns them as positive integers (1-5).
/// For MQTT v5 reason codes, Paho returns them directly.
pub fn map_connack_reason_code(reason_code: u8) -> c_int {
    // Paho returns CONNACK reason codes directly as positive integers
    reason_code as c_int
}
