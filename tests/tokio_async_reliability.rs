// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "async-client")]

#[cfg(test)]
mod tests {
    use flowsdk::mqtt_client::client::ConnectionResult;
    use flowsdk::mqtt_client::{
        MqttClientError, MqttClientOptions, MqttMessage, TokioAsyncClientConfig,
        TokioAsyncMqttClient, TokioMqttEventHandler,
    };
    use flowsdk::mqtt_serde::mqttv5::{common::properties::Property, pubackv5::MqttPubAck};
    use flowsdk::mqtt_serde::{control_packet::MqttPacket, parser::ParseOk};
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        time::timeout,
    };

    #[derive(Debug)]
    enum Event {
        Connected,
        Disconnected,
        Lost,
        Message,
        Error(String),
    }
    struct Handler(mpsc::UnboundedSender<Event>);
    #[async_trait::async_trait]
    impl TokioMqttEventHandler for Handler {
        async fn on_connected(&mut self, _: &ConnectionResult) {
            let _ = self.0.send(Event::Connected);
        }
        async fn on_disconnected(&mut self, _: Option<u8>) {
            let _ = self.0.send(Event::Disconnected);
        }
        async fn on_connection_lost(&mut self) {
            let _ = self.0.send(Event::Lost);
        }
        async fn on_message_received(&mut self, _: &MqttMessage) {
            let _ = self.0.send(Event::Message);
        }
        async fn on_error(&mut self, e: &MqttClientError) {
            let _ = self.0.send(Event::Error(format!("{e:?}")));
        }
    }

    async fn read_packet(peer: &mut TcpStream) -> MqttPacket {
        let header = peer.read_u8().await.unwrap();
        let mut bytes = vec![header];
        let mut length = 0usize;
        let mut shift = 0;
        loop {
            let byte = peer.read_u8().await.unwrap();
            bytes.push(byte);
            length |= usize::from(byte & 127) << shift;
            if byte & 128 == 0 {
                break;
            }
            shift += 7;
        }
        let offset = bytes.len();
        bytes.resize(offset + length, 0);
        peer.read_exact(&mut bytes[offset..]).await.unwrap();
        match MqttPacket::from_bytes_with_version(&bytes, 5).unwrap() {
            ParseOk::Packet(packet, _) => packet,
            _ => panic!("incomplete packet"),
        }
    }

    fn options() -> MqttClientOptions {
        MqttClientOptions::builder()
            .reconnect(false)
            .keep_alive(0)
            .build()
    }
    fn config() -> TokioAsyncClientConfig {
        TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .build()
    }

    async fn open(
        mut options: MqttClientOptions,
        config: TokioAsyncClientConfig,
        acknowledge: bool,
    ) -> (
        TokioAsyncMqttClient,
        TcpListener,
        TcpStream,
        mpsc::UnboundedReceiver<Event>,
        MqttPacket,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        options.peer = listener.local_addr().unwrap().to_string();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = TokioAsyncMqttClient::new(options, Box::new(Handler(tx)), config)
            .await
            .unwrap();
        client.connect().await.unwrap();
        let (mut peer, _) = timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let packet = timeout(Duration::from_secs(2), read_packet(&mut peer))
            .await
            .unwrap();
        if acknowledge {
            peer.write_all(&[0x20, 3, 0, 0, 0]).await.unwrap();
            assert!(matches!(
                timeout(Duration::from_secs(2), rx.recv()).await.unwrap(),
                Some(Event::Connected)
            ));
        }
        (client, listener, peer, rx, packet)
    }

    #[tokio::test]
    async fn async_config_limits_reach_connect_packet() {
        let cfg = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .receive_maximum(2)
            .topic_alias_maximum(3)
            .build();
        let (client, _, _peer, _events, packet) = open(options(), cfg, true).await;
        client.shutdown().await.unwrap();
        let MqttPacket::Connect5(packet) = packet else {
            panic!("expected CONNECT");
        };
        assert!(
            packet.properties.contains(&Property::ReceiveMaximum(2))
                && packet.properties.contains(&Property::TopicAliasMaximum(3)),
            "configured CONNECT properties missing: {:?}",
            packet.properties
        );
    }

    #[tokio::test]
    async fn raw_manual_ack_allows_packet_id_reuse() {
        let (client, _, mut peer, mut events, _) =
            open(options().auto_ack(false), config(), true).await;
        peer.write_all(&[0x32, 6, 0, 1, b't', 0, 7, 0])
            .await
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap(),
            Some(Event::Message)
        ));
        client
            .send_packet(MqttPacket::PubAck5(MqttPubAck::new_success(7)))
            .await
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), read_packet(&mut peer))
                .await
                .unwrap(),
            MqttPacket::PubAck5(_)
        ));
        peer.write_all(&[0x34, 6, 0, 1, b't', 0, 7, 0])
            .await
            .unwrap();
        let event = timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap();
        client.shutdown().await.unwrap();
        assert!(
            matches!(event, Some(Event::Message)),
            "reused ID was rejected: {event:?}"
        );
    }

    #[tokio::test]
    async fn engine_deadline_resolves_subscribe_future() {
        let mut opts = options();
        opts.operation_timeouts.subscribe = Some(Duration::from_millis(30));
        let cfg = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .no_subscribe_timeout()
            .build();
        let (client, _, _peer, mut events, _) = open(opts, cfg, true).await;
        let result = timeout(Duration::from_millis(300), client.subscribe_sync("t", 1)).await;
        let event = events.try_recv().unwrap();
        client.shutdown().await.unwrap();
        assert!(
            matches!(&event, Event::Error(message) if message.contains("OperationTimeout")),
            "{event:?}"
        );
        assert!(
            matches!(result, Ok(Err(_))),
            "engine error {event:?} did not resolve caller: {result:?}"
        );
    }

    #[tokio::test]
    async fn local_disconnect_emits_disconnect_callback() {
        let (client, _, mut peer, mut events, _) = open(options(), config(), true).await;
        client.disconnect().await.unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), read_packet(&mut peer))
                .await
                .unwrap(),
            MqttPacket::Disconnect5(_)
        ));
        peer.shutdown().await.unwrap();
        drop(peer);
        let result = timeout(Duration::from_millis(300), async {
            while let Some(event) = events.recv().await {
                if matches!(event, Event::Disconnected) {
                    return;
                }
            }
            panic!("callback channel closed");
        })
        .await;
        client.shutdown().await.unwrap();
        assert!(
            result.is_ok(),
            "local DISCONNECT followed by peer EOF never called on_disconnected"
        );
    }

    #[tokio::test]
    async fn repeated_connect_waits_for_connack() {
        let (client, _, mut peer, _events, _) = open(options(), config(), false).await;
        let result = timeout(Duration::from_millis(150), client.connect_sync()).await;
        peer.write_all(&[0x20, 3, 0, 0, 0]).await.unwrap();
        client.shutdown().await.unwrap();
        assert!(
            !matches!(&result, Ok(Ok(reply)) if reply.is_success()),
            "connect_sync reported success before any CONNACK: {result:?}"
        );
    }

    #[tokio::test]
    async fn disabled_async_reconnect_prevents_new_connection() {
        let mut opts = options().reconnect(true);
        opts.reconnect_base_delay_ms = 10;
        let (client, listener, peer, _events, _) = open(opts, config(), true).await;
        drop(peer);
        let reconnected = timeout(Duration::from_millis(300), listener.accept())
            .await
            .is_ok();
        client.shutdown().await.unwrap();
        assert!(
            !reconnected,
            "auto_reconnect(false) was overridden by engine reconnect setting"
        );
    }
}
