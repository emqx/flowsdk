// SPDX-License-Identifier: Apache-2.0

use super::super::tracked::{PublishOperation, Receipt, TrackedCommand};
use super::*;
use std::sync::{Arc, Mutex};

struct PauseOnSecondMessage {
    count: usize,
    reached: Option<oneshot::Sender<()>>,
    resume: Option<oneshot::Receiver<()>>,
}

#[async_trait]
impl TokioMqttEventHandler for PauseOnSecondMessage {
    async fn on_message_received(&mut self, _: &MqttMessage) {
        self.count += 1;
        if self.count == 2 {
            self.reached.take().unwrap().send(()).unwrap();
            self.resume.take().unwrap().await.unwrap();
        }
    }
}

fn request(qos: u8) -> (TrackedCommand, PublishOperation) {
    let receipt = Arc::new(Receipt::new(
        tokio::time::Instant::now() + Duration::from_secs(5),
    ));
    (
        TrackedCommand {
            command: PublishCommand::simple("out", b"body".to_vec(), qos, false),
            receipt: receipt.clone(),
        },
        PublishOperation::new(receipt),
    )
}

async fn buffered_closure(tracked: bool) {
    for version in [4, 5] {
        for protocol_failure in [false, true] {
            if version == 4 && !protocol_failure {
                continue; // Broker DISCONNECT is an MQTT 5 packet.
            }
            for qos in [0, 1, 2] {
                let (reached, second_message) = oneshot::channel();
                let (resume, resume_rx) = oneshot::channel();
                let (mut worker, commands) = test_support::connected_worker(
                    MqttClientOptions::builder()
                        .mqtt_version(version)
                        .max_event_count(1)
                        .keep_alive(0)
                        .build(),
                    Box::new(PauseOnSecondMessage {
                        count: 0,
                        reached: Some(reached),
                        resume: Some(resume_rx),
                    }),
                    TokioAsyncClientConfig::builder()
                        .auto_reconnect(false)
                        .build(),
                )
                .await;
                let (transport, mut peer) = tokio::io::duplex(1024);
                worker.stream = Some(Box::new(transport));
                let task = tokio::spawn(worker.run());

                let terminal = match (version, protocol_failure) {
                    (4, _) => vec![0x20, 2, 0, 0],
                    (5, true) => vec![0x20, 3, 0, 0, 0],
                    _ => vec![0xe0, 0],
                };
                let input = [
                    test_support::publish(version, 1, 0),
                    test_support::publish(version, 2, 0),
                    terminal,
                ]
                .concat();
                // One message is handled by the read branch, one by the output
                // drain. The terminal packet remains buffered for the next drain.
                peer.write_all(&input).await.unwrap();
                second_message.await.unwrap();
                if tracked {
                    let (request, mut operation) = request(qos);
                    commands
                        .send(TokioClientCommand::PublishTracked(request))
                        .await
                        .unwrap();
                    resume.send(()).unwrap();
                    let error = operation.wait().await.unwrap_err();
                    assert!(!error.admitted);
                    assert!(matches!(error.error, MqttClientError::NotConnected));
                    assert!(operation.receipt.generation.get().is_none());
                    assert!(matches!(
                        operation
                            .drain_receipt()
                            .drain_through(operation.receipt.deadline)
                            .await,
                        Err(MqttClientError::NotConnected)
                    ));
                } else {
                    let (response_tx, response) = oneshot::channel();
                    commands
                        .send(TokioClientCommand::PublishSync {
                            command: PublishCommand::simple("out", b"body".to_vec(), qos, false),
                            response_tx,
                        })
                        .await
                        .unwrap();
                    resume.send(()).unwrap();
                    let result = response.await.unwrap();
                    if qos == 0 {
                        // Legacy QoS 0 reports admission, which precedes the drain.
                        result.unwrap();
                    } else {
                        assert!(matches!(
                            result,
                            Err(MqttClientError::ConnectionLost { .. })
                        ));
                    }
                }
                let mut output = Vec::new();
                peer.read_to_end(&mut output).await.unwrap();
                assert!(
                    output.is_empty(),
                    "version={version}, qos={qos}, protocol_failure={protocol_failure}: {output:?}"
                );
                commands.send(TokioClientCommand::Shutdown).await.unwrap();
                task.await.unwrap().unwrap();
            }
        }
    }
}

#[tokio::test]
async fn buffered_closure_rejects_tracked_admission_in_run_loop() {
    tokio::time::timeout(Duration::from_secs(5), buffered_closure(true))
        .await
        .unwrap();
}

#[tokio::test]
async fn buffered_closure_suppresses_ordinary_output_in_run_loop() {
    tokio::time::timeout(Duration::from_secs(5), buffered_closure(false))
        .await
        .unwrap();
}

struct IgnoreEvents;
#[async_trait]
impl TokioMqttEventHandler for IgnoreEvents {}

