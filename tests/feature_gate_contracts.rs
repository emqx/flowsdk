// SPDX-License-Identifier: MPL-2.0

use flowsdk::mqtt_serde::{
    control_packet::MqttControlPacket,
    mqttv5::{
        publishv5::MqttPublish,
        subscribev5::{MqttSubscribe, TopicSubscription},
    },
};

#[test]
fn strict_validation_follows_the_feature_gate() {
    let strict = cfg!(feature = "strict-protocol-compliance");
    let publish = MqttPublish::new(0, "topic".into(), None, vec![], false, false);
    let mut bytes = publish.to_bytes().unwrap();
    bytes[0] |= 0x08; // DUP on QoS 0 is rejected by strict validation.
    assert_eq!(MqttPublish::from_bytes(&bytes).is_err(), strict);

    let subscribe = MqttSubscribe::new_simple(
        0, // Packet ID zero is rejected by strict validation.
        vec![TopicSubscription::new_simple("topic".into(), 1)],
    );
    assert_eq!(subscribe.to_bytes().is_err(), strict);
}
