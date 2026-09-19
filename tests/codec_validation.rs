// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "strict-protocol-compliance")]

use flowsdk::mqtt_client::{MqttClientOptions, MqttEvent, NoIoMqttClient};
use flowsdk::mqtt_serde::{
    control_packet::{MqttControlPacket, MqttPacket},
    mqttv3, mqttv5,
    parser::ParseOk,
};

fn frame(header: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![header];
    let mut len = body.len();
    loop {
        let byte = (len % 128) as u8;
        len /= 128;
        bytes.push(byte | if len > 0 { 128 } else { 0 });
        if len == 0 {
            break;
        }
    }
    bytes.extend(body);
    bytes
}

fn string(value: &str) -> Vec<u8> {
    let mut bytes = (value.len() as u16).to_be_bytes().to_vec();
    bytes.extend(value.as_bytes());
    bytes
}

fn publish(version: u8, topic: &str, qos: u8, id: u16, dup: bool) -> Vec<u8> {
    let mut body = string(topic);
    if qos > 0 {
        body.extend(id.to_be_bytes());
    }
    if version == 5 {
        body.push(0);
    }
    body.extend(b"payload\0is binary");
    frame(0x30 | (qos << 1) | (u8::from(dup) << 3), &body)
}

fn connect3(flags: u8) -> Vec<u8> {
    let mut body = string("MQTT");
    body.extend([4, flags, 0, 60]);
    body.extend(string("client"));
    if flags & 4 != 0 {
        body.extend(string("will"));
        body.extend(string("bye"));
    }
    frame(0x10, &body)
}

fn subscription(version: u8, filter: &str, id: u16, options: Option<u8>) -> Vec<u8> {
    let mut body = id.to_be_bytes().to_vec();
    if version == 5 {
        body.push(0);
    }
    body.extend(string(filter));
    if let Some(options) = options {
        body.push(options);
    }
    frame(if options.is_some() { 0x82 } else { 0xa2 }, &body)
}

macro_rules! rejects {
    ($name:ident, $version:expr, $bytes:expr) => {
        #[test]
        fn $name() {
            let bytes = $bytes;
            assert!(
                MqttPacket::from_bytes_with_version(&bytes, $version).is_err(),
                "accepted {bytes:02x?}"
            );
        }
    };
}

rejects!(v3_will_qos_without_will, 4, connect3(0x0a));
rejects!(v3_will_retain_without_will, 4, connect3(0x22));
rejects!(v3_will_qos_three, 4, connect3(0x1e));
rejects!(
    v3_failed_connack_with_session_present,
    4,
    vec![0x20, 2, 1, 1]
);
rejects!(v3_zero_publish_identifier, 4, publish(4, "a", 1, 0, false));
rejects!(v3_zero_puback_identifier, 4, vec![0x40, 2, 0, 0]);
rejects!(
    v3_zero_subscribe_identifier,
    4,
    subscription(4, "a", 0, Some(0))
);
rejects!(
    v3_zero_unsubscribe_identifier,
    4,
    subscription(4, "a", 0, None)
);
rejects!(v3_empty_subscribe, 4, vec![0x82, 2, 0, 1]);
rejects!(v3_empty_suback, 4, vec![0x90, 2, 0, 1]);
rejects!(v3_empty_unsubscribe, 4, vec![0xa2, 2, 0, 1]);
rejects!(v3_empty_publish_topic, 4, publish(4, "", 0, 0, false));
rejects!(
    v3_empty_subscribe_filter,
    4,
    subscription(4, "", 1, Some(0))
);
rejects!(v3_empty_unsubscribe_filter, 4, subscription(4, "", 1, None));
rejects!(
    v3_nonterminal_hash_filter,
    4,
    subscription(4, "a/#/b", 1, Some(0))
);
rejects!(
    v3_embedded_plus_filter,
    4,
    subscription(4, "a/b+", 1, Some(0))
);
rejects!(v3_publish_wildcard_topic, 4, publish(4, "a/+", 0, 0, false));
rejects!(v3_null_topic, 4, publish(4, "a\0b", 0, 0, false));
rejects!(v3_connect_fixed_header_flags, 4, {
    let mut b = connect3(2);
    b[0] |= 1;
    b
});
rejects!(v3_connect_reserved_connect_flag, 4, connect3(3));
rejects!(v3_qos0_dup, 4, publish(4, "a", 0, 0, true));
rejects!(v5_null_topic, 5, publish(5, "a\0b", 0, 0, false));
rejects!(v5_zero_publish_identifier, 5, publish(5, "a", 1, 0, false));
rejects!(
    v5_subscription_qos_three,
    5,
    subscription(5, "a", 1, Some(3))
);
rejects!(
    v5_retain_handling_three,
    5,
    subscription(5, "a", 1, Some(0x30))
);
rejects!(v5_puback_reserved_flags, 5, vec![0x41, 2, 0, 1]);