async fn connected(version: u8, capacity: usize) -> (TokioClientWorker, tokio::io::DuplexStream) {
    let (mut worker, _commands) = test_support::connected_worker(
        MqttClientOptions::builder()
            .mqtt_version(version)
            .max_event_count(1)
            .keep_alive(0)
            .build(),
        Box::new(IgnoreEvents),
        TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .build(),
    )
    .await;
    let (transport, peer) = tokio::io::duplex(capacity);
    worker.stream = Some(Box::new(transport));
    (worker, peer)
}

fn encoded(command: PublishCommand, version: u8) -> Vec<u8> {
    if version == 5 {
        MqttPacket::Publish5(command.into_mqtt_publish())
    } else {
        MqttPacket::Publish3(command.into_mqttv3_publish())
    }
    .to_bytes()
    .unwrap()
}

#[tokio::test]
async fn closure_during_drain_fails_an_already_admitted_write_fence() {
    for qos in [0, 1, 2] {
        let (mut worker, mut peer) = connected(5, 1024).await;
        let (request, mut operation) = request(qos);
        worker.admit_tracked(&request).await;
        assert!(request.receipt.is_admitted());
        let input = [test_support::publish(5, 1, 0), vec![0xe0, 0]].concat();
        test_support::receive(&mut worker, &input).await;
        assert!(worker.engine.is_connected());
        worker.handle_outgoing().await.unwrap();
        assert!(matches!(worker.state, ConnectionState::Disconnected));
        assert!(matches!(
            operation
                .drain_receipt()
                .drain_through(request.receipt.deadline)
                .await,
            Err(MqttClientError::ConnectionLost { .. })
        ));
        if qos == 0 {
            operation.wait().await.unwrap(); // Engine admission already succeeded.
        } else {
            assert!(operation.wait().await.unwrap_err().admitted);
        }
        let mut output = Vec::new();
        peer.read_to_end(&mut output).await.unwrap();
        assert!(output.is_empty());
    }
}

#[tokio::test]
async fn intentional_disconnect_preserves_pending_publication_and_disconnect_packet() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            let (mut worker, mut peer) = connected(version, 1024).await;
            let (request, _operation) = request(qos);
            worker.admit_tracked(&request).await;
            let mut command = request.command.clone();
            command.packet_id = worker.tracked.keys().next().copied();
            let mut expected = encoded(command, version);
            let disconnect = if version == 5 {
                MqttPacket::Disconnect5(
                    crate::mqtt_serde::mqttv5::disconnectv5::MqttDisconnect::new(0, vec![]),
                )
            } else {
                MqttPacket::Disconnect3(
                    crate::mqtt_serde::mqttv3::disconnectv3::MqttDisconnect::new(),
                )
            };
            expected.extend(disconnect.to_bytes().unwrap());
            worker.disconnect(0, vec![]).await.unwrap();
            let mut output = Vec::new();
            peer.read_to_end(&mut output).await.unwrap();
            assert_eq!(output, expected);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn dropping_or_expiring_wait_during_preparation_does_not_cancel_publish() {
    for expire in [false, true] {
        // The one-byte pipe stalls halfway through the preceding PINGREQ.
        let (mut worker, mut peer) = connected(5, 1).await;
        worker.engine.send_ping().unwrap();
        let (request, operation) = request(1);
        let mut operation = Some(operation);
        let drain = operation.as_ref().unwrap().drain_receipt();
        let mut preparation = Box::pin(worker.admit_tracked(&request));
        assert!(futures::poll!(preparation.as_mut()).is_pending());
        assert!(!request.receipt.is_admitted());
        if expire {
            tokio::time::advance(Duration::from_secs(5)).await;
            assert!(
                !operation
                    .as_mut()
                    .unwrap()
                    .wait()
                    .await
                    .unwrap_err()
                    .admitted
            );
        } else {
            drop(operation.take());
        }
        let mut ping = [0; 2];
        let ((), result) = tokio::join!(preparation.as_mut(), peer.read_exact(&mut ping));
        result.unwrap();
        drop(preparation);
        assert_eq!(ping, [0xc0, 0]);
        assert!(request.receipt.is_admitted());
        let mut command = request.command.clone();
        command.packet_id = worker.tracked.keys().next().copied();
        let expected = encoded(command, 5);
        let mut output = vec![0; expected.len()];
        let (write, read, drained) = tokio::join!(
            worker.handle_outgoing(),
            peer.read_exact(&mut output),
            drain.drain_through(tokio::time::Instant::now() + Duration::from_secs(1)),
        );
        write.unwrap();
        read.unwrap();
        drained.unwrap();
        assert_eq!(output, expected);
        assert!(drain.is_written());
        if expire {
            let error = operation.as_mut().unwrap().wait().await.unwrap_err();
            assert!(error.admitted);
            assert!(matches!(
                error.error,
                MqttClientError::OperationTimeout { .. }
            ));
        }
    }
}

