// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::mqtt_serde::mqttv5::publishv5::MqttPublish;

pub(crate) async fn connected_worker(
    options: MqttClientOptions,
    handler: Box<dyn TokioMqttEventHandler>,
    config: TokioAsyncClientConfig,
) -> (TokioClientWorker, mpsc::Sender<TokioClientCommand>) {
    let (commands, command_rx) = mpsc::channel(4);
    let mut worker = TokioClientWorker::new(options, handler, command_rx, config);
    worker.state = ConnectionState::Connecting;
    worker.engine.connect().unwrap();
    worker.engine.take_outgoing();
    let (response, mut result) = oneshot::channel();
    worker.pending.connect.push(response);
    let connack: &[u8] = if worker.engine.mqtt_version() == 5 {
        &[0x20, 3, 0, 0, 0]
    } else {
        &[0x20, 2, 0, 0]
    };
    receive(&mut worker, connack).await;
    assert!(result.try_recv().unwrap().unwrap().is_success());
    (worker, commands)
}

pub(crate) async fn receive(worker: &mut TokioClientWorker, bytes: &[u8]) {
    let events = worker.engine.handle_incoming(bytes);
    worker.dispatch_events(events).await;
}

pub(crate) fn publish(version: u8, id: u8, qos: u8) -> Vec<u8> {
    let packet_id = (qos > 0).then_some(u16::from(id));
    let packet = if version == 5 {
        MqttPacket::Publish5(MqttPublish::new(
            qos,
            "t".into(),
            packet_id,
            vec![id],
            false,
            false,
        ))
    } else {
        MqttPacket::Publish3(crate::mqtt_serde::mqttv3::publishv3::MqttPublish::new(
            "t".into(),
            qos,
            vec![id],
            packet_id,
            false,
            false,
        ))
    };
    packet.to_bytes().unwrap()
}
