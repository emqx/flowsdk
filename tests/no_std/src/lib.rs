// SPDX-License-Identifier: Apache-2.0
#![no_std]

extern crate alloc;

use alloc::{boxed::Box, vec, vec::Vec};
use flowsdk::{
    mqtt_client::{MqttClientOptions, MqttEvent, PortableNoIoMqttClient, PublishCommand},
    time::Timestamp,
};

pub fn time(ms: u64) -> Timestamp {
    Timestamp::try_from_millis(ms).unwrap()
}

pub fn options(version: u8) -> MqttClientOptions {
    MqttClientOptions::builder()
        .mqtt_version(version)
        .client_id("portable")
        .keep_alive(0)
        .parser_buffer_size(256)
        .max_incoming_buffer_bytes(512)
        .max_outgoing_buffer_bytes(1024)
        .max_outgoing_packet_count(4)
        .max_event_count(8)
        .build()
}

pub fn connack(version: u8) -> &'static [u8] {
    if version == 5 {
        &[0x20, 3, 0, 0, 0]
    } else {
        &[0x20, 2, 0, 0]
    }
}

#[inline(never)]
pub fn connected(options: MqttClientOptions, now: Timestamp) -> PortableNoIoMqttClient {
    let version = options.mqtt_version;
    let mut client = PortableNoIoMqttClient::try_new_at(options, now).unwrap();
    client.connect_at(now).unwrap();
    assert_eq!(client.take_outgoing_at(now).unwrap()[0], 0x10);
    assert!(client
        .handle_incoming_at(connack(version), now)
        .unwrap()
        .iter()
        .any(|event| matches!(event, MqttEvent::Connected(_))));
    client
}

pub fn ack(kind: u8, id: u16) -> [u8; 4] {
    [kind, 2, (id >> 8) as u8, id as u8]
}

/// Executed both by the host harness with SDK `std` disabled and by Zephyr.
pub fn protocol_smoke() {
    for version in [3, 4, 5] {
        let now = time(123_456);
        let mut client = smoke_client(version, now);
        for qos in [0, 1, 2] {
            let id = client
                .publish_at(PublishCommand::simple("t", vec![42], qos, false), now)
                .unwrap();
            let wire = client.take_outgoing_at(now).unwrap();
            assert_eq!(wire[0] & 0xf0, 0x30);
            if let Some(id) = id {
                let events = client
                    .handle_incoming_at(&ack(if qos == 1 { 0x40 } else { 0x50 }, id), now)
                    .unwrap();
                if qos == 1 {
                    assert!(
                        events.iter().any(|e| matches!(e, MqttEvent::Published(_))),
                        "version={version} id={id} events={events:?}"
                    );
                } else {
                    assert_eq!(client.take_outgoing_at(now).unwrap()[0], 0x62);
                    assert!(client
                        .handle_incoming_at(&ack(0x70, id), now)
                        .unwrap()
                        .iter()
                        .any(|e| matches!(e, MqttEvent::Published(_))));
                }
            }
        }
        client.disconnect_at(now).unwrap();
        assert_eq!(client.take_outgoing_at(now).unwrap()[0], 0xe0);
    }
}

// Avoid inlining construction into the protocol loop on small ARM stacks.
#[inline(never)]
fn smoke_client(version: u8, now: Timestamp) -> Box<PortableNoIoMqttClient> {
    Box::new(connected(options(version), now))
}

/// Force heap-backed shared Bytes, including refcount increments and final drop.
#[cfg(feature = "shared-payloads")]
pub fn bytes_smoke() {
    let mut data = Vec::new();
    data.extend_from_slice(&[7; 64]);
    let bytes = bytes::Bytes::from(data);
    let clone = bytes.clone();
    let slice = clone.slice(4..12);
    let command = PublishCommand::from_bytes("shared", slice.clone(), 1, false);
    assert_eq!(command.payload.as_ptr(), slice.as_ptr());
    let packet = command.into_mqtt_publish();
    assert_eq!(packet.payload.as_ptr(), slice.as_ptr());
    drop(bytes);
    drop(clone);
    assert_eq!(slice.as_ref(), &[7; 8]);
    drop(slice);
    assert_eq!(&packet.payload[..], &[7; 8]);
}

/// Owned payloads move into packets without replacing their allocation.
pub fn owned_payload_smoke() {
    for version in [4, 5] {
        let mut data = Vec::with_capacity(128);
        data.extend_from_slice(&[7; 64]);
        let ptr = data.as_ptr();
        let command = PublishCommand::simple("owned", data, 1, false);
        assert_eq!(command.payload.as_ptr(), ptr);
        let payload = if version == 4 {
            command.into_mqttv3_publish().payload
        } else {
            command.into_mqtt_publish().payload
        };
        assert_eq!(payload.as_ptr(), ptr);
        assert_eq!(&payload[..], &[7; 64]);
    }
}
