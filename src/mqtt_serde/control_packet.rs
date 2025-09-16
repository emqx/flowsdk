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

    // encoder
    fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        let mut bytes = Vec::new();

        let vhdr = self.variable_header()?;
        let payload = self.payload()?;
        let remaining_length = vhdr.len() + payload.len();
        bytes.extend(self.fixed_header(remaining_length));
        bytes.extend(vhdr);
        bytes.extend(payload);
        Ok(bytes)
    }
}

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
