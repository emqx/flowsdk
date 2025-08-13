use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

use super::encode_variable_length;
use super::parser::packet_type;
use super::parser::{ParseError, ParseOk};

use crate::mqtt_serde::mqttv5;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[serde(tag = "type")]
pub enum MqttPacket {
    Connect(mqttv5::connect::MqttConnect),
    ConnAck(mqttv5::connack::MqttConnAck),
    Publish(mqttv5::publish::MqttPublish),
    PubAck(mqttv5::puback::MqttPubAck),
    PubRec(mqttv5::pubrec::MqttPubRec),
    PubRel(mqttv5::pubrel::MqttPubRel),
    PubComp(mqttv5::pubcomp::MqttPubComp),
    Subscribe(mqttv5::subscribe::MqttSubscribe),
    SubAck(mqttv5::suback::MqttSubAck),
    Unsubscribe(mqttv5::unsubscribe::MqttUnsubscribe),
    UnsubAck(mqttv5::unsuback::MqttUnsubAck),
    PingReq(mqttv5::pingreq::MqttPingReq),
    PingResp(mqttv5::pingresp::MqttPingResp),
    Disconnect(mqttv5::disconnect::MqttDisconnect),
    Auth(mqttv5::auth::MqttAuth),
}

impl MqttPacket {
    pub fn to_bytes(&self) -> Result<Vec<u8>, ParseError> {
        match self {
            MqttPacket::Connect(p) => p.to_bytes(),
            MqttPacket::ConnAck(p) => p.to_bytes(),
            MqttPacket::Publish(p) => p.to_bytes(),
            MqttPacket::PubAck(p) => p.to_bytes(),
            MqttPacket::PubRec(p) => p.to_bytes(),
            MqttPacket::PubRel(p) => p.to_bytes(),
            MqttPacket::PubComp(p) => p.to_bytes(),
            MqttPacket::Subscribe(p) => p.to_bytes(),
            MqttPacket::SubAck(p) => p.to_bytes(),
            MqttPacket::Unsubscribe(p) => p.to_bytes(),
            MqttPacket::UnsubAck(p) => p.to_bytes(),
            MqttPacket::PingReq(p) => p.to_bytes(),
            MqttPacket::PingResp(p) => p.to_bytes(),
            MqttPacket::Disconnect(p) => p.to_bytes(),
            MqttPacket::Auth(p) => p.to_bytes(),
        }
    }

    pub fn from_bytes(buffer: &[u8]) -> Result<ParseOk, ParseError> {
        let packet_type_byte = packet_type(buffer)?;
        let packet_type = ControlPacketType::try_from(packet_type_byte)?;

        match packet_type {
            ControlPacketType::CONNECT => mqttv5::connect::MqttConnect::from_bytes(buffer),
            ControlPacketType::CONNACK => mqttv5::connack::MqttConnAck::from_bytes(buffer),
            ControlPacketType::PUBLISH => mqttv5::publish::MqttPublish::from_bytes(buffer),
            ControlPacketType::PUBACK => mqttv5::puback::MqttPubAck::from_bytes(buffer),
            ControlPacketType::PUBREC => mqttv5::pubrec::MqttPubRec::from_bytes(buffer),
            ControlPacketType::PUBREL => mqttv5::pubrel::MqttPubRel::from_bytes(buffer),
            ControlPacketType::PUBCOMP => mqttv5::pubcomp::MqttPubComp::from_bytes(buffer),
            ControlPacketType::SUBSCRIBE => mqttv5::subscribe::MqttSubscribe::from_bytes(buffer),
            ControlPacketType::SUBACK => mqttv5::suback::MqttSubAck::from_bytes(buffer),
            ControlPacketType::UNSUBSCRIBE => {
                mqttv5::unsubscribe::MqttUnsubscribe::from_bytes(buffer)
            }
            ControlPacketType::UNSUBACK => mqttv5::unsuback::MqttUnsubAck::from_bytes(buffer),
            ControlPacketType::PINGREQ => mqttv5::pingreq::MqttPingReq::from_bytes(buffer),
            ControlPacketType::PINGRESP => mqttv5::pingresp::MqttPingResp::from_bytes(buffer),
            ControlPacketType::DISCONNECT => mqttv5::disconnect::MqttDisconnect::from_bytes(buffer),
            ControlPacketType::AUTH => mqttv5::auth::MqttAuth::from_bytes(buffer),
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
    let pkt = MqttPacket::Connect(mqttv5::connect::MqttConnect {
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

    let expected ="{\"type\":\"Connect\",\"protocol_name\":\"MQTT\",\"protocol_version\":5,\"keep_alive\":60,\"client_id\":\"test_client\",\"will\":null,\"username\":null,\"password\":null,\"properties\":[],\"clean_start\":true}";
    assert_eq!(json, expected);
}
