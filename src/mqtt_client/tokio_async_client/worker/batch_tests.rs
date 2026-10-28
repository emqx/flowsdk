// SPDX-License-Identifier: MPL-2.0

use super::test_support::{publish, receive};
use super::*;
use crate::mqtt_serde::mqttv5::publishv5::MqttPublish;

struct BatchRecorder(mpsc::UnboundedSender<Vec<TokioMqttEvent>>);

#[async_trait]
impl TokioMqttEventHandler for BatchRecorder {
    async fn on_event_batch(&mut self, events: Vec<TokioMqttEvent>) {
        assert!(!events.is_empty());
        self.0.send(events).unwrap();
    }

    async fn on_message_received(&mut self, _: &MqttMessage) {
        panic!("batch override must replace individual callbacks");
    }

    async fn on_error(&mut self, _: &MqttClientError) {
        panic!("worker errors must also reach the batch override");
    }

    async fn on_disconnected(&mut self, _: Option<u8>) {
        panic!("worker disconnects must also reach the batch override");
    }
}

async fn connected_worker(
    options: MqttClientOptions,
) -> (
    TokioClientWorker,
    mpsc::Sender<TokioClientCommand>,
    mpsc::UnboundedReceiver<Vec<TokioMqttEvent>>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (worker, commands) = test_support::connected_worker(
        options,
        Box::new(BatchRecorder(tx)),
        TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .build(),
    )
    .await;
    (worker, commands, rx)
}

fn options() -> MqttClientOptions {
    MqttClientOptions::builder()
        .reconnect(false)
        .keep_alive(0)
        .build()
}

fn messages(events: &[TokioMqttEvent]) -> Vec<u8> {
    events
        .iter()
        .map(|event| match event {
            TokioMqttEvent::MessageReceived(message) => message.payload[0],
            other => panic!("expected only messages, got {other:?}"),
        })
        .collect()
}

fn take_connected(rx: &mut mpsc::UnboundedReceiver<Vec<TokioMqttEvent>>) {
    assert!(matches!(
        rx.try_recv().unwrap().as_slice(),
        [TokioMqttEvent::Connected(_)]
    ));
}

