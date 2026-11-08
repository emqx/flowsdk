// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::sync::Arc;

struct IgnoreEvents;

#[async_trait]
impl TokioMqttEventHandler for IgnoreEvents {
    async fn on_event_batch(&mut self, _: Vec<TokioMqttEvent>) {}
}

fn limits() -> PublishAdmissionConfig {
    PublishAdmissionConfig {
        max_batch_messages: 8,
        max_batch_bytes: 16 * 1024,
        max_pending_messages: 16,
        max_pending_bytes: 32 * 1024,
    }
}

fn command(value: u8, qos: u8) -> PublishCommand {
    PublishCommand::simple("batch", vec![value; 32], qos, false)
}

async fn connected(
    version: u8,
    capacity: usize,
    limits: Option<PublishAdmissionConfig>,
) -> (TokioClientWorker, TokioAsyncMqttClient) {
    let (command_tx, command_rx) = mpsc::channel(4);
    let (admission_tx, admission_rx) = mpsc::channel(4);
    let mut config = TokioAsyncClientConfig::builder()
        .auto_reconnect(false)
        .buffer_messages(false)
        .priority_queue_enabled(false)
        .build();
    config.publish_admission = limits.clone();
    let mut worker = TokioClientWorker::new(
        MqttClientOptions::builder()
            .mqtt_version(version)
            .keep_alive(0)
            .reconnect(false)
            .max_outgoing_packet_count(capacity)
            .build(),
        Box::new(IgnoreEvents),
        command_rx,
        admission_rx,
        config.clone(),
    );
    worker.state = ConnectionState::Connecting;
    worker.engine.connect().unwrap();
    worker.engine.take_outgoing();
    let connack: &[u8] = if version == 5 {
        // Advertise two outgoing topic aliases for stateful ordering tests.
        &[0x20, 6, 0, 0, 3, 0x22, 0, 2]
    } else {
        &[0x20, 2, 0, 0]
    };
    let events = worker.engine.handle_incoming(connack);
    worker.dispatch_events(events).await;
    assert!(worker.engine.is_connected());
    let client = TokioAsyncMqttClient {
        command_tx,
        admission_tx,
        admission_budget: limits.map(AdmissionBudget::new),
        _config: config,
        worker: tokio::spawn(async { Ok(()) }),
    };
    (worker, client)
}

async fn admit(
    worker: &mut TokioClientWorker,
    client: &TokioAsyncMqttClient,
    commands: Vec<PublishCommand>,
) -> PublishBatchAdmission {
    let (report, ()) = tokio::join!(client.publish_batch_admitted(commands), async {
        let request = worker.admission_rx.recv().await.unwrap();
        assert!(matches!(request, PublishAdmission::Batch { .. }));
        worker.admit(request);
    });
    assert!(worker.admission_rx.try_recv().is_err());
    report.unwrap()
}

#[tokio::test]
async fn mixed_qos_batch_matches_individual_engine_packets_without_io() {
    for version in [4, 5] {
        let (mut worker, client) = connected(version, 16, Some(limits())).await;
        let (mut reference, _) = connected(version, 16, None).await;
        let commands = vec![command(1, 0), command(2, 1), command(3, 2)];
        let expected: Vec<_> = commands
            .iter()
            .cloned()
            .map(|command| reference.engine.publish(command).unwrap())
            .collect();
        let report = admit(&mut worker, &client, commands).await;
        let actual: Vec<_> = report.results.into_iter().map(Result::unwrap).collect();
        assert_eq!(actual, expected);
        assert_eq!(
            worker.engine.take_outgoing(),
            reference.engine.take_outgoing()
        );
        assert!(
            worker.stream.is_none(),
            "admission cannot depend on socket writes or ACKs"
        );
    }
}

#[tokio::test]
async fn empty_batch_is_local_and_disabled_batch_returns_owned_commands() {
    let (mut worker, client) = connected(5, 4, None).await;
    assert!(client
        .publish_batch_admitted(vec![])
        .await
        .unwrap()
        .results
        .is_empty());
    let command = command(1, 0);
    let pointer = command.payload.as_ptr();
    let error = client
        .publish_batch_admitted(vec![command])
        .await
        .unwrap_err();
    let PublishBatchAdmissionError::NotSubmitted { error, commands } = error else {
        panic!("not submitted")
    };
    assert!(matches!(
        error,
        MqttClientError::InvalidConfiguration { .. }
    ));
    assert_eq!(commands[0].payload.as_ptr(), pointer);
    assert!(worker.admission_rx.try_recv().is_err());
}

