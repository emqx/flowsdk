// SPDX-License-Identifier: MPL-2.0

#![no_main]

use libfuzzer_sys::fuzz_target;
extern crate flowsdk;
fuzz_target!(|data: &[u8]| {
    flowsdk::mqtt_serde::parser::parse_remaining_length(data).ok();
    flowsdk::mqtt_serde::parser::parse_utf8_string(data).ok();
    flowsdk::mqtt_serde::parser::parse_topic_name(data).ok();
    flowsdk::mqtt_serde::parser::parse_packet_id(data).ok();
    flowsdk::mqtt_serde::parser::parse_vbi(data).ok();
    flowsdk::mqtt_serde::parser::parse_binary_data(data).ok();
    flowsdk::mqtt_serde::parser::packet_type(data).ok();
});