fn connack_properties(properties: &[u8]) -> Vec<u8> {
    let mut body = vec![0, 0, properties.len() as u8];
    body.extend(properties);
    frame(0x20, &body)
}

fn publish_properties(properties: &[u8]) -> Vec<u8> {
    let mut body = string("a");
    body.push(properties.len() as u8);
    body.extend(properties);
    frame(0x30, &body)
}

rejects!(
    v5_prohibited_connack_property,
    5,
    connack_properties(&[0x23, 0, 1])
);
rejects!(
    v5_prohibited_publish_property,
    5,
    publish_properties(&[0x21, 0, 1])
);
rejects!(
    v5_duplicate_connack_singleton,
    5,
    connack_properties(&[0x21, 0, 1, 0x21, 0, 1])
);
rejects!(
    v5_duplicate_publish_singleton,
    5,
    publish_properties(&[0x23, 0, 1, 0x23, 0, 1])
);
rejects!(
    v5_zero_receive_maximum,
    5,
    connack_properties(&[0x21, 0, 0])
);
rejects!(
    v5_zero_maximum_packet_size,
    5,
    connack_properties(&[0x27, 0, 0, 0, 0])
);
rejects!(v5_invalid_maximum_qos, 5, connack_properties(&[0x24, 2]));
rejects!(
    v5_invalid_boolean_property,
    5,
    connack_properties(&[0x25, 2])
);
rejects!(v5_zero_topic_alias, 5, publish_properties(&[0x23, 0, 0]));
rejects!(
    v5_zero_subscription_identifier,
    5,
    publish_properties(&[0x0b, 0])
);
rejects!(
    v5_invalid_payload_format_indicator,
    5,
    publish_properties(&[1, 2])
);
rejects!(
    v5_wildcard_response_topic,
    5,
    publish_properties(&[8, 0, 1, b'#'])
);
rejects!(v5_empty_response_topic, 5, publish_properties(&[8, 0, 0]));
rejects!(
    v5_duplicate_subscription_identifier_in_subscribe,
    5,
    vec![0x82, 11, 0, 1, 4, 0x0b, 1, 0x0b, 2, 0, 1, b'a', 0]
);

rejects!(v5_invalid_connack_reason, 5, vec![0x20, 3, 0, 1, 0]);
rejects!(
    v5_failed_connack_with_session_present,
    5,
    vec![0x20, 3, 1, 0x80, 0]
);
rejects!(v5_invalid_auth_reason, 5, vec![0xf0, 2, 0x80, 0]);
rejects!(v5_invalid_disconnect_reason, 5, vec![0xe0, 2, 1, 0]);
rejects!(v5_invalid_puback_reason, 5, vec![0x40, 4, 0, 1, 1, 0]);
rejects!(v5_invalid_pubrec_reason, 5, vec![0x50, 4, 0, 1, 1, 0]);
rejects!(v5_invalid_pubrel_reason, 5, vec![0x62, 4, 0, 1, 0x80, 0]);
rejects!(v5_invalid_pubcomp_reason, 5, vec![0x70, 4, 0, 1, 0x80, 0]);
rejects!(v5_invalid_suback_reason, 5, vec![0x90, 4, 0, 1, 0, 3]);
rejects!(v5_invalid_unsuback_reason, 5, vec![0xb0, 4, 0, 1, 0, 1]);

