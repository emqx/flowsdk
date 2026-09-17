// SPDX-License-Identifier: MPL-2.0

use flowsdk::mqtt_serde::{
    control_packet::MqttPacket,
    mqttv5::{
        pubackv5::MqttPubAck,
        pubcompv5::MqttPubComp,
        publishv5::MqttPublish,
        pubrecv5::MqttPubRec,
        pubrelv5::MqttPubRel,
        subscribev5::{MqttSubscribe, TopicSubscription},
        unsubscribev5::MqttUnsubscribe,
    },
};
use flowsdk::mqtt_session::{ClientSession, ServerSession};

fn publication(topic: &str, qos: u8, id: u16) -> MqttPublish {
    MqttPublish::new(
        qos,
        topic.into(),
        (qos > 0).then_some(id),
        b"payload".to_vec(),
        false,
        false,
    )
}

fn subscribe(session: &mut ServerSession, filter: &str, qos: u8) {
    session.handle_incoming_subscribe(MqttSubscribe::new(
        1,
        vec![TopicSubscription::new_simple(filter.into(), qos)],
        vec![],
    ));
}

fn publications(session: &mut ServerSession) -> Vec<MqttPublish> {
    session
        .resend_pending_messages()
        .into_iter()
        .filter_map(|packet| match packet {
            MqttPacket::Publish5(publish) => Some(publish),
            _ => None,
        })
        .collect()
}

fn acknowledge(session: &mut ServerSession, publish: &MqttPublish) {
    match publish.qos {
        1 => session.handle_incoming_puback(MqttPubAck::new_success(publish.packet_id.unwrap())),
        2 => {
            let id = publish.packet_id.unwrap();
            session
                .handle_incoming_pubrec(MqttPubRec::new_success(id))
                .unwrap();
            session.handle_incoming_pubcomp(MqttPubComp::new_success(id));
        }
        _ => {}
    }
}

#[test]
fn duplicate_inbound_qos2_is_forwarded_once() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/temp", 2);
    let publish = publication("sensors/temp", 2, 41);
    assert!(matches!(
        session.handle_incoming_publish(publish.clone()),
        Some(MqttPacket::PubRec5(rec)) if rec.packet_id == 41
    ));
    let mut duplicate = publish;
    duplicate.dup = true;
    assert!(matches!(
        session.handle_incoming_publish(duplicate),
        Some(MqttPacket::PubRec5(rec)) if rec.packet_id == 41
    ));
    let first = publications(&mut session);
    for publish in &first {
        acknowledge(&mut session, publish);
    }
    let remaining = publications(&mut session);
    assert_eq!(first.len() + remaining.len(), 1);
}

#[test]
fn error_pubrec_restores_publish_quota() {
    let mut session = ServerSession::new(1);
    subscribe(&mut session, "sensors/temp", 2);
    session.handle_incoming_publish(publication("sensors/temp", 2, 41));
    let sent = publications(&mut session);
    assert_eq!(sent.len(), 1);
    assert!(session
        .handle_incoming_pubrec(MqttPubRec::new(sent[0].packet_id.unwrap(), 0x80, vec![]))
        .is_none());
    session.handle_incoming_publish(publication("sensors/temp", 2, 42));
    assert_eq!(publications(&mut session).len(), 1);
}

#[test]
fn exhausted_publish_quota_does_not_block_pubrel_retransmission() {
    let mut session = ServerSession::new(1);
    subscribe(&mut session, "sensors/temp", 2);
    session.handle_incoming_publish(publication("sensors/temp", 2, 41));
    let sent = publications(&mut session);
    let id = sent[0].packet_id.unwrap();
    session
        .handle_incoming_pubrec(MqttPubRec::new_success(id))
        .unwrap();
    assert!(matches!(
        session.resend_pending_messages().as_slice(),
        [MqttPacket::PubRel5(rel)] if rel.packet_id == id
    ));
}

