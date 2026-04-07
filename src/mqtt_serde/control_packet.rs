// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

use super::encode_variable_length;
use super::parser::packet_type;
use super::parser::{ParseError, ParseOk};

use crate::mqtt_serde::mqttv3::*;
use crate::mqtt_serde::mqttv5::*;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum MqttPacket {
    // V5
    Connect5(connectv5::MqttConnect),
    ConnAck5(connackv5::MqttConnAck),
    Publish5(publishv5::MqttPublish),
    PubAck5(pubackv5::MqttPubAck),
    PubRec5(pubrecv5::MqttPubRec),
    PubRel5(pubrelv5::MqttPubRel),
    PubComp5(pubcompv5::MqttPubComp),
    Subscribe5(subscribev5::MqttSubscribe),
    SubAck5(subackv5::MqttSubAck),
    Unsubscribe5(unsubscribev5::MqttUnsubscribe),
    UnsubAck5(unsubackv5::MqttUnsubAck),
    PingReq5(pingreqv5::MqttPingReq),
    PingResp5(pingrespv5::MqttPingResp),
    Disconnect5(disconnectv5::MqttDisconnect),
    Auth(authv5::MqttAuth),

    // V3
    Connect3(connectv3::MqttConnect),
    ConnAck3(connackv3::MqttConnAck),
    Publish3(publishv3::MqttPublish),
    PubAck3(pubackv3::MqttPubAck),
    PubRec3(pubrecv3::MqttPubRec),
    PubRel3(pubrelv3::MqttPubRel),
    PubComp3(pubcompv3::MqttPubComp),
    Subscribe3(subscribev3::MqttSubscribe),
    SubAck3(subackv3::MqttSubAck),
    Unsubscribe3(unsubscribev3::MqttUnsubscribe),
    UnsubAck3(unsubackv3::MqttUnsubAck),
    PingReq3(pingreqv3::MqttPingReq),
    PingResp3(pingrespv3::MqttPingResp),
    Disconnect3(disconnectv3::MqttDisconnect),
}

