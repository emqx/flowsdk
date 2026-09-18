// SPDX-License-Identifier: MPL-2.0

//! ServerSession component regressions, not native broker conformance tests.
//! MQTT 5 sections 3.3.4 and 3.8.4 associate identifiers with subscriptions,
//! including repeated values on overlapping filters and replacement/removal.

use flowsdk::mqtt_serde::{
    control_packet::MqttPacket,
    mqttv5::{
        common::properties::Property,
        publishv5::MqttPublish,
        subscribev5::{MqttSubscribe, TopicSubscription},
        unsubscribev5::MqttUnsubscribe,
    },
    parser::ParseOk,
};
use flowsdk::mqtt_session::ServerSession;

fn subscribe(session: &mut ServerSession, filter: &str, qos: u8, identifier: Option<u32>) {
    session.handle_incoming_subscribe(MqttSubscribe::new(
        1,
        vec![TopicSubscription::new_simple(filter.into(), qos)],
        identifier
            .map(Property::SubscriptionIdentifier)
            .into_iter()
            .collect(),
    ));
}

fn publication(id: u16, retain: bool) -> MqttPublish {
    MqttPublish::new_with_prop(
        2,
        "sensors/temp".into(),
        Some(id),
        b"payload".to_vec(),
        retain,
        false,
        vec![Property::ContentType("text/plain".into())],
    )
}

fn identifiers(publish: &MqttPublish) -> Vec<u32> {
    let mut identifiers: Vec<_> = publish
        .properties
        .iter()
        .filter_map(|property| match property {
            Property::SubscriptionIdentifier(id) => Some(*id),
            _ => None,
        })
        .collect();
    // Order is insignificant, but multiplicity must be preserved.
    identifiers.sort_unstable();
    identifiers
}

fn decode_publications(packets: Vec<MqttPacket>) -> Vec<MqttPublish> {
    packets
        .into_iter()
        .map(|packet| {
            let bytes = packet.to_bytes().unwrap();
            match MqttPacket::from_bytes_with_version(&bytes, 5).unwrap() {
                ParseOk::Packet(MqttPacket::Publish5(publish), used) => {
                    assert_eq!(used, bytes.len());
                    assert_eq!(publish.topic_name, "sensors/temp");
                    assert_eq!(publish.payload, b"payload");
                    assert!(publish
                        .properties
                        .contains(&Property::ContentType("text/plain".into())));
                    publish
                }
                other => panic!("expected a forwarded PUBLISH, got {other:?}"),
            }
        })
        .collect()
}

fn pending(session: &mut ServerSession) -> Vec<MqttPublish> {
    // This API requests new transmissions, not retransmission of old exchanges.
    decode_publications(session.take_pending_messages())
}

fn one_pending(session: &mut ServerSession) -> MqttPublish {
    let mut publications = pending(session);
    assert_eq!(publications.len(), 1, "{publications:?}");
    publications.pop().unwrap()
}

fn assert_overlap(publications: Vec<MqttPublish>, high_id: u32, low_id: u32) {
    let mut actual: Vec<_> = publications
        .iter()
        .map(|publish| (publish.qos, identifiers(publish)))
        .collect();
    actual.sort_unstable();
    let mut combined = vec![high_id, low_id];
    combined.sort_unstable();
    assert!(
        actual == [(2, combined)] || actual == [(1, vec![low_id]), (2, vec![high_id])],
        "overlapping subscriptions lost QoS or identifier correspondence: {actual:?}"
    );
    if publications.len() == 2 {
        assert_ne!(publications[0].packet_id, publications[1].packet_id);
    }
}

#[test]
fn subscription_identifier_forwarded() {
    for qos in [0, 1, 2] {
        let mut session = ServerSession::new(16);
        subscribe(&mut session, "sensors/#", qos, Some(17));
        session.handle_incoming_publish(publication(41, false));
        let forwarded = one_pending(&mut session);
        assert_eq!(forwarded.qos, qos);
        assert_eq!(identifiers(&forwarded), [17]);
    }
}

#[test]
fn subscription_identifier_on_retained_replay() {
    let mut session = ServerSession::new(16);
    session.handle_incoming_publish(publication(41, true));
    assert!(pending(&mut session).is_empty());
    subscribe(&mut session, "sensors/#", 1, Some(17));
    let forwarded = one_pending(&mut session);
    assert_eq!(forwarded.qos, 1);
    assert!(forwarded.retain);
    assert_eq!(identifiers(&forwarded), [17]);
}

#[test]
fn overlap_distinct_identifiers() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    subscribe(&mut session, "sensors/+", 1, Some(23));
    session.handle_incoming_publish(publication(41, false));
    assert_overlap(pending(&mut session), 17, 23);
}

#[test]
fn overlap_repeated_identifiers() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    subscribe(&mut session, "sensors/+", 1, Some(17));
    session.handle_incoming_publish(publication(41, false));
    assert_overlap(pending(&mut session), 17, 17);
}