#[test]
fn client_session_retransmission_sets_dup() {
    for qos in [1, 2] {
        let mut session = ClientSession::new();
        session.handle_outgoing_publish(publication("sensors/temp", qos, 41));
        assert!(matches!(
            session.resend_pending_messages().as_slice(),
            [MqttPacket::Publish5(p)] if p.dup && p.packet_id == Some(41)
        ));
    }
}

#[test]
fn first_forwarded_publication_clears_incoming_dup() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/temp", 1);
    let mut incoming = publication("sensors/temp", 1, 41);
    incoming.dup = true;
    session.handle_incoming_publish(incoming);
    let sent = publications(&mut session);
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].dup);
}

#[test]
fn nonmatching_subscription_does_not_receive_publication() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/temp", 0);
    session.handle_incoming_publish(publication("sensors/humidity", 0, 0));
    assert!(publications(&mut session).is_empty());
}

#[test]
fn leading_wildcards_do_not_match_system_topics() {
    for filter in ["#", "+/uptime", "+/#"] {
        let mut session = ServerSession::new(16);
        subscribe(&mut session, filter, 0);
        session.handle_incoming_publish(publication("$SYS/uptime", 0, 0));
        assert!(publications(&mut session).is_empty(), "filter: {filter}");
    }
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "$SYS/#", 0);
    session.handle_incoming_publish(publication("$SYS/uptime", 0, 0));
    assert_eq!(publications(&mut session).len(), 1);
}

#[test]
fn forwarded_qos_does_not_exceed_subscription_qos() {
    for granted in [0, 1] {
        let mut session = ServerSession::new(16);
        subscribe(&mut session, "sensors/temp", granted);
        session.handle_incoming_publish(publication("sensors/temp", 2, 41));
        let sent = publications(&mut session);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].qos, granted);
        assert_eq!(sent[0].packet_id.is_some(), granted > 0);
    }
}

#[test]
fn retained_publication_is_replayed_to_new_subscription() {
    let mut session = ServerSession::new(16);
    let mut retained = publication("sensors/temp", 1, 41);
    retained.retain = true;
    session.handle_incoming_publish(retained);
    assert!(publications(&mut session).is_empty());
    subscribe(&mut session, "sensors/#", 1);
    let sent = publications(&mut session);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].topic_name, "sensors/temp");
    assert_eq!(sent[0].payload, b"payload");
    assert!(sent[0].retain);
    assert!(!sent[0].dup);
}

#[test]
fn live_forwarding_obeys_retain_as_published() {
    for retain_as_published in [false, true] {
        let mut session = ServerSession::new(16);
        session.handle_incoming_subscribe(MqttSubscribe::new(
            1,
            vec![TopicSubscription::new(
                "sensors/temp".into(),
                0,
                false,
                retain_as_published,
                0,
            )],
            vec![],
        ));
        let mut incoming = publication("sensors/temp", 0, 0);
        incoming.retain = true;
        session.handle_incoming_publish(incoming);
        let sent = publications(&mut session);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].retain, retain_as_published);
    }
}

#[test]
fn completed_inbound_qos2_identifier_can_be_reused() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/temp", 0);
    let publish = publication("sensors/temp", 2, 41);
    session.handle_incoming_publish(publish.clone());
    assert_eq!(publications(&mut session).len(), 1);
    let complete = session.handle_incoming_pubrel(MqttPubRel::new_success(41));
    assert_eq!(complete.reason_code, 0);
    let repeated = session.handle_incoming_pubrel(MqttPubRel::new_success(41));
    assert_eq!(repeated.reason_code, 0x92);
    assert!(publications(&mut session).is_empty());
    session.handle_incoming_publish(publish);
    assert_eq!(publications(&mut session).len(), 1);
}

