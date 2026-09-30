// SPDX-License-Identifier: MPL-2.0

// Each benchmark is a separate crate and uses a subset of these fixtures.
#![allow(dead_code)]

use flowsdk::mqtt_client::PublishCommand;
use flowsdk::mqtt_serde::control_packet::{ControlPacketType, MqttPacket};
use flowsdk::mqtt_serde::parser::ParseOk;
use flowsdk::mqtt_serde::{mqttv3 as v3, mqttv5 as v5};

pub const VERSIONS: [(u8, &str); 2] = [(4, "v3_1_1"), (5, "v5")];
pub const PAYLOAD_SIZES: [usize; 4] = [0, 256, 4096, 65536];
pub const TOPIC: &str = "bench/sensors/temperature";
pub const PACKET_ID: u16 = 1;

pub fn command(qos: u8, size: usize, packet_id: u16) -> PublishCommand {
    let mut command = PublishCommand::simple(TOPIC, vec![0x5a; size], qos, false);
    command.packet_id = (qos > 0).then_some(packet_id);
    command
}

pub fn publish(version: u8, qos: u8, size: usize, packet_id: u16) -> MqttPacket {
    let command = command(qos, size, packet_id);
    match version {
        4 => MqttPacket::Publish3(command.to_mqttv3_publish()),
        5 => MqttPacket::Publish5(command.to_mqtt_publish()),
        _ => unreachable!("unsupported benchmark protocol"),
    }
}

/// Validate fixtures outside the timed loop, including complete consumption.
pub fn encode_checked(packet: &MqttPacket, version: u8) -> Vec<u8> {
    let bytes = packet.to_bytes().unwrap();
    match MqttPacket::from_bytes_with_version(&bytes, version).unwrap() {
        ParseOk::Packet(decoded, consumed) => {
            assert_eq!(&decoded, packet);
            assert_eq!(consumed, bytes.len());
        }
        result => panic!("fixture is not a complete packet: {result:?}"),
    }
    bytes
}

pub fn ack(version: u8, kind: ControlPacketType, packet_id: u16) -> MqttPacket {
    use ControlPacketType::{PUBACK, PUBCOMP, PUBREC, PUBREL};
    match (version, kind) {
        (4, PUBACK) => MqttPacket::PubAck3(v3::puback::MqttPubAck::new(packet_id)),
        (4, PUBREC) => MqttPacket::PubRec3(v3::pubrec::MqttPubRec::new(packet_id)),
        (4, PUBREL) => MqttPacket::PubRel3(v3::pubrel::MqttPubRel::new(packet_id)),
        (4, PUBCOMP) => MqttPacket::PubComp3(v3::pubcomp::MqttPubComp::new(packet_id)),
        (5, PUBACK) => MqttPacket::PubAck5(v5::puback::MqttPubAck::new(packet_id, 0, vec![])),
        (5, PUBREC) => MqttPacket::PubRec5(v5::pubrec::MqttPubRec::new(packet_id, 0, vec![])),
        (5, PUBREL) => MqttPacket::PubRel5(v5::pubrel::MqttPubRel::new(packet_id, 0, vec![])),
        (5, PUBCOMP) => MqttPacket::PubComp5(v5::pubcomp::MqttPubComp::new(packet_id, 0, vec![])),
        _ => unreachable!("unsupported benchmark acknowledgement"),
    }
}

pub fn control_packets(version: u8) -> Vec<(&'static str, MqttPacket)> {
    let (connect, subscribe) = match version {
        4 => (
            MqttPacket::Connect3(v3::connect::MqttConnect::new("microbench".into(), 60, true)),
            MqttPacket::Subscribe3(v3::subscribe::MqttSubscribe::new(
                PACKET_ID,
                vec![v3::subscribe::SubscriptionTopic {
                    topic_filter: "bench/sensors/+".into(),
                    qos: 1,
                }],
            )),
        ),
        5 => (
            MqttPacket::Connect5(v5::connect::MqttConnect::new(
                "microbench".into(),
                None,
                None,
                None,
                60,
                true,
                vec![],
            )),
            MqttPacket::Subscribe5(v5::subscribe::MqttSubscribe::new_simple(
                PACKET_ID,
                vec![v5::subscribe::TopicSubscription::new_simple(
                    "bench/sensors/+".into(),
                    1,
                )],
            )),
        ),
        _ => unreachable!("unsupported benchmark protocol"),
    };
    vec![
        ("connect", connect),
        ("subscribe", subscribe),
        ("puback", ack(version, ControlPacketType::PUBACK, PACKET_ID)),
        ("pubrec", ack(version, ControlPacketType::PUBREC, PACKET_ID)),
        ("pubrel", ack(version, ControlPacketType::PUBREL, PACKET_ID)),
        (
            "pubcomp",
            ack(version, ControlPacketType::PUBCOMP, PACKET_ID),
        ),
    ]
}
