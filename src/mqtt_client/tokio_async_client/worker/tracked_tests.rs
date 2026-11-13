// SPDX-License-Identifier: Apache-2.0
use super::super::tracked::{PublishOperation, Receipt, TrackedCommand};
use super::*;
use crate::mqtt_client::transport::TransportError;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct Handler;
#[async_trait]
impl TokioMqttEventHandler for Handler {}

struct Wire {
    bytes: Arc<Mutex<Vec<u8>>>,
    limit: usize,
    write_gate: Option<oneshot::Receiver<()>>,
    flush_gate: Option<oneshot::Receiver<io::Result<()>>>,
}
impl AsyncRead for Wire {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}
impl AsyncWrite for Wire {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(gate) = self.write_gate.as_mut() {
            std::task::ready!(Pin::new(gate).poll(cx)).unwrap();
            self.write_gate = None;
        }
        let mut output = self.bytes.lock().unwrap();
        let available = self.limit.saturating_sub(output.len());
        if available == 0 {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let len = available.min(bytes.len());
        output.extend_from_slice(&bytes[..len]);
        Poll::Ready(Ok(len))
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(gate) = self.flush_gate.as_mut() {
            let result = std::task::ready!(Pin::new(gate).poll(cx)).unwrap();
            self.flush_gate = None;
            return Poll::Ready(result);
        }
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
#[async_trait]
impl Transport for Wire {
    async fn connect(_: &str) -> Result<Self, TransportError> {
        unreachable!()
    }
    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
    fn peer_addr(&self) -> Result<String, TransportError> {
        Ok("test".into())
    }
    fn local_addr(&self) -> Result<String, TransportError> {
        Ok("test".into())
    }
}

async fn worker(
    version: u8,
) -> (
    TokioClientWorker,
    mpsc::Sender<TokioClientCommand>,
    Arc<Mutex<Vec<u8>>>,
) {
    let (mut worker, tx) = test_support::connected_worker(
        MqttClientOptions::builder()
            .mqtt_version(version)
            .keep_alive(0)
            .build(),
        Box::new(Handler),
        TokioAsyncClientConfig::default(),
    )
    .await;
    let bytes = Arc::new(Mutex::new(Vec::new()));
    worker.stream = Some(Box::new(Wire {
        bytes: bytes.clone(),
        limit: usize::MAX,
        write_gate: None,
        flush_gate: None,
    }));
    (worker, tx, bytes)
}

fn request(qos: u8, deadline: tokio::time::Instant) -> (TrackedCommand, PublishOperation) {
    let receipt = Arc::new(Receipt::new(deadline));
    (
        TrackedCommand {
            command: PublishCommand::simple("t", b"body".to_vec(), qos, false),
            receipt: receipt.clone(),
        },
        PublishOperation::new(receipt),
    )
}
fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(1)
}

async fn queued_publish_continues(expired: bool) {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let (worker, command_tx, bytes) = worker(version).await;
            let config = worker.config.clone();
            let client = TokioAsyncMqttClient {
                command_tx,
                admission_tx: mpsc::channel(1).0,
                admission_budget: None,
                _config: config,
                worker: tokio::spawn(worker.run()),
            };
            let until = if expired {
                tokio::time::Instant::now()
            } else {
                deadline()
            };
            let mut operation = client
                .try_publish_tracked(
                    PublishCommand::simple("t", b"body".to_vec(), qos, false),
                    until,
                )
                .unwrap();
            let drain = operation.drain_receipt();
            if expired {
                assert!(matches!(
                    operation.wait().await.unwrap_err().error,
                    MqttClientError::OperationTimeout { .. }
                ));
            }
            // The current-thread worker has not run yet: dropping the handle
            // or expiring its confirmation window must not retract this request.
            drop(operation);
            drain.drain_through(deadline()).await.unwrap();
            assert!(bytes.lock().unwrap().ends_with(b"body"));
            client.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn queued_tracked_publish_survives_dropped_operation() {
    queued_publish_continues(false).await;
}

#[tokio::test]
async fn queued_tracked_publish_survives_expired_confirmation_deadline() {
    queued_publish_continues(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmation_and_cloned_drains_wake_across_threads() {
    for stop_worker in [false, true] {
        let (mut worker, _tx, _) = worker(5).await;
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        let (command, mut operation) = request(1, until);
        worker.admit_tracked(&command).await;
        let id = *worker.tracked.keys().next().unwrap();
        let (ready_tx, mut ready_rx) = mpsc::channel(9);
        let mut drains = Vec::new();
        for _ in 0..8 {
            let drain = operation.drain_receipt();
            let ready = ready_tx.clone();
            drains.push(tokio::spawn(async move {
                let mut waiting = Box::pin(drain.drain_through(until));
                assert!(futures::poll!(waiting.as_mut()).is_pending());
                ready.send(()).await.unwrap();
                waiting.await
            }));
        }
        let confirmation = tokio::spawn(async move {
            // Dropping a registered waiter must not disturb subsequent waiters.
            {
                let mut cancelled = Box::pin(operation.wait());
                assert!(futures::poll!(cancelled.as_mut()).is_pending());
            }
            let mut waiting = Box::pin(operation.wait());
            assert!(futures::poll!(waiting.as_mut()).is_pending());
            ready_tx.send(()).await.unwrap();
            waiting.await
        });
        for _ in 0..9 {
            ready_rx.recv().await.unwrap();
        }
        if stop_worker {
            drop(worker);
        } else {
            worker.handle_outgoing().await.unwrap();
            test_support::receive(&mut worker, &[0x40, 2, (id >> 8) as u8, id as u8]).await;
        }
        // Finish well before receipt deadlines: a missed notification must not
        // be hidden by a timer eventually waking the same task.
        tokio::time::timeout(Duration::from_secs(1), async {
            for waiter in drains {
                let result = waiter.await.unwrap();
                if stop_worker {
                    assert!(matches!(
                        result,
                        Err(MqttClientError::OperationCancelled { .. })
                    ));
                } else {
                    result.unwrap();
                }
            }
            let result = confirmation.await.unwrap();
            if stop_worker {
                let error = result.unwrap_err();
                assert!(error.admitted);
                assert!(matches!(
                    error.error,
                    MqttClientError::OperationCancelled { .. }
                ));
            } else {
                assert!(result.unwrap().is_success());
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn stopping_worker_fails_queued_confirmation_and_drain() {
    let (worker, tx, _) = worker(5).await;
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    let (command, mut operation) = request(0, until);
    tx.try_send(TokioClientCommand::PublishTracked(command))
        .unwrap();
    let drain = operation.drain_receipt();
    let mut confirmation = Box::pin(operation.wait());
    let mut draining = Box::pin(drain.drain_through(until));
    assert!(futures::poll!(confirmation.as_mut()).is_pending());
    assert!(futures::poll!(draining.as_mut()).is_pending());
    drop(worker);
    let (confirmation, draining) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(confirmation, draining)
    })
    .await
    .unwrap();
    let error = confirmation.unwrap_err();
    assert!(!error.admitted);
    assert!(matches!(
        error.error,
        MqttClientError::OperationCancelled { .. }
    ));
    assert!(matches!(
        draining,
        Err(MqttClientError::OperationCancelled { .. })
    ));
}

#[test]
fn reconnect_only_policy_disables_timer_output_but_preserves_session_replay() {
    let mut options = MqttClientOptions::builder()
        .mqtt_version(4)
        .clean_start(false)
        .keep_alive(0)
        .build();
    options.retransmit_on_timer = false;
    let now = Instant::now();
    let later = now + Duration::from_secs(10);
    let mut engine = MqttEngine::new_at(options, now);
    engine.connect_at(now).unwrap();
    engine.take_outgoing_at(now).unwrap();
    engine.handle_incoming_at(&[0x20, 2, 0, 0], now).unwrap();
    let id = engine
        .publish_at(
            PublishCommand::simple("t", b"recover".to_vec(), 1, false),
            now,
        )
        .unwrap()
        .unwrap();
    assert_eq!(engine.take_outgoing_at(now).unwrap()[0], 0x32);
    engine.handle_tick_at(later).unwrap();
    assert!(engine.take_outgoing_at(later).unwrap().is_empty());
    assert!(engine.next_tick_at().is_none());
    engine.handle_connection_lost_at(later).unwrap();
    engine.connect_at(later).unwrap();
    engine.take_outgoing_at(later).unwrap();
    engine.handle_incoming_at(&[0x20, 2, 1, 0], later).unwrap();
    let replay = engine.take_outgoing_at(later).unwrap();
    assert_eq!(replay[0], 0x3a);
    assert!(replay.ends_with(b"recover"));
    let events = engine
        .handle_incoming_at(&[0x40, 2, (id >> 8) as u8, id as u8], later)
        .unwrap();
    assert!(events.iter().any(
        |event| matches!(event, MqttEvent::Published(result) if result.packet_id == Some(id))
    ));
}

#[tokio::test]
async fn qos0_admission_and_write_are_distinct_and_old_fences_survive_loss() {
    for version in [4, 5] {
        let (mut worker, _tx, bytes) = worker(version).await;
        let (command, mut first) = request(0, deadline());
        worker.admit_tracked(&command).await;
        first.wait().await.unwrap();
        let first_drain = first.drain_receipt();
        assert!(bytes.lock().unwrap().is_empty());
        assert!(!first_drain.is_written());
        worker.handle_outgoing().await.unwrap();
        first_drain.drain_through(deadline()).await.unwrap();
        let (command, mut second) = request(0, deadline());
        worker.admit_tracked(&command).await;
        second.wait().await.unwrap();
        let second_drain = second.drain_receipt();
        let partial = Arc::new(Mutex::new(Vec::new()));
        worker.stream = Some(Box::new(Wire {
            bytes: partial.clone(),
            limit: 2,
            write_gate: None,
            flush_gate: None,
        }));
        assert!(worker.handle_outgoing().await.is_err());
        assert_eq!(partial.lock().unwrap().len(), 2);
        worker.record_connection_lost(None);
        first_drain.drain_through(deadline()).await.unwrap();
        assert!(second_drain.drain_through(deadline()).await.is_err());
        // QoS 0 encoded bytes never enter the engine reconnect replay queue.
        worker.engine.reset_for_new_transport();
        worker.engine.connect().unwrap();
        worker.engine.take_outgoing();
        let connack: &[u8] = if version == 5 {
            &[0x20, 3, 0, 0, 0]
        } else {
            &[0x20, 2, 0, 0]
        };
        test_support::receive(&mut worker, connack).await;
        assert!(worker.engine.take_outgoing().is_empty());
        let (command, _operation) = request(0, deadline());
        worker.admit_tracked(&command).await;
        let new_drain = PublishOperation::new(command.receipt.clone()).drain_receipt();
        assert!(!first_drain.same_connection(&new_drain));
    }
}

#[tokio::test(start_paused = true)]
async fn tracked_drain_waits_for_flush_and_preserves_success_after_loss() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let (mut worker, _tx, bytes) = worker(version).await;
            let (flush_tx, flush_rx) = oneshot::channel();
            worker.stream = Some(Box::new(Wire {
                bytes: bytes.clone(),
                limit: usize::MAX,
                write_gate: None,
                flush_gate: Some(flush_rx),
            }));
            let (command, mut operation) = request(qos, deadline());
            worker.admit_tracked(&command).await;
            if qos == 0 {
                operation.wait().await.unwrap();
            }
            let drain = operation.drain_receipt();
            let mut outgoing = Box::pin(worker.handle_outgoing());
            assert!(futures::poll!(outgoing.as_mut()).is_pending());
            // write_all accepted the payload, but the transport still needs flushing.
            assert!(bytes.lock().unwrap().ends_with(b"body"));
            assert!(!drain.is_written());
            let mut draining = Box::pin(drain.drain_through(deadline()));
            assert!(futures::poll!(draining.as_mut()).is_pending());

            flush_tx.send(Ok(())).unwrap();
            outgoing.await.unwrap();
            draining.await.unwrap();
            assert!(drain.is_written());
            worker.record_connection_lost(None);
            drain.drain_through(deadline()).await.unwrap();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn tracked_drain_preserves_flush_failure() {
    for qos in [0, 1, 2] {
        let (mut worker, _tx, bytes) = worker(5).await;
        let (flush_tx, flush_rx) = oneshot::channel();
        worker.stream = Some(Box::new(Wire {
            bytes: bytes.clone(),
            limit: usize::MAX,
            write_gate: None,
            flush_gate: Some(flush_rx),
        }));
        let (command, mut operation) = request(qos, deadline());
        worker.admit_tracked(&command).await;
        flush_tx
            .send(Err(io::ErrorKind::BrokenPipe.into()))
            .unwrap();
        let error = worker.handle_outgoing().await.unwrap_err();
        assert!(matches!(
            error,
            MqttClientError::NetworkError {
                kind: io::ErrorKind::BrokenPipe,
                ..
            }
        ));
        assert!(bytes.lock().unwrap().ends_with(b"body"));
        let drain = operation.drain_receipt();
        assert!(!drain.is_written());
        assert_eq!(
            drain
                .drain_through(deadline())
                .await
                .unwrap_err()
                .to_string(),
            error.to_string()
        );
        worker.record_connection_lost(None);
        assert_eq!(
            drain
                .drain_through(deadline())
                .await
                .unwrap_err()
                .to_string(),
            error.to_string()
        );
        if qos == 0 {
            // Admission succeeded even though local transport draining failed.
            operation.wait().await.unwrap();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn tracked_write_and_flush_share_the_transport_timeout() {
    let (mut worker, _tx, bytes) = worker(5).await;
    worker.config.default_operation_timeout_ms = 100;
    let (write_tx, write_rx) = oneshot::channel();
    let (_flush_tx, flush_rx) = oneshot::channel();
    worker.stream = Some(Box::new(Wire {
        bytes: bytes.clone(),
        limit: usize::MAX,
        write_gate: Some(write_rx),
        flush_gate: Some(flush_rx),
    }));
    let (command, operation) = request(0, deadline());
    worker.admit_tracked(&command).await;
    let drain = operation.drain_receipt();
    let mut outgoing = Box::pin(worker.handle_outgoing());
    assert!(futures::poll!(outgoing.as_mut()).is_pending());
    tokio::time::advance(Duration::from_millis(60)).await;
    write_tx.send(()).unwrap();
    assert!(futures::poll!(outgoing.as_mut()).is_pending());
    assert!(bytes.lock().unwrap().ends_with(b"body"));
    tokio::time::advance(Duration::from_millis(40)).await;
    assert!(matches!(
        futures::poll!(outgoing.as_mut()),
        Poll::Ready(Err(MqttClientError::OperationTimeout {
            timeout_ms: 100,
            ..
        }))
    ));
    drop(outgoing);
    let error = drain.drain_through(deadline()).await.unwrap_err();
    assert!(matches!(
        error,
        MqttClientError::OperationTimeout {
            timeout_ms: 100,
            ..
        }
    ));
    worker.record_connection_lost(None);
    assert_eq!(
        drain
            .drain_through(deadline())
            .await
            .unwrap_err()
            .to_string(),
        error.to_string()
    );
}

#[tokio::test]
async fn qos1_and_qos2_wait_for_final_ack_and_preserve_rejection_codes() {
    for version in [4, 5] {
        for qos in [1, 2] {
            for reason in if version == 5 {
                vec![0, 0x87, 0x97]
            } else {
                vec![0]
            } {
                let (mut worker, _tx, _) = worker(version).await;
                let (command, mut operation) = request(qos, deadline());
                worker.admit_tracked(&command).await;
                let id = *worker.tracked.keys().next().unwrap();
                worker.handle_outgoing().await.unwrap();
                assert!(operation.drain_receipt().is_written());
                assert!(operation.receipt.completion.get().is_none());
                // Dropping a wait future leaves the same operation resumable.
                assert!(
                    tokio::time::timeout(Duration::from_millis(1), operation.wait())
                        .await
                        .is_err()
                );
                let mut ack = vec![
                    if qos == 1 { 0x40 } else { 0x50 },
                    2,
                    (id >> 8) as u8,
                    id as u8,
                ];
                if version == 5 {
                    ack[1] = 4;
                    ack.extend([reason, 0]);
                }
                test_support::receive(&mut worker, &ack).await;
                if qos == 2 && reason == 0 {
                    assert!(operation.receipt.completion.get().is_none());
                    worker.handle_outgoing().await.unwrap();
                    test_support::receive(&mut worker, &[0x70, 2, (id >> 8) as u8, id as u8]).await;
                }
                let result = operation.wait().await.unwrap();
                assert_eq!(result.is_success(), reason == 0);
                assert_eq!(result.reason_code.unwrap_or(0), reason);
            }
        }
    }
}

#[tokio::test]
async fn late_ack_does_not_complete_another_operation_and_worker_drop_fails_pending() {
    let (mut worker, _tx, _) = worker(5).await;
    let (command, mut first) = request(1, tokio::time::Instant::now() + Duration::from_millis(5));
    worker.admit_tracked(&command).await;
    worker.handle_outgoing().await.unwrap();
    let first_id = *worker.tracked.keys().next().unwrap();
    assert!(first.wait().await.unwrap_err().admitted);
    let (command, mut second) = request(1, deadline());
    worker.admit_tracked(&command).await;
    worker.handle_outgoing().await.unwrap();
    let second_id = *worker.tracked.keys().find(|&&id| id != first_id).unwrap();
    test_support::receive(
        &mut worker,
        &[0x40, 2, (first_id >> 8) as u8, first_id as u8],
    )
    .await;
    assert!(second.receipt.completion.get().is_none());
    assert!(worker.tracked.contains_key(&second_id));
    drop(worker);
    assert!(second.wait().await.unwrap_err().admitted);
}

#[tokio::test(start_paused = true)]
async fn cancelled_wait_keeps_original_confirmation_deadline() {
    for version in [4, 5] {
        for qos in [1, 2] {
            for ack_delay_ms in [999, 1000, 1001] {
                let (mut worker, _tx, _) = worker(version).await;
                let (command, mut operation) = request(qos, deadline());
                worker.admit_tracked(&command).await;
                let id = *worker.tracked.keys().next().unwrap();
                worker.handle_outgoing().await.unwrap();
                {
                    let mut wait = Box::pin(operation.wait());
                    assert!(futures::poll!(wait.as_mut()).is_pending());
                }
                if qos == 2 {
                    test_support::receive(&mut worker, &[0x50, 2, (id >> 8) as u8, id as u8]).await;
                    worker.handle_outgoing().await.unwrap();
                    assert!(operation.receipt.completion.get().is_none());
                }

                tokio::time::advance(Duration::from_millis(ack_delay_ms)).await;
                let ack = [
                    if qos == 1 { 0x40 } else { 0x70 },
                    2,
                    (id >> 8) as u8,
                    id as u8,
                ];
                test_support::receive(&mut worker, &ack).await;
                // The final ACK still finishes the exchange and releases its tracking.
                assert!(worker.tracked.is_empty());
                assert!(operation.drain_receipt().is_written());

                // Resume only after the deadline, even for a timely confirmation.
                tokio::time::advance(Duration::from_secs(1)).await;
                for _ in 0..2 {
                    let result = operation.wait().await;
                    if ack_delay_ms < 1000 {
                        assert!(result.unwrap().is_success());
                    } else {
                        let error = result.unwrap_err();
                        assert!(error.admitted);
                        assert!(matches!(
                            error.error,
                            MqttClientError::OperationTimeout { .. }
                        ));
                    }
                }
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn cancelled_wait_preserves_only_timely_broker_rejections() {
    for qos in [1, 2] {
        for ack_delay_ms in [999, 1000, 1001] {
            let (mut worker, _tx, _) = worker(5).await;
            let (command, mut operation) = request(qos, deadline());
            worker.admit_tracked(&command).await;
            let id = *worker.tracked.keys().next().unwrap();
            worker.handle_outgoing().await.unwrap();
            {
                let mut wait = Box::pin(operation.wait());
                assert!(futures::poll!(wait.as_mut()).is_pending());
            }
            tokio::time::advance(Duration::from_millis(ack_delay_ms)).await;
            let ack = [
                if qos == 1 { 0x40 } else { 0x50 },
                4,
                (id >> 8) as u8,
                id as u8,
                0x87,
                0,
            ];
            test_support::receive(&mut worker, &ack).await;
            assert!(worker.tracked.is_empty());
            tokio::time::advance(Duration::from_secs(1)).await;
            let result = operation.wait().await;
            if ack_delay_ms < 1000 {
                assert_eq!(result.unwrap().reason_code, Some(0x87));
            } else {
                let error = result.unwrap_err();
                assert!(error.admitted);
                assert!(matches!(
                    error.error,
                    MqttClientError::OperationTimeout { .. }
                ));
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn confirmation_timeout_does_not_cancel_later_admission_or_poison_drain() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            for wait_before_admission in [false, true] {
                let (mut worker, _tx, bytes) = worker(version).await;
                let (command, mut operation) =
                    request(qos, tokio::time::Instant::now() + Duration::from_millis(1));
                tokio::time::advance(Duration::from_millis(2)).await;
                if wait_before_admission {
                    let error = operation.wait().await.unwrap_err();
                    assert!(!error.admitted);
                    assert!(matches!(
                        error.error,
                        MqttClientError::OperationTimeout { .. }
                    ));
                }
                worker.admit_tracked(&command).await;
                worker.handle_outgoing().await.unwrap();
                operation
                    .drain_receipt()
                    .drain_through(deadline())
                    .await
                    .unwrap();
                assert!(bytes.lock().unwrap().ends_with(b"body"));
                if qos > 0 {
                    let id = *worker.tracked.keys().next().unwrap();
                    if qos == 2 {
                        test_support::receive(&mut worker, &[0x50, 2, (id >> 8) as u8, id as u8])
                            .await;
                        worker.handle_outgoing().await.unwrap();
                    }
                    let ack = [
                        if qos == 1 { 0x40 } else { 0x70 },
                        2,
                        (id >> 8) as u8,
                        id as u8,
                    ];
                    test_support::receive(&mut worker, &ack).await;
                }
                assert!(worker.tracked.is_empty());
                // The exchange finishes, but its confirmation missed the original
                // deadline whether or not the caller was waiting at that time.
                for _ in 0..2 {
                    let error = operation.wait().await.unwrap_err();
                    assert!(error.admitted);
                    assert!(matches!(
                        error.error,
                        MqttClientError::OperationTimeout { .. }
                    ));
                }
            }
        }
    }
}

#[tokio::test]
async fn intentional_disconnect_and_terminal_loss_wake_receipts_immediately() {
    for disconnect in [false, true] {
        let (mut worker, _tx, _) = worker(5).await;
        worker.config.auto_reconnect = false;
        let (command, mut operation) = request(1, deadline());
        worker.admit_tracked(&command).await;
        worker.handle_outgoing().await.unwrap();
        if disconnect {
            worker.disconnect(0, vec![]).await.unwrap();
        } else {
            worker.record_connection_lost(None);
        }
        let result = tokio::time::timeout(Duration::from_millis(20), operation.wait())
            .await
            .unwrap();
        assert!(result.unwrap_err().admitted);
        assert!(worker.tracked.is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn connection_identity_changes_after_intentional_and_unexpected_reconnects() {
    for version in [4, 5] {
        for intentional in [true, false] {
            let (mut worker, _tx, _) = worker(version).await;
            worker
                .handle_command(TokioClientCommand::SetAutoReconnect { enabled: true })
                .await;
            let mut old_drains = Vec::new();
            for qos in [0, 1, 2] {
                let (command, operation) = request(qos, deadline());
                worker.admit_tracked(&command).await;
                worker.handle_outgoing().await.unwrap();
                old_drains.push(operation.drain_receipt());
            }
            for drain in &old_drains {
                assert!(drain.same_connection(&old_drains[0]));
            }
            if intentional {
                let (response_tx, response_rx) = oneshot::channel();
                worker
                    .handle_command(TokioClientCommand::Disconnect {
                        reason_code: 0,
                        properties: vec![],
                        response_tx: Some(response_tx),
                    })
                    .await;
                response_rx.await.unwrap().unwrap();
            } else {
                worker.record_connection_lost(None);
                assert_eq!(worker.tracked.len(), 2);
            }

            let (response_tx, response_rx) = oneshot::channel();
            worker
                .handle_command(TokioClientCommand::ConnectSync { response_tx })
                .await;
            // Supply a fresh fake transport in place of the unpolled network dial.
            worker.connecting = None;
            worker
                .transport_connected(Ok(Box::new(Wire {
                    bytes: Arc::new(Mutex::new(Vec::new())),
                    limit: usize::MAX,
                    write_gate: None,
                    flush_gate: None,
                })))
                .await;
            worker.handle_outgoing().await.unwrap();
            let connack: &[u8] = if version == 5 {
                &[0x20, 3, 0, 0, 0]
            } else {
                &[0x20, 2, 0, 0]
            };
            test_support::receive(&mut worker, connack).await;
            assert!(response_rx.await.unwrap().unwrap().is_success());

            let (command, operation) = request(0, deadline());
            worker.admit_tracked(&command).await;
            worker.handle_outgoing().await.unwrap();
            let new_drain = operation.drain_receipt();
            for drain in &old_drains {
                assert!(drain.same_connection(&old_drains[0]));
                assert!(!drain.same_connection(&new_drain));
                drain.drain_through(deadline()).await.unwrap();
            }
            // A redundant connect on an existing transport keeps its identity.
            let (response_tx, response_rx) = oneshot::channel();
            worker
                .handle_command(TokioClientCommand::ConnectSync { response_tx })
                .await;
            response_rx.await.unwrap().unwrap();
            let (command, operation) = request(0, deadline());
            worker.admit_tracked(&command).await;
            assert!(new_drain.same_connection(&operation.drain_receipt()));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn disabling_reconnect_while_disconnected_fails_retained_receipts() {
    for version in [4, 5] {
        for (qos, received_pubrec) in [(1, false), (2, false), (2, true)] {
            let (mut worker, _tx, _) = worker(version).await;
            worker
                .handle_command(TokioClientCommand::SetAutoReconnect { enabled: true })
                .await;
            let started = tokio::time::Instant::now();
            let (command, mut operation) = request(qos, started + Duration::from_secs(3600));
            worker.admit_tracked(&command).await;
            let id = *worker.tracked.keys().next().unwrap();
            worker.handle_outgoing().await.unwrap();
            if received_pubrec {
                test_support::receive(&mut worker, &[0x50, 2, (id >> 8) as u8, id as u8]).await;
                worker.handle_outgoing().await.unwrap();
            }
            let drain = operation.drain_receipt();
            let mut waiting = Box::pin(operation.wait());
            assert!(futures::poll!(waiting.as_mut()).is_pending());
            worker.record_connection_lost(None);
            assert!(worker.tracked.contains_key(&id));
            assert!(futures::poll!(waiting.as_mut()).is_pending());

            worker
                .handle_command(TokioClientCommand::SetAutoReconnect { enabled: false })
                .await;
            assert!(worker.tracked.is_empty());
            let Poll::Ready(Err(error)) = futures::poll!(waiting.as_mut()) else {
                panic!("stopping recovery must immediately fail the pending receipt");
            };
            assert!(error.admitted);
            assert!(matches!(error.error, MqttClientError::NotConnected));
            drop(waiting);
            assert_eq!(tokio::time::Instant::now(), started);
            drain.drain_through(deadline()).await.unwrap();

            worker
                .handle_command(TokioClientCommand::SetAutoReconnect { enabled: true })
                .await;
            assert!(worker.tracked.is_empty());
            assert!(matches!(
                operation.wait().await.unwrap_err().error,
                MqttClientError::NotConnected
            ));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn disabling_reconnect_while_connected_preserves_pending_receipts() {
    for qos in [1, 2] {
        let (mut worker, _tx, _) = worker(5).await;
        let (command, mut operation) = request(qos, deadline());
        worker.admit_tracked(&command).await;
        let id = *worker.tracked.keys().next().unwrap();
        worker.handle_outgoing().await.unwrap();
        worker
            .handle_command(TokioClientCommand::SetAutoReconnect { enabled: false })
            .await;
        assert!(worker.tracked.contains_key(&id));
        assert!(operation.receipt.completion.get().is_none());
        if qos == 2 {
            test_support::receive(&mut worker, &[0x50, 2, (id >> 8) as u8, id as u8]).await;
            worker.handle_outgoing().await.unwrap();
        }
        let ack = [
            if qos == 1 { 0x40 } else { 0x70 },
            2,
            (id >> 8) as u8,
            id as u8,
        ];
        test_support::receive(&mut worker, &ack).await;
        assert!(operation.wait().await.unwrap().is_success());
        assert!(worker.tracked.is_empty());
    }
}