impl MqttPacket {
    /// Returns the `ControlPacketType` for this packet.
    pub fn packet_type(&self) -> ControlPacketType {
        match self {
            MqttPacket::Connect5(_) | MqttPacket::Connect3(_) => ControlPacketType::CONNECT,
            MqttPacket::ConnAck5(_) | MqttPacket::ConnAck3(_) => ControlPacketType::CONNACK,
            MqttPacket::Publish5(_) | MqttPacket::Publish3(_) => ControlPacketType::PUBLISH,
            MqttPacket::PubAck5(_) | MqttPacket::PubAck3(_) => ControlPacketType::PUBACK,
            MqttPacket::PubRec5(_) | MqttPacket::PubRec3(_) => ControlPacketType::PUBREC,
            MqttPacket::PubRel5(_) | MqttPacket::PubRel3(_) => ControlPacketType::PUBREL,
            MqttPacket::PubComp5(_) | MqttPacket::PubComp3(_) => ControlPacketType::PUBCOMP,
            MqttPacket::Subscribe5(_) | MqttPacket::Subscribe3(_) => ControlPacketType::SUBSCRIBE,
            MqttPacket::SubAck5(_) | MqttPacket::SubAck3(_) => ControlPacketType::SUBACK,
            MqttPacket::Unsubscribe5(_) | MqttPacket::Unsubscribe3(_) => {
                ControlPacketType::UNSUBSCRIBE
            }
            MqttPacket::UnsubAck5(_) | MqttPacket::UnsubAck3(_) => ControlPacketType::UNSUBACK,
            MqttPacket::PingReq5(_) | MqttPacket::PingReq3(_) => ControlPacketType::PINGREQ,
            MqttPacket::PingResp5(_) | MqttPacket::PingResp3(_) => ControlPacketType::PINGRESP,
            MqttPacket::Disconnect5(_) | MqttPacket::Disconnect3(_) => {
                ControlPacketType::DISCONNECT
            }
            MqttPacket::Auth(_) => ControlPacketType::AUTH,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        match self {
            // V5
            MqttPacket::Connect5(p) => p.to_bytes(),
            MqttPacket::ConnAck5(p) => p.to_bytes(),
            MqttPacket::Publish5(p) => p.to_bytes(),
            MqttPacket::PubAck5(p) => p.to_bytes(),
            MqttPacket::PubRec5(p) => p.to_bytes(),
            MqttPacket::PubRel5(p) => p.to_bytes(),
            MqttPacket::PubComp5(p) => p.to_bytes(),
            MqttPacket::Subscribe5(p) => p.to_bytes(),
            MqttPacket::SubAck5(p) => p.to_bytes(),
            MqttPacket::Unsubscribe5(p) => p.to_bytes(),
            MqttPacket::UnsubAck5(p) => p.to_bytes(),
            MqttPacket::PingReq5(p) => p.to_bytes(),
            MqttPacket::PingResp5(p) => p.to_bytes(),
            MqttPacket::Disconnect5(p) => p.to_bytes(),
            MqttPacket::Auth(p) => p.to_bytes(),

            // V3
            MqttPacket::Connect3(p) => p.to_bytes(),
            MqttPacket::ConnAck3(p) => p.to_bytes(),
            MqttPacket::Publish3(p) => p.to_bytes(),
            MqttPacket::PubAck3(p) => p.to_bytes(),
            MqttPacket::PubRec3(p) => p.to_bytes(),
            MqttPacket::PubRel3(p) => p.to_bytes(),
            MqttPacket::PubComp3(p) => p.to_bytes(),
            MqttPacket::Subscribe3(p) => p.to_bytes(),
            MqttPacket::SubAck3(p) => p.to_bytes(),
            MqttPacket::Unsubscribe3(p) => p.to_bytes(),
            MqttPacket::UnsubAck3(p) => p.to_bytes(),
            MqttPacket::PingReq3(p) => p.to_bytes(),
            MqttPacket::PingResp3(p) => p.to_bytes(),
            MqttPacket::Disconnect3(p) => p.to_bytes(),
        }
    }

    pub fn set_dup(&mut self, dup: bool) {
        match self {
            MqttPacket::Publish5(p) => p.dup = dup,
            MqttPacket::Publish3(p) => p.dup = dup,
            _ => {}
        }
    }

    pub fn from_bytes_with_version(buffer: &[u8], mqtt_version: u8) -> Result<ParseOk, ParseError> {
        match mqtt_version {
            3 | 4 => Self::from_bytes_v3(buffer), // Both MQTT v3.1 and v3.1.1 use v3 parser
            5 => Self::from_bytes_v5(buffer),
            _ => Err(ParseError::InvalidPacketType),
        }
    }

    pub fn from_bytes_v5(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type_byte = packet_type(buffer)?;
        let packet_type = ControlPacketType::try_from(packet_type_byte)?;

        match packet_type {
            ControlPacketType::CONNECT => connectv5::MqttConnect::from_bytes(buffer),
            ControlPacketType::CONNACK => connackv5::MqttConnAck::from_bytes(buffer),
            ControlPacketType::PUBLISH => publishv5::MqttPublish::from_bytes(buffer),
            ControlPacketType::PUBACK => pubackv5::MqttPubAck::from_bytes(buffer),
            ControlPacketType::PUBREC => pubrecv5::MqttPubRec::from_bytes(buffer),
            ControlPacketType::PUBREL => pubrelv5::MqttPubRel::from_bytes(buffer),
            ControlPacketType::PUBCOMP => pubcompv5::MqttPubComp::from_bytes(buffer),
            ControlPacketType::SUBSCRIBE => subscribev5::MqttSubscribe::from_bytes(buffer),
            ControlPacketType::SUBACK => subackv5::MqttSubAck::from_bytes(buffer),
            ControlPacketType::UNSUBSCRIBE => unsubscribev5::MqttUnsubscribe::from_bytes(buffer),
            ControlPacketType::UNSUBACK => unsubackv5::MqttUnsubAck::from_bytes(buffer),
            ControlPacketType::PINGREQ => pingreqv5::MqttPingReq::from_bytes(buffer),
            ControlPacketType::PINGRESP => pingrespv5::MqttPingResp::from_bytes(buffer),
            ControlPacketType::DISCONNECT => disconnectv5::MqttDisconnect::from_bytes(buffer),
            ControlPacketType::AUTH => authv5::MqttAuth::from_bytes(buffer),
        }
    }

    pub fn from_bytes_v3(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type_byte = packet_type(buffer)?;
        let packet_type = ControlPacketType::try_from(packet_type_byte)?;

        match packet_type {
            ControlPacketType::CONNECT => connectv3::MqttConnect::from_bytes(buffer),
            ControlPacketType::CONNACK => connackv3::MqttConnAck::from_bytes(buffer),
            ControlPacketType::PUBLISH => publishv3::MqttPublish::from_bytes(buffer),
            ControlPacketType::PUBACK => pubackv3::MqttPubAck::from_bytes(buffer),
            ControlPacketType::PUBREC => pubrecv3::MqttPubRec::from_bytes(buffer),
            ControlPacketType::PUBREL => pubrelv3::MqttPubRel::from_bytes(buffer),
            ControlPacketType::PUBCOMP => pubcompv3::MqttPubComp::from_bytes(buffer),
            ControlPacketType::SUBSCRIBE => subscribev3::MqttSubscribe::from_bytes(buffer),
            ControlPacketType::SUBACK => subackv3::MqttSubAck::from_bytes(buffer),
            ControlPacketType::UNSUBSCRIBE => unsubscribev3::MqttUnsubscribe::from_bytes(buffer),
            ControlPacketType::UNSUBACK => unsubackv3::MqttUnsubAck::from_bytes(buffer),
            ControlPacketType::PINGREQ => pingreqv3::MqttPingReq::from_bytes(buffer),
            ControlPacketType::PINGRESP => pingrespv3::MqttPingResp::from_bytes(buffer),
            ControlPacketType::DISCONNECT => disconnectv3::MqttDisconnect::from_bytes(buffer),
            _ => Err(ParseError::InvalidPacketType),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlPacketType {
    CONNECT = 1,
    CONNACK = 2,
    PUBLISH = 3,
    PUBACK = 4,
    PUBREC = 5,
    PUBREL = 6,
    PUBCOMP = 7,
    SUBSCRIBE = 8,
    SUBACK = 9,
    UNSUBSCRIBE = 10,
    UNSUBACK = 11,
    PINGREQ = 12,
    PINGRESP = 13,
    DISCONNECT = 14,
    AUTH = 15,
}

impl TryFrom<u8> for ControlPacketType {
    type Error = ParseError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ControlPacketType::CONNECT),
            2 => Ok(ControlPacketType::CONNACK),
            3 => Ok(ControlPacketType::PUBLISH),
            4 => Ok(ControlPacketType::PUBACK),
            5 => Ok(ControlPacketType::PUBREC),
            6 => Ok(ControlPacketType::PUBREL),
            7 => Ok(ControlPacketType::PUBCOMP),
            8 => Ok(ControlPacketType::SUBSCRIBE),
            9 => Ok(ControlPacketType::SUBACK),
            10 => Ok(ControlPacketType::UNSUBSCRIBE),
            11 => Ok(ControlPacketType::UNSUBACK),
            12 => Ok(ControlPacketType::PINGREQ),
            13 => Ok(ControlPacketType::PINGRESP),
            14 => Ok(ControlPacketType::DISCONNECT),
            15 => Ok(ControlPacketType::AUTH),
            _ => Err(ParseError::InvalidPacketType),
        }
    }
}

impl ControlPacketType {
    /// Extracts the packet type and flags from the first byte of a fixed header.
    pub fn from_first_byte(byte: u8) -> Result<(Self, u8), ParseError> {
        let pkt_type = Self::try_from(byte >> 4)?;
        let flags = byte & 0x0F;
        Ok((pkt_type, flags))
    }
}

pub trait MqttControlPacket {
    // MQTT 5.0: 2.1.2, MQTT control packet type
    fn control_packet_type(&self) -> u8;

