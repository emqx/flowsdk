// SPDX-License-Identifier: MPL-2.0

use super::*;

struct Recorder(mpsc::UnboundedSender<Vec<TokioMqttEvent>>);

#[async_trait]
impl TokioMqttEventHandler for Recorder {
    async fn on_event_batch(&mut self, events: Vec<TokioMqttEvent>) {
        self.0.send(events).unwrap();
    }
}

struct BoundedRecorder(mpsc::Sender<Vec<TokioMqttEvent>>);

#[async_trait]
impl TokioMqttEventHandler for BoundedRecorder {
    async fn on_event_batch(&mut self, events: Vec<TokioMqttEvent>) {
        self.0.send(events).await.unwrap();
    }
}

fn message(value: u8) -> TokioMqttEvent {
    TokioMqttEvent::MessageReceived(MqttMessage::new(
        0,
        "input".into(),
        None,
        vec![value],
        false,
        false,
    ))
}

fn message_values(events: &[TokioMqttEvent]) -> Vec<u8> {
    events
        .iter()
        .map(|event| match event {
            TokioMqttEvent::MessageReceived(message) => message.payload[0],
            other => panic!("expected message, got {other:?}"),
        })
        .collect()
}

async fn connected_worker(
    version: u8,
    capacity: usize,
) -> (
    TokioClientWorker,
    TokioAsyncMqttClient,
    mpsc::UnboundedReceiver<Vec<TokioMqttEvent>>,
) {
    let (events, mut received) = mpsc::unbounded_channel();
    let (command_tx, command_rx) = mpsc::channel(4);
    let (admission_tx, admission_rx) = mpsc::channel(4);
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(false)
        .buffer_messages(false)
        .build();
    let mut worker = TokioClientWorker::new(
        MqttClientOptions::builder()
            .mqtt_version(version)
            .keep_alive(0)
            .reconnect(false)
            .max_outgoing_packet_count(capacity)
            .build(),
        Box::new(Recorder(events)),
        command_rx,
        admission_rx,
        config.clone(),
    );
    worker.state = ConnectionState::Connecting;
    worker.engine.connect().unwrap();
    worker.engine.take_outgoing();
    let connack: &[u8] = if version == 5 {
        &[0x20, 3, 0, 0, 0]
    } else {
        &[0x20, 2, 0, 0]
    };
    let events = worker.engine.handle_incoming(connack);
    worker.dispatch_events(events).await;
    assert!(matches!(
        received.try_recv().unwrap().as_slice(),
        [TokioMqttEvent::Connected(result)] if result.is_success()
    ));
    let client = TokioAsyncMqttClient {
        command_tx,
        admission_tx,
        _config: config,
        worker: tokio::spawn(async { Ok(()) }),
    };
    (worker, client, received)
}

async fn admit(
    worker: &mut TokioClientWorker,
    client: &TokioAsyncMqttClient,
    command: PublishCommand,
) -> Result<Option<u16>, MqttClientError> {
    let (result, ()) = tokio::join!(client.publish_with_command_admitted(command), async {
        let request = worker.admission_rx.recv().await.unwrap();
        worker.admit(request);
    });
    result
}

#[tokio::test]
async fn admission_completes_before_write_or_ack_for_each_qos_and_version() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let (mut worker, client, mut events) = connected_worker(version, 4).await;
            // There is no transport, so neither a network write nor an ACK can
            // complete this request. Exercise the public API and command channel.
            assert!(worker.stream.is_none());
            let id = tokio::time::timeout(
                Duration::from_secs(1),
                admit(
                    &mut worker,
                    &client,
                    PublishCommand::simple("out", vec![qos], qos, false),
                ),
            )
            .await
            .expect("admission must not await I/O")
            .unwrap();
            assert_eq!(id.is_some(), qos > 0);
            assert!(worker.pending.publish.is_empty());
            assert!(!worker.engine.take_outgoing().is_empty());
            assert!(events.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn local_validation_rejection_does_not_report_connection_failure() {
    for version in [4, 5] {
        let (mut worker, client, mut events) = connected_worker(version, 4).await;
        let error = admit(
            &mut worker,
            &client,
            PublishCommand::simple("out", vec![1], 3, false),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            MqttClientError::InvalidConfiguration { .. }
        ));
        assert!(worker.engine.take_outgoing().is_empty());
        assert!(events.try_recv().is_err());
        assert!(matches!(worker.state, ConnectionState::Connected(_)));
        admit(
            &mut worker,
            &client,
            PublishCommand::simple("out", vec![2], 1, false),
        )
        .await
        .unwrap();
        assert!(!worker.engine.take_outgoing().is_empty());
        assert!(events.try_recv().is_err());
    }
}

