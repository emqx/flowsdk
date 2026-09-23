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
        PubRel(u16),
        Published(u16),
        DisconnectReceived(u8, Vec<Property>),
        Reconnect(u32),
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
        async fn on_pubrel_received(&mut self, packet_id: u16) {
            let _ = self.0.send(Event::PubRel(packet_id));
        }
        async fn on_published(&mut self, result: &flowsdk::mqtt_client::client::PublishResult) {
            if let Some(id) = result.packet_id {
                let _ = self.0.send(Event::Published(id));
            }
        }
        async fn on_disconnect_received(&mut self, reason: u8, properties: &[Property]) {
            let _ = self
                .0
                .send(Event::DisconnectReceived(reason, properties.to_vec()));
        }
        async fn on_reconnect_attempt(&mut self, attempt: u32) {
            let _ = self.0.send(Event::Reconnect(attempt));
        }
    }

    async fn read_packet<R: tokio::io::AsyncRead + Unpin>(peer: &mut R) -> MqttPacket {
        read_packet_version(peer, 5).await
    }

    async fn read_packet_version<R: tokio::io::AsyncRead + Unpin>(
        peer: &mut R,
        version: u8,
    ) -> MqttPacket {
        timeout(Duration::from_secs(2), read_packet_bytes(peer, version))
            .await
            .expect("expected MQTT packet did not arrive")
    }

    async fn read_packet_bytes<R: tokio::io::AsyncRead + Unpin>(
        peer: &mut R,
        version: u8,
    ) -> MqttPacket {
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
        match MqttPacket::from_bytes_with_version(&bytes, version).unwrap() {
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
        let version = options.mqtt_version;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = TokioAsyncMqttClient::new(options, Box::new(Handler(tx)), config)
            .await
            .unwrap();
        client.connect().await.unwrap();
        let (mut peer, _) = timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let packet = timeout(
            Duration::from_secs(2),
            read_packet_version(&mut peer, version),
        )
        .await
        .unwrap();
        if acknowledge {
            peer.write_all(if version == 5 {
                &[0x20, 3, 0, 0, 0]
            } else {
                &[0x20, 2, 0, 0]
            })
            .await
            .unwrap();
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
    async fn engine_deadlines_resolve_publish_unsubscribe_and_connect_futures() {
        let mut opts = options();
        opts.operation_timeouts.publish = Some(Duration::from_millis(30));
        opts.operation_timeouts.unsubscribe = Some(Duration::from_millis(30));
        let cfg = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .no_timeouts()
            .build();
        let (client, _, _peer, _, _) = open(opts, cfg.clone(), true).await;
        let results = timeout(Duration::from_secs(2), async {
            let publish = client.publish_sync("t", b"x", 1, false).await;
            let unsubscribe = client.unsubscribe_sync(vec!["t"]).await;
            (publish, unsubscribe)
        })
        .await
        .unwrap();
        assert!(matches!(
            results.0,
            Err(MqttClientError::OperationTimeout { timeout_ms: 30, .. })
        ));
        assert!(matches!(
            results.1,
            Err(MqttClientError::OperationTimeout { timeout_ms: 30, .. })
        ));
        client.shutdown().await.unwrap();

        let mut opts = options();
        opts.operation_timeouts.connect = Some(Duration::from_millis(100));
        let (client, _, _peer, _, _) = open(opts, cfg, false).await;
        assert!(matches!(
            timeout(Duration::from_secs(2), client.connect_sync())
                .await
                .unwrap(),
            Err(MqttClientError::OperationTimeout {
                timeout_ms: 100,
                ..
            })
        ));
        client.shutdown().await.unwrap();
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
        for core_reconnect in [false, true] {
            let opts = options()
                .reconnect(core_reconnect)
                .reconnect_base_delay_ms(10);
            let (client, listener, peer, mut events, _) = open(opts, config(), true).await;
            drop(peer);
            event_matching(&mut events, |e| matches!(e, Event::Lost)).await;
            let reconnected = timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_ok();
            client.shutdown().await.unwrap();
            assert!(
                !reconnected,
                "auto_reconnect(false) must prevent retries with core reconnect={core_reconnect}"
            );
        }
    }

    #[tokio::test]
    async fn enabled_async_reconnect_establishes_new_connection() {
        for core_reconnect in [false, true] {
            let opts = options()
                .reconnect(core_reconnect)
                .reconnect_base_delay_ms(10);
            let (client, listener, peer, mut events, _) =
                open(opts, TokioAsyncClientConfig::default(), true).await;
            drop(peer);
            let (mut retry, _) = timeout(Duration::from_secs(2), listener.accept())
                .await
                .unwrap_or_else(|_| {
                    panic!("auto_reconnect(true) must retry with core reconnect={core_reconnect}")
                })
                .unwrap();
            assert!(matches!(
                read_packet(&mut retry).await,
                MqttPacket::Connect5(_)
            ));
            retry.write_all(&[0x20, 3, 0, 0, 0]).await.unwrap();
            event_matching(&mut events, |e| matches!(e, Event::Connected)).await;
            client.shutdown().await.unwrap();
        }
    }

    async fn event_matching(
        events: &mut mpsc::UnboundedReceiver<Event>,
        predicate: impl Fn(&Event) -> bool,
    ) -> Event {
        timeout(Duration::from_secs(2), async {
            loop {
                let event = events
                    .recv()
                    .await
                    .expect("worker stopped before expected event");
                if predicate(&event) {
                    return event;
                }
            }
        })
        .await
        .expect("expected event did not arrive")
    }

    #[tokio::test]
    async fn concurrent_connects_return_actual_connack_metadata() {
        let (client, _, mut peer, _, _) = open(options(), config(), false).await;
        let first = client.connect_sync();
        let second = client.connect_sync();
        let respond = async {
            let packet =
                MqttPacket::ConnAck5(flowsdk::mqtt_serde::mqttv5::connackv5::MqttConnAck::new(
                    false,
                    0,
                    Some(vec![Property::ReceiveMaximum(12)]),
                ));
            peer.write_all(&packet.to_bytes().unwrap()).await.unwrap();
        };
        let (first, second, ()) = tokio::join!(first, second, respond);
        for result in [first, second, client.connect_sync().await] {
            assert_eq!(
                result.unwrap().properties,
                Some(vec![Property::ReceiveMaximum(12)])
            );
        }
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rejected_connack_returns_broker_result_and_allows_a_fresh_connection() {
        let (client, listener, mut peer, _, _) = open(options(), config(), false).await;
        let (result, ()) = tokio::join!(client.connect_sync(), async {
            peer.write_all(&[0x20, 3, 0, 0x87, 0]).await.unwrap();
        });
        assert_eq!(result.unwrap().reason_code, 0x87);
        assert_eq!(
            timeout(Duration::from_secs(2), peer.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let (result, _peer) = timeout(Duration::from_secs(2), async {
            tokio::join!(client.connect_sync(), async {
                let (mut peer, _) = listener.accept().await.unwrap();
                assert!(matches!(
                    read_packet(&mut peer).await,
                    MqttPacket::Connect5(_)
                ));
                peer.write_all(&[0x20, 3, 0, 0, 0]).await.unwrap();
                peer
            })
        })
        .await
        .unwrap();
        assert!(result.unwrap().is_success());
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn conflicting_connect_properties_are_rejected() {
        for opts in [
            options().incoming_receive_maximum(5),
            options().connect_properties(vec![Property::ReceiveMaximum(5)]),
            options().connect_properties(vec![Property::TopicAliasMaximum(5)]),
        ] {
            let (tx, _) = mpsc::unbounded_channel();
            let result = TokioAsyncMqttClient::new(
                opts,
                Box::new(Handler(tx)),
                TokioAsyncClientConfig::builder()
                    .receive_maximum(2)
                    .topic_alias_maximum(3)
                    .build(),
            )
            .await;
            assert!(result.is_err());
        }
    }

    #[tokio::test]
    async fn matching_and_core_only_connect_properties_are_preserved() {
        for explicit in [false, true] {
            let opts = options()
                .incoming_receive_maximum(2)
                .connect_properties(vec![Property::TopicAliasMaximum(3)]);
            let cfg = if explicit {
                TokioAsyncClientConfig::builder()
                    .receive_maximum(2)
                    .topic_alias_maximum(3)
                    .build()
            } else {
                config()
            };
            let (client, _, _, _, packet) = open(opts, cfg, true).await;
            let MqttPacket::Connect5(packet) = packet else {
                panic!("CONNECT expected");
            };
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .filter(|p| matches!(p, Property::ReceiveMaximum(2)))
                    .count(),
                1
            );
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .filter(|p| matches!(p, Property::TopicAliasMaximum(3)))
                    .count(),
                1
            );
            client.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn disconnect_completion_flushes_properties_and_closes_once() {
        let (client, listener, mut peer, mut events, _) = open(
            options()
                .session_expiry_interval(60)
                .reconnect_base_delay_ms(10),
            TokioAsyncClientConfig::default(),
            true,
        )
        .await;
        let properties = vec![
            Property::SessionExpiryInterval(0),
            Property::ReasonString("done".into()),
        ];
        client
            .disconnect_with_sync(0, properties.clone())
            .await
            .unwrap();
        let packet = read_packet(&mut peer).await;
        let MqttPacket::Disconnect5(packet) = packet else {
            panic!("DISCONNECT expected");
        };
        assert_eq!(packet.properties, properties);
        assert_eq!(
            timeout(Duration::from_secs(2), peer.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(matches!(events.recv().await, Some(Event::Disconnected)));
        client.disconnect_sync().await.unwrap();
        assert!(events.try_recv().is_err());
        client.set_auto_reconnect(true).await.unwrap();
        assert!(timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err());
        client.shutdown().await.unwrap();
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn peer_disconnect_preserves_properties_and_completes_waiters() {
        let cfg = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .no_subscribe_timeout()
            .build();
        let (client, _, mut peer, mut events, _) = open(options(), cfg, true).await;
        let properties = vec![
            Property::ReasonString("maintenance".into()),
            Property::ServerReference("other:1883".into()),
        ];
        let respond = async {
            assert!(matches!(
                read_packet(&mut peer).await,
                MqttPacket::Subscribe5(_)
            ));
            let packet = MqttPacket::Disconnect5(
                flowsdk::mqtt_serde::mqttv5::disconnectv5::MqttDisconnect::new(
                    0x9c,
                    properties.clone(),
                ),
            );
            peer.write_all(&packet.to_bytes().unwrap()).await.unwrap();
        };
        let (result, ()) = timeout(Duration::from_secs(2), async {
            tokio::join!(client.subscribe_sync("t", 1), respond)
        })
        .await
        .unwrap();
        assert!(matches!(
            result,
            Err(MqttClientError::ConnectionLost { .. })
        ));
        assert!(
            matches!(event_matching(&mut events, |e| matches!(e, Event::DisconnectReceived(..))).await,
            Event::DisconnectReceived(0x9c, got) if got == properties)
        );
        event_matching(&mut events, |e| matches!(e, Event::Disconnected)).await;
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn disabling_reconnect_cancels_scheduled_attempt() {
        let opts = options().reconnect(true).reconnect_base_delay_ms(200);
        let (client, listener, peer, mut events, _) =
            open(opts, TokioAsyncClientConfig::default(), true).await;
        drop(peer);
        assert!(matches!(
            event_matching(&mut events, |e| matches!(e, Event::Reconnect(_))).await,
            Event::Reconnect(1)
        ));
        client.set_auto_reconnect(false).await.unwrap();
        assert!(timeout(Duration::from_millis(350), listener.accept())
            .await
            .is_err());
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_scheduled_reconnect() {
        let opts = options().reconnect_base_delay_ms(200);
        let (client, listener, peer, mut events, _) =
            open(opts, TokioAsyncClientConfig::default(), true).await;
        drop(peer);
        assert!(matches!(
            event_matching(&mut events, |e| matches!(e, Event::Reconnect(_))).await,
            Event::Reconnect(1)
        ));
        client.shutdown().await.unwrap();
        assert!(timeout(Duration::from_millis(350), listener.accept())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn disabling_reconnect_keeps_live_operations_pending() {
        let (client, _, mut peer, _, _) = open(
            options().reconnect(true),
            TokioAsyncClientConfig::default(),
            true,
        )
        .await;
        let respond = async {
            let MqttPacket::Subscribe5(packet) = read_packet(&mut peer).await else {
                panic!("SUBSCRIBE expected");
            };
            client.set_auto_reconnect(false).await.unwrap();
            peer.write_all(&[
                0x90,
                4,
                (packet.packet_id >> 8) as u8,
                packet.packet_id as u8,
                0,
                1,
            ])
            .await
            .unwrap();
        };
        let (result, ()) = tokio::join!(client.subscribe_sync("t", 1), respond);
        assert!(result.unwrap().is_success());
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn manual_qos_exchanges_release_ids_only_at_correct_stage() {
        for version in [3, 4, 5] {
            let (client, _, mut peer, mut events, _) = open(
                options().mqtt_version(version).auto_ack(false),
                config(),
                true,
            )
            .await;
            for qos in [1, 2, 1] {
                let mut bytes = vec![
                    0x30 | (qos << 1),
                    if version == 5 { 6 } else { 5 },
                    0,
                    1,
                    b't',
                    0,
                    7,
                ];
                if version == 5 {
                    bytes.push(0);
                }
                peer.write_all(&bytes).await.unwrap();
                event_matching(&mut events, |e| matches!(e, Event::Message)).await;
                if qos == 1 {
                    client.puback(7, 0, vec![]).await.unwrap();
                } else {
                    client.pubrec(7, 0, vec![]).await.unwrap();
                }
                let packet = read_packet_version(&mut peer, version).await;
                assert!(matches!(
                    packet,
                    MqttPacket::PubAck3(_)
                        | MqttPacket::PubAck5(_)
                        | MqttPacket::PubRec3(_)
                        | MqttPacket::PubRec5(_)
                ));
                if qos == 2 {
                    assert!(client.pubcomp(7, 0, vec![]).await.is_err());
                    peer.write_all(&[0x62, 2, 0, 7]).await.unwrap();
                    assert!(matches!(
                        event_matching(&mut events, |e| matches!(e, Event::PubRel(_))).await,
                        Event::PubRel(7)
                    ));
                    client.pubcomp(7, 0, vec![]).await.unwrap();
                    assert!(matches!(
                        read_packet_version(&mut peer, version).await,
                        MqttPacket::PubComp3(_) | MqttPacket::PubComp5(_)
                    ));
                    assert!(client.pubcomp(7, 0, vec![]).await.is_err());
                } else {
                    assert!(client.puback(7, 0, vec![]).await.is_err());
                }
            }
            client.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn validation_errors_reach_waiting_callers() {
        let (client, _, _peer, _, _) = open(options(), config(), true).await;
        assert!(client.subscribe_sync("invalid/#/filter", 1).await.is_err());
        assert!(client
            .unsubscribe_sync(vec!["invalid/#/filter"])
            .await
            .is_err());
        assert!(client
            .publish_sync("invalid/+", b"x", 0, false)
            .await
            .is_err());
        assert!(client
            .publish_sync("invalid/+", b"x", 1, false)
            .await
            .is_err());
        assert!(client.puback(0, 0, vec![]).await.is_err());
        client.disconnect_sync().await.unwrap();
        assert!(matches!(
            client.ping_sync().await,
            Err(MqttClientError::NotConnected)
        ));
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn late_ack_keeps_id_reserved_after_caller_timeout() {
        use flowsdk::mqtt_client::commands::PublishCommand;
        let cfg = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .publish_ack_timeout_ms(30)
            .build();
        let (client, _, mut peer, mut events, _) = open(options(), cfg, true).await;
        let mut command = PublishCommand::simple("t", b"x".to_vec(), 1, false);
        command.packet_id = Some(42);
        let (result, packet) = tokio::join!(
            client.publish_with_command_sync(command.clone()),
            read_packet(&mut peer)
        );
        assert!(matches!(packet, MqttPacket::Publish5(_)));
        assert!(matches!(
            result,
            Err(MqttClientError::OperationTimeout { .. })
        ));
        assert!(matches!(
            client.publish_with_command_sync(command.clone()).await,
            Err(MqttClientError::InvalidPacketId { packet_id: 42 })
        ));
        peer.write_all(&[0x40, 2, 0, 42]).await.unwrap();
        assert!(matches!(
            event_matching(&mut events, |e| matches!(e, Event::Published(_))).await,
            Event::Published(42)
        ));
        let respond = async {
            assert!(matches!(
                read_packet(&mut peer).await,
                MqttPacket::Publish5(_)
            ));
            peer.write_all(&[0x40, 2, 0, 42]).await.unwrap();
        };
        let (result, ()) = tokio::join!(client.publish_with_command_sync(command), respond);
        assert!(result.unwrap().is_success());
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn session_loss_resolves_publish_waiter_with_original_error() {
        let cfg = TokioAsyncClientConfig::builder()
            .no_publish_ack_timeout()
            .build();
        let opts = options()
            .reconnect(true)
            .clean_start(false)
            .session_expiry_interval(60)
            .reconnect_base_delay_ms(10);
        let (client, listener, mut peer, _, _) = open(opts, cfg, true).await;
        let reconnect = async {
            assert!(matches!(
                read_packet(&mut peer).await,
                MqttPacket::Publish5(_)
            ));
            drop(peer);
            let (mut resumed, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_packet(&mut resumed).await,
                MqttPacket::Connect5(_)
            ));
            resumed.write_all(&[0x20, 3, 0, 0, 0]).await.unwrap();
            resumed
        };
        let (result, _resumed) = timeout(Duration::from_secs(2), async {
            tokio::join!(client.publish_sync("t", b"x", 1, false), reconnect)
        })
        .await
        .unwrap();
        assert!(
            matches!(result, Err(MqttClientError::SessionExpired)),
            "{result:?}"
        );
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_resolves_pending_operations_and_joins_worker() {
        let cfg = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .no_subscribe_timeout()
            .build();
        let (client, _, mut peer, _, _) = open(options(), cfg, true).await;
        let client = std::sync::Arc::new(client);
        let pending = tokio::spawn({
            let client = client.clone();
            async move { client.subscribe_sync("t", 1).await }
        });
        assert!(matches!(
            read_packet(&mut peer).await,
            MqttPacket::Subscribe5(_)
        ));
        client.disconnect_sync().await.unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            Err(MqttClientError::OperationCancelled { .. })
        ));
        let client = std::sync::Arc::try_unwrap(client).ok().unwrap();
        client.shutdown().await.unwrap();
        assert!(matches!(
            read_packet(&mut peer).await,
            MqttPacket::Disconnect5(_)
        ));
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn offline_buffer_and_priority_limits_are_enforced() {
        use flowsdk::mqtt_client::commands::PublishCommand;
        for (enabled, capacity) in [(false, 2), (true, 1)] {
            let (tx, _) = mpsc::unbounded_channel();
            let cfg = TokioAsyncClientConfig::builder()
                .auto_reconnect(false)
                .buffer_messages(enabled)
                .max_buffer_size(capacity)
                .build();
            let client = TokioAsyncMqttClient::new(options(), Box::new(Handler(tx)), cfg)
                .await
                .unwrap();
            let first = client.publish_sync("t", b"x", 0, false).await;
            if enabled {
                assert!(first.is_ok());
                assert!(matches!(
                    client.publish_sync("t", b"x", 0, false).await,
                    Err(MqttClientError::BufferFull { capacity: 1, .. })
                ));
            } else {
                assert!(matches!(first, Err(MqttClientError::NotConnected)));
            }
            client.shutdown().await.unwrap();
        }
        for priorities in [false, true] {
            let (tx, _) = mpsc::unbounded_channel();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let cfg = TokioAsyncClientConfig::builder()
                .auto_reconnect(false)
                .priority_queue_enabled(priorities)
                .priority_queue_limit(2)
                .build();
            let client = TokioAsyncMqttClient::new(
                options().peer(listener.local_addr().unwrap().to_string()),
                Box::new(Handler(tx)),
                cfg,
            )
            .await
            .unwrap();
            for (value, priority) in [(1, 0), (2, 255)] {
                client
                    .publish_with_command_sync(PublishCommand::with_priority(
                        "t",
                        vec![value],
                        0,
                        false,
                        priority,
                    ))
                    .await
                    .unwrap();
            }
            if priorities {
                assert!(matches!(
                    client.publish_sync("t", b"x", 0, false).await,
                    Err(MqttClientError::BufferFull { .. })
                ));
            }
            let respond = async {
                let (mut peer, _) = timeout(Duration::from_secs(2), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
                read_packet(&mut peer).await;
                peer.write_all(&[0x20, 3, 0, 0, 0]).await.unwrap();
                let mut values = Vec::new();
                for _ in 0..2 {
                    let MqttPacket::Publish5(p) = read_packet(&mut peer).await else {
                        panic!("PUBLISH expected");
                    };
                    values.push(p.payload[0]);
                }
                (peer, values)
            };
            let (result, (_peer, values)) = tokio::join!(client.connect_sync(), respond);
            assert!(result.unwrap().is_success());
            assert_eq!(values, if priorities { vec![2, 1] } else { vec![1, 2] });
            client.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn resumed_session_keeps_publish_waiter_and_original_packet_id() {
        let cfg = TokioAsyncClientConfig::builder()
            .no_publish_ack_timeout()
            .build();
        let opts = options()
            .reconnect(true)
            .clean_start(false)
            .session_expiry_interval(60)
            .reconnect_base_delay_ms(10);
        let (client, listener, mut peer, _, _) = open(opts, cfg, true).await;
        let reconnect = async {
            let MqttPacket::Publish5(original) = read_packet(&mut peer).await else {
                panic!("PUBLISH expected");
            };
            drop(peer);
            let (mut resumed, _) = listener.accept().await.unwrap();
            read_packet(&mut resumed).await;
            resumed.write_all(&[0x20, 3, 1, 0, 0]).await.unwrap();
            let MqttPacket::Publish5(replay) = read_packet(&mut resumed).await else {
                panic!("PUBLISH replay expected");
            };
            assert!(replay.dup);
            assert_eq!(replay.packet_id, original.packet_id);
            let id = replay.packet_id.unwrap();
            resumed
                .write_all(&[0x40, 2, (id >> 8) as u8, id as u8])
                .await
                .unwrap();
            resumed
        };
        let (result, _resumed) = timeout(Duration::from_secs(2), async {
            tokio::join!(client.publish_sync("t", b"x", 1, false), reconnect)
        })
        .await
        .unwrap();
        assert!(result.unwrap().is_success());
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn retry_limit_and_delay_from_async_config_complete_stranded_waiters() {
        let opts = options()
            .reconnect(true)
            .reconnect_base_delay_ms(10_000)
            .max_reconnect_attempts(100);
        let cfg = TokioAsyncClientConfig::builder()
            .max_reconnect_attempts(1)
            .max_reconnect_delay_ms(10)
            .no_publish_ack_timeout()
            .build();
        let (client, listener, mut peer, _, _) = open(opts, cfg, true).await;
        let reject_retry = async {
            read_packet(&mut peer).await;
            drop(peer);
            let (mut retry, _) = listener.accept().await.unwrap();
            read_packet(&mut retry).await;
            retry.write_all(&[0x20, 3, 0, 0x87, 0]).await.unwrap();
            retry
        };
        let (result, _retry) = timeout(Duration::from_secs(2), async {
            tokio::join!(client.publish_sync("t", b"x", 1, false), reject_retry)
        })
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err());
        client.shutdown().await.unwrap();
    }

    #[cfg(feature = "tls")]
    #[tokio::test]
    async fn shutdown_cancels_a_stalled_transport_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (tx, _) = mpsc::unbounded_channel();
        let cfg = TokioAsyncClientConfig::builder()
            .auto_reconnect(false)
            .no_connect_timeout()
            .build();
        let client = TokioAsyncMqttClient::new(
            options().peer(format!("mqtts://{}", listener.local_addr().unwrap())),
            Box::new(Handler(tx)),
            cfg,
        )
        .await
        .unwrap();
        client.connect().await.unwrap();
        let (mut peer, _) = timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(2), client.shutdown())
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(2), peer.read_to_end(&mut Vec::new()))
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(all(feature = "tls", feature = "rustls-tls"))]
    #[tokio::test]
    async fn native_and_rustls_clients_use_custom_ca_and_complete_disconnect() {
        use flowsdk::mqtt_client::transport::{tls::TlsConfig, RustlsTlsConfig};
        use openssl::{
            asn1::Asn1Time,
            bn::BigNum,
            ec::{EcGroup, EcKey},
            hash::MessageDigest,
            nid::Nid,
            pkey::{PKey, Private},
            x509::{
                extension::{BasicConstraints, KeyUsage, SubjectAlternativeName},
                X509NameBuilder, X509,
            },
        };
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use std::sync::Arc;
        fn certificate(
            key: &PKey<Private>,
            issuer: Option<&X509>,
            signing_key: &PKey<Private>,
        ) -> X509 {
            let mut name = X509NameBuilder::new().unwrap();
            name.append_entry_by_text(
                "CN",
                if issuer.is_some() {
                    "localhost"
                } else {
                    "test CA"
                },
            )
            .unwrap();
            let name = name.build();
            let mut cert = X509::builder().unwrap();
            cert.set_version(2).unwrap();
            cert.set_serial_number(
                &BigNum::from_u32(if issuer.is_some() { 2 } else { 1 })
                    .unwrap()
                    .to_asn1_integer()
                    .unwrap(),
            )
            .unwrap();
            cert.set_subject_name(&name).unwrap();
            cert.set_issuer_name(issuer.map_or(name.as_ref(), |issuer| issuer.subject_name()))
                .unwrap();
            cert.set_pubkey(key).unwrap();
            cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
                .unwrap();
            cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
                .unwrap();
            if issuer.is_none() {
                cert.append_extension(BasicConstraints::new().critical().ca().build().unwrap())
                    .unwrap();
                cert.append_extension(KeyUsage::new().key_cert_sign().build().unwrap())
                    .unwrap();
            } else {
                cert.append_extension(BasicConstraints::new().critical().build().unwrap())
                    .unwrap();
                let san = SubjectAlternativeName::new()
                    .dns("localhost")
                    .ip("127.0.0.1")
                    .build(&cert.x509v3_context(issuer.map(|c| c.as_ref()), None))
                    .unwrap();
                cert.append_extension(san).unwrap();
            }
            cert.sign(signing_key, MessageDigest::sha256()).unwrap();
            cert.build()
        }
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let ca_key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
        let ca = certificate(&ca_key, None, &ca_key);
        let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
        let cert = certificate(&key, Some(&ca), &ca_key);
        let server = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.to_der().unwrap())],
                PrivateKeyDer::try_from(key.private_key_to_pkcs8().unwrap()).unwrap(),
            )
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        for native in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let opts = options().peer(format!("mqtts://{}", listener.local_addr().unwrap()));
            let opts = if native {
                opts.tls_config(
                    TlsConfig::builder()
                        .add_root_certificate(
                            native_tls::Certificate::from_der(&ca.to_der().unwrap()).unwrap(),
                        )
                        .build(),
                )
            } else {
                opts.rustls_tls_config(
                    RustlsTlsConfig::builder()
                        .use_system_roots(false)
                        .add_roots_from_pem(&ca.to_pem().unwrap())
                        .unwrap()
                        .build(),
                )
            };
            let (tx, _) = mpsc::unbounded_channel();
            let client = TokioAsyncMqttClient::new(opts, Box::new(Handler(tx)), config())
                .await
                .unwrap();
            let broker = async {
                let (peer, _) = listener.accept().await.unwrap();
                let mut peer = acceptor.accept(peer).await.unwrap();
                assert!(matches!(
                    read_packet(&mut peer).await,
                    MqttPacket::Connect5(_)
                ));
                peer.write_all(&[0x20, 3, 0, 0, 0]).await.unwrap();
                assert!(matches!(
                    read_packet(&mut peer).await,
                    MqttPacket::Disconnect5(_)
                ));
                assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
            };
            let exchange = async {
                assert!(client.connect_sync().await.unwrap().is_success());
                client.disconnect_sync().await.unwrap();
                client.shutdown().await.unwrap();
            };
            timeout(Duration::from_secs(5), async {
                tokio::join!(broker, exchange)
            })
            .await
            .unwrap();
        }
    }
}