    // MQTT 5.0: 2.1.3, Flags in the fixed header
    fn flags(&self) -> u8 {
        0u8
    }

    // Default implementations
    // Constructs the fixed header for the MQTT packet.
    // The fixed header consists of a control packet type, flags, and the remaining length.
    fn fixed_header(&self, len: usize) -> Vec<u8> {
        let byte1: u8 = (self.control_packet_type()) << 4 | self.flags();
        let variable_length = encode_variable_length(len);
        let mut hdr = vec![byte1];
        hdr.extend(variable_length);
        hdr
    }

    // return variable header
    fn variable_header(&self) -> Result<Vec<u8>, ParseError>;

    // return payload
    fn payload(&self) -> Result<Vec<u8>, ParseError>;

    // decoder
    fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError>;

    // encoder to existing buffer
    fn encode_to_buffer(&self, bytes: &mut Vec<u8>) -> Result<(), ParseError> {
        let vhdr = self.variable_header()?;
        let payload = self.payload()?;
        let remaining_length = vhdr.len() + payload.len();
        bytes.extend(self.fixed_header(remaining_length));
        bytes.extend(vhdr);
        bytes.extend(payload);
        Ok(())
    }

    // encoder
    fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        let vhdr = self.variable_header()?;
        let payload = self.payload()?;
        let remaining_length = vhdr.len() + payload.len();
        let fixed_hdr = self.fixed_header(remaining_length);