#[tokio::test]
async fn full_publish_queue_rejects_only_its_request_and_recovers_after_drain() {
    for version in [4, 5] {
        let (mut worker, client, mut events) = connected_worker(version, 1).await;
        for byte in [1, 2] {
            admit(
                &mut worker,
                &client,
                PublishCommand::simple("out", vec![byte], 0, false),
            )
            .await
            .unwrap();
        }
        assert!(matches!(
            admit(
                &mut worker,
                &client,
                PublishCommand::simple("out", vec![3], 0, false),
            )
            .await,
            Err(MqttClientError::BufferFull { .. })
        ));
        assert!(events.try_recv().is_err());
        assert!(matches!(worker.state, ConnectionState::Connected(_)));
        assert!(!worker.engine.take_outgoing().is_empty());
        admit(
            &mut worker,
            &client,
            PublishCommand::simple("out", vec![4], 0, false),
        )
        .await
        .unwrap();
        assert!(!worker.engine.take_outgoing().is_empty());
        assert!(events.try_recv().is_err());
    }
}

#[tokio::test]
async fn admission_preserves_ack_waiters_and_published_events() {
    for version in [4, 5] {
        let (mut worker, client, mut events) = connected_worker(version, 4).await;
        let admitted_id = admit(
            &mut worker,
            &client,
            PublishCommand::simple("out", vec![1], 1, false),
        )
        .await
        .unwrap()
        .unwrap();
        let (response_tx, mut response_rx) = oneshot::channel();
        worker
            .handle_command(TokioClientCommand::PublishSync {
                command: PublishCommand::simple("out", vec![2], 1, false),
                response_tx,
            })
            .await;
        let awaited_id = *worker.pending.publish.keys().next().unwrap();
        assert_ne!(admitted_id, awaited_id);
        assert!(matches!(
            response_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        worker.engine.take_outgoing();

        for id in [admitted_id, awaited_id] {
            let [hi, lo] = id.to_be_bytes();
            let received = worker.engine.handle_incoming(&[0x40, 2, hi, lo]);
            worker.dispatch_events(received).await;
            assert!(matches!(
                events.try_recv().unwrap().as_slice(),
                [TokioMqttEvent::Published(result)] if result.packet_id == Some(id)
            ));
            if id == admitted_id {
                assert!(matches!(
                    response_rx.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ));
            }
        }
        assert_eq!(
            response_rx.await.unwrap().unwrap().packet_id,
            Some(awaited_id)
        );
        assert!(worker.pending.publish.is_empty());
    }
}

#[tokio::test]
async fn cancelling_response_after_enqueue_does_not_retract_publish() {
    let (mut worker, client, mut events) = connected_worker(5, 4).await;
    let command = {
        let response =
            client.publish_with_command_admitted(PublishCommand::simple("out", vec![1], 1, false));
        tokio::pin!(response);
        tokio::select! {
            biased;
            result = &mut response => panic!("worker has not admitted the command: {result:?}"),
            command = worker.admission_rx.recv() => command.unwrap(),
        }
        // Dropping the future closes only its response receiver.
    };
    assert!(command.response_tx.is_closed());
    worker.admit(command);
    assert!(!worker.engine.take_outgoing().is_empty());
    assert!(worker.pending.publish.is_empty());
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn disconnected_and_closed_worker_errors_are_correlated() {
    let (mut worker, client, mut events) = connected_worker(5, 4).await;
    worker.engine.handle_connection_lost();
    assert!(matches!(
        admit(
            &mut worker,
            &client,
            PublishCommand::simple("out", vec![1], 0, false),
        )
        .await,
        Err(MqttClientError::NotConnected)
    ));
    assert!(events.try_recv().is_err());
    drop(worker);
    assert!(matches!(
        client
            .publish_with_command_admitted(PublishCommand::simple("out", vec![2], 0, false))
            .await,
        Err(MqttClientError::ChannelClosed { .. })
    ));
}

#[tokio::test]
async fn cancelled_queued_subscription_is_not_admitted() {
    for version in [4, 5] {
        let (mut worker, _client, mut events) = connected_worker(version, 4).await;
        let (response_tx, response_rx) = oneshot::channel();
        let stale = TokioClientCommand::SubscribeSync {
            command: SubscribeCommand::single("source/#", 1),
            response_tx,
        };
        drop(response_rx);
        worker.handle_command(stale).await;
        assert!(worker.pending.subscribe.is_empty());
        assert!(worker.engine.take_outgoing().is_empty());
        assert!(events.try_recv().is_err());

        let (response_tx, _response_rx) = oneshot::channel();
        worker
            .handle_command(TokioClientCommand::SubscribeSync {
                command: SubscribeCommand::single("source/#", 1),
                response_tx,
            })
            .await;
        assert_eq!(worker.pending.subscribe.len(), 1);
        assert!(!worker.engine.take_outgoing().is_empty());
        assert!(events.try_recv().is_err());
    }
}

#[tokio::test]
async fn blocked_callback_services_admission_despite_full_normal_command_queue() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let (mut worker, client, _events) = connected_worker(version, 1).await;
            let (tx, mut rx) = mpsc::channel(1);
            tx.send(vec![message(1)]).await.unwrap();
            worker.event_handler = Box::new(BoundedRecorder(tx));
            for _ in 0..4 {
                client
                    .command_tx
                    .try_send(TokioClientCommand::Ping)
                    .unwrap();
            }
            assert!(matches!(
                client.command_tx.try_send(TokioClientCommand::Ping),
                Err(mpsc::error::TrySendError::Full(_))
            ));

            // No transport exists. Only admission may advance while callback
            // delivery is full; writes and normal commands remain untouched.
            assert!(worker.stream.is_none());
            let callback = worker.deliver_batch(vec![message(2)]);
            tokio::pin!(callback);
            for attempt in 0..3 {
                let response = client.publish_with_command_admitted(PublishCommand::simple(
                    "out",
                    vec![attempt],
                    qos,
                    false,
                ));
                let result = tokio::time::timeout(Duration::from_secs(1), async {
                    tokio::select! {
                        biased;
                        _ = &mut callback => panic!("callback channel is still full"),
                        result = response => result,
                    }
                })
                .await
                .expect("admission must break the shared source/sink wait cycle");
                if attempt < 2 {
                    assert_eq!(result.unwrap().is_some(), qos > 0);
                } else {
                    assert!(matches!(result, Err(MqttClientError::BufferFull { .. })));
                }
            }
            assert_eq!(message_values(&rx.recv().await.unwrap()), [1]);
            callback.await;
            assert_eq!(message_values(&rx.recv().await.unwrap()), [2]);
            // Drop the completed callback before accessing its worker again.
        }
    }
}