#[test]
fn acknowledgments_only_release_their_own_exchange_quota() {
    let mut session = ServerSession::new(1);
    subscribe(&mut session, "sensors/#", 2);
    session.handle_incoming_publish(publication("sensors/temp", 2, 41));
    let sent = publications(&mut session);
    let id = sent[0].packet_id.unwrap();
    session.handle_incoming_publish(publication("sensors/humidity", 1, 42));
    session.handle_incoming_puback(MqttPubAck::new_success(id));
    session.handle_incoming_pubcomp(MqttPubComp::new_success(id));
    session.handle_incoming_pubrec(MqttPubRec::new(999, 0x80, vec![]));
    assert!(session.take_pending_messages().is_empty());
    let pubrel = session
        .handle_incoming_pubrec(MqttPubRec::new_success(id))
        .unwrap();
    assert_eq!(
        session.handle_incoming_pubrec(MqttPubRec::new_success(id)),
        Some(pubrel)
    );
    assert!(session.take_pending_messages().is_empty());
    session.handle_incoming_pubcomp(MqttPubComp::new_success(id));
    assert_eq!(session.take_pending_messages().len(), 1);
    session.handle_incoming_pubcomp(MqttPubComp::new_success(id));
    session.handle_incoming_pubrec(MqttPubRec::new(id, 0x80, vec![]));
    session.handle_incoming_publish(publication("sensors/pressure", 1, 43));
    assert!(session.take_pending_messages().is_empty());
}

#[test]
fn repeated_error_pubrec_does_not_release_another_messages_quota() {
    let mut session = ServerSession::new(1);
    subscribe(&mut session, "sensors/#", 2);
    session.handle_incoming_publish(publication("sensors/temp", 2, 41));
    let id = publications(&mut session)[0].packet_id.unwrap();
    session.handle_incoming_pubrec(MqttPubRec::new(id, 0x80, vec![]));
    session.handle_incoming_publish(publication("sensors/humidity", 2, 42));
    assert_eq!(session.take_pending_messages().len(), 1);
    session.handle_incoming_pubrec(MqttPubRec::new(id, 0x80, vec![]));
    session.handle_incoming_publish(publication("sensors/pressure", 2, 43));
    assert!(session.take_pending_messages().is_empty());
}

#[test]
fn qos0_forwarding_does_not_consume_publish_quota() {
    let mut session = ServerSession::new(1);
    subscribe(&mut session, "sensors/#", 1);
    session.handle_incoming_publish(publication("sensors/temp", 1, 41));
    assert_eq!(session.take_pending_messages().len(), 1);
    session.handle_incoming_publish(publication("sensors/temp", 1, 42));
    session.handle_incoming_publish(publication("sensors/humidity", 0, 0));
    assert!(matches!(
        session.take_pending_messages().as_slice(),
        [MqttPacket::Publish5(p)] if p.qos == 0 && p.packet_id.is_none()
    ));
}

#[test]
fn forwarded_packet_ids_are_allocated_independently_of_the_publisher() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/temp", 1);
    for _ in 0..2 {
        session.handle_incoming_publish(publication("sensors/temp", 1, 41));
    }
    let sent = publications(&mut session);
    assert_eq!(sent.len(), 2);
    assert_ne!(sent[0].packet_id, sent[1].packet_id);
    assert!(sent.iter().all(|p| p.packet_id.is_some_and(|id| id != 0)));
    acknowledge(&mut session, &sent[0]);
    let retry = publications(&mut session);
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].packet_id, sent[1].packet_id);
    assert!(retry[0].dup);
}

#[test]
fn pending_publications_are_drained_in_order_without_skipping() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 0);
    for topic in ["sensors/one", "sensors/two", "sensors/three"] {
        session.handle_incoming_publish(publication(topic, 0, 0));
    }
    let sent = publications(&mut session);
    assert_eq!(
        sent.iter()
            .map(|p| p.topic_name.as_str())
            .collect::<Vec<_>>(),
        ["sensors/one", "sensors/two", "sensors/three"]
    );
    assert!(session.take_pending_messages().is_empty());
}

