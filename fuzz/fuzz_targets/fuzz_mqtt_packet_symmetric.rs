#![no_main]

use libfuzzer_sys::fuzz_target;
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::parser::ParseOk;

fuzz_target!(|packet: MqttPacket| {
    if let Ok(bytes) = packet.to_bytes() {
        if let Ok(ParseOk::Packet(decoded_packet, 0)) = MqttPacket::from_bytes(&bytes) {
            assert_eq!(packet, decoded_packet);
        }
    }
});