        // Pre-allocate the entire packet size to avoid intermediate reallocations
        let mut bytes = Vec::with_capacity(fixed_hdr.len() + remaining_length);
        bytes.extend(fixed_hdr);
        bytes.extend(vhdr);
        bytes.extend(payload);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_control_packet_type_conversion() {
        let pkt = MqttPacket::Connect5(connectv5::MqttConnect {
            protocol_name: "MQTT".to_string(),
            protocol_version: 5,
            keep_alive: 60,
            client_id: "test_client".to_string(),
            will: None,
            username: None,
            password: None,
            properties: Vec::new(),
            clean_start: true,
        });

        let json = serde_json::to_string(&pkt).unwrap();

        let expected ="{\"type\":\"Connect5\",\"protocol_name\":\"MQTT\",\"protocol_version\":5,\"keep_alive\":60,\"client_id\":\"test_client\",\"will\":null,\"username\":null,\"password\":null,\"properties\":[],\"clean_start\":true}";
        assert_eq!(json, expected);
    }

    // Test packet_type() for all MQTTv5 packet variants
    #[test]
    fn test_packet_type_v5_connect() {
        let packet = MqttPacket::Connect5(connectv5::MqttConnect {
            protocol_name: "MQTT".to_string(),
            protocol_version: 5,
            keep_alive: 60,
            client_id: "test".to_string(),
            will: None,
            username: None,
            password: None,
            properties: Vec::new(),
            clean_start: true,
        });
        assert_eq!(packet.packet_type(), ControlPacketType::CONNECT);
    }

