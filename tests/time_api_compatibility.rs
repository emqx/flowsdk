// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "std")]

use flowsdk::{
    mqtt_client::{
        inflight::InflightEntry, MqttClientOptions, MqttEngine, MqttEvent, NoIoMqttClient,
        OperationTimeouts, PortableMqttEngine, PortableNoIoMqttClient,
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
    // Host ticks can have been sampled before a convenience call advanced time.
    assert!(client.handle_tick(Instant::now()).is_empty());
    // Explicit callers still get a clock error before protocol state changes.
    assert!(client.handle_tick_at(Instant::now()).is_err());
    assert!(client.is_connected());
}

#[test]
fn host_ticks_accept_rounded_time_without_moving_deadlines_backwards() {
    let sampled = Instant::now();
    let accepted = sampled + Duration::from_nanos(1);
    let options = MqttClientOptions::builder()
        .operation_timeouts(OperationTimeouts::cloud())
        .build();
    let mut engine = MqttEngine::new_at(options, accepted);
    engine.connect_at(accepted).unwrap();
    let deadline = engine.next_tick_at();
    assert_eq!(deadline, Some(accepted + Duration::from_secs(30)));

    assert!(engine.handle_tick(sampled).is_empty());
    assert_eq!(engine.next_tick_at(), deadline);
    assert!(engine.handle_tick_at(sampled).is_err());
    assert!(engine.has_pending_output());
    assert!(engine.take_events().is_empty());
}

#[test]
fn host_reconnect_accepts_rounded_time_without_scheduling_early() {
    let sampled = Instant::now();
    let accepted = sampled + Duration::from_nanos(1);
    let mut engine = MqttEngine::new_at(MqttClientOptions::default(), accepted);

    assert!(engine.schedule_reconnect_at(sampled).is_err());
    assert_eq!(engine.next_tick_at(), None);
    engine.schedule_reconnect(sampled);
    assert_eq!(
        engine.next_tick_at(),
        Some(accepted + Duration::from_secs(1))
    );
    assert!(matches!(
        engine.take_events().as_slice(),
        [MqttEvent::ReconnectScheduled { attempt: 1, .. }]
    ));
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