#[tokio::test]
async fn callback_admission_does_not_merge_or_reorder_later_batches() {
    let (mut worker, client, _events) = connected_worker(5, 4).await;
    let (tx, mut rx) = mpsc::channel(1);
    tx.send(vec![message(1)]).await.unwrap();
    worker.event_handler = Box::new(BoundedRecorder(tx));
    {
        let callback = worker.deliver_batch(vec![message(2), message(3)]);
        tokio::pin!(callback);
        tokio::select! {
            biased;
            _ = &mut callback => panic!("callback channel is still full"),
            result = client.publish_with_command_admitted(
                PublishCommand::simple("out", vec![1], 0, false)
            ) => { result.unwrap(); }
        }
        assert_eq!(message_values(&rx.recv().await.unwrap()), [1]);
        callback.await;
    }
    assert_eq!(message_values(&rx.recv().await.unwrap()), [2, 3]);
    worker.deliver_batch(vec![message(4)]).await;
    assert_eq!(message_values(&rx.recv().await.unwrap()), [4]);
    assert!(!worker.engine.take_outgoing().is_empty());
}

#[tokio::test]
async fn known_transport_failure_rejects_admission_before_error_callback_finishes() {
    let (mut worker, client, _events) = connected_worker(5, 4).await;
    let (tx, mut rx) = mpsc::channel(1);
    tx.send(vec![message(1)]).await.unwrap();
    worker.event_handler = Box::new(BoundedRecorder(tx));
    {
        let failure = worker.transport_failed(MqttClientError::ConnectionLost {
            reason: "in-memory transport closed".into(),
        });
        tokio::pin!(failure);
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::select! {
                biased;
                _ = &mut failure => panic!("error callback channel is still full"),
                result = client.publish_with_command_admitted(
                    PublishCommand::simple("out", vec![1], 0, false)
                ) => result,
            }
        })
        .await
        .unwrap();
        assert!(matches!(result, Err(MqttClientError::NotConnected)));
        assert_eq!(message_values(&rx.recv().await.unwrap()), [1]);
        tokio::join!(failure, async {
            assert!(matches!(
                rx.recv().await.unwrap().as_slice(),
                [
                    TokioMqttEvent::Error(_),
                    TokioMqttEvent::Disconnected(None),
                    TokioMqttEvent::ConnectionLost
                ]
            ));
        });
    }
    assert!(!worker.engine.is_connected());
    assert!(worker.engine.take_outgoing().is_empty());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn failed_write_rejects_admission_during_buffered_input_dispatch() {
    let (mut worker, client, _events) = connected_worker(5, 1).await;
    let incoming: Vec<_> = (1..=3)
        .flat_map(|id| {
            MqttPacket::Publish5(MqttMessage::new(
                1,
                "input".into(),
                Some(id),
                vec![id as u8],
                false,
                false,
            ))
            .to_bytes()
            .unwrap()
        })
        .collect();
    let first = worker.engine.handle_incoming(&incoming);
    assert_eq!(
        first
            .iter()
            .filter(|event| matches!(event, MqttEvent::MessageReceived(_)))
            .count(),
        2
    );
    let (tx, mut rx) = mpsc::channel(1);
    tx.send(vec![message(9)]).await.unwrap();
    worker.event_handler = Box::new(BoundedRecorder(tx));
    let drain = worker.handle_outgoing();
    tokio::pin!(drain);
    let result = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            biased;
            _ = &mut drain => panic!("buffered message callback channel is still full"),
            result = client.publish_with_command_admitted(
                PublishCommand::simple("out", vec![1], 0, false)
            ) => result,
        }
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(MqttClientError::NotConnected)));
    assert_eq!(message_values(&rx.recv().await.unwrap()), [9]);
    let (result, ()) = tokio::join!(drain, async {
        assert_eq!(message_values(&rx.recv().await.unwrap()), [3]);
    });
    assert!(result.is_err());
}

