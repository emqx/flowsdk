use flowsdk::mqtt_client::{
    async_client::{AsyncClientConfig, AsyncMqttClient, MqttEventHandler},
    client::{
        ConnectionResult, MqttClient, PingResult, PublishResult, SubscribeResult, UnsubscribeResult,
    },
    MqttClientOptions,
};
use flowsdk::mqtt_serde::{control_packet::MqttPacket, mqttv5, MqttStream};
use std::{io::Write, net::TcpListener, sync::mpsc, thread, time::Duration};

fn broker(version: u8) -> (String, thread::JoinHandle<Vec<MqttPacket>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let worker = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut writer = socket.try_clone().unwrap();
        let mut stream = MqttStream::new(socket, 4096, version);
        let mut received = Vec::new();
        while let Some(packet) = stream.next() {
            let packet = packet.unwrap();
            let response = match &packet {
                MqttPacket::Connect3(_) => vec![0x20, 2, 0, 0],
                MqttPacket::Connect5(_) => vec![0x20, 3, 0, 0, 0],
                MqttPacket::Subscribe3(p) => vec![
                    0x90,
                    3,
                    (p.message_id >> 8) as u8,
                    p.message_id as u8,
                    p.subscriptions[0].qos,
                ],
                MqttPacket::Subscribe5(p) => vec![
                    0x90,
                    4,
                    (p.packet_id >> 8) as u8,
                    p.packet_id as u8,
                    0,
                    p.subscriptions[0].qos,
                ],
                MqttPacket::Unsubscribe3(p) => {
                    vec![0xb0, 2, (p.message_id >> 8) as u8, p.message_id as u8]
                }
                MqttPacket::Unsubscribe5(p) => {
                    vec![0xb0, 4, (p.packet_id >> 8) as u8, p.packet_id as u8, 0, 0]
                }
                MqttPacket::Publish3(p) if p.qos > 0 => vec![
                    if p.qos == 1 { 0x40 } else { 0x50 },
                    2,
                    (p.message_id.unwrap() >> 8) as u8,
                    p.message_id.unwrap() as u8,
                ],
                MqttPacket::Publish5(p) if p.qos > 0 => vec![
                    if p.qos == 1 { 0x40 } else { 0x50 },
                    4,
                    (p.packet_id.unwrap() >> 8) as u8,
                    p.packet_id.unwrap() as u8,
                    0,
                    0,
                ],
                MqttPacket::PubRel3(p) => {
                    vec![0x70, 2, (p.message_id >> 8) as u8, p.message_id as u8]
                }
                MqttPacket::PubRel5(p) => {
                    vec![0x70, 4, (p.packet_id >> 8) as u8, p.packet_id as u8, 0, 0]
                }
                MqttPacket::PingReq3(_) | MqttPacket::PingReq5(_) => vec![0xd0, 0],
                MqttPacket::Disconnect3(_) | MqttPacket::Disconnect5(_) => {
                    received.push(packet);
                    break;
                }
                MqttPacket::Publish3(_) | MqttPacket::Publish5(_) => vec![],
                other => panic!("Unexpected client packet: {other:?}"),
            };
            received.push(packet);
            writer.write_all(&response).unwrap();
        }
        received
    });
    (address, worker)
}

#[test]
fn blocking_clients_complete_qos_and_subscription_workflows_in_both_versions() {
    for version in [3, 5] {
        let (address, server) = broker(version);
        let opts = MqttClientOptions::builder()
            .peer(address)
            .client_id("blocking-test")
            .mqtt_version(version)
            .username("username")
            .password("password")
            .will(mqttv5::will::Will::new(
                "status".into(),
                b"offline".to_vec(),
                1,
                true,
            ))
            .build();
        let mut client = MqttClient::new(opts);
        assert!(client.connected().unwrap().is_success());
        let subscription = client.subscribed("topic", 2).unwrap();
        assert_eq!(subscription.reason_codes, [2]);
        for qos in [0, 1, 2] {
            let published = client.published("topic", b"payload", qos, true).unwrap();
            assert!(published.is_success());
            assert_eq!(published.qos, qos);
            assert_eq!(published.packet_id.is_some(), qos > 0);
        }
        assert!(client.pingd().unwrap().success);
        assert!(client.unsubscribed_single("topic").unwrap().is_success());
        client.disconnected(0).unwrap();
        let received = server.join().unwrap();
        assert_eq!(received.len(), 9);
        match &received[0] {
            MqttPacket::Connect3(p) => {
                assert_eq!(p.username.as_deref(), Some("username"));
                assert!(p.will.is_some());
            }
            MqttPacket::Connect5(p) => {
                assert_eq!(p.username.as_deref(), Some("username"));
                assert!(p.will.is_some());
            }
            other => panic!("Expected CONNECT, got {other:?}"),
        }
    }
}