#[test]
fn endpoint_rejects_null_topic_without_delivering_the_message() {
    for version in [4, 5] {
        let mut client =
            NoIoMqttClient::new(MqttClientOptions::builder().mqtt_version(version).build());
        client.connect().unwrap();
        client.take_outgoing();
        client.handle_incoming(if version == 5 {
            &[0x20, 3, 0, 0, 0]
        } else {
            &[0x20, 2, 0, 0]
        });
        let events = client.handle_incoming(&publish(version, "a\0b", 0, 0, false));
        assert!(!client.is_connected());
        assert!(events
            .iter()
            .any(|e| matches!(e, MqttEvent::Disconnected(_))));
        assert!(!events
            .iter()
            .any(|e| matches!(e, MqttEvent::MessageReceived(_))));
    }
}

#[test]
fn leading_feff_is_preserved_by_both_codecs() {
    let topic = "\u{feff}topic";
    for packet in [
        MqttPacket::Publish3(mqttv3::publishv3::MqttPublish::new(
            topic.into(),
            0,
            vec![],
            None,
            false,
            false,
        )),
        MqttPacket::Publish5(mqttv5::publishv5::MqttPublish::new(
            0,
            topic.into(),
            None,
            vec![],
            false,
            false,
        )),
    ] {
        let version = if matches!(packet, MqttPacket::Publish3(_)) {
            4
        } else {
            5
        };
        let encoded = packet
            .to_bytes()
            .expect("U+FEFF is a valid MQTT string character");
        assert!(
            matches!(MqttPacket::from_bytes_with_version(&encoded, version).unwrap(), ParseOk::Packet(decoded, _) if decoded == packet)
        );
    }
}

#[test]
fn encoders_reject_invalid_packet_fields() {
    let packets = [
        MqttPacket::Publish3(mqttv3::publishv3::MqttPublish::new(
            "a/#".into(),
            0,
            vec![],
            None,
            false,
            false,
        )),
        MqttPacket::Publish3(mqttv3::publishv3::MqttPublish::new(
            "a".into(),
            1,
            vec![],
            Some(0),
            false,
            false,
        )),
        MqttPacket::Publish5(mqttv5::publishv5::MqttPublish::new(
            1,
            "a".into(),
            Some(0),
            vec![],
            false,
            false,
        )),
        MqttPacket::PubAck5(mqttv5::pubackv5::MqttPubAck::new(1, 1, vec![])),
    ];
    for packet in packets {
        assert!(packet.to_bytes().is_err(), "encoded {packet:?}");
    }
}

#[test]
fn typed_codecs_validate_without_enum_dispatch() {
    assert!(mqttv3::publishv3::MqttPublish::from_bytes(&publish(4, "a\0b", 0, 0, false)).is_err());
    assert!(mqttv5::pubackv5::MqttPubAck::from_bytes(&[0x40, 4, 0, 1, 1, 0]).is_err());
    assert!(mqttv5::pubackv5::MqttPubAck::new(0, 0, vec![])
        .to_bytes()
        .is_err());
}

fn packet_with_properties(kind: u8, properties: &[u8]) -> Vec<u8> {
    assert!(properties.len() < 128);
    let mut body = match kind {
        1 => {
            let mut body = string("MQTT");
            body.extend([5, 2, 0, 60]);
            body
        }
        2 => vec![0, 0],
        3 => string("a"),
        4..=7 => vec![0, 1, 0],
        8..=11 => vec![0, 1],
        14 | 15 => vec![0],
        _ => panic!("unsupported packet type"),
    };
    body.push(properties.len() as u8);
    body.extend(properties);
    match kind {
        1 => body.extend(string("client")),
        8 | 10 => {
            body.extend(string("a"));
            if kind == 8 {
                body.push(0);
            }
        }
        9 | 11 => body.push(0),
        _ => {}
    }
    frame(
        (kind << 4) | if matches!(kind, 6 | 8 | 10) { 2 } else { 0 },
        &body,
    )
}