#[tokio::test]
async fn full_publish_pair_is_atomic_and_folds_for_every_version_and_qos() {
    for version in [3, 4, 5] {
        for qos in [0, 1, 2] {
            let (mut worker, _commands, mut rx) =
                connected_worker(options().mqtt_version(version).max_event_count(1)).await;
            take_connected(&mut rx);
            let events = worker.engine.handle_incoming(&publish(version, 7, qos));
            // Even a one-event budget must preserve the documented adjacent
            // metadata/message pair on which the Tokio adapter relies.
            assert!(matches!(
                events.as_slice(),
                [
                    MqttEvent::PublishReceived { packet_id, stream: None },
                    MqttEvent::MessageReceived(message),
                ] if *packet_id == message.packet_id
            ));
            worker.dispatch_events(events).await;
            assert_eq!(messages(&rx.try_recv().unwrap()), [7]);
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn reduced_parsing_keeps_metadata_as_an_ordering_boundary() {
    use crate::mqtt_serde::parser::leveled::ParseLevel;

    for version in [3, 4, 5] {
        for (level, qos) in [(ParseLevel::TypeOnly, 0), (ParseLevel::HeadersParsed, 1)] {
            let (mut worker, _commands, mut rx) =
                connected_worker(options().mqtt_version(version)).await;
            take_connected(&mut rx);
            worker.engine.set_parse_level(level);
            receive(&mut worker, &publish(version, 7, qos)).await;
            assert!(matches!(
                rx.try_recv().unwrap().as_slice(),
                [TokioMqttEvent::EngineEvent(MqttEvent::PublishReceived {
                    packet_id,
                    stream: None,
                })] if *packet_id == (qos > 0).then_some(7)
            ));
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn repeated_disconnect_preserves_boundary_without_duplicate_callbacks() {
    let (mut worker, _commands, mut rx) = connected_worker(options()).await;
    take_connected(&mut rx);
    worker
        .dispatch_events(vec![
            MqttEvent::Disconnected(Some(1)),
            MqttEvent::Disconnected(Some(2)),
        ])
        .await;
    assert!(matches!(
        rx.try_recv().unwrap().as_slice(),
        [
            TokioMqttEvent::Disconnected(Some(1)),
            TokioMqttEvent::ConnectionLost,
            TokioMqttEvent::EngineEvent(MqttEvent::Disconnected(Some(2))),
        ]
    ));
    worker.transport_failed(MqttClientError::NotConnected).await;
    assert!(matches!(
        rx.try_recv().unwrap().as_slice(),
        [TokioMqttEvent::Error(MqttClientError::NotConnected)]
    ));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn reconnect_events_respect_policy_and_keep_their_order() {
    for auto_reconnect in [false, true] {
        for intentional_stop in [false, true] {
            let (mut worker, _commands, mut rx) = connected_worker(options()).await;
            take_connected(&mut rx);
            worker
                .dispatch_events(vec![MqttEvent::Disconnected(None)])
                .await;
            rx.try_recv().unwrap();
            worker.config.auto_reconnect = auto_reconnect;
            worker.intentional_stop = intentional_stop;
            let delay = Duration::from_millis(20);
            worker
                .dispatch_events(vec![
                    MqttEvent::ReconnectScheduled { attempt: 2, delay },
                    MqttEvent::ReconnectNeeded,
                ])
                .await;
            let batch = rx.try_recv().unwrap();
            let reconnect = auto_reconnect && !intentional_stop;
            if reconnect {
                assert!(matches!(batch[0], TokioMqttEvent::ReconnectAttempt(2)));
            } else {
                assert!(matches!(
                    batch[0],
                    TokioMqttEvent::EngineEvent(MqttEvent::ReconnectScheduled {
                        attempt: 2,
                        delay: actual,
                    }) if actual == delay
                ));
            }
            assert!(matches!(
                batch[1..],
                [TokioMqttEvent::EngineEvent(MqttEvent::ReconnectNeeded)]
            ));
            assert_eq!(
                matches!(worker.state, ConnectionState::Connecting),
                reconnect
            );
            assert_eq!(worker.connecting.is_some(), reconnect);
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn mixed_read_keeps_publish_runs_and_disconnect_order() {
    let (mut worker, _commands, mut rx) = connected_worker(options()).await;
    take_connected(&mut rx);
    worker.engine.send_ping().unwrap();
    worker.engine.take_outgoing();
    let (response, mut ping) = oneshot::channel();
    worker.pending.ping.push(response);
    let bytes = [
        publish(5, 1, 0),
        publish(5, 2, 0),
        vec![0xd0, 0],
        publish(5, 3, 0),
        vec![0xe0, 2, 0, 0],
    ]
    .concat();
    receive(&mut worker, &bytes).await;
    let batch = rx.try_recv().unwrap();
    assert_eq!(messages(&batch[..2]), [1, 2]);
    assert!(matches!(batch[2], TokioMqttEvent::PingResponse(_)));
    assert_eq!(messages(&batch[3..4]), [3]);
    assert!(
        matches!(
            batch[4..],
            [
                TokioMqttEvent::DisconnectReceived { reason_code: 0, .. },
                TokioMqttEvent::Disconnected(Some(0)),
                TokioMqttEvent::ConnectionLost,
            ]
        ),
        "{batch:?}"
    );
    assert!(ping.try_recv().unwrap().unwrap().success);
    assert!(matches!(worker.state, ConnectionState::Disconnected));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn partial_packet_does_not_delay_ready_messages_or_merge_dispatches() {
    let (mut worker, _commands, mut rx) = connected_worker(options()).await;
    take_connected(&mut rx);
    let second = publish(5, 2, 0);
    let split = second.len() - 1;
    receive(
        &mut worker,
        &[publish(5, 1, 0), second[..split].to_vec()].concat(),
    )
    .await;
    // The first message is delivered while the second still needs more input.
    assert_eq!(messages(&rx.try_recv().unwrap()), [1]);
    worker.dispatch_events(Vec::new()).await;
    assert!(rx.try_recv().is_err());
    receive(&mut worker, &second[split..]).await;
    receive(&mut worker, &publish(5, 3, 0)).await;
    assert_eq!(messages(&rx.try_recv().unwrap()), [2]);
    assert_eq!(messages(&rx.try_recv().unwrap()), [3]);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn outgoing_drain_delivers_its_own_batch_even_when_write_fails() {
    for write_fails in [false, true] {
        let (mut worker, _commands, mut rx) =
            connected_worker(options().max_outgoing_packet_count(1)).await;
        take_connected(&mut rx);
        let (transport, peer) = tokio::io::duplex(1024);
        worker.stream = Some(Box::new(transport));
        let _peer = if write_fails {
            drop(peer);
            None
        } else {
            Some(peer)
        };
        receive(
            &mut worker,
            &[publish(5, 1, 1), publish(5, 2, 1), publish(5, 3, 1)].concat(),
        )
        .await;
        assert_eq!(messages(&rx.try_recv().unwrap()), [1, 2]);
        assert_eq!(worker.handle_outgoing().await.is_err(), write_fails);
        assert_eq!(messages(&rx.try_recv().unwrap()), [3]);
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn batch_override_keeps_manual_ack_and_publish_completion_working() {
    let (mut worker, _commands, mut rx) = connected_worker(options().auto_ack(false)).await;
    take_connected(&mut rx);
    receive(&mut worker, &publish(5, 7, 2)).await;
    assert_eq!(messages(&rx.try_recv().unwrap()), [7]);
    worker.engine.pubrec(7, 0, vec![]).unwrap();
    assert!(!worker.engine.take_outgoing().is_empty());
    receive(&mut worker, &[0x62, 2, 0, 7]).await;
    assert!(matches!(
        rx.try_recv().unwrap().as_slice(),
        [TokioMqttEvent::PubRelReceived { packet_id: 7 }]
    ));
    worker.engine.pubcomp(7, 0, vec![]).unwrap();
    worker.engine.take_outgoing();

    let (response_tx, mut result) = oneshot::channel();
    worker
        .handle_command(TokioClientCommand::PublishSync {
            command: PublishCommand::simple("out", vec![1], 1, false),
            response_tx,
        })
        .await;
    let id = *worker.pending.publish.keys().next().unwrap();
    worker.engine.take_outgoing();
    let [hi, lo] = id.to_be_bytes();
    receive(&mut worker, &[0x40, 2, hi, lo]).await;
    assert_eq!(result.try_recv().unwrap().unwrap().packet_id, Some(id));
    assert!(
        matches!(rx.try_recv().unwrap().as_slice(), [TokioMqttEvent::Published(result)] if result.packet_id == Some(id))
    );
}

#[tokio::test]
async fn operation_failures_and_unmapped_events_remain_ordering_boundaries() {
    let (mut worker, _commands, mut rx) = connected_worker(options()).await;
    take_connected(&mut rx);
    let (response, mut result) = oneshot::channel();
    worker.pending.subscribe.insert(42, response);
    worker
        .dispatch_events(vec![
            MqttEvent::MessageReceived(MqttPublish::new(
                0,
                "t".into(),
                None,
                vec![1],
                false,
                false,
            )),
            MqttEvent::OperationFailed {
                operation: OperationKind::Subscribe,
                packet_id: Some(42),
                error: MqttClientError::OperationTimeout {
                    operation: "subscribe".into(),
                    timeout_ms: 1,
                },
            },
            MqttEvent::PublishReceived {
                packet_id: Some(5),
                stream: None,
            },
            MqttEvent::StreamClosed {
                stream_id: 9,
                reason: "closed".into(),
                by_peer: true,
            },
            MqttEvent::MessageReceived(MqttPublish::new(
                0,
                "t".into(),
                None,
                vec![2],
                false,
                false,
            )),
        ])
        .await;
    let batch = rx.try_recv().unwrap();
    assert!(matches!(
        batch.as_slice(),
        [
            TokioMqttEvent::MessageReceived(_),
            TokioMqttEvent::Error(MqttClientError::OperationTimeout { .. }),
            TokioMqttEvent::EngineEvent(MqttEvent::PublishReceived {
                packet_id: Some(5),
                ..
            }),
            TokioMqttEvent::EngineEvent(MqttEvent::StreamClosed { stream_id: 9, .. }),
            TokioMqttEvent::MessageReceived(_),
        ]
    ));
    assert!(matches!(
        result.try_recv().unwrap(),
        Err(MqttClientError::OperationTimeout { .. })
    ));
}

#[tokio::test]
async fn running_worker_delivers_before_another_read_and_reports_transport_loss() {
    let (mut worker, commands, mut rx) = connected_worker(options()).await;
    take_connected(&mut rx);
    let (transport, mut peer) = tokio::io::duplex(1024);
    worker.stream = Some(Box::new(transport));
    peer.write_all(&[publish(5, 1, 0), publish(5, 2, 0)].concat())
        .await
        .unwrap();
    let task = tokio::spawn(worker.run());
    let batch = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(messages(&batch), [1, 2]);
    // Supply more data only after the first batch has been delivered.
    peer.write_all(&publish(5, 3, 0)).await.unwrap();
    let batch = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(messages(&batch), [3]);
    drop(peer);
    let batch = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        batch.as_slice(),
        [
            TokioMqttEvent::Error(_),
            TokioMqttEvent::Disconnected(None),
            TokioMqttEvent::ConnectionLost
        ]
    ));
    commands.send(TokioClientCommand::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn local_disconnect_and_command_error_use_batch_callback() {
    let (mut worker, _commands, mut rx) = connected_worker(options()).await;
    take_connected(&mut rx);
    let (transport, _peer) = tokio::io::duplex(1024);
    worker.stream = Some(Box::new(transport));
    worker.disconnect(0, vec![]).await.unwrap();
    assert!(matches!(
        rx.try_recv().unwrap().as_slice(),
        [TokioMqttEvent::Disconnected(Some(0))]
    ));
    worker.handle_command(TokioClientCommand::Ping).await;
    assert!(matches!(
        rx.try_recv().unwrap().as_slice(),
        [TokioMqttEvent::Error(MqttClientError::NotConnected)]
    ));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn default_batch_dispatch_preserves_legacy_callback_order() {
    struct Legacy(mpsc::UnboundedSender<&'static str>);
    #[async_trait]
    impl TokioMqttEventHandler for Legacy {
        async fn on_message_received(&mut self, _: &MqttMessage) {
            self.0.send("message").unwrap();
        }
        async fn on_ping_response(&mut self, _: &PingResult) {
            self.0.send("ping").unwrap();
        }
        async fn on_disconnect_received(&mut self, _: u8, _: &[Property]) {
            self.0.send("disconnect packet").unwrap();
        }
        async fn on_disconnected(&mut self, _: Option<u8>) {
            self.0.send("disconnected").unwrap();
        }
        async fn on_connection_lost(&mut self) {
            self.0.send("lost").unwrap();
        }
    }
    let (mut worker, _commands, _batches) = connected_worker(options()).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    worker.event_handler = Box::new(Legacy(tx));
    worker.engine.send_ping().unwrap();
    worker.engine.take_outgoing();
    receive(
        &mut worker,
        &[
            publish(5, 1, 0),
            publish(5, 2, 0),
            vec![0xd0, 0],
            publish(5, 3, 0),
            vec![0xe0, 2, 0, 0],
        ]
        .concat(),
    )
    .await;
    for expected in [
        "message",
        "message",
        "ping",
        "message",
        "disconnect packet",
        "disconnected",
        "lost",
    ] {
        assert_eq!(rx.try_recv().unwrap(), expected);
    }
    assert!(rx.try_recv().is_err());
}
