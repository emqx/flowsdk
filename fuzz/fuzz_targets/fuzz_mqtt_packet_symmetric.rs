#![no_main]

use libfuzzer_sys::fuzz_target;
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::parser::ParseOk;

fuzz_target!(|packet: MqttPacket| {
    if let Ok(bytes) = packet.to_bytes() {
        // @TODO support from_bytes v3 or v5
        let vsn = match packet {
            MqttPacket::Connect3(_) | MqttPacket::ConnAck3(_) | MqttPacket::Publish3(_) | MqttPacket::PubAck3(_) | MqttPacket::PubRec3(_) | MqttPacket::PubRel3(_)
            | MqttPacket::PubComp3(_) | MqttPacket::Subscribe3(_) | MqttPacket::SubAck3(_) | MqttPacket::Unsubscribe3(_) | MqttPacket::UnsubAck3(_)
            | MqttPacket::PingReq3(_) | MqttPacket::PingResp3(_) | MqttPacket::Disconnect3(_) => {
                3
            }
            MqttPacket::Connect5(_) | MqttPacket::ConnAck5(_) | MqttPacket::Publish5(_) | MqttPacket::PubAck5(_) | MqttPacket::PubRec5(_) | MqttPacket::PubRel5(_)
            | MqttPacket::PubComp5(_) | MqttPacket::Subscribe5(_) | MqttPacket::SubAck5(_) | MqttPacket::Unsubscribe5(_) | MqttPacket::UnsubAck5(_)
            | MqttPacket::PingReq5(_) | MqttPacket::PingResp5(_) | MqttPacket::Disconnect5(_) | MqttPacket::Auth(_) => {
                5
            }
        };
        if let Ok(ParseOk::Packet(decoded_packet, 0)) = MqttPacket::from_bytes_with_version(&bytes, vsn) {
                    assert_eq!(packet, decoded_packet);
                }
    }
});