struct Events(mpsc::Sender<&'static str>);
impl MqttEventHandler for Events {
    fn on_connected(&mut self, result: &ConnectionResult) {
        assert!(result.is_success());
        self.0.send("connected").unwrap();
    }
    fn on_subscribed(&mut self, result: &SubscribeResult) {
        assert!(result.is_success());
        self.0.send("subscribed").unwrap();
    }
    fn on_published(&mut self, result: &PublishResult) {
        assert!(result.is_success());
        self.0.send("published").unwrap();
    }
    fn on_unsubscribed(&mut self, result: &UnsubscribeResult) {
        assert!(result.is_success());
        self.0.send("unsubscribed").unwrap();
    }
    fn on_ping_response(&mut self, result: &PingResult) {
        assert!(result.success);
        self.0.send("ping").unwrap();
    }
    fn on_disconnected(&mut self, _: Option<u8>) {
        self.0.send("disconnected").unwrap();
    }
    fn on_error(&mut self, error: &std::io::Error) {
        panic!("Unexpected worker error: {error}");
    }
}

#[test]
fn threaded_client_reports_acknowledgements_in_command_order() {
    let (address, server) = broker(5);
    let (send, receive) = mpsc::channel();
    let client = AsyncMqttClient::new(
        MqttClientOptions::builder().peer(address).build(),
        Box::new(Events(send)),
        AsyncClientConfig {
            auto_reconnect: false,
            ..Default::default()
        },
    )
    .unwrap();
    client.set_auto_reconnect(false).unwrap();
    client.connect().unwrap();
    assert_eq!(
        receive.recv_timeout(Duration::from_secs(5)).unwrap(),
        "connected"
    );
    // A timeout while the broker is idle must not become a connection failure.
    thread::sleep(Duration::from_millis(250));
    client.subscribe("topic", 1).unwrap();
    client.publish("topic", b"payload", 1, false).unwrap();
    client.ping().unwrap();
    client.unsubscribe(vec!["topic"]).unwrap();
    client.disconnect().unwrap();
    for expected in [
        "subscribed",
        "published",
        "ping",
        "unsubscribed",
        "disconnected",
    ] {
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(5)).unwrap(),
            expected
        );
    }
    client.shutdown().unwrap();
    assert_eq!(server.join().unwrap().len(), 6);
}

#[test]
fn operations_without_a_connection_fail_without_recording_pending_work() {
    let mut client = MqttClient::new(MqttClientOptions::builder().build());
    assert!(client.published("topic", b"payload", 1, false).is_err());
    assert!(client.subscribed("topic", 1).is_err());
    assert!(client.unsubscribed(vec!["topic"]).is_err());
    assert!(client.pingd().is_err());
    assert!(client.disconnected(0).is_err());
    assert!(client.recv_packet().is_err());
    assert_eq!(
        client
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotConnected
    );
    assert!(client.recv_for_packet(|_| true).is_err());
    assert!(client.subscribe_send("topic", 1).is_err());
    assert!(client.unsubscribe_send_single("topic").is_err());
    assert!(client.publish_send("topic", b"payload", 1, false).is_err());
    assert!(client.ping_send().is_err());
    assert!(client.disconnect_send().is_err());
    assert!(client.get_pending_subscribes().is_empty());
    assert!(client.get_pending_unsubscribes().is_empty());
    assert!(client.get_pending_publishes().is_empty());
    assert!(client.complete_subscribe(1).is_none());
    assert!(client.complete_unsubscribe(1).is_none());
    assert!(client.complete_publish(1).is_none());
    client.clear_pending_operations();
    assert!(client.pop_unhandled_packet().is_none());
    client
        .unhandled_packets_mut()
        .push(MqttPacket::PingResp5(mqttv5::pingresp::MqttPingResp::new()));
    assert_eq!(client.peek_unhandled_packets().len(), 1);
    assert!(client.pop_unhandled_packet().is_some());
    client.clear_unhandled_packets();
    assert!(client.peek_unhandled_packets().is_empty());
}

#[test]
fn connect_send_propagates_connection_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut client = MqttClient::new(
        MqttClientOptions::builder()
            .peer(address.to_string())
            .build(),
    );
    assert_eq!(
        client.connect_send().unwrap_err().kind(),
        std::io::ErrorKind::ConnectionRefused
    );
}