#[test]
fn property_contexts_follow_the_mqtt5_table() {
    // MQTT 5.0 Table 2-4. The wire bytes are independent of the SDK encoder.
    let cases: &[(&[u8], &[u8])] = &[
        (&[1, 0], &[3]),
        (&[2, 0, 0, 0, 1], &[3]),
        (&[3, 0, 1, b'x'], &[3]),
        (&[8, 0, 1, b'a'], &[3]),
        (&[9, 0, 1, 0], &[3]),
        (&[0x0b, 1], &[3, 8]),
        (&[0x11, 0, 0, 0, 1], &[1, 2, 14]),
        (&[0x12, 0, 1, b'c'], &[2]),
        (&[0x13, 0, 1], &[2]),
        (&[0x15, 0, 1, b'm'], &[1, 2, 15]),
        (&[0x17, 1], &[1]),
        (&[0x18, 0, 0, 0, 1], &[]),
        (&[0x19, 1], &[1]),
        (&[0x1a, 0, 1, b'r'], &[2]),
        (&[0x1c, 0, 1, b's'], &[2, 14]),
        (&[0x1f, 0, 1, b'r'], &[2, 4, 5, 6, 7, 9, 11, 14, 15]),
        (&[0x21, 0, 1], &[1, 2]),
        (&[0x22, 0, 0], &[1, 2]),
        (&[0x23, 0, 1], &[3]),
        (&[0x24, 1], &[2]),
        (&[0x25, 1], &[2]),
        (
            &[0x26, 0, 1, b'k', 0, 1, b'v'],
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15],
        ),
        (&[0x27, 0, 0, 0, 1], &[1, 2]),
        (&[0x28, 1], &[2]),
        (&[0x29, 1], &[2]),
        (&[0x2a, 1], &[2]),
    ];
    for kind in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15] {
        for &(property, allowed) in cases {
            let bytes = packet_with_properties(kind, property);
            let result = MqttPacket::from_bytes_v5(&bytes);
            assert_eq!(
                matches!(result, Ok(ParseOk::Packet(_, _))),
                allowed.contains(&kind),
                "type {kind}, property {:02x?}: {result:?}",
                property
            );
            if allowed.contains(&kind) {
                let repeated = [property, property].concat();
                let result = MqttPacket::from_bytes_v5(&packet_with_properties(kind, &repeated));
                let repeatable = property[0] == 0x26 || (property[0] == 0x0b && kind == 3);
                assert_eq!(
                    matches!(result, Ok(ParseOk::Packet(_, _))),
                    repeatable,
                    "duplicate property {:02x} in type {kind}: {result:?}",
                    property[0]
                );
            }
        }
    }
}

fn reason_packet(kind: u8, reason: u8) -> Vec<u8> {
    let body = match kind {
        2 => vec![0, reason, 0],
        4..=7 => vec![0, 1, reason, 0],
        9 | 11 => vec![0, 1, 0, reason],
        14 | 15 => vec![reason, 0],
        _ => panic!("unsupported packet type"),
    };
    frame((kind << 4) | if kind == 6 { 2 } else { 0 }, &body)
}

macro_rules! reason_codes {
    ($name:ident, $kind:expr, [$($valid:expr),* $(,)?]) => {
        #[test]
        fn $name() {
            let valid: &[u8] = &[$($valid),*];
            for reason in 0..=255 {
                let bytes = reason_packet($kind, reason);
                let result = MqttPacket::from_bytes_v5(&bytes);
                assert_eq!(matches!(result, Ok(ParseOk::Packet(_, _))), valid.contains(&reason),
                    "reason {reason:02x}: {result:?}");
                if let Ok(ParseOk::Packet(packet, _)) = result {
                    let encoded = packet.to_bytes().unwrap();
                    assert!(matches!(MqttPacket::from_bytes_v5(&encoded).unwrap(), ParseOk::Packet(p, _) if p == packet));
                }
            }
        }
    };
}