#[test]
fn subscription_identifier_replaced() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    session.handle_incoming_publish(publication(41, false));
    assert_eq!(identifiers(&one_pending(&mut session)), [17]);
    subscribe(&mut session, "sensors/#", 1, Some(23));
    session.handle_incoming_publish(publication(42, false));
    let forwarded = one_pending(&mut session);
    assert_eq!(forwarded.qos, 1);
    assert_eq!(identifiers(&forwarded), [23]);
}

#[test]
fn subscription_identifier_removed() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    session.handle_incoming_publish(publication(41, false));
    assert_eq!(identifiers(&one_pending(&mut session)), [17]);
    subscribe(&mut session, "sensors/#", 1, None);
    session.handle_incoming_publish(publication(42, false));
    let forwarded = one_pending(&mut session);
    assert_eq!(forwarded.qos, 1);
    assert!(identifiers(&forwarded).is_empty());
}

#[test]
fn replacement_applies_without_requiring_prior_delivery() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    subscribe(&mut session, "sensors/#", 1, Some(23));
    session.handle_incoming_publish(publication(41, false));
    assert_eq!(identifiers(&one_pending(&mut session)), [23]);
}

#[test]
fn removal_applies_without_requiring_prior_delivery() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    subscribe(&mut session, "sensors/#", 1, None);
    session.handle_incoming_publish(publication(41, false));
    assert!(identifiers(&one_pending(&mut session)).is_empty());
}

#[test]
fn identifier_applies_to_every_filter_in_one_subscribe() {
    let mut session = ServerSession::new(16);
    session.handle_incoming_subscribe(MqttSubscribe::new(
        1,
        vec![
            TopicSubscription::new_simple("sensors/#".into(), 2),
            TopicSubscription::new_simple("sensors/+".into(), 1),
        ],
        vec![Property::SubscriptionIdentifier(17)],
    ));
    session.handle_incoming_publish(publication(41, false));
    assert_overlap(pending(&mut session), 17, 17);
}

#[test]
fn nonmatching_subscription_does_not_contribute_its_identifier() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    subscribe(&mut session, "alerts/#", 2, Some(23));
    session.handle_incoming_publish(publication(41, false));
    assert_eq!(identifiers(&one_pending(&mut session)), [17]);
}

#[test]
fn retained_replay_uses_replaced_or_removed_identifier() {
    for replacement in [Some(23), None] {
        let mut session = ServerSession::new(16);
        subscribe(&mut session, "sensors/#", 2, Some(17));
        session.handle_incoming_publish(publication(41, true));
        // Drain the live delivery without making the replacement assertion
        // depend on whether its old identifier was implemented correctly.
        one_pending(&mut session);
        subscribe(&mut session, "sensors/#", 1, replacement);
        let replay = one_pending(&mut session);
        assert!(replay.retain);
        assert_eq!(
            identifiers(&replay),
            replacement.into_iter().collect::<Vec<_>>()
        );
    }
}

#[test]
fn unsubscribe_does_not_leave_an_identifier_on_a_new_subscription() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    session.handle_incoming_unsubscribe(MqttUnsubscribe::new(2, vec!["sensors/#".into()], vec![]));
    session.handle_incoming_publish(publication(41, false));
    assert!(pending(&mut session).is_empty());
    subscribe(&mut session, "sensors/#", 2, None);
    session.handle_incoming_publish(publication(42, false));
    assert!(identifiers(&one_pending(&mut session)).is_empty());
}

#[test]
fn retransmission_preserves_subscription_identifier() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 2, Some(17));
    session.handle_incoming_publish(publication(41, false));
    let original = one_pending(&mut session);
    let replay = decode_publications(session.resend_pending_messages());
    assert_eq!(replay.len(), 1);
    assert!(replay[0].dup);
    assert_eq!(replay[0].packet_id, original.packet_id);
    assert_eq!(identifiers(&replay[0]), [17]);
}

#[test]
fn forwarding_uses_only_the_receiving_subscriptions_identifier() {
    // Forwarding may receive a publication from another routing context. Its
    // identifiers must not become this subscriber's identifiers; this is not a
    // claim that client-to-server PUBLISH may carry Subscription Identifier.
    for identifier in [Some(17), None] {
        for retained in [false, true] {
            let mut session = ServerSession::new(16);
            if !retained {
                subscribe(&mut session, "sensors/#", 2, identifier);
            }
            let mut incoming = publication(41, retained);
            incoming.properties.extend([
                Property::SubscriptionIdentifier(99),
                Property::SubscriptionIdentifier(100),
            ]);
            session.handle_incoming_publish(incoming);
            if retained {
                subscribe(&mut session, "sensors/#", 2, identifier);
            }
            assert_eq!(
                identifiers(&one_pending(&mut session)),
                identifier.into_iter().collect::<Vec<_>>()
            );
        }
    }
}