#[tokio::test]
async fn oversized_requests_never_enter_queue_and_properties_count_toward_bytes() {
    let (mut worker, client) = connected(5, 16, Some(limits())).await;
    let mut too_big = command(1, 0);
    too_big
        .properties
        .push(Property::UserProperty("key".into(), "x".repeat(16 * 1024)));
    let batches = [vec![command(1, 0); 9], vec![too_big]];
    for commands in batches {
        let pointers: Vec<_> = commands
            .iter()
            .map(|command| command.payload.as_ptr())
            .collect();
        let err = client.publish_batch_admitted(commands).await.unwrap_err();
        let PublishBatchAdmissionError::NotSubmitted { error, commands } = err else {
            panic!("not submitted")
        };
        assert!(matches!(error, MqttClientError::BufferFull { .. }));
        assert_eq!(
            commands
                .iter()
                .map(|command| command.payload.as_ptr())
                .collect::<Vec<_>>(),
            pointers
        );
    }
    assert!(worker.admission_rx.try_recv().is_err());
}

#[tokio::test]
async fn validation_and_packet_id_failures_do_not_rollback_or_skip_other_items() {
    for version in [4, 5] {
        let (mut worker, client) = connected(version, 16, Some(limits())).await;
        let mut first = command(1, 1);
        first.packet_id = Some(7);
        let mut duplicate = command(2, 1);
        duplicate.packet_id = Some(7);
        let mut invalid = command(3, 0);
        invalid.qos = 3;
        let report = admit(
            &mut worker,
            &client,
            vec![first, duplicate, invalid, command(4, 0)],
        )
        .await;
        assert!(matches!(report.results[0], Ok(Some(7))));
        assert!(matches!(
            report.results[1],
            Err(MqttClientError::InvalidPacketId { packet_id: 7 })
        ));
        assert!(report.results[2].is_err());
        assert!(matches!(report.results[3], Ok(None)));
        assert!(worker
            .engine
            .take_events()
            .iter()
            .all(|event| !matches!(event, MqttEvent::Error(_))));
        let bytes = worker.engine.take_outgoing();
        assert!(bytes.windows(32).any(|part| part == [1; 32]));
        assert!(bytes.windows(32).any(|part| part == [4; 32]));
        assert!(!bytes
            .windows(32)
            .any(|part| part == [2; 32] || part == [3; 32]));
    }
}

#[tokio::test]
async fn aliases_observe_prior_success_but_not_prior_rejection() {
    let (mut worker, client) = connected(5, 16, Some(limits())).await;
    let mut register = command(1, 0);
    register.properties.push(Property::TopicAlias(1));
    let mut reuse = command(2, 0);
    reuse.topic_name.clear();
    reuse.properties.push(Property::TopicAlias(1));
    let mut rejected = command(3, 0);
    rejected.qos = 3;
    rejected.properties.push(Property::TopicAlias(2));
    let mut undefined = command(4, 0);
    undefined.topic_name.clear();
    undefined.properties.push(Property::TopicAlias(2));
    let report = admit(
        &mut worker,
        &client,
        vec![register, reuse, rejected, undefined],
    )
    .await;
    assert!(report.results[0].is_ok());
    assert!(report.results[1].is_ok());
    assert!(report.results[2].is_err());
    assert!(report.results[3].is_err());
}

#[tokio::test]
async fn queue_capacity_returns_partial_results_without_waiting_for_drain() {
    let (mut worker, client) = connected(5, 1, Some(limits())).await;
    let report = admit(
        &mut worker,
        &client,
        vec![command(1, 0), command(2, 0), command(3, 0)],
    )
    .await;
    // One publication is in outgoing storage, one in the queue, then capacity is full.
    assert!(report.results[0].is_ok());
    assert!(report.results[1].is_ok());
    assert!(matches!(
        report.results[2],
        Err(MqttClientError::BufferFull { .. })
    ));
}

