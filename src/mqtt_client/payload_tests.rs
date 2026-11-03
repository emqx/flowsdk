// SPDX-License-Identifier: Apache-2.0

use super::PublishCommand;
use bytes::Bytes;
use std::sync::{Arc, Weak};

struct Owner(Arc<Vec<u8>>);

impl AsRef<[u8]> for Owner {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// The weak reference observes the backing allocation, not just a Bytes handle.
pub(crate) fn tracked_payload(size: usize) -> (Bytes, Weak<Vec<u8>>) {
    let data = Arc::new(vec![42; size]);
    let weak = Arc::downgrade(&data);
    (Bytes::from_owner(Owner(data)), weak)
}

#[test]
fn shared_payload_all_conversions_keep_the_allocation_alive() {
    for qos in 0..=2 {
        let (backing, weak) = tracked_payload(65536);
        let slice = backing.slice(17..4113);
        let ptr = slice.as_ptr();
        let command = PublishCommand::from_bytes("topic", slice, qos, false);
        let cloned = command.clone();
        drop(command);
        let mut command = cloned;
        command.packet_id = (qos > 0).then_some(1);
        let borrowed_v3 = command.to_mqttv3_publish();
        let borrowed_v5 = command.to_mqtt_publish();
        let owned_v3 = command.clone().into_mqttv3_publish();
        let owned_v5 = command.into_mqtt_publish();
        for payload in [
            &borrowed_v3.payload,
            &borrowed_v5.payload,
            &owned_v3.payload,
            &owned_v5.payload,
        ] {
            assert_eq!(payload.as_ptr(), ptr);
            assert_eq!(payload.as_ref(), &[42; 4096]);
        }
        drop(backing);
        drop((borrowed_v3, borrowed_v5, owned_v3));
        assert!(weak.upgrade().is_some());
        assert_eq!(owned_v5.payload.as_ref(), &[42; 4096]);
        drop(owned_v5);
        assert!(weak.upgrade().is_none());
    }
}

#[test]
fn shared_payload_vec_inputs_transfer_allocation_including_spare_capacity() {
    for capacity in [4096, 8192] {
        let mut data = Vec::with_capacity(capacity);
        data.resize(4096, 7);
        let ptr = data.as_ptr();
        let command = PublishCommand::simple("topic", data, 0, false);
        assert_eq!(command.payload.as_ptr(), ptr);
        let command = PublishCommand::builder()
            .topic("topic")
            .payload_bytes(command.payload)
            .build()
            .unwrap();
        assert_eq!(command.payload.as_ptr(), ptr);
        let packet = command.into_mqtt_publish();
        assert_eq!(packet.payload.as_ptr(), ptr);
        let mut data = Vec::with_capacity(capacity);
        data.resize(4096, 7);
        let ptr = data.as_ptr();
        let command = PublishCommand::builder()
            .topic("topic")
            .payload(data)
            .build()
            .unwrap();
        assert_eq!(command.payload.as_ptr(), ptr);
    }
}

#[test]
fn shared_payload_serde_preserves_legacy_sequence_shape() {
    // This is the 0.7 command representation, including non-UTF-8 payload bytes.
    let legacy = serde_json::json!({
        "topic_name": "t", "payload": [0, 255, 128], "qos": 1,
        "retain": false, "dup": false, "packet_id": 7,
        "properties": [], "priority": 128
    });
    let command: PublishCommand = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&command).unwrap(), legacy);
    for packet in [
        crate::mqtt_serde::control_packet::MqttPacket::Publish3(command.to_mqttv3_publish()),
        crate::mqtt_serde::control_packet::MqttPacket::Publish5(command.into_mqtt_publish()),
    ] {
        let json = serde_json::to_value(&packet).unwrap();
        assert_eq!(json["payload"], serde_json::json!([0, 255, 128]));
        let decoded: crate::mqtt_serde::control_packet::MqttPacket =
            serde_json::from_value(json).unwrap();
        assert_eq!(decoded.to_bytes().unwrap(), packet.to_bytes().unwrap());
    }
}
