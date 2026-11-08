// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::mqtt_client::payload_tests::tracked_payload;

struct IgnoreEvents;

#[async_trait]
impl TokioMqttEventHandler for IgnoreEvents {}

#[tokio::test]
async fn shared_payload_command_and_admission_channels_keep_the_same_buffer() {
    for version in [4, 5] {
        for qos in [0, 1, 2] {
            for admission in [false, true] {
                let (mut worker, commands) = test_support::connected_worker(
                    MqttClientOptions::builder()
                        .mqtt_version(version)
                        .keep_alive(0)
                        .build(),
                    Box::new(IgnoreEvents),
                    TokioAsyncClientConfig::builder()
                        .auto_reconnect(false)
                        .build(),
                )
                .await;
                let application = bytes::Bytes::from(vec![42; 4096]);
                let command = PublishCommand::from_bytes("shared", application.clone(), qos, false);
                let (response_tx, response_rx) = oneshot::channel();
                if admission {
                    let (tx, rx) = mpsc::channel(1);
                    worker.admission_rx = rx;
                    tx.send(PublishAdmission::Single {
                        command,
                        response_tx,
                        _permit: None,
                    })
                    .await
                    .unwrap();
                    let request = worker.admission_rx.recv().await.unwrap();
                    let PublishAdmission::Single { command, .. } = &request else {
                        unreachable!()
                    };
                    assert_eq!(command.payload.as_ptr(), application.as_ptr());
                    worker.admit(request);
                    assert_eq!(response_rx.await.unwrap().unwrap().is_some(), qos > 0);
                } else {
                    drop((response_tx, response_rx));
                    commands
                        .send(TokioClientCommand::Publish(command))
                        .await
                        .unwrap();
                    let command = worker.command_rx.recv().await.unwrap();
                    assert!(
                        matches!(&command, TokioClientCommand::Publish(command) if command.payload.as_ptr() == application.as_ptr())
                    );
                    worker.handle_command(command).await;
                }
                let wire = worker.engine.take_outgoing();
                assert!(wire.ends_with(&application));
                if qos > 0 {
                    assert!(!application.is_unique());
                    // The first publication uses packet ID 1 in a fresh session.
                    test_support::receive(
                        &mut worker,
                        &[if qos == 1 { 0x40 } else { 0x50 }, 2, 0, 1],
                    )
                    .await;
                    if qos == 2 {
                        assert_eq!(worker.engine.take_outgoing(), [0x62, 2, 0, 1]);
                        test_support::receive(&mut worker, &[0x70, 2, 0, 1]).await;
                    }
                }
                assert!(application.is_unique());
            }
        }
    }
}

#[tokio::test]
async fn shared_payload_shutdown_drops_inflight_queued_and_unread_commands() {
    for full_output in [false, true] {
        let (mut worker, commands) = test_support::connected_worker(
            MqttClientOptions::builder()
                .receive_maximum(1)
                .max_outgoing_packet_count(if full_output { 1 } else { 4 })
                .keep_alive(0)
                .build(),
            Box::new(IgnoreEvents),
            TokioAsyncClientConfig::builder()
                .auto_reconnect(false)
                .build(),
        )
        .await;
        let (transport, _peer) = tokio::io::duplex(16384);
        worker.stream = Some(Box::new(transport));
        let mut owners = Vec::new();
        for _ in 0..2 {
            let (payload, weak) = tracked_payload(4096);
            worker
                .handle_command(TokioClientCommand::Publish(PublishCommand::from_bytes(
                    "shared", payload, 1, false,
                )))
                .await;
            assert!(weak.upgrade().is_some());
            owners.push(weak);
            if !full_output {
                worker.engine.take_outgoing();
            }
        }
        commands.send(TokioClientCommand::Shutdown).await.unwrap();
        let (payload, weak) = tracked_payload(4096);
        commands
            .send(TokioClientCommand::Publish(PublishCommand::from_bytes(
                "unread", payload, 1, false,
            )))
            .await
            .unwrap();
        owners.push(weak);
        let (admissions, rx) = mpsc::channel(1);
        worker.admission_rx = rx;
        let (response_tx, _response_rx) = oneshot::channel();
        if !full_output {
            let (payload, weak) = tracked_payload(4096);
            admissions
                .send(PublishAdmission::Single {
                    command: PublishCommand::from_bytes("unread", payload, 1, false),
                    response_tx,
                    _permit: None,
                })
                .await
                .unwrap();
            owners.push(weak);
        }
        let result = tokio::time::timeout(Duration::from_secs(1), worker.run())
            .await
            .unwrap();
        if full_output {
            assert!(matches!(result, Err(MqttClientError::BufferFull { .. })));
        } else {
            result.unwrap();
        }
        assert!(owners.iter().all(|weak| weak.upgrade().is_none()));
    }
}
