// SPDX-License-Identifier: Apache-2.0
use super::super::tracked::{PublishOperation, Receipt, State, TrackedCommand};
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
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut output = self.bytes.lock().unwrap();
        let available = self.limit.saturating_sub(output.len());
        if available == 0 {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let len = available.min(bytes.len());
        output.extend_from_slice(&bytes[..len]);
        Poll::Ready(Ok(len))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
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
    }));
    (worker, tx, bytes)
}

fn request(qos: u8, deadline: tokio::time::Instant) -> (TrackedCommand, PublishOperation) {
    let (changed, _) = tokio::sync::watch::channel(());
    let receipt = Arc::new(Receipt {
        state: Mutex::new(State {
            generation: None,
            admitted: false,
            cancelled: false,
            result: None,
            written: None,
        }),
        changed,
        deadline,
    });
    (
        TrackedCommand {
            command: PublishCommand::simple("t", b"body".to_vec(), qos, false),
            receipt: receipt.clone(),
        },
        PublishOperation { receipt },
    )
}
fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(1)
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
        worker.admit_tracked(&command);
        first.wait().await.unwrap();
        let first_drain = first.drain_receipt();
        assert!(bytes.lock().unwrap().is_empty());
        assert!(!first_drain.is_written());
        worker.handle_outgoing().await.unwrap();
        first_drain.drain_through(deadline()).await.unwrap();
        let (command, mut second) = request(0, deadline());
        worker.admit_tracked(&command);
        second.wait().await.unwrap();
        let second_drain = second.drain_receipt();
        let partial = Arc::new(Mutex::new(Vec::new()));
        worker.stream = Some(Box::new(Wire {
            bytes: partial.clone(),
            limit: 2,
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
        worker.admit_tracked(&command);
        let new_drain = super::super::tracked::PublishOperation {
            receipt: command.receipt.clone(),
        }
        .drain_receipt();
        assert!(!first_drain.same_connection(&new_drain));
    }
}

#[tokio::test]
async fn cancelled_or_expired_before_admission_never_reaches_wire() {
    let (mut worker, _tx, bytes) = worker(5).await;
    let (command, operation) = request(1, deadline());
    drop(operation);
    worker.admit_tracked(&command);
    let (expired, mut operation) = request(1, tokio::time::Instant::now());
    assert!(!operation.wait().await.unwrap_err().admitted);
    worker.admit_tracked(&expired);
    worker.handle_outgoing().await.unwrap();
    assert!(bytes.lock().unwrap().is_empty());
    assert!(worker.tracked.is_empty());
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
                worker.admit_tracked(&command);
                let id = *worker.tracked.keys().next().unwrap();
                worker.handle_outgoing().await.unwrap();
                assert!(operation.drain_receipt().is_written());
                assert!(operation.receipt.state.lock().unwrap().result.is_none());
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
                    assert!(operation.receipt.state.lock().unwrap().result.is_none());
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
    worker.admit_tracked(&command);
    worker.handle_outgoing().await.unwrap();
    let first_id = *worker.tracked.keys().next().unwrap();
    assert!(first.wait().await.unwrap_err().admitted);
    let (command, mut second) = request(1, deadline());
    worker.admit_tracked(&command);
    worker.handle_outgoing().await.unwrap();
    let second_id = *worker.tracked.keys().find(|&&id| id != first_id).unwrap();
    test_support::receive(
        &mut worker,
        &[0x40, 2, (first_id >> 8) as u8, first_id as u8],
    )
    .await;
    assert!(second.receipt.state.lock().unwrap().result.is_none());
    assert!(worker.tracked.contains_key(&second_id));
    drop(worker);
    assert!(second.wait().await.unwrap_err().admitted);
}

#[tokio::test]
async fn intentional_disconnect_and_terminal_loss_wake_receipts_immediately() {
    for disconnect in [false, true] {
        let (mut worker, _tx, _) = worker(5).await;
        worker.config.auto_reconnect = false;
        let (command, mut operation) = request(1, deadline());
        worker.admit_tracked(&command);
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