    #[test]
    fn test_packet_type_v5_connack() {
        let packet = MqttPacket::ConnAck5(connackv5::MqttConnAck {
            session_present: false,
            reason_code: 0,
            properties: Some(Vec::new()),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::CONNACK);
    }

    #[test]
    fn test_packet_type_v5_publish() {
        let packet = MqttPacket::Publish5(publishv5::MqttPublish {
            dup: false,
            qos: 0,
            retain: false,
            topic_name: "test/topic".to_string(),
            packet_id: None,
            properties: Vec::new(),
            payload: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBLISH);
    }

    #[test]
    fn test_packet_type_v5_puback() {
        let packet = MqttPacket::PubAck5(pubackv5::MqttPubAck {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBACK);
    }

    #[test]
    fn test_packet_type_v5_pubrec() {
        let packet = MqttPacket::PubRec5(pubrecv5::MqttPubRec {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBREC);
    }

    #[test]
    fn test_packet_type_v5_pubrel() {
        let packet = MqttPacket::PubRel5(pubrelv5::MqttPubRel {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBREL);
    }

    #[test]
    fn test_packet_type_v5_pubcomp() {
        let packet = MqttPacket::PubComp5(pubcompv5::MqttPubComp {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBCOMP);
    }

    #[test]
    fn test_packet_type_v5_subscribe() {
        let packet = MqttPacket::Subscribe5(subscribev5::MqttSubscribe {
            packet_id: 1,
            properties: Vec::new(),
            subscriptions: vec![subscribev5::TopicSubscription {
                topic_filter: "test/#".to_string(),
                qos: 1,
                no_local: false,
                retain_as_published: false,
                retain_handling: 0,
            }],
        });
        assert_eq!(packet.packet_type(), ControlPacketType::SUBSCRIBE);
    }

    #[test]
    fn test_packet_type_v5_suback() {
        let packet = MqttPacket::SubAck5(subackv5::MqttSubAck {
            packet_id: 1,
            properties: Vec::new(),
            reason_codes: vec![0],
        });
        assert_eq!(packet.packet_type(), ControlPacketType::SUBACK);
    }

    #[test]
    fn test_packet_type_v5_unsubscribe() {
        let packet = MqttPacket::Unsubscribe5(unsubscribev5::MqttUnsubscribe {
            packet_id: 1,
            properties: Vec::new(),
            topic_filters: vec!["test/#".to_string()],
        });
        assert_eq!(packet.packet_type(), ControlPacketType::UNSUBSCRIBE);
    }

    #[test]
    fn test_packet_type_v5_unsuback() {
        let packet = MqttPacket::UnsubAck5(unsubackv5::MqttUnsubAck {
            packet_id: 1,
            properties: Vec::new(),
            reason_codes: vec![0],
        });
        assert_eq!(packet.packet_type(), ControlPacketType::UNSUBACK);
    }

    #[test]
    fn test_packet_type_v5_pingreq() {
        let packet = MqttPacket::PingReq5(pingreqv5::MqttPingReq);
        assert_eq!(packet.packet_type(), ControlPacketType::PINGREQ);
    }

    #[test]
    fn test_packet_type_v5_pingresp() {
        let packet = MqttPacket::PingResp5(pingrespv5::MqttPingResp);
        assert_eq!(packet.packet_type(), ControlPacketType::PINGRESP);
    }

    #[test]
    fn test_packet_type_v5_disconnect() {
        let packet = MqttPacket::Disconnect5(disconnectv5::MqttDisconnect {
            reason_code: 0,
            properties: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::DISCONNECT);
    }

    #[test]
    fn test_packet_type_v5_auth() {
        let packet = MqttPacket::Auth(authv5::MqttAuth {
            reason_code: 0,
            properties: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::AUTH);
    }

    // Test packet_type() for all MQTTv3 packet variants
    #[test]
    fn test_packet_type_v3_connect() {
        let packet = MqttPacket::Connect3(connectv3::MqttConnect {
            protocol_name: "MQTT".to_string(),
            protocol_version: 4,
            keep_alive: 60,
            client_id: "test".to_string(),
            will: None,
            username: None,
            password: None,
            clean_session: true,
        });
        assert_eq!(packet.packet_type(), ControlPacketType::CONNECT);
    }

    #[test]
    fn test_packet_type_v3_connack() {
        let packet = MqttPacket::ConnAck3(connackv3::MqttConnAck {
            session_present: false,
            return_code: 0,
        });
        assert_eq!(packet.packet_type(), ControlPacketType::CONNACK);
    }

    #[test]
    fn test_packet_type_v3_publish() {
        let packet = MqttPacket::Publish3(publishv3::MqttPublish {
            dup: false,
            qos: 0,
            retain: false,
            topic_name: "test/topic".to_string(),
            message_id: None,
            payload: Vec::new(),
        });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBLISH);
    }

    #[test]
    fn test_packet_type_v3_puback() {
        let packet = MqttPacket::PubAck3(pubackv3::MqttPubAck { message_id: 1 });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBACK);
    }

    #[test]
    fn test_packet_type_v3_pubrec() {
        let packet = MqttPacket::PubRec3(pubrecv3::MqttPubRec { message_id: 1 });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBREC);
    }

    #[test]
    fn test_packet_type_v3_pubrel() {
        let packet = MqttPacket::PubRel3(pubrelv3::MqttPubRel { message_id: 1 });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBREL);
    }

    #[test]
    fn test_packet_type_v3_pubcomp() {
        let packet = MqttPacket::PubComp3(pubcompv3::MqttPubComp { message_id: 1 });
        assert_eq!(packet.packet_type(), ControlPacketType::PUBCOMP);
    }

    #[test]
    fn test_packet_type_v3_subscribe() {
        let packet = MqttPacket::Subscribe3(subscribev3::MqttSubscribe {
            message_id: 1,
            subscriptions: vec![subscribev3::SubscriptionTopic {
                topic_filter: "test/#".to_string(),
                qos: 1,
            }],
        });
        assert_eq!(packet.packet_type(), ControlPacketType::SUBSCRIBE);
    }

    #[test]
    fn test_packet_type_v3_suback() {
        let packet = MqttPacket::SubAck3(subackv3::MqttSubAck {
            message_id: 1,
            return_codes: vec![1],
        });
        assert_eq!(packet.packet_type(), ControlPacketType::SUBACK);
    }

    #[test]
    fn test_packet_type_v3_unsubscribe() {
        let packet = MqttPacket::Unsubscribe3(unsubscribev3::MqttUnsubscribe {
            message_id: 1,
            topic_filters: vec!["test/#".to_string()],
        });
        assert_eq!(packet.packet_type(), ControlPacketType::UNSUBSCRIBE);
    }

    #[test]
    fn test_packet_type_v3_unsuback() {
        let packet = MqttPacket::UnsubAck3(unsubackv3::MqttUnsubAck { message_id: 1 });
        assert_eq!(packet.packet_type(), ControlPacketType::UNSUBACK);
    }

    #[test]
    fn test_packet_type_v3_pingreq() {
        let packet = MqttPacket::PingReq3(pingreqv3::MqttPingReq);
        assert_eq!(packet.packet_type(), ControlPacketType::PINGREQ);
    }

    #[test]
    fn test_packet_type_v3_pingresp() {
        let packet = MqttPacket::PingResp3(pingrespv3::MqttPingResp);
        assert_eq!(packet.packet_type(), ControlPacketType::PINGRESP);
    }

    #[test]
    fn test_packet_type_v3_disconnect() {
        let packet = MqttPacket::Disconnect3(disconnectv3::MqttDisconnect {});
        assert_eq!(packet.packet_type(), ControlPacketType::DISCONNECT);
    }

    // Test that V3 and V5 variants of the same packet type return the same ControlPacketType
    #[test]
    fn test_packet_type_consistency_connect() {
        let v5 = MqttPacket::Connect5(connectv5::MqttConnect {
            protocol_name: "MQTT".to_string(),
            protocol_version: 5,
            keep_alive: 60,
            client_id: "test".to_string(),
            will: None,
            username: None,
            password: None,
            properties: Vec::new(),
            clean_start: true,
        });
        let v3 = MqttPacket::Connect3(connectv3::MqttConnect {
            protocol_name: "MQTT".to_string(),
            protocol_version: 4,
            keep_alive: 60,
            client_id: "test".to_string(),
            will: None,
            username: None,
            password: None,
            clean_session: true,
        });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_connack() {
        let v5 = MqttPacket::ConnAck5(connackv5::MqttConnAck {
            session_present: false,
            reason_code: 0,
            properties: Some(Vec::new()),
        });
        let v3 = MqttPacket::ConnAck3(connackv3::MqttConnAck {
            session_present: false,
            return_code: 0,
        });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_publish() {
        let v5 = MqttPacket::Publish5(publishv5::MqttPublish {
            dup: false,
            qos: 0,
            retain: false,
            topic_name: "test".to_string(),
            packet_id: None,
            properties: Vec::new(),
            payload: Vec::new(),
        });
        let v3 = MqttPacket::Publish3(publishv3::MqttPublish {
            dup: false,
            qos: 0,
            retain: false,
            topic_name: "test".to_string(),
            message_id: None,
            payload: Vec::new(),
        });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_puback() {
        let v5 = MqttPacket::PubAck5(pubackv5::MqttPubAck {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        let v3 = MqttPacket::PubAck3(pubackv3::MqttPubAck { message_id: 1 });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_pubrec() {
        let v5 = MqttPacket::PubRec5(pubrecv5::MqttPubRec {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        let v3 = MqttPacket::PubRec3(pubrecv3::MqttPubRec { message_id: 1 });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_pubrel() {
        let v5 = MqttPacket::PubRel5(pubrelv5::MqttPubRel {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        let v3 = MqttPacket::PubRel3(pubrelv3::MqttPubRel { message_id: 1 });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_pubcomp() {
        let v5 = MqttPacket::PubComp5(pubcompv5::MqttPubComp {
            packet_id: 1,
            reason_code: 0,
            properties: Vec::new(),
        });
        let v3 = MqttPacket::PubComp3(pubcompv3::MqttPubComp { message_id: 1 });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_subscribe() {
        let v5 = MqttPacket::Subscribe5(subscribev5::MqttSubscribe {
            packet_id: 1,
            properties: Vec::new(),
            subscriptions: vec![subscribev5::TopicSubscription {
                topic_filter: "test".to_string(),
                qos: 1,
                no_local: false,
                retain_as_published: false,
                retain_handling: 0,
            }],
        });
        let v3 = MqttPacket::Subscribe3(subscribev3::MqttSubscribe {
            message_id: 1,
            subscriptions: vec![subscribev3::SubscriptionTopic {
                topic_filter: "test".to_string(),
                qos: 1,
            }],
        });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_suback() {
        let v5 = MqttPacket::SubAck5(subackv5::MqttSubAck {
            packet_id: 1,
            properties: Vec::new(),
            reason_codes: vec![0],
        });
        let v3 = MqttPacket::SubAck3(subackv3::MqttSubAck {
            message_id: 1,
            return_codes: vec![1],
        });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_unsubscribe() {
        let v5 = MqttPacket::Unsubscribe5(unsubscribev5::MqttUnsubscribe {
            packet_id: 1,
            properties: Vec::new(),
            topic_filters: vec!["test".to_string()],
        });
        let v3 = MqttPacket::Unsubscribe3(unsubscribev3::MqttUnsubscribe {
            message_id: 1,
            topic_filters: vec!["test".to_string()],
        });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_unsuback() {
        let v5 = MqttPacket::UnsubAck5(unsubackv5::MqttUnsubAck {
            packet_id: 1,
            properties: Vec::new(),
            reason_codes: vec![0],
        });
        let v3 = MqttPacket::UnsubAck3(unsubackv3::MqttUnsubAck { message_id: 1 });
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_pingreq() {
        let v5 = MqttPacket::PingReq5(pingreqv5::MqttPingReq);
        let v3 = MqttPacket::PingReq3(pingreqv3::MqttPingReq);
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_pingresp() {
        let v5 = MqttPacket::PingResp5(pingrespv5::MqttPingResp);
        let v3 = MqttPacket::PingResp3(pingrespv3::MqttPingResp);
        assert_eq!(v5.packet_type(), v3.packet_type());
    }

    #[test]
    fn test_packet_type_consistency_disconnect() {
        let v5 = MqttPacket::Disconnect5(disconnectv5::MqttDisconnect {
            reason_code: 0,
            properties: Vec::new(),
        });
        let v3 = MqttPacket::Disconnect3(disconnectv3::MqttDisconnect {});
        assert_eq!(v5.packet_type(), v3.packet_type());
    }
}