#[tokio::test]
async fn batch_admission_progresses_during_blocked_event_callback() {
    struct Blocked;
    #[async_trait]
    impl TokioMqttEventHandler for Blocked {
        async fn on_event_batch(&mut self, _: Vec<TokioMqttEvent>) {
            std::future::pending::<()>().await;
        }
    }
    let (mut worker, client) = connected(5, 8, Some(limits())).await;
    worker.event_handler = Box::new(Blocked);
    let report = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            () = worker.deliver_batch(vec![TokioMqttEvent::ConnectionLost]) => panic!("callback is blocked"),
            report = client.publish_batch_admitted(vec![command(1, 0), command(2, 1)]) => report.unwrap(),
        }
    }).await.unwrap();
    assert!(report.results.iter().all(Result::is_ok));
    assert!(worker.admission_needs_drain);
    assert!(worker.engine.has_pending_output());
}

#[tokio::test]
async fn single_and_batch_requests_share_fifo_admission_order() {
    let (mut worker, client) = connected(5, 16, Some(limits())).await;
    let first = client.publish_with_command_admitted(command(1, 0));
    let batch = client.publish_batch_admitted(vec![command(2, 0), command(3, 0)]);
    let last = client.publish_with_command_admitted(command(4, 0));
    tokio::pin!(first, batch, last);
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut batch).is_pending());
    assert!(futures::poll!(&mut last).is_pending());
    for _ in 0..3 {
        let request = worker.admission_rx.recv().await.unwrap();
        worker.admit(request);
    }
    first.await.unwrap();
    assert!(batch.await.unwrap().results.iter().all(Result::is_ok));
    last.await.unwrap();
    let (mut reference, _) = connected(5, 16, None).await;
    for value in 1..=4 {
        reference.engine.publish(command(value, 0)).unwrap();
    }
    assert_eq!(
        worker.engine.take_outgoing(),
        reference.engine.take_outgoing()
    );
}

#[tokio::test]
async fn cancellation_while_waiting_for_channel_releases_all_budget_credits() {
    let mut bounds = limits();
    bounds.max_batch_messages = 2;
    bounds.max_pending_messages = 3;
    let (mut worker, mut client) = connected(5, 16, Some(bounds)).await;
    let (tx, rx) = mpsc::channel(1);
    client.admission_tx = tx;
    worker.admission_rx = rx;
    let mut first = Box::pin(client.publish_with_command_admitted(command(1, 0)));
    assert!(futures::poll!(&mut first).is_pending());
    let mut cancelled = Box::pin(client.publish_batch_admitted(vec![command(2, 0), command(3, 0)]));
    assert!(futures::poll!(&mut cancelled).is_pending());
    let mut last = Box::pin(client.publish_with_command_admitted(command(4, 0)));
    assert!(futures::poll!(&mut last).is_pending());
    drop(cancelled);
    let request = worker.admission_rx.recv().await.unwrap();
    worker.admit(request);
    first.await.unwrap();
    assert!(futures::poll!(&mut last).is_pending());
    let request = worker.admission_rx.recv().await.unwrap();
    worker.admit(request);
    last.await.unwrap();
    assert!(worker.admission_rx.try_recv().is_err());
    // A single could fit even if the cancelled request leaked two credits.
    // A new two-item request proves every cancelled reservation was released.
    let report = tokio::time::timeout(
        Duration::from_secs(1),
        admit(&mut worker, &client, vec![command(5, 0), command(6, 0)]),
    )
    .await
    .expect("cancelled batch must release all credits");
    assert!(report.results.iter().all(Result::is_ok));
    let bytes = worker.engine.take_outgoing();
    assert!(!bytes
        .windows(32)
        .any(|part| part == [2; 32] || part == [3; 32]));
}

#[tokio::test]
async fn pending_byte_budget_is_shared_with_single_admissions() {
    let cost = admission::command_bytes(&command(1, 0)).unwrap();
    let bounds = PublishAdmissionConfig {
        max_batch_messages: 2,
        max_batch_bytes: cost * 2,
        max_pending_messages: 8,
        max_pending_bytes: cost * 2,
    };
    let (mut worker, client) = connected(5, 16, Some(bounds)).await;
    let mut batch = Box::pin(client.publish_batch_admitted(vec![command(1, 0), command(2, 0)]));
    assert!(futures::poll!(&mut batch).is_pending());
    let request = worker.admission_rx.recv().await.unwrap();
    let mut single = Box::pin(client.publish_with_command_admitted(command(3, 0)));
    assert!(futures::poll!(&mut single).is_pending());
    assert!(
        worker.admission_rx.try_recv().is_err(),
        "active batch still owns byte credits"
    );
    worker.admit(request);
    assert!(batch.await.unwrap().results.iter().all(Result::is_ok));
    assert!(futures::poll!(&mut single).is_pending());
    let request = worker.admission_rx.recv().await.unwrap();
    worker.admit(request);
    single.await.unwrap();
}

