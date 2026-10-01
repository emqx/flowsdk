// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;
use alloc::vec::Vec;

use crate::mqtt_serde::mqttv5::common::properties::Property;

pub struct Subscription {
    pub topic: String,
    pub qos: u8,
}

/// Returns a human-readable description for MQTT v5 reason codes
/// Based on MQTT 5.0 Specification Table 2-6 - Reason Codes
/// Used across CONNACK, PUBACK, PUBREC, PUBREL, PUBCOMP, SUBACK, UNSUBACK, DISCONNECT, and AUTH packets
pub fn reason_code_to_string(code: u8) -> &'static str {
    match code {
        0x00 => "Success",
        0x01 => "Granted QoS 1",
        0x02 => "Granted QoS 2",
        0x04 => "Disconnect with Will Message",
        0x10 => "No matching subscribers",
        0x11 => "No subscription existed",
        0x18 => "Continue authentication",
        0x19 => "Re-authenticate",
        0x80 => "Unspecified error",
        0x81 => "Malformed Packet",
        0x82 => "Protocol Error",
        0x83 => "Implementation specific error",
        0x84 => "Unsupported Protocol Version",
        0x85 => "Client Identifier not valid",
        0x86 => "Bad User Name or Password",
        0x87 => "Not authorized",
        0x88 => "Server unavailable",
        0x89 => "Server busy",
        0x8A => "Banned",
        0x8B => "Server shutting down",
        0x8C => "Bad authentication method",
        0x8D => "Keep Alive timeout",
        0x8E => "Session taken over",
        0x8F => "Topic Filter invalid",
        0x90 => "Topic Name invalid",
        0x91 => "Packet Identifier in use",
        0x92 => "Packet Identifier not found",
        0x93 => "Receive Maximum exceeded",
        0x94 => "Topic Alias invalid",
        0x95 => "Packet too large",
        0x96 => "Message rate too high",
        0x97 => "Quota exceeded",
        0x98 => "Administrative action",
        0x99 => "Payload format invalid",
        0x9A => "Retain not supported",
        0x9B => "QoS not supported",
        0x9C => "Use another server",
        0x9D => "Server moved",
        0x9E => "Shared Subscriptions not supported",
        0x9F => "Connection rate exceeded",
        0xA0 => "Maximum connect time",
        0xA1 => "Subscription Identifiers not supported",
        0xA2 => "Wildcard Subscriptions not supported",
        _ => "Unknown reason code",
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConnectionResult {
    pub reason_code: u8,
    pub session_present: bool,
    pub properties: Option<Vec<Property>>,
}

impl ConnectionResult {
    /// Returns true if the connection was successful (reason code 0)
    pub fn is_success(&self) -> bool {
        self.reason_code == 0
    }

    /// Returns true if the connection failed
    pub fn is_failure(&self) -> bool {
        self.reason_code != 0
    }

    /// Returns a description of the reason code
    pub fn reason_description(&self) -> &'static str {
        reason_code_to_string(self.reason_code)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthResult {
    pub reason_code: u8,
    pub properties: Vec<Property>,
}

impl AuthResult {
    /// Returns true if authentication was successful (reason code 0x00)
    pub fn is_success(&self) -> bool {
        self.reason_code == 0x00
    }

    /// Returns true if authentication requires continuation (reason code 0x18)
    pub fn is_continue(&self) -> bool {
        self.reason_code == 0x18
    }

    /// Returns true if re-authentication is requested (reason code 0x19)
    pub fn is_re_authenticate(&self) -> bool {
        self.reason_code == 0x19
    }

    /// Returns a description of the authentication reason code
    pub fn reason_description(&self) -> &'static str {
        match self.reason_code {
            0x00 => "Success",
            0x18 => "Continue authentication",
            0x19 => "Re-authenticate",
            _ => "Unknown authentication reason code",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SubscribeResult {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<Property>,
}

impl SubscribeResult {
    pub fn is_success(&self) -> bool {
        self.reason_codes.iter().all(|&code| code <= 2) // 0, 1, 2 are success codes
    }

    pub fn successful_subscriptions(&self) -> usize {
        self.reason_codes.iter().filter(|&&code| code <= 2).count()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnsubscribeResult {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<Property>,
}

impl UnsubscribeResult {
    pub fn is_success(&self) -> bool {
        self.reason_codes
            .iter()
            .all(|&code| code == 0 || code == 17) // 0 = Success, 17 = No subscription existed
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PublishResult {
    pub packet_id: Option<u16>,
    pub reason_code: Option<u8>, // None for QoS 0
    pub properties: Option<Vec<Property>>,
    pub qos: u8,
}

impl PublishResult {
    pub fn is_success(&self) -> bool {
        // MQTT 5 reason codes below 0x80 are successful outcomes. This includes
        // 0x10 (No matching subscribers) for PUBACK/PUBREC.
        self.reason_code.is_none_or(|code| code < 0x80)
    }

    /// Returns a description of the PUBACK/PUBREC reason code
    pub fn reason_description(&self) -> &'static str {
        match self.reason_code {
            None => "Success (QoS 0)",
            Some(code) => reason_code_to_string(code),
        }
    }
}

#[cfg(test)]
mod publish_result_tests {
    use super::PublishResult;

    fn result(reason_code: u8) -> PublishResult {
        PublishResult {
            packet_id: Some(1),
            reason_code: Some(reason_code),
            properties: None,
            qos: 1,
        }
    }

    #[test]
    fn mqtt5_no_matching_subscribers_is_successful() {
        assert!(result(0x10).is_success());
        assert!(!result(0x80).is_success());
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PingResult {
    // PINGRESP has no variable header or payload, just the fact that we received it
    pub success: bool,
}
