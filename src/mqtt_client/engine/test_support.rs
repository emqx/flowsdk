// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::mqtt_serde::mqttv5::connackv5::MqttConnAck;

pub(super) fn connect(engine: &mut MqttEngine, properties: Vec<Property>) -> Vec<MqttEvent> {
    engine.connect().unwrap();
    engine.take_outgoing();
    let ack = if engine.mqtt_version() == 5 {
        MqttPacket::ConnAck5(MqttConnAck::new(false, 0, Some(properties)))
            .to_bytes()
            .unwrap()
    } else {
        vec![0x20, 2, 0, 0]
    };
    let events = engine.handle_incoming(&ack);
    assert!(engine.is_connected(), "{events:?}");
    events
}