#[tokio::test]
async fn shutdown_rejects_queued_batch_and_wakes_budget_waiter() {
    let mut bounds = limits();
    bounds.max_batch_messages = 1;
    bounds.max_pending_messages = 1;
    let (mut worker, client) = connected(5, 8, Some(bounds)).await;
    let mut first = Box::pin(client.publish_batch_admitted(vec![command(1, 0)]));
    let mut waiting = Box::pin(client.publish_batch_admitted(vec![command(2, 0)]));
    assert!(futures::poll!(&mut first).is_pending());
    assert!(futures::poll!(&mut waiting).is_pending());
    worker.close_admissions().await.unwrap();
    assert!(matches!(
        first.await.unwrap().results[0],
        Err(MqttClientError::OperationCancelled { .. })
    ));
    assert!(matches!(
        waiting.await,
        Err(PublishBatchAdmissionError::NotSubmitted { .. })
    ));
    assert!(!worker.engine.has_pending_output());
}

#[tokio::test]
async fn response_loss_is_unknown_but_dropping_receiver_does_not_retract_batch() {
    let (mut worker, client) = connected(5, 8, Some(limits())).await;
    let mut lost = Box::pin(client.publish_batch_admitted(vec![command(1, 0)]));
    assert!(futures::poll!(&mut lost).is_pending());
    drop(worker.admission_rx.recv().await.unwrap());
    assert!(matches!(
        lost.await,
        Err(PublishBatchAdmissionError::OutcomeUnknown { .. })
    ));
    let mut cancelled = Box::pin(client.publish_batch_admitted(vec![command(2, 0), command(3, 0)]));
    assert!(futures::poll!(&mut cancelled).is_pending());
    drop(cancelled);
    let request = worker.admission_rx.recv().await.unwrap();
    worker.admit(request);
    let bytes = worker.engine.take_outgoing();
    assert!(bytes.windows(32).any(|part| part == [2; 32]));
    assert!(bytes.windows(32).any(|part| part == [3; 32]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn competing_reservations_finish_without_lost_wakes() {
    let mut bounds = limits();
    bounds.max_batch_messages = 2;
    bounds.max_pending_messages = 2;
    let (mut worker, client) = connected(5, 16, Some(bounds)).await;
    let client = Arc::new(client);
    let mut tasks = Vec::new();
    for value in 0..8 {
        let client = Arc::clone(&client);
        tasks.push(tokio::spawn(async move {
            for _ in 0..16 {
                let batch = vec![command(value, 0); 1 + usize::from(value % 2)];
                assert!(client
                    .publish_batch_admitted(batch)
                    .await
                    .unwrap()
                    .results
                    .iter()
                    .all(Result::is_ok));
            }
        }));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..128 {
            let request = worker.admission_rx.recv().await.unwrap();
            worker.admit(request);
            worker.engine.take_outgoing();
        }
        for task in tasks {
            task.await.unwrap();
        }
    })
    .await
    .expect("every budget waiter must make progress");
}

#[tokio::test]
async fn invalid_admission_config_is_rejected_before_worker_spawn() {
    let invalid = [
        PublishAdmissionConfig {
            max_batch_messages: 0,
            ..limits()
        },
        PublishAdmissionConfig {
            max_batch_bytes: 0,
            ..limits()
        },
        PublishAdmissionConfig {
            max_pending_messages: 0,
            ..limits()
        },
        PublishAdmissionConfig {
            max_pending_bytes: 0,
            ..limits()
        },
        PublishAdmissionConfig {
            max_pending_messages: 1,
            ..limits()
        },
        PublishAdmissionConfig {
            max_pending_bytes: 1,
            ..limits()
        },
    ];
    for bounds in invalid {
        let mut options = MqttClientOptions::default();
        let config = TokioAsyncClientConfig::builder()
            .publish_admission(bounds)
            .build();
        assert!(
            matches!(resolve_options(&mut options, &config), Err(MqttClientError::InvalidConfiguration { field, .. }) if field == "publish_admission")
        );
    }
}

#[cfg(feature = "shared-payloads")]
#[tokio::test]
async fn shared_batch_keeps_payload_allocations_and_releases_rejected_storage() {
    use crate::mqtt_client::payload_tests::tracked_payload;
    let (mut worker, client) = connected(5, 16, Some(limits())).await;
    let (shared, owner) = tracked_payload(256);
    let retained = shared.clone();
    let mut bad = PublishCommand::from_bytes("rejected", shared, 3, false);
    bad.properties
        .push(Property::ContentType("application/json".into()));
    let mut publish = Box::pin(client.publish_batch_admitted(vec![bad]));
    assert!(futures::poll!(&mut publish).is_pending());
    let request = worker.admission_rx.recv().await.unwrap();
    let PublishAdmission::Batch { commands, .. } = &request else {
        panic!("batch envelope")
    };
    assert_eq!(commands[0].payload.as_ptr(), retained.as_ptr());
    drop(retained);
    assert!(owner.upgrade().is_some());
    worker.admit(request);
    assert!(publish.await.unwrap().results[0].is_err());
    assert!(owner.upgrade().is_none());
}

#[tokio::test]
async fn graceful_shutdown_drains_single_and_batch_permits_reserved_before_close() {
    for batch in [false, true] {
        let (mut worker, client) = connected(5, 8, Some(limits())).await;
        let slot = client.admission_tx.reserve().await.unwrap();
        let (closed, ()) = tokio::join!(worker.close_admissions(), async {
            client.admission_tx.closed().await;
            if batch {
                let (response_tx, response_rx) = oneshot::channel();
                slot.send(PublishAdmission::Batch {
                    commands: vec![command(1, 0), command(2, 0)],
                    response_tx,
                    _permit: None,
                });
                let report = response_rx
                    .await
                    .expect("graceful close must not lose batch report");
                assert_eq!(report.results.len(), 2);
                assert!(report.results.iter().all(|result| matches!(
                    result,
                    Err(MqttClientError::OperationCancelled { .. })
                )));
            } else {
                let (response_tx, response_rx) = oneshot::channel();
                slot.send(PublishAdmission::Single {
                    command: command(1, 0),
                    response_tx,
                    _permit: None,
                });
                assert!(matches!(
                    response_rx.await.unwrap(),
                    Err(MqttClientError::OperationCancelled { .. })
                ));
            }
        });
        closed.unwrap();
        assert!(!worker.engine.has_pending_output());
    }
}

#[tokio::test]
async fn shutdown_wait_for_a_stalled_admission_permit_is_bounded() {
    let (mut worker, client) = connected(5, 8, Some(limits())).await;
    worker.config.default_operation_timeout_ms = 2;
    let slot = client.admission_tx.reserve().await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), worker.close_admissions())
        .await
        .expect("shutdown cannot hang on an outstanding permit")
        .unwrap_err();
    assert!(
        matches!(error, MqttClientError::OperationTimeout { operation, .. } if operation == "publish admission shutdown")
    );
    drop(slot);
    assert!(client.admission_tx.is_closed());
}