#[tokio::test]
async fn shutdown_closes_admission_lane_and_rejects_queued_requests() {
    let (mut worker, client, _events) = connected_worker(5, 4).await;
    let (response_tx, response_rx) = oneshot::channel();
    client
        .admission_tx
        .send(PublishAdmission {
            command: PublishCommand::simple("out", vec![1], 0, false),
            response_tx,
        })
        .await
        .unwrap();
    worker.close_admissions();
    assert!(matches!(
        response_rx.await.unwrap(),
        Err(MqttClientError::OperationCancelled { .. })
    ));
    assert!(matches!(
        client
            .publish_with_command_admitted(PublishCommand::simple("out", vec![2], 0, false))
            .await,
        Err(MqttClientError::ChannelClosed { .. })
    ));
    assert!(worker.engine.take_outgoing().is_empty());
}

#[tokio::test]
async fn callback_time_admission_flushes_without_another_read_or_keepalive() {
    let (mut worker, client, _events) = connected_worker(5, 1).await;
    let incoming: Vec<_> = (1..=3)
        .flat_map(|id| {
            MqttPacket::Publish5(MqttMessage::new(
                1,
                "input".into(),
                Some(id),
                vec![id as u8],
                false,
                false,
            ))
            .to_bytes()
            .unwrap()
        })
        .collect();
    worker.engine.handle_incoming(&incoming);
    let (transport, mut peer) = tokio::io::duplex(1024);
    worker.stream = Some(Box::new(transport));
    let (tx, mut rx) = mpsc::channel(1);
    tx.send(vec![message(9)]).await.unwrap();
    worker.event_handler = Box::new(BoundedRecorder(tx));
    let task = tokio::spawn(worker.run());

    // The output drain parses the third buffered input and blocks in its callback.
    // Consume the existing three PUBACK packets before admitting the new output.
    let mut acknowledgements = [0; 12];
    tokio::time::timeout(
        Duration::from_secs(1),
        peer.read_exact(&mut acknowledgements),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        acknowledgements,
        [0x40, 2, 0, 1, 0x40, 2, 0, 2, 0x40, 2, 0, 3]
    );
    tokio::time::timeout(
        Duration::from_secs(1),
        client.publish_with_command_admitted(PublishCommand::simple("out", vec![7], 0, false)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(message_values(&rx.recv().await.unwrap()), [9]);
    assert_eq!(message_values(&rx.recv().await.unwrap()), [3]);

    // No input is written to peer, and keepalive is disabled in the fixture.
    let expected = MqttPacket::Publish5(MqttMessage::new(
        0,
        "out".into(),
        None,
        vec![7],
        false,
        false,
    ))
    .to_bytes()
    .unwrap();
    let mut actual = vec![0; expected.len()];
    tokio::time::timeout(Duration::from_secs(1), peer.read_exact(&mut actual))
        .await
        .expect("callback-time admission must promptly reach the transport")
        .unwrap();
    assert_eq!(actual, expected);
    client
        .command_tx
        .send(TokioClientCommand::Shutdown)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn callback_time_admission_drains_pending_events_without_output() {
    let (mut worker, client, _events) = connected_worker(5, 4).await;
    let (transport, _peer) = tokio::io::duplex(1024);
    worker.stream = Some(Box::new(transport));
    let (tx, mut rx) = mpsc::channel(1);
    tx.send(vec![message(1)]).await.unwrap();
    worker.event_handler = Box::new(BoundedRecorder(tx));
    // Queue an engine notification without output bytes. The connected fixture
    // has no keepalive/tick deadline to make the worker drain this by accident.
    worker.engine.schedule_reconnect(Instant::now());
    assert!(!worker.engine.has_pending_output());
    assert!(worker.engine.next_tick_at().is_none());
    {
        let callback = worker.deliver_batch(vec![message(2)]);
        tokio::pin!(callback);
        tokio::select! {
            biased;
            _ = &mut callback => panic!("callback channel is still full"),
            result = client.publish_with_command_admitted(
                PublishCommand::simple("out", vec![1], 3, false)
            ) => { assert!(result.is_err()); }
        }
        assert_eq!(message_values(&rx.recv().await.unwrap()), [1]);
        callback.await;
    }
    assert_eq!(message_values(&rx.recv().await.unwrap()), [2]);
    assert!(!worker.engine.has_pending_output());
    let task = tokio::spawn(worker.run());
    let notification = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("callback-time admission must schedule an event drain without output")
        .unwrap();
    assert!(matches!(
        notification.as_slice(),
        [TokioMqttEvent::EngineEvent(
            MqttEvent::ReconnectScheduled { .. }
        )]
    ));
    client
        .command_tx
        .send(TokioClientCommand::Shutdown)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