#[tokio::test]
async fn preparation_write_failure_rejects_without_registering_publication() {
    let (mut worker, peer) = connected(5, 1024).await;
    drop(peer);
    worker.engine.send_ping().unwrap();
    let (request, mut operation) = request(1);
    worker.admit_tracked(&request).await;
    let error = operation.wait().await.unwrap_err();
    assert!(!error.admitted);
    assert!(matches!(error.error, MqttClientError::NetworkError { .. }));
    assert!(worker.tracked.is_empty() && worker.tracked_output.is_empty());
    assert!(matches!(worker.state, ConnectionState::Disconnected));
    assert_eq!(
        operation
            .drain_receipt()
            .drain_through(request.receipt.deadline)
            .await
            .unwrap_err()
            .to_string(),
        error.error.to_string()
    );
}

#[tokio::test]
async fn tracked_preparation_keeps_callback_output_before_its_publication() {
    for version in [4, 5] {
        let (mut worker, mut peer) = connected(version, 1024).await;
        let (reached, mut second_message) = oneshot::channel();
        let (resume, resume_rx) = oneshot::channel();
        worker.event_handler = Box::new(PauseOnSecondMessage {
            count: 0,
            reached: Some(reached),
            resume: Some(resume_rx),
        });
        let (admissions, admission_rx) = mpsc::channel(1);
        worker.admission_rx = admission_rx;
        worker.admission_open = true;
        let input = [
            test_support::publish(version, 1, 0),
            test_support::publish(version, 2, 0),
        ]
        .concat();
        test_support::receive(&mut worker, &input).await;
        worker.engine.send_ping().unwrap();
        let (request, mut operation) = request(0);
        let generation = worker.tracked_generation.clone();
        let mut preparation = Box::pin(worker.admit_tracked(&request));
        assert!(futures::poll!(preparation.as_mut()).is_pending());
        second_message.try_recv().unwrap();
        assert!(!request.receipt.is_admitted());
        let earlier = PublishCommand::simple("earlier", b"callback".to_vec(), 0, false);
        let (response_tx, mut response) = oneshot::channel();
        admissions
            .send(PublishAdmission::Single {
                command: earlier.clone(),
                response_tx,
                _permit: None,
            })
            .await
            .unwrap();
        assert!(futures::poll!(preparation.as_mut()).is_pending());
        response.try_recv().unwrap().unwrap();
        resume.send(()).unwrap();
        preparation.await;
        operation.wait().await.unwrap();
        assert!(Arc::ptr_eq(
            request.receipt.generation.get().unwrap(),
            &generation
        ));
        assert!(!operation.drain_receipt().is_written());
        let mut expected = vec![0xc0, 0];
        expected.extend(encoded(earlier, version));
        let mut preceding = vec![0; expected.len()];
        peer.read_exact(&mut preceding).await.unwrap();
        assert_eq!(preceding, expected);
        worker.handle_outgoing().await.unwrap();
        let expected = encoded(request.command.clone(), version);
        let mut output = vec![0; expected.len()];
        peer.read_exact(&mut output).await.unwrap();
        assert_eq!(output, expected);
        assert!(operation.drain_receipt().is_written());
    }
}

struct CountedTransport {
    inner: BoxedTransport,
    calls: Arc<Mutex<(usize, usize)>>,
}

impl tokio::io::AsyncRead for CountedTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buffer)
    }
}

impl tokio::io::AsyncWrite for CountedTransport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.calls.lock().unwrap().0 += 1;
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.calls.lock().unwrap().1 += 1;
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[async_trait]
impl Transport for CountedTransport {
    async fn connect(_: &str) -> Result<Self, crate::mqtt_client::transport::TransportError> {
        unreachable!()
    }
    async fn close(&mut self) -> Result<(), crate::mqtt_client::transport::TransportError> {
        self.inner.close().await
    }
    fn peer_addr(&self) -> Result<String, crate::mqtt_client::transport::TransportError> {
        self.inner.peer_addr()
    }
    fn local_addr(&self) -> Result<String, crate::mqtt_client::transport::TransportError> {
        self.inner.local_addr()
    }
}

#[tokio::test]
async fn tracked_preparation_adds_io_only_for_preceding_bytes() {
    for preceding in [false, true] {
        let (mut worker, _peer) = connected(5, 1024).await;
        let calls = Arc::new(Mutex::new((0, 0)));
        worker.stream = Some(Box::new(CountedTransport {
            inner: worker.stream.take().unwrap(),
            calls: calls.clone(),
        }));
        if preceding {
            worker.engine.send_ping().unwrap();
        }
        let (request, operation) = request(0);
        worker.admit_tracked(&request).await;
        let prior = usize::from(preceding);
        assert_eq!(*calls.lock().unwrap(), (prior, prior));
        worker.handle_outgoing().await.unwrap();
        assert_eq!(*calls.lock().unwrap(), (prior + 1, prior + 1));
        assert!(operation.drain_receipt().is_written());
    }
}