// MQTT 5.0 per-packet reason-code tables in sections 3.2 through 3.15.
reason_codes!(
    connack_reason_code_matrix,
    2,
    [
        0, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8c, 0x90, 0x95,
        0x97, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9f
    ]
);
reason_codes!(
    puback_reason_code_matrix,
    4,
    [0, 0x10, 0x80, 0x83, 0x87, 0x90, 0x91, 0x97, 0x99]
);
reason_codes!(
    pubrec_reason_code_matrix,
    5,
    [0, 0x10, 0x80, 0x83, 0x87, 0x90, 0x91, 0x97, 0x99]
);
reason_codes!(pubrel_reason_code_matrix, 6, [0, 0x92]);
reason_codes!(pubcomp_reason_code_matrix, 7, [0, 0x92]);
reason_codes!(
    suback_reason_code_matrix,
    9,
    [0, 1, 2, 0x80, 0x83, 0x87, 0x8f, 0x91, 0x97, 0x9e, 0xa1, 0xa2]
);
reason_codes!(
    unsuback_reason_code_matrix,
    11,
    [0, 0x11, 0x80, 0x83, 0x87, 0x8f, 0x91]
);
reason_codes!(
    disconnect_reason_code_matrix,
    14,
    [
        0, 4, 0x80, 0x81, 0x82, 0x83, 0x87, 0x89, 0x8b, 0x8d, 0x8e, 0x8f, 0x90, 0x93, 0x94, 0x95,
        0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f, 0xa0, 0xa1, 0xa2
    ]
);
reason_codes!(auth_reason_code_matrix, 15, [0, 0x18, 0x19]);

fn connect_with_will_properties(properties: &[u8]) -> Vec<u8> {
    let mut body = string("MQTT");
    body.extend([5, 6, 0, 60, 0]);
    body.extend(string("client"));
    body.push(properties.len() as u8);
    body.extend(properties);
    body.extend(string("will"));
    body.extend(string("payload"));
    frame(0x10, &body)
}

rejects!(
    v5_prohibited_will_property,
    5,
    connect_with_will_properties(&[0x21, 0, 1])
);
rejects!(
    v5_duplicate_will_singleton,
    5,
    connect_with_will_properties(&[1, 0, 1, 0])
);
rejects!(
    v5_connect_authentication_data_without_method,
    5,
    packet_with_properties(1, &[0x16, 0, 1, 0])
);

#[test]
fn utf8_decoder_rejects_null_and_preserves_feff() {
    use flowsdk::mqtt_serde::parser::parse_utf8_string;
    assert!(parse_utf8_string(&string("a\0b")).is_err());
    for value in ["\u{feff}a", "a\u{feff}b", "a\u{1f321}"] {
        assert_eq!(parse_utf8_string(&string(value)).unwrap().0, value);
    }
}

#[test]
fn valid_binary_and_repeated_properties_roundtrip() {
    for version in [4, 5] {
        let bytes = publish(version, "a", 0, 0, false);
        let decoded = MqttPacket::from_bytes_with_version(&bytes, version).unwrap();
        assert!(matches!(decoded, ParseOk::Packet(_, _)));
    }
    let props = [
        0x26, 0, 1, b'k', 0, 1, b'v', 0x26, 0, 1, b'k', 0, 1, b'v', 0x0b, 1, 0x0b, 1, 9, 0, 1, 0,
    ];
    let bytes = publish_properties(&props);
    let ParseOk::Packet(packet, _) = MqttPacket::from_bytes_v5(&bytes).unwrap() else {
        panic!("expected publish")
    };
    assert_eq!(packet.to_bytes().unwrap(), bytes);
}

#[test]
fn topic_alias_allows_an_empty_publish_topic() {
    let bytes = frame(0x30, &[0, 0, 3, 0x23, 0, 1]);
    let ParseOk::Packet(packet, _) = MqttPacket::from_bytes_v5(&bytes).unwrap() else {
        panic!("expected publish")
    };
    assert_eq!(packet.to_bytes().unwrap(), bytes);
    assert!(MqttPacket::from_bytes_v5(&frame(0x30, &[0, 0, 0])).is_err());
}

#[test]
fn buffered_encoder_rejects_invalid_publish_before_writing() {
    let packet = mqttv5::publishv5::MqttPublish::new(1, "a".into(), Some(0), vec![], false, false);
    let mut buffer = vec![0xaa];
    assert!(packet.encode_to_buffer(&mut buffer).is_err());
    assert_eq!(buffer, [0xaa]);
}