#[test]
fn topic_filters_match_complete_levels_and_preserve_empty_levels() {
    for (filter, topic, matches) in [
        ("sensors/+", "sensors/temp", true),
        ("sensors/+", "sensors/", true),
        ("sensors/+", "sensors", false),
        ("sensors/+", "sensors/temp/value", false),
        ("sensors/#", "sensors", true),
        ("sensors/#", "sensors/temp/value", true),
        ("sensors/#", "sensor", false),
        ("+/temp", "/temp", true),
        ("+/temp", "site/temp/", false),
        ("sensors/+/temp", "sensors//temp", true),
        ("sensors/temp", "sensors/Temp", false),
        ("#", "/$SYS/uptime", true),
        ("sensors/+", "sensors/$value", true),
        ("$SYS/+", "$SYS/uptime", true),
    ] {
        let mut session = ServerSession::new(16);
        subscribe(&mut session, filter, 0);
        session.handle_incoming_publish(publication(topic, 0, 0));
        assert_eq!(
            publications(&mut session).len(),
            usize::from(matches),
            "{filter}: {topic}"
        );
    }
}

#[test]
fn unsubscribed_filters_no_longer_forward_publications() {
    let mut session = ServerSession::new(16);
    subscribe(&mut session, "sensors/#", 0);
    session.handle_incoming_unsubscribe(MqttUnsubscribe::new(2, vec!["sensors/#".into()], vec![]));
    session.handle_incoming_publish(publication("sensors/temp", 0, 0));
    assert!(publications(&mut session).is_empty());
}

#[test]
fn retained_messages_are_replaced_and_deleted_by_empty_payload() {
    let mut session = ServerSession::new(16);
    for payload in [b"old".to_vec(), b"latest".to_vec()] {
        let mut retained = publication("sensors/temp", 0, 0);
        retained.retain = true;
        retained.payload = payload;
        session.handle_incoming_publish(retained);
    }
    session.handle_incoming_publish(publication("sensors/temp", 0, 0));
    subscribe(&mut session, "sensors/#", 0);
    let replay = publications(&mut session);
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].payload, b"latest");

    let mut deletion = publication("sensors/temp", 0, 0);
    deletion.retain = true;
    deletion.payload.clear();
    session.handle_incoming_publish(deletion);
    let live = publications(&mut session);
    assert_eq!(live.len(), 1);
    assert!(live[0].payload.is_empty());
    assert!(!live[0].retain);
    subscribe(&mut session, "sensors/#", 0);
    assert!(publications(&mut session).is_empty());
}

#[test]
fn retained_replay_obeys_retain_handling_options() {
    for handling in [0, 1, 2] {
        let mut session = ServerSession::new(16);
        let mut retained = publication("sensors/temp", 0, 0);
        retained.retain = true;
        session.handle_incoming_publish(retained);
        let command = MqttSubscribe::new(
            1,
            vec![TopicSubscription::new(
                "sensors/#".into(),
                0,
                false,
                false,
                handling,
            )],
            vec![],
        );
        session.handle_incoming_subscribe(command.clone());
        assert_eq!(publications(&mut session).len(), usize::from(handling != 2));
        session.handle_incoming_subscribe(command.clone());
        assert_eq!(publications(&mut session).len(), usize::from(handling == 0));
        session.handle_incoming_unsubscribe(MqttUnsubscribe::new(
            2,
            vec!["sensors/#".into()],
            vec![],
        ));
        session.handle_incoming_subscribe(command);
        assert_eq!(publications(&mut session).len(), usize::from(handling != 2));
    }
}

#[test]
fn retained_replay_obeys_topic_matching_and_subscription_qos() {
    let mut session = ServerSession::new(16);
    for (id, topic) in [
        (41, "sensors/temp"),
        (42, "$SYS/uptime"),
        (43, "other/topic"),
    ] {
        let mut retained = publication(topic, 2, id);
        retained.retain = true;
        session.handle_incoming_publish(retained);
    }
    subscribe(&mut session, "sensors/#", 0);
    let sent = publications(&mut session);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].topic_name, "sensors/temp");
    assert_eq!(sent[0].qos, 0);
    assert!(sent[0].packet_id.is_none());
    assert!(sent[0].retain);
    subscribe(&mut session, "+/uptime", 0);
    subscribe(&mut session, "$share/group/sensors/#", 0);
    assert!(publications(&mut session).is_empty());
}