#[cfg(feature = "shared-payloads")]
#[tokio::test]
async fn accepted_shared_batch_retains_storage_until_all_qos1_acks() {
    use crate::mqtt_client::payload_tests::tracked_payload;
    for version in [4, 5] {
        let (mut worker, client) = connected(version, 16, Some(limits())).await;
        let (payload, owner) = tracked_payload(256);
        let pointer = payload.as_ptr();
        let mut publish = Box::pin(client.publish_batch_admitted(vec![
            PublishCommand::from_bytes("first", payload.clone(), 1, false),
            PublishCommand::from_bytes("second", payload, 1, false),
        ]));
        assert!(futures::poll!(&mut publish).is_pending());
        let request = worker.admission_rx.recv().await.unwrap();
        let PublishAdmission::Batch { commands, .. } = &request else {
            panic!("batch envelope")
        };
        assert!(commands
            .iter()
            .all(|command| command.payload.as_ptr() == pointer));
        worker.admit(request);
        let report = publish.await.unwrap();
        let ids: Vec<_> = report
            .results
            .into_iter()
            .map(|result| result.unwrap().unwrap())
            .collect();
        worker.engine.take_outgoing();
        assert!(owner.upgrade().is_some());
        for (index, id) in ids.into_iter().enumerate() {
            let [high, low] = id.to_be_bytes();
            worker.engine.handle_incoming(&[0x40, 2, high, low]);
            assert_eq!(owner.upgrade().is_some(), index == 0);
        }
    }
}
