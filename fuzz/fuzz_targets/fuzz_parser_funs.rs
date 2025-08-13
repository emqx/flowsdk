#![no_main]

use libfuzzer_sys::fuzz_target;
extern crate mqtt_grpc_duality;
fuzz_target!(|data: &[u8]| {
    mqtt_grpc_duality::mqtt_serde::parser::parse_remaining_length(data).ok();
    mqtt_grpc_duality::mqtt_serde::parser::parse_utf8_string(data).ok();
    mqtt_grpc_duality::mqtt_serde::parser::parse_topic_name(data).ok();
    mqtt_grpc_duality::mqtt_serde::parser::parse_packet_id(data).ok();
    mqtt_grpc_duality::mqtt_serde::parser::parse_vbi(data).ok();
    mqtt_grpc_duality::mqtt_serde::parser::parse_binary_data(data).ok();
    mqtt_grpc_duality::mqtt_serde::parser::packet_type(data).ok();
});
