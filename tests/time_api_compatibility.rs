// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "std")]

use flowsdk::{
    mqtt_client::{
        inflight::InflightEntry, MqttClientOptions, MqttEngine, MqttEvent, NoIoMqttClient,
        PortableMqttEngine, PortableNoIoMqttClient,
    },
    time::Timestamp,
};
use std::time::{Duration, Instant};

#[test]
fn desktop_time_signatures_remain_source_compatible() {
    let _: fn(MqttClientOptions) -> MqttEngine = MqttEngine::new;
    let _: fn(&mut MqttEngine, Instant) -> Vec<MqttEvent> = MqttEngine::handle_tick;
    let _: fn(&MqttEngine) -> Option<Instant> = MqttEngine::next_tick_at;
    let _: fn(&mut NoIoMqttClient, Instant) -> Vec<MqttEvent> = NoIoMqttClient::handle_tick;
    let _: fn(&NoIoMqttClient) -> Option<Instant> = NoIoMqttClient::next_tick_at;
    let _: fn(&InflightEntry) -> Instant = |entry| entry.sent_at;
}

#[test]
fn convenience_calls_follow_a_future_explicit_tick() {
    let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    let future = Instant::now() + Duration::from_secs(3600);
    assert!(client.handle_tick(future).is_empty());
    client.connect().unwrap();
    assert_eq!(client.take_outgoing()[0], 0x10);
    assert!(client
        .handle_incoming(&[0x20, 3, 0, 0, 0])
        .iter()
        .any(|event| matches!(event, MqttEvent::Connected(_))));
    // Explicit callers still get a clock error before protocol state changes.
    assert!(client
        .handle_tick(Instant::now())
        .iter()
        .any(|event| matches!(event, MqttEvent::Error(_))));
    assert!(client.is_connected());
}

#[test]
fn enabling_std_does_not_change_the_portable_aliases() {
    let mut engine =
        PortableMqttEngine::try_new_at(MqttClientOptions::default(), Timestamp::ZERO).unwrap();
    engine.connect_at(Timestamp::ZERO).unwrap();
    let _: Option<Timestamp> = engine.next_tick_at();
    let mut facade = PortableNoIoMqttClient::new_at(MqttClientOptions::default(), Timestamp::ZERO);
    facade.connect_at(Timestamp::ZERO).unwrap();
    let _: Option<Timestamp> = facade.next_tick_at();
}
