// SPDX-License-Identifier: MPL-2.0
use flowsdk::mqtt_client::engine::{MqttEngine, MqttEvent};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::time::{Duration, Instant};

mod advanced;
#[cfg(test)]
mod api_tests;
#[cfg(test)]
mod c_tests;
pub mod ffi_types;
pub mod properties;
#[cfg(feature = "quic")]
mod quic;
#[cfg(any(feature = "tls", feature = "quic"))]
mod tls_config;
use ffi_types::*;

use std::sync::Mutex;

#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Object))]
pub struct MqttEngineFFI {
    engine: Mutex<MqttEngine>,
    start_time: Instant,
    events: Mutex<Vec<MqttEventFFI>>,
}

#[cfg(feature = "quic")]
use flowsdk::mqtt_client::engine::QuicMqttEngine;
#[cfg(feature = "tls")]
use flowsdk::mqtt_client::tls_engine::TlsMqttEngine;
#[cfg(feature = "quic")]
use std::net::SocketAddr;
#[cfg(feature = "tls")]
use std::sync::Arc;

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl MqttEngineFFI {
    /// Current milliseconds since this engine's clock origin.
    pub fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new(client_id: Option<String>, mqtt_version: u8) -> Result<Self, MqttErrorFFI> {
        Self::new_with_opts(MqttOptionsFFI {
            client_id: client_id.unwrap_or_else(|| "mqtt_client".into()),
            mqtt_version,
            clean_start: true,
            keep_alive: 60,
            username: None,
            password: None,
            reconnect_base_delay_ms: 1000,
            reconnect_max_delay_ms: 60000,
            max_reconnect_attempts: 0,
        })
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new_with_opts(opts: MqttOptionsFFI) -> Result<Self, MqttErrorFFI> {
        Self::new_with_options(opts.into())
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new_with_options(opts: MqttConnectOptionsFFI) -> Result<Self, MqttErrorFFI> {
        let engine = MqttEngine::new(opts.into_core()?);
        Ok(MqttEngineFFI {
            engine: Mutex::new(engine),
            start_time: Instant::now(),
            events: Mutex::new(Vec::new()),
        })
    }

    pub fn handle_connection_lost(&self) {
        self.engine.lock().unwrap().handle_connection_lost();
    }

    pub fn connect(&self) {
        let mut engine = self.engine.lock().unwrap();
        engine.reset_for_new_transport();
        self.events.lock().unwrap().clear();
        if let Err(error) = engine.connect() {
            self.events
                .lock()
                .unwrap()
                .extend(map_events(vec![MqttEvent::Error(error)]));
        }
    }

    pub fn handle_incoming(&self, data: Vec<u8>) -> Vec<MqttEventFFI> {
        let mut engine = self.engine.lock().unwrap();
        let events = engine.handle_incoming(&data);
        let mapped: Vec<_> = map_events(events);
        self.events.lock().unwrap().extend(mapped.iter().cloned());
        mapped
    }

    pub fn handle_tick(&self, now_ms: u64) -> Vec<MqttEventFFI> {
        let now = self.start_time + Duration::from_millis(now_ms);
        let mut engine = self.engine.lock().unwrap();
        let mut events = engine.handle_incoming(&[]);
        events.extend(engine.handle_tick(now));
        let mapped: Vec<_> = map_events(events);
        self.events.lock().unwrap().extend(mapped.iter().cloned());
        mapped
    }

    pub fn next_tick_ms(&self) -> i64 {
        match self.engine.lock().unwrap().next_tick_at() {
            Some(tick) => {
                if tick <= self.start_time {
                    0
                } else {
                    let duration = tick.duration_since(self.start_time);
                    duration.as_millis() as i64
                }
            }
            None => -1,
        }
    }

    pub fn take_outgoing(&self) -> Vec<u8> {
        self.engine.lock().unwrap().take_outgoing()
    }

    pub fn take_events(&self) -> Vec<MqttEventFFI> {
        let mut events = std::mem::take(&mut *self.events.lock().unwrap());
        let engine_events = self.engine.lock().unwrap().take_events();
        events.extend(map_events(engine_events));
        events
    }

    // Internal helper for C bridge
    pub fn push_event_ffi(&self, event: MqttEventFFI) {
        self.events.lock().unwrap().push(event);
    }

    pub fn ping(&self) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .try_send_ping()
            .map_err(Into::into)
    }

    pub fn auth_with_properties(
        &self,
        reason_code: u8,
        properties: Vec<MqttPropertyFFI>,
    ) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let properties = properties::validate(properties, engine.mqtt_version(), |p| {
            matches!(
                p,
                MqttPropertyFFI::AuthenticationMethod { .. }
                    | MqttPropertyFFI::AuthenticationData { .. }
                    | MqttPropertyFFI::ReasonString { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        engine.try_auth(reason_code, properties).map_err(Into::into)
    }

    pub fn publish(&self, topic: String, payload: Vec<u8>, qos: u8, priority: Option<u8>) -> i32 {
        match self.publish_with_options(
            topic,
            payload,
            MqttPublishOptionsFFI {
                qos,
                priority,
                ..Default::default()
            },
        ) {
            Ok(pid) => pid.map(i32::from).unwrap_or(0),
            Err(_) => -1,
        }
    }

    pub fn publish_with_options(
        &self,
        topic: String,
        payload: Vec<u8>,
        options: MqttPublishOptionsFFI,
    ) -> Result<Option<u16>, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(topic, payload, engine.mqtt_version(), true)?;
        engine.publish(command).map_err(Into::into)
    }

    pub fn subscribe(&self, topic_filter: String, qos: u8) -> i32 {
        self.subscribe_with_options(MqttSubscribeOptionsFFI {
            subscriptions: vec![MqttSubscriptionFFI {
                topic_filter,
                qos,
                ..Default::default()
            }],
            ..Default::default()
        })
        .map(i32::from)
        .unwrap_or(-1)
    }

    pub fn subscribe_with_options(
        &self,
        options: MqttSubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.mqtt_version())?;
        engine.subscribe(command).map_err(Into::into)
    }

    pub fn unsubscribe(&self, topic_filter: String) -> i32 {
        self.unsubscribe_with_options(MqttUnsubscribeOptionsFFI {
            topics: vec![topic_filter],
            ..Default::default()
        })
        .map(i32::from)
        .unwrap_or(-1)
    }

    pub fn unsubscribe_with_options(
        &self,
        options: MqttUnsubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.mqtt_version())?;
        engine.unsubscribe(command).map_err(Into::into)
    }

    pub fn disconnect(&self) -> Result<(), MqttErrorFFI> {
        self.disconnect_with_options(MqttDisconnectOptionsFFI::default())
    }

    pub fn disconnect_with_options(
        &self,
        options: MqttDisconnectOptionsFFI,
    ) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let properties = properties::validate(options.properties, engine.mqtt_version(), |p| {
            matches!(
                p,
                MqttPropertyFFI::SessionExpiryInterval { .. }
                    | MqttPropertyFFI::ReasonString { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        engine
            .try_disconnect_with(options.reason_code, properties)
            .map_err(Into::into)
    }

    pub fn disconnect_complete(&self) -> bool {
        !self.is_connected()
    }

    pub fn is_connected(&self) -> bool {
        self.engine.lock().unwrap().is_connected()
    }

    pub fn get_version(&self) -> u8 {
        self.engine.lock().unwrap().mqtt_version()
    }

    pub fn auth(&self, reason_code: u8) -> Result<(), MqttErrorFFI> {
        self.auth_with_properties(reason_code, Vec::new())
    }
}

impl From<flowsdk::mqtt_client::engine::QuicZeroRttStatus> for QuicZeroRttStatusFFI {
    fn from(status: flowsdk::mqtt_client::engine::QuicZeroRttStatus) -> Self {
        use flowsdk::mqtt_client::engine::QuicZeroRttStatus as Core;
        match status {
            Core::Disabled => Self::Disabled,
            Core::Unavailable => Self::Unavailable,
            Core::Attempted => Self::Attempted,
            Core::Accepted => Self::Accepted,
            Core::Rejected => Self::Rejected,
        }
    }
}

fn map_events(events: Vec<MqttEvent>) -> Vec<MqttEventFFI> {
    let mut stream = None;
    events
        .into_iter()
        .filter_map(|event| {
            let mut event = map_event(event)?;
            if let MqttEventFFI::PublishReceived { stream_id, .. } = &event {
                stream = *stream_id;
            }
            if let MqttEventFFI::MessageReceived(message) = &mut event {
                message.stream_id = stream.take();
            }
            Some(event)
        })
        .collect()
}

fn map_event(event: MqttEvent) -> Option<MqttEventFFI> {
    match event {
        MqttEvent::AuthReceived(res) => Some(MqttEventFFI::AuthReceived(AuthResultFFI {
            reason_code: res.reason_code,
            properties: res.properties.into_iter().map(Into::into).collect(),
        })),
        MqttEvent::Connected(res) => Some(MqttEventFFI::Connected(ConnectionResultFFI {
            reason_code: res.reason_code,
            session_present: res.session_present,
            properties: res
                .properties
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
        })),
        MqttEvent::Disconnected(code) => Some(MqttEventFFI::Disconnected {
            reason_code: code,
            properties: vec![],
        }),
        MqttEvent::DisconnectReceived {
            reason_code,
            properties,
        } => Some(MqttEventFFI::Disconnected {
            reason_code: Some(reason_code),
            properties: properties.into_iter().map(Into::into).collect(),
        }),
        MqttEvent::PublishReceived { packet_id, stream } => Some(MqttEventFFI::PublishReceived {
            packet_id,
            stream_id: stream,
        }),
        MqttEvent::PubRelReceived { packet_id, stream } => Some(MqttEventFFI::PubRelReceived {
            packet_id,
            stream_id: stream,
        }),
        MqttEvent::MessageReceived(msg) => Some(MqttEventFFI::MessageReceived(MqttMessageFFI {
            stream_id: None,
            topic: msg.topic_name,
            payload: msg.payload,
            qos: msg.qos,
            retain: msg.retain,
            dup: msg.dup,
            packet_id: msg.packet_id,
            properties: msg.properties.into_iter().map(Into::into).collect(),
        })),
        MqttEvent::Published(res) => Some(MqttEventFFI::Published(PublishResultFFI {
            packet_id: res.packet_id,
            reason_code: res.reason_code,
            qos: res.qos,
            properties: res
                .properties
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
        })),
        MqttEvent::Subscribed(res) => Some(MqttEventFFI::Subscribed(SubscribeResultFFI {
            packet_id: res.packet_id,
            reason_codes: res.reason_codes,
            properties: res.properties.into_iter().map(Into::into).collect(),
        })),
        MqttEvent::Unsubscribed(res) => Some(MqttEventFFI::Unsubscribed(UnsubscribeResultFFI {
            packet_id: res.packet_id,
            reason_codes: res.reason_codes,
            properties: res.properties.into_iter().map(Into::into).collect(),
        })),
        MqttEvent::PingResponse(res) => Some(MqttEventFFI::PingResponse {
            success: res.success,
        }),
        MqttEvent::OperationFailed { error: err, .. } | MqttEvent::Error(err) => {
            Some(MqttEventFFI::Error {
                message: format!("{:?}", err),
            })
        }
        MqttEvent::TransportClosed {
            reason,
            by_peer,
            error_code,
        } => Some(MqttEventFFI::TransportClosed {
            reason,
            by_peer,
            error_code,
        }),
        MqttEvent::StreamClosed {
            stream_id,
            reason,
            by_peer,
        } => Some(MqttEventFFI::StreamClosed {
            stream_id,
            reason,
            by_peer,
        }),
        MqttEvent::StreamReset {
            stream_id,
            error_code,
        } => Some(MqttEventFFI::StreamReset {
            stream_id,
            error_code,
        }),
        MqttEvent::StreamStopped {
            stream_id,
            error_code,
        } => Some(MqttEventFFI::StreamStopped {
            stream_id,
            error_code,
        }),
        MqttEvent::ZeroRttStatusChanged { status } => Some(MqttEventFFI::ZeroRttStatusChanged {
            status: status.into(),
        }),
        MqttEvent::ReconnectNeeded => Some(MqttEventFFI::ReconnectNeeded),
        MqttEvent::ReconnectScheduled { attempt, delay } => {
            Some(MqttEventFFI::ReconnectScheduled {
                attempt,
                delay_ms: delay.as_millis() as u64,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowsdk::mqtt_client::engine::QuicZeroRttStatus;
    use flowsdk::mqtt_serde::control_packet::MqttPacket;
    use flowsdk::mqtt_serde::parser::ParseOk;
    use std::time::Duration;

    fn authenticated_options(version: u8) -> MqttOptionsFFI {
        MqttOptionsFFI {
            client_id: "ffi-auth-test".to_string(),
            mqtt_version: version,
            clean_start: true,
            keep_alive: 30,
            username: Some("test-user".to_string()),
            password: Some("test-password".to_string()),
            reconnect_base_delay_ms: 1000,
            reconnect_max_delay_ms: 30000,
            max_reconnect_attempts: 0,
        }
    }

    #[test]
    fn full_message_mapping_keeps_originating_stream() {
        let message = flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish::new_with_prop(
            1,
            "test".into(),
            Some(7),
            vec![],
            false,
            false,
            vec![],
        );
        let events = map_events(vec![
            MqttEvent::PublishReceived {
                packet_id: Some(7),
                stream: Some(12),
            },
            MqttEvent::MessageReceived(message),
        ]);
        assert!(matches!(
            &events[0],
            MqttEventFFI::PublishReceived {
                stream_id: Some(12),
                ..
            }
        ));
        assert!(
            matches!(&events[1], MqttEventFFI::MessageReceived(message) if message.stream_id == Some(12))
        );
    }

    #[test]
    fn connect_encodes_ffi_credentials() {
        for version in [3, 4, 5] {
            let engine = MqttEngineFFI::new_with_opts(authenticated_options(version)).unwrap();
            engine.connect();
            let bytes = engine.take_outgoing();
            let (username, password) =
                match MqttPacket::from_bytes_with_version(&bytes, version).unwrap() {
                    ParseOk::Packet(MqttPacket::Connect3(packet), _) => {
                        (packet.username, packet.password)
                    }
                    ParseOk::Packet(MqttPacket::Connect5(packet), _) => {
                        (packet.username, packet.password)
                    }
                    packet => panic!("Expected CONNECT, got {packet:?}"),
                };
            assert_eq!(username.as_deref(), Some("test-user"));
            assert_eq!(password.as_deref(), Some(b"test-password".as_slice()));
        }
    }

    #[test]
    fn publish_options_reach_the_wire() {
        for version in [3, 5] {
            let engine = MqttEngineFFI::new_with_opts(authenticated_options(version)).unwrap();
            engine.connect();
            engine.take_outgoing();
            engine.handle_incoming(if version == 5 {
                vec![0x20, 3, 0, 0, 0]
            } else {
                vec![0x20, 2, 0, 0]
            });
            let properties = if version == 5 {
                vec![
                    MqttPropertyFFI::UserProperty {
                        key: "source".into(),
                        value: "one".into(),
                    },
                    MqttPropertyFFI::UserProperty {
                        key: "source".into(),
                        value: "two".into(),
                    },
                    MqttPropertyFFI::CorrelationData {
                        value: vec![0, 255],
                    },
                ]
            } else {
                vec![]
            };
            engine
                .publish_with_options(
                    "test/topic".into(),
                    b"data".to_vec(),
                    MqttPublishOptionsFFI {
                        retain: true,
                        properties: properties.clone(),
                        ..Default::default()
                    },
                )
                .unwrap();
            let bytes = engine.take_outgoing();
            match MqttPacket::from_bytes_with_version(&bytes, version).unwrap() {
                ParseOk::Packet(MqttPacket::Publish5(packet), _) => {
                    assert!(packet.retain);
                    assert_eq!(
                        packet.properties,
                        properties.into_iter().map(Into::into).collect::<Vec<_>>()
                    );
                }
                ParseOk::Packet(MqttPacket::Publish3(packet), _) => assert!(packet.retain),
                other => panic!("Expected PUBLISH, got {other:?}"),
            }
        }
    }

    #[cfg(feature = "quic")]
    #[test]
    fn quic_constructor_preserves_mqtt_credentials() {
        for version in [3, 4, 5] {
            let engine = QuicMqttEngineFFI::new(authenticated_options(version)).unwrap();
            let inner = engine.engine.lock().unwrap();
            let options = inner.engine().options();
            assert_eq!(options.username.as_deref(), Some("test-user"));
            assert_eq!(
                options.password.as_deref(),
                Some(b"test-password".as_slice())
            );
        }
    }

    #[test]
    fn connect_options_encode_will_properties_and_binary_password() {
        for version in [3, 5] {
            let mut opts = authenticated_options(version);
            opts.password = None;
            let properties = if version == 5 {
                vec![
                    MqttPropertyFFI::SessionExpiryInterval { value: 60 },
                    MqttPropertyFFI::ReceiveMaximum { value: 10 },
                ]
            } else {
                vec![]
            };
            let engine = MqttEngineFFI::new_with_options(MqttConnectOptionsFFI {
                options: opts,
                properties: properties.clone(),
                engine_options: None,
                binary_password: Some(vec![0, 255]),
                will: Some(MqttWillFFI {
                    topic: "status".into(),
                    payload: vec![1, 255],
                    qos: 1,
                    retain: true,
                    properties: if version == 5 {
                        vec![MqttPropertyFFI::WillDelayInterval { value: 30 }]
                    } else {
                        vec![]
                    },
                }),
            })
            .unwrap();
            engine.connect();
            match MqttPacket::from_bytes_with_version(&engine.take_outgoing(), version).unwrap() {
                ParseOk::Packet(MqttPacket::Connect5(packet), _) => {
                    assert_eq!(packet.password, Some(vec![0, 255]));
                    assert_eq!(
                        packet.properties,
                        properties.into_iter().map(Into::into).collect::<Vec<_>>()
                    );
                    let will = packet.will.unwrap();
                    assert_eq!(will.will_message, vec![1, 255]);
                    assert_eq!(will.properties.will_delay_interval, Some(30));
                }
                ParseOk::Packet(MqttPacket::Connect3(packet), _) => {
                    assert_eq!(packet.password, Some(vec![0, 255]));
                    assert_eq!(packet.will.unwrap().message, vec![1, 255]);
                }
                packet => panic!("Expected CONNECT, got {packet:?}"),
            }
        }
        assert!(MqttEngineFFI::new(None, 0).is_err());
        let mut options: MqttConnectOptionsFFI = authenticated_options(3).into();
        options.properties = vec![MqttPropertyFFI::SessionExpiryInterval { value: 60 }];
        assert!(MqttEngineFFI::new_with_options(options).is_err());
    }

    #[cfg(all(feature = "tls", feature = "quic"))]
    #[test]
    fn invalid_transport_configuration_returns_errors() {
        let tls_opts = MqttTlsOptionsFFI {
            insecure_skip_verify: true,
            ..Default::default()
        };
        assert!(TlsMqttEngineFFI::new(
            authenticated_options(5),
            tls_opts.clone(),
            "invalid server name".into(),
        )
        .is_err());
        let quic = QuicMqttEngineFFI::new(authenticated_options(5)).unwrap();
        assert!(matches!(
            quic.connect("not an address".into(), "localhost".into(), tls_opts, 0),
            Err(MqttErrorFFI::InvalidArgument { .. })
        ));
    }

    #[test]
    fn zero_rtt_status_event_is_not_reported_as_ffi_error() {
        let event = MqttEvent::ZeroRttStatusChanged {
            status: QuicZeroRttStatus::Attempted,
        };

        assert!(matches!(
            map_event(event),
            Some(MqttEventFFI::ZeroRttStatusChanged {
                status: QuicZeroRttStatusFFI::Attempted
            })
        ));
    }

    #[test]
    fn transport_closed_event_is_not_reported_as_ffi_error() {
        let event = MqttEvent::TransportClosed {
            reason: "connection closed".to_string(),
            by_peer: true,
            error_code: Some(0),
        };

        assert!(matches!(
            map_event(event),
            Some(MqttEventFFI::TransportClosed {
                by_peer: true,
                error_code: Some(0),
                ..
            })
        ));
    }

    #[test]
    fn maps_stream_and_reconnect_events() {
        assert!(matches!(
            map_event(MqttEvent::StreamClosed {
                stream_id: 7,
                reason: "recv_finished".to_string(),
                by_peer: true,
            }),
            Some(MqttEventFFI::StreamClosed {
                stream_id: 7,
                by_peer: true,
                ..
            })
        ));
        assert!(matches!(
            map_event(MqttEvent::StreamReset {
                stream_id: 8,
                error_code: 42,
            }),
            Some(MqttEventFFI::StreamReset {
                stream_id: 8,
                error_code: 42,
            })
        ));
        assert!(matches!(
            map_event(MqttEvent::StreamStopped {
                stream_id: 9,
                error_code: 43,
            }),
            Some(MqttEventFFI::StreamStopped {
                stream_id: 9,
                error_code: 43,
            })
        ));
        assert!(matches!(
            map_event(MqttEvent::ReconnectNeeded),
            Some(MqttEventFFI::ReconnectNeeded)
        ));
        assert!(matches!(
            map_event(MqttEvent::ReconnectScheduled {
                attempt: 3,
                delay: Duration::from_millis(250),
            }),
            Some(MqttEventFFI::ReconnectScheduled {
                attempt: 3,
                delay_ms: 250,
            })
        ));
    }
}

#[cfg(feature = "tls")]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Object))]
pub struct TlsMqttEngineFFI {
    engine: Mutex<TlsMqttEngine>,
    start_time: Instant,
    events: Mutex<Vec<MqttEventFFI>>,
}

#[cfg(feature = "tls")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl TlsMqttEngineFFI {
    /// Current milliseconds since this engine's clock origin.
    pub fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new(
        opts: MqttOptionsFFI,
        tls_opts: MqttTlsOptionsFFI,
        server_name: String,
    ) -> Result<Self, MqttErrorFFI> {
        Self::new_with_options(opts.into(), tls_opts, server_name)
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new_with_options(
        opts: MqttConnectOptionsFFI,
        tls_opts: MqttTlsOptionsFFI,
        server_name: String,
    ) -> Result<Self, MqttErrorFFI> {
        let config = tls_config::client_config(&tls_opts)?;
        let engine = TlsMqttEngine::new(opts.into_core()?, &server_name, Arc::new(config))?;
        Ok(TlsMqttEngineFFI {
            engine: Mutex::new(engine),
            start_time: Instant::now(),
            events: Mutex::new(Vec::new()),
        })
    }

    pub fn handle_socket_data(&self, data: Vec<u8>) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .handle_socket_data(&data)
            .map_err(Into::into)
    }

    pub fn take_socket_data(&self) -> Vec<u8> {
        self.engine.lock().unwrap().take_socket_data()
    }

    pub fn handle_tick(&self, now_ms: u64) -> Vec<MqttEventFFI> {
        let now = self.start_time + Duration::from_millis(now_ms);
        let events = self.engine.lock().unwrap().handle_tick(now);
        let mapped: Vec<_> = map_events(events);
        self.events.lock().unwrap().extend(mapped.iter().cloned());
        mapped
    }

    pub fn take_events(&self) -> Vec<MqttEventFFI> {
        let mut events = std::mem::take(&mut *self.events.lock().unwrap());
        let engine_events = self.engine.lock().unwrap().take_events();
        events.extend(map_events(engine_events));
        events
    }

    pub fn connect(&self) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        engine.reset_for_new_transport()?;
        self.events.lock().unwrap().clear();
        engine.connect()?;
        Ok(())
    }

    pub fn handle_connection_lost(&self) {
        self.engine.lock().unwrap().handle_connection_lost();
    }

    pub fn ping(&self) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .try_send_ping()
            .map_err(Into::into)
    }

    pub fn auth_with_properties(
        &self,
        reason_code: u8,
        properties: Vec<MqttPropertyFFI>,
    ) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let properties = properties::validate(properties, engine.mqtt_version(), |p| {
            matches!(
                p,
                MqttPropertyFFI::AuthenticationMethod { .. }
                    | MqttPropertyFFI::AuthenticationData { .. }
                    | MqttPropertyFFI::ReasonString { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        engine.try_auth(reason_code, properties).map_err(Into::into)
    }

    pub fn auth(&self, reason_code: u8) -> Result<(), MqttErrorFFI> {
        self.auth_with_properties(reason_code, Vec::new())
    }

    pub fn publish(&self, topic: String, payload: Vec<u8>, qos: u8) -> i32 {
        match self.publish_with_options(
            topic,
            payload,
            MqttPublishOptionsFFI {
                qos,
                priority: None,
                ..Default::default()
            },
        ) {
            Ok(pid) => pid.map(i32::from).unwrap_or(0),
            Err(_) => -1,
        }
    }

    pub fn publish_with_options(
        &self,
        topic: String,
        payload: Vec<u8>,
        options: MqttPublishOptionsFFI,
    ) -> Result<Option<u16>, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(topic, payload, engine.mqtt_version(), true)?;
        engine.publish(command).map_err(Into::into)
    }

    pub fn subscribe(&self, topic_filter: String, qos: u8) -> i32 {
        self.subscribe_with_options(MqttSubscribeOptionsFFI {
            subscriptions: vec![MqttSubscriptionFFI {
                topic_filter,
                qos,
                ..Default::default()
            }],
            ..Default::default()
        })
        .map(i32::from)
        .unwrap_or(-1)
    }

    pub fn subscribe_with_options(
        &self,
        options: MqttSubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.mqtt_version())?;
        engine.subscribe(command).map_err(Into::into)
    }

    pub fn unsubscribe(&self, topic_filter: String) -> i32 {
        self.unsubscribe_with_options(MqttUnsubscribeOptionsFFI {
            topics: vec![topic_filter],
            ..Default::default()
        })
        .map(i32::from)
        .unwrap_or(-1)
    }

    pub fn unsubscribe_with_options(
        &self,
        options: MqttUnsubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.mqtt_version())?;
        engine.unsubscribe(command).map_err(Into::into)
    }

    pub fn disconnect(&self) -> Result<(), MqttErrorFFI> {
        self.disconnect_with_options(MqttDisconnectOptionsFFI::default())
    }

    pub fn disconnect_with_options(
        &self,
        options: MqttDisconnectOptionsFFI,
    ) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let properties = properties::validate(options.properties, engine.mqtt_version(), |p| {
            matches!(
                p,
                MqttPropertyFFI::SessionExpiryInterval { .. }
                    | MqttPropertyFFI::ReasonString { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        engine
            .try_disconnect_with(options.reason_code, properties)
            .map_err(Into::into)
    }

    pub fn disconnect_complete(&self) -> bool {
        self.engine.lock().unwrap().disconnect_complete()
    }

    pub fn is_connected(&self) -> bool {
        self.engine.lock().unwrap().is_connected()
    }
}

#[cfg(not(feature = "tls"))]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Object))]
pub struct TlsMqttEngineFFI {
    start_time: Instant,
    events: Mutex<Vec<MqttEventFFI>>,
}

#[cfg(not(feature = "tls"))]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl TlsMqttEngineFFI {
    /// Current milliseconds since this engine's clock origin.
    pub fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new(
        _opts: MqttOptionsFFI,
        _tls_opts: MqttTlsOptionsFFI,
        _server_name: String,
    ) -> Result<Self, MqttErrorFFI> {
        Err(MqttErrorFFI::Unsupported {
            detail: "TLS support is not enabled".into(),
        })
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new_with_options(
        _opts: MqttConnectOptionsFFI,
        _tls_opts: MqttTlsOptionsFFI,
        _server_name: String,
    ) -> Result<Self, MqttErrorFFI> {
        Err(MqttErrorFFI::Unsupported {
            detail: "TLS support is not enabled".into(),
        })
    }

    pub fn handle_socket_data(&self, _data: Vec<u8>) -> Result<(), MqttErrorFFI> {
        Err(MqttErrorFFI::Unsupported {
            detail: "TLS support is not enabled".into(),
        })
    }

    pub fn take_socket_data(&self) -> Vec<u8> {
        Vec::new()
    }

    pub fn handle_tick(&self, _now_ms: u64) -> Vec<MqttEventFFI> {
        let _ = self.start_time;
        Vec::new()
    }

    pub fn take_events(&self) -> Vec<MqttEventFFI> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    pub fn connect(&self) -> Result<(), MqttErrorFFI> {
        Ok(())
    }

    pub fn publish(&self, _topic: String, _payload: Vec<u8>, _qos: u8) -> i32 {
        -1
    }

    pub fn subscribe(&self, _topic_filter: String, _qos: u8) -> i32 {
        -1
    }

    pub fn unsubscribe(&self, _topic_filter: String) -> i32 {
        -1
    }

    pub fn disconnect(&self) -> Result<(), MqttErrorFFI> {
        Err(MqttErrorFFI::Unsupported {
            detail: "TLS support is not enabled".into(),
        })
    }

    pub fn disconnect_complete(&self) -> bool {
        true
    }

    pub fn is_connected(&self) -> bool {
        false
    }
}

#[cfg(any(feature = "tls", feature = "quic"))]
#[derive(Debug)]
struct InsecureServerCertVerifier;

#[cfg(any(feature = "tls", feature = "quic"))]
impl rustls::client::danger::ServerCertVerifier for InsecureServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
        ]
    }
}

#[cfg(feature = "quic")]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Object))]
pub struct QuicMqttEngineFFI {
    engine: Mutex<QuicMqttEngine>,
    start_time: Instant,
    events: Mutex<Vec<MqttEventFFI>>,
}

#[cfg(feature = "quic")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl QuicMqttEngineFFI {
    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new(opts: MqttOptionsFFI) -> Result<Self, MqttErrorFFI> {
        Self::new_with_options(opts.into())
    }

    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new_with_options(opts: MqttConnectOptionsFFI) -> Result<Self, MqttErrorFFI> {
        let engine = QuicMqttEngine::new(opts.into_core()?)?;
        Ok(QuicMqttEngineFFI {
            engine: Mutex::new(engine),
            start_time: Instant::now(),
            events: Mutex::new(Vec::new()),
        })
    }

    pub fn connect(
        &self,
        server_addr: String,
        server_name: String,
        tls_opts: MqttTlsOptionsFFI,
        now_ms: u64,
    ) -> Result<(), MqttErrorFFI> {
        let addr: SocketAddr = server_addr.parse().map_err(|e: std::net::AddrParseError| {
            MqttErrorFFI::InvalidArgument {
                detail: e.to_string(),
            }
        })?;
        let now = self.start_time + Duration::from_millis(now_ms);
        let config = tls_config::client_config(&tls_opts)?;
        self.engine
            .lock()
            .unwrap()
            .connect(addr, &server_name, config, now)
            .map_err(Into::into)
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    pub fn handle_datagram(
        &self,
        data: Vec<u8>,
        remote_addr: String,
        now_ms: u64,
    ) -> Result<(), MqttErrorFFI> {
        let addr: SocketAddr = remote_addr.parse().map_err(|e: std::net::AddrParseError| {
            MqttErrorFFI::InvalidArgument {
                detail: e.to_string(),
            }
        })?;
        let now = self.start_time + Duration::from_millis(now_ms);
        self.engine.lock().unwrap().handle_datagram(data, addr, now);
        Ok(())
    }

    pub fn take_outgoing_datagrams(&self) -> Vec<MqttDatagramFFI> {
        let datagrams = self.engine.lock().unwrap().take_outgoing_datagrams();
        datagrams
            .into_iter()
            .map(|(addr, data)| MqttDatagramFFI {
                addr: addr.to_string(),
                data,
            })
            .collect()
    }

    pub fn handle_tick(&self, now_ms: u64) -> Vec<MqttEventFFI> {
        let now = self.start_time + Duration::from_millis(now_ms);
        let mut engine = self.engine.lock().unwrap();
        let events = engine.handle_tick(now);
        let mapped: Vec<_> = map_events(events);
        self.events.lock().unwrap().extend(mapped.iter().cloned());
        mapped
    }

    pub fn take_events(&self) -> Vec<MqttEventFFI> {
        let mut events = std::mem::take(&mut *self.events.lock().unwrap());
        let engine_events = self.engine.lock().unwrap().take_events();
        events.extend(map_events(engine_events));
        events
    }

    pub fn ping(&self) -> Result<(), MqttErrorFFI> {
        self.engine.lock().unwrap().ping().map_err(Into::into)
    }

    pub fn auth_with_properties(
        &self,
        reason_code: u8,
        properties: Vec<MqttPropertyFFI>,
    ) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let properties = properties::validate(properties, engine.engine().mqtt_version(), |p| {
            matches!(
                p,
                MqttPropertyFFI::AuthenticationMethod { .. }
                    | MqttPropertyFFI::AuthenticationData { .. }
                    | MqttPropertyFFI::ReasonString { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        engine
            .engine_mut()
            .try_auth(reason_code, properties)
            .map_err(Into::into)
    }

    pub fn auth(&self, reason_code: u8) -> Result<(), MqttErrorFFI> {
        self.auth_with_properties(reason_code, Vec::new())
    }

    pub fn publish(&self, topic: String, payload: Vec<u8>, qos: u8) -> i32 {
        match self.publish_with_options(
            topic,
            payload,
            MqttPublishOptionsFFI {
                qos,
                priority: None,
                ..Default::default()
            },
        ) {
            Ok(pid) => pid.map(i32::from).unwrap_or(0),
            Err(_) => -1,
        }
    }

    pub fn publish_with_options(
        &self,
        topic: String,
        payload: Vec<u8>,
        options: MqttPublishOptionsFFI,
    ) -> Result<Option<u16>, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(topic, payload, engine.engine().mqtt_version(), true)?;
        engine.publish(command).map_err(Into::into)
    }

    pub fn subscribe(&self, topic_filter: String, qos: u8) -> i32 {
        self.subscribe_with_options(MqttSubscribeOptionsFFI {
            subscriptions: vec![MqttSubscriptionFFI {
                topic_filter,
                qos,
                ..Default::default()
            }],
            ..Default::default()
        })
        .map(i32::from)
        .unwrap_or(-1)
    }

    pub fn subscribe_with_options(
        &self,
        options: MqttSubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.engine().mqtt_version())?;
        engine.subscribe(command).map_err(Into::into)
    }

    pub fn unsubscribe(&self, topic_filter: String) -> i32 {
        self.unsubscribe_with_options(MqttUnsubscribeOptionsFFI {
            topics: vec![topic_filter],
            ..Default::default()
        })
        .map(i32::from)
        .unwrap_or(-1)
    }

    pub fn unsubscribe_with_options(
        &self,
        options: MqttUnsubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.engine().mqtt_version())?;
        engine.unsubscribe(command).map_err(Into::into)
    }

    pub fn disconnect(&self) -> Result<(), MqttErrorFFI> {
        self.disconnect_with_options(MqttDisconnectOptionsFFI::default())
    }

    pub fn disconnect_with_options(
        &self,
        options: MqttDisconnectOptionsFFI,
    ) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let properties =
            properties::validate(options.properties, engine.engine().mqtt_version(), |p| {
                matches!(
                    p,
                    MqttPropertyFFI::SessionExpiryInterval { .. }
                        | MqttPropertyFFI::ReasonString { .. }
                        | MqttPropertyFFI::UserProperty { .. }
                )
            })?;
        engine
            .disconnect_and_close_with(options.reason_code, properties, 0, b"")
            .map_err(Into::into)
    }

    pub fn disconnect_complete(&self) -> bool {
        self.engine.lock().unwrap().disconnect_complete()
    }

    pub fn is_connected(&self) -> bool {
        self.engine.lock().unwrap().is_connected()
    }
}

// --- C-Compatible FFI Layer ---
// This layer provides a stable ABI for the C examples, mapping to the UniFFI objects.

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer for `client_id`
/// and returns a raw pointer to a new `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_new(
    client_id: *const c_char,
    mqtt_version: u8,
) -> *mut MqttEngineFFI {
    let client_id = if client_id.is_null() {
        None
    } else {
        Some(CStr::from_ptr(client_id).to_string_lossy().into_owned())
    };
    match MqttEngineFFI::new(client_id, mqtt_version) {
        Ok(engine) => Box::into_raw(Box::new(engine)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer for `opts`
/// and returns a raw pointer to a new `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_new_with_opts(
    opts: *const MqttOptionsC,
) -> *mut MqttEngineFFI {
    if opts.is_null() {
        return std::ptr::null_mut();
    }
    let r = &*opts;
    let client_id = if r.client_id.is_null() {
        "mqtt_client".to_string()
    } else {
        CStr::from_ptr(r.client_id).to_string_lossy().into_owned()
    };
    let username = if r.username.is_null() {
        None
    } else {
        Some(CStr::from_ptr(r.username).to_string_lossy().into_owned())
    };
    let password = if r.password.is_null() {
        None
    } else {
        Some(CStr::from_ptr(r.password).to_string_lossy().into_owned())
    };

    let new_opts = MqttOptionsFFI {
        client_id,
        mqtt_version: r.mqtt_version,
        clean_start: r.clean_start,
        keep_alive: r.keep_alive,
        username,
        password,
        reconnect_base_delay_ms: r.reconnect_base_delay_ms,
        reconnect_max_delay_ms: r.reconnect_max_delay_ms,
        max_reconnect_attempts: r.max_reconnect_attempts,
    };
    match MqttEngineFFI::new_with_opts(new_opts) {
        Ok(engine) => Box::into_raw(Box::new(engine)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`
/// and performs manual memory deallocation.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_free(ptr: *mut MqttEngineFFI) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_connect(ptr: *mut MqttEngineFFI) {
    if let Some(engine) = ptr.as_ref() {
        engine.connect();
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `data`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_incoming(
    ptr: *mut MqttEngineFFI,
    data: *const u8,
    len: usize,
) {
    if let (Some(engine), true) = (ptr.as_ref(), !data.is_null()) {
        let buf = std::slice::from_raw_parts(data, len);
        engine.handle_incoming(buf.to_vec());
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_tick(ptr: *mut MqttEngineFFI, now_ms: u64) {
    if let Some(engine) = ptr.as_ref() {
        engine.handle_tick(now_ms);
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_next_tick_ms(ptr: *mut MqttEngineFFI) -> i64 {
    if let Some(engine) = ptr.as_ref() {
        engine.next_tick_ms()
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_take_outgoing(
    ptr: *mut MqttEngineFFI,
    out_len: *mut usize,
) -> *mut u8 {
    if let Some(engine) = ptr.as_ref() {
        let bytes = engine.take_outgoing();
        if bytes.is_empty() {
            if !out_len.is_null() {
                *out_len = 0;
            }
            return std::ptr::null_mut();
        }
        if !out_len.is_null() {
            *out_len = bytes.len();
        }
        let mut b = bytes.into_boxed_slice();
        let p = b.as_mut_ptr();
        std::mem::forget(b);
        p
    } else {
        std::ptr::null_mut()
    }
}

/// # Safety
///
/// This function is unsafe because it performs manual memory deallocation.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_free_bytes(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr`, `topic`, and `payload`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_publish(
    ptr: *mut MqttEngineFFI,
    topic: *const c_char,
    payload: *const u8,
    payload_len: usize,
    qos: u8,
) -> i32 {
    if let (Some(engine), true, true) = (ptr.as_ref(), !topic.is_null(), !payload.is_null()) {
        let topic = CStr::from_ptr(topic).to_string_lossy().into_owned();
        let payload = std::slice::from_raw_parts(payload, payload_len).to_vec();
        engine.publish(topic, payload, qos, None)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `topic_filter`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_subscribe(
    ptr: *mut MqttEngineFFI,
    topic_filter: *const c_char,
    qos: u8,
) -> i32 {
    if let (Some(engine), true) = (ptr.as_ref(), !topic_filter.is_null()) {
        let topic = CStr::from_ptr(topic_filter).to_string_lossy().into_owned();
        engine.subscribe(topic, qos)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `topic_filter`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_unsubscribe(
    ptr: *mut MqttEngineFFI,
    topic_filter: *const c_char,
) -> i32 {
    if let (Some(engine), true) = (ptr.as_ref(), !topic_filter.is_null()) {
        let topic = CStr::from_ptr(topic_filter).to_string_lossy().into_owned();
        engine.unsubscribe(topic)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_disconnect(ptr: *mut MqttEngineFFI) {
    if let Some(engine) = ptr.as_ref() {
        if let Err(error) = engine.disconnect() {
            engine.events.lock().unwrap().push(MqttEventFFI::Error {
                message: error.to_string(),
            });
        }
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_is_connected(ptr: *mut MqttEngineFFI) -> c_int {
    if let Some(engine) = ptr.as_ref() {
        if engine.is_connected() {
            1
        } else {
            0
        }
    } else {
        0
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_get_version(ptr: *mut MqttEngineFFI) -> u8 {
    if let Some(engine) = ptr.as_ref() {
        engine.get_version()
    } else {
        0
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_auth(ptr: *mut MqttEngineFFI, reason_code: u8) {
    if let Some(engine) = ptr.as_ref() {
        if let Err(error) = engine.auth(reason_code) {
            engine.events.lock().unwrap().push(MqttEventFFI::Error {
                message: error.to_string(),
            });
        }
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_connection_lost(ptr: *mut MqttEngineFFI) {
    if let Some(engine) = ptr.as_ref() {
        engine.handle_connection_lost();
    }
}

/// # Safety
///
/// This function is unsafe because it performs manual memory deallocation of a `CString`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

// TLS Engine C wrappers
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `client_id`,
/// `server_name`, and `tls_opts`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_new(
    client_id: *const c_char,
    mqtt_version: u8,
    server_name: *const c_char,
    tls_opts: *const MqttTlsOptionsC,
) -> *mut TlsMqttEngineFFI {
    let client_id = if client_id.is_null() {
        "mqtt_client".to_string()
    } else {
        CStr::from_ptr(client_id).to_string_lossy().into_owned()
    };
    let server_name = if server_name.is_null() {
        "localhost".to_string()
    } else {
        CStr::from_ptr(server_name).to_string_lossy().into_owned()
    };

    let opts = MqttOptionsFFI {
        client_id,
        mqtt_version,
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        reconnect_base_delay_ms: 1000,
        reconnect_max_delay_ms: 30000,
        max_reconnect_attempts: 0,
    };

    let tls_opts_v = if tls_opts.is_null() {
        MqttTlsOptionsFFI {
            ca_cert_file: None,
            client_cert_file: None,
            client_key_file: None,
            insecure_skip_verify: false,
            alpn_protocols: vec!["mqtt".to_string()],
            enable_key_log: false,
        }
    } else {
        let r = &*tls_opts;
        let ca_cert_file = if r.ca_cert_file.is_null() {
            None
        } else {
            Some(
                CStr::from_ptr(r.ca_cert_file)
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let client_cert_file = if r.client_cert_file.is_null() {
            None
        } else {
            Some(
                CStr::from_ptr(r.client_cert_file)
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let client_key_file = if r.client_key_file.is_null() {
            None
        } else {
            Some(
                CStr::from_ptr(r.client_key_file)
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let alpn_protocols = if r.alpn.is_null() {
            vec!["mqtt".to_string()]
        } else {
            vec![CStr::from_ptr(r.alpn).to_string_lossy().into_owned()]
        };
        MqttTlsOptionsFFI {
            ca_cert_file,
            client_cert_file,
            client_key_file,
            insecure_skip_verify: r.insecure_skip_verify != 0,
            alpn_protocols,
            enable_key_log: r.enable_key_log != 0,
        }
    };

    match TlsMqttEngineFFI::new(opts, tls_opts_v, server_name) {
        Ok(engine) => Box::into_raw(Box::new(engine)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
///
/// This function is unsafe because it performs manual memory deallocation of a `TlsMqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_free(ptr: *mut TlsMqttEngineFFI) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `TlsMqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_connect(ptr: *mut TlsMqttEngineFFI) {
    if let Some(engine) = ptr.as_ref() {
        if let Err(error) = engine.connect() {
            engine.events.lock().unwrap().push(MqttEventFFI::Error {
                message: error.to_string(),
            });
        }
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `data`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_handle_socket_data(
    ptr: *mut TlsMqttEngineFFI,
    data: *const u8,
    len: usize,
) {
    if let (Some(engine), true) = (ptr.as_ref(), !data.is_null()) {
        let buf = std::slice::from_raw_parts(data, len);
        if let Err(error) = engine.handle_socket_data(buf.to_vec()) {
            engine.events.lock().unwrap().push(MqttEventFFI::Error {
                message: error.to_string(),
            });
        }
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `out_len`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_take_socket_data(
    ptr: *mut TlsMqttEngineFFI,
    out_len: *mut usize,
) -> *mut u8 {
    if let Some(engine) = ptr.as_ref() {
        let bytes = engine.take_socket_data();
        if bytes.is_empty() {
            if !out_len.is_null() {
                *out_len = 0;
            }
            return std::ptr::null_mut();
        }
        if !out_len.is_null() {
            *out_len = bytes.len();
        }
        let mut b = bytes.into_boxed_slice();
        let p = b.as_mut_ptr();
        std::mem::forget(b);
        p
    } else {
        std::ptr::null_mut()
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `TlsMqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_handle_tick(ptr: *mut TlsMqttEngineFFI, now_ms: u64) {
    if let Some(engine) = ptr.as_ref() {
        engine.handle_tick(now_ms);
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr`, `topic`, and `payload`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_publish(
    ptr: *mut TlsMqttEngineFFI,
    topic: *const c_char,
    payload: *const u8,
    payload_len: usize,
    qos: u8,
) -> i32 {
    if let (Some(engine), true, true) = (ptr.as_ref(), !topic.is_null(), !payload.is_null()) {
        let topic = CStr::from_ptr(topic).to_string_lossy().into_owned();
        let payload = std::slice::from_raw_parts(payload, payload_len).to_vec();
        engine.publish(topic, payload, qos)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `topic_filter`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_subscribe(
    ptr: *mut TlsMqttEngineFFI,
    topic_filter: *const c_char,
    qos: u8,
) -> i32 {
    if let (Some(engine), true) = (ptr.as_ref(), !topic_filter.is_null()) {
        let topic = CStr::from_ptr(topic_filter).to_string_lossy().into_owned();
        engine.subscribe(topic, qos)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `topic_filter`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_unsubscribe(
    ptr: *mut TlsMqttEngineFFI,
    topic_filter: *const c_char,
) -> i32 {
    if let (Some(engine), true) = (ptr.as_ref(), !topic_filter.is_null()) {
        let topic = CStr::from_ptr(topic_filter).to_string_lossy().into_owned();
        engine.unsubscribe(topic)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `TlsMqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_disconnect(ptr: *mut TlsMqttEngineFFI) {
    if let Some(engine) = ptr.as_ref() {
        if let Err(error) = engine.disconnect() {
            engine.events.lock().unwrap().push(MqttEventFFI::Error {
                message: error.to_string(),
            });
        }
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `TlsMqttEngineFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_is_connected(ptr: *mut TlsMqttEngineFFI) -> i32 {
    if let Some(engine) = ptr.as_ref() {
        if engine.is_connected() {
            1
        } else {
            0
        }
    } else {
        0
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`
/// and returns an allocated `c_char` pointer that must be freed using `mqtt_engine_free_string`.
#[no_mangle]
#[cfg(feature = "uniffi-bindings")]
pub unsafe extern "C" fn mqtt_engine_take_events(ptr: *mut MqttEngineFFI) -> *mut c_char {
    if let Some(engine) = ptr.as_ref() {
        let events = engine.take_events();
        let json = serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string());
        CString::new(json).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `TlsMqttEngineFFI`
/// and returns an allocated `c_char` pointer that must be freed using `mqtt_engine_free_string`.
#[no_mangle]
#[cfg(feature = "uniffi-bindings")]
pub unsafe extern "C" fn mqtt_tls_engine_take_events(ptr: *mut TlsMqttEngineFFI) -> *mut c_char {
    if let Some(engine) = ptr.as_ref() {
        let events = engine.take_events();
        let json = serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string());
        CString::new(json).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

// QUIC Engine C wrappers
/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer for `client_id`
/// and returns a raw pointer to a new `QuicMqttEngineFFI`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_new(
    client_id: *const c_char,
    mqtt_version: u8,
) -> *mut QuicMqttEngineFFI {
    let client_id = if client_id.is_null() {
        "mqtt_client".to_string()
    } else {
        CStr::from_ptr(client_id).to_string_lossy().into_owned()
    };
    let opts = MqttOptionsFFI {
        client_id,
        mqtt_version,
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        reconnect_base_delay_ms: 1000,
        reconnect_max_delay_ms: 30000,
        max_reconnect_attempts: 0,
    };
    match QuicMqttEngineFFI::new(opts) {
        Ok(engine) => Box::into_raw(Box::new(engine)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
///
/// This function is unsafe because it performs manual memory deallocation of a `QuicMqttEngineFFI`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_free(ptr: *mut QuicMqttEngineFFI) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr`, `server_addr`,
/// `server_name`, and `tls_opts`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_connect(
    ptr: *mut QuicMqttEngineFFI,
    server_addr: *const c_char,
    server_name: *const c_char,
    tls_opts: *const MqttTlsOptionsC,
) -> i32 {
    if let (Some(engine), true, true) =
        (ptr.as_ref(), !server_addr.is_null(), !server_name.is_null())
    {
        let server_addr = CStr::from_ptr(server_addr).to_string_lossy().into_owned();
        let server_name = CStr::from_ptr(server_name).to_string_lossy().into_owned();

        let tls_opts_v = if tls_opts.is_null() {
            MqttTlsOptionsFFI {
                ca_cert_file: None,
                client_cert_file: None,
                client_key_file: None,
                insecure_skip_verify: false,
                alpn_protocols: vec!["mqtt".to_string()],
                enable_key_log: false,
            }
        } else {
            let r = &*tls_opts;
            let ca_cert_file = if r.ca_cert_file.is_null() {
                None
            } else {
                Some(
                    CStr::from_ptr(r.ca_cert_file)
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            let client_cert_file = if r.client_cert_file.is_null() {
                None
            } else {
                Some(
                    CStr::from_ptr(r.client_cert_file)
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            let client_key_file = if r.client_key_file.is_null() {
                None
            } else {
                Some(
                    CStr::from_ptr(r.client_key_file)
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            MqttTlsOptionsFFI {
                ca_cert_file,
                client_cert_file,
                client_key_file,
                insecure_skip_verify: r.insecure_skip_verify != 0,
                alpn_protocols: vec!["mqtt".to_string()],
                enable_key_log: r.enable_key_log != 0,
            }
        };

        match engine.connect(server_addr, server_name, tls_opts_v, engine.elapsed_ms()) {
            Ok(()) => 0,
            Err(error) => {
                engine.events.lock().unwrap().push(MqttEventFFI::Error {
                    message: error.to_string(),
                });
                -1
            }
        }
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr`, `data`, and `remote_addr`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_handle_datagram(
    ptr: *mut QuicMqttEngineFFI,
    data: *const u8,
    len: usize,
    remote_addr: *const c_char,
) {
    if let (Some(engine), true, true) = (ptr.as_ref(), !data.is_null(), !remote_addr.is_null()) {
        let buf = std::slice::from_raw_parts(data, len);
        let remote_addr = CStr::from_ptr(remote_addr).to_string_lossy().into_owned();
        if let Err(error) = engine.handle_datagram(buf.to_vec(), remote_addr, engine.elapsed_ms()) {
            engine.events.lock().unwrap().push(MqttEventFFI::Error {
                message: error.to_string(),
            });
        }
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `out_count`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_take_outgoing_datagrams(
    ptr: *mut QuicMqttEngineFFI,
    out_count: *mut usize,
) -> *mut MqttDatagramC {
    if let Some(engine) = ptr.as_ref() {
        let dgs = engine.take_outgoing_datagrams();
        if dgs.is_empty() {
            if !out_count.is_null() {
                *out_count = 0;
            }
            return std::ptr::null_mut();
        }

        let mut result = Vec::with_capacity(dgs.len());
        for dg in dgs {
            let addr = CString::new(dg.addr).unwrap().into_raw();
            let data_len = dg.data.len();
            let mut b = dg.data.into_boxed_slice();
            let data = b.as_mut_ptr();
            std::mem::forget(b);
            result.push(MqttDatagramC {
                addr,
                data,
                data_len,
            });
        }

        if !out_count.is_null() {
            *out_count = result.len();
        }
        let mut b = result.into_boxed_slice();
        let p = b.as_mut_ptr();
        std::mem::forget(b);
        p
    } else {
        std::ptr::null_mut()
    }
}

/// # Safety
///
/// This function is unsafe because it performs manual memory deallocation of a datagram slice.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_free_datagrams(ptr: *mut MqttDatagramC, count: usize) {
    if !ptr.is_null() {
        let slice = std::slice::from_raw_parts_mut(ptr, count);
        for dg in &mut *slice {
            if !dg.addr.is_null() {
                drop(CString::from_raw(dg.addr));
            }
            if !dg.data.is_null() {
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    dg.data,
                    dg.data_len,
                )));
            }
        }
        drop(Box::from_raw(slice));
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `QuicMqttEngineFFI`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_handle_tick(ptr: *mut QuicMqttEngineFFI, now_ms: u64) {
    if let Some(engine) = ptr.as_ref() {
        engine.handle_tick(now_ms);
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `QuicMqttEngineFFI`
/// and returns an allocated `c_char` pointer that must be freed using `mqtt_engine_free_string`.
#[no_mangle]
#[cfg(feature = "uniffi-bindings")]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_take_events(ptr: *mut QuicMqttEngineFFI) -> *mut c_char {
    if let Some(engine) = ptr.as_ref() {
        let events = engine.take_events();
        let json = serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string());
        CString::new(json).unwrap().into_raw()
    } else {
        std::ptr::null_mut()
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr`, `topic`, and `payload`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_publish(
    ptr: *mut QuicMqttEngineFFI,
    topic: *const c_char,
    payload: *const u8,
    payload_len: usize,
    qos: u8,
) -> i32 {
    if let (Some(engine), true, true) = (ptr.as_ref(), !topic.is_null(), !payload.is_null()) {
        let topic = CStr::from_ptr(topic).to_string_lossy().into_owned();
        let payload = std::slice::from_raw_parts(payload, payload_len).to_vec();
        engine.publish(topic, payload, qos)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `topic_filter`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_subscribe(
    ptr: *mut QuicMqttEngineFFI,
    topic_filter: *const c_char,
    qos: u8,
) -> i32 {
    if let (Some(engine), true) = (ptr.as_ref(), !topic_filter.is_null()) {
        let topic = CStr::from_ptr(topic_filter).to_string_lossy().into_owned();
        engine.subscribe(topic, qos)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `topic_filter`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_unsubscribe(
    ptr: *mut QuicMqttEngineFFI,
    topic_filter: *const c_char,
) -> i32 {
    if let (Some(engine), true) = (ptr.as_ref(), !topic_filter.is_null()) {
        let topic = CStr::from_ptr(topic_filter).to_string_lossy().into_owned();
        engine.unsubscribe(topic)
    } else {
        -1
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `QuicMqttEngineFFI`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_disconnect(ptr: *mut QuicMqttEngineFFI) {
    if let Some(engine) = ptr.as_ref() {
        if let Err(error) = engine.disconnect() {
            engine.events.lock().unwrap().push(MqttEventFFI::Error {
                message: error.to_string(),
            });
        }
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `QuicMqttEngineFFI`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_is_connected(ptr: *mut QuicMqttEngineFFI) -> i32 {
    if let Some(engine) = ptr.as_ref() {
        if engine.is_connected() {
            1
        } else {
            0
        }
    } else {
        0
    }
}

#[repr(C)]
pub struct MqttOptionsC {
    pub client_id: *const c_char,
    pub mqtt_version: u8,
    pub clean_start: bool,
    pub keep_alive: u16,
    pub username: *const c_char,
    pub password: *const c_char,
    pub reconnect_base_delay_ms: u64,
    pub reconnect_max_delay_ms: u64,
    pub max_reconnect_attempts: u32,
}

#[repr(C)]
pub struct MqttTlsOptionsC {
    pub ca_cert_file: *const c_char,
    pub client_cert_file: *const c_char,
    pub client_key_file: *const c_char,
    pub alpn: *const c_char,
    pub insecure_skip_verify: u8,
    pub enable_key_log: u8,
}

#[repr(C)]
pub struct MqttDatagramC {
    pub addr: *mut c_char,
    pub data: *mut u8,
    pub data_len: usize,
}

// Event Inspection API for C (Native Structs)

// Actually, let's just use a dedicated "C Event List" object to manage the lifetime.
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Object))]
pub struct MqttEventListFFI {
    events: Vec<MqttEventFFI>,
}

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl MqttEventListFFI {
    pub fn len(&self) -> u32 {
        self.events.len() as u32
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn get(&self, index: u32) -> Option<MqttEventFFI> {
        self.events.get(index as usize).cloned()
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEngineFFI`
/// and returns an allocated `MqttEventListFFI` pointer that must be freed with `mqtt_event_list_free`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_take_events_list(
    ptr: *mut MqttEngineFFI,
) -> *mut MqttEventListFFI {
    if let Some(engine) = ptr.as_ref() {
        let events = engine.take_events();
        Box::into_raw(Box::new(MqttEventListFFI { events }))
    } else {
        std::ptr::null_mut()
    }
}

/// # Safety
///
/// This function is unsafe because it performs manual memory deallocation of a `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_free(ptr: *mut MqttEventListFFI) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_len(ptr: *const MqttEventListFFI) -> usize {
    if let Some(list) = ptr.as_ref() {
        list.events.len()
    } else {
        0
    }
}

// I'll provide a way to get event details as raw types
/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_tag(ptr: *const MqttEventListFFI, index: usize) -> u8 {
    if let Some(list) = ptr.as_ref() {
        if let Some(event) = list.events.get(index) {
            match event {
                MqttEventFFI::Connected(_) => 1,
                MqttEventFFI::Disconnected { .. } => 2,
                MqttEventFFI::MessageReceived(_) => 3,
                MqttEventFFI::Published(_) => 4,
                MqttEventFFI::Subscribed(_) => 5,
                MqttEventFFI::Unsubscribed(_) => 6,
                MqttEventFFI::PingResponse { .. } => 7,
                MqttEventFFI::Error { .. } => 8,
                MqttEventFFI::ReconnectNeeded => 9,
                MqttEventFFI::ReconnectScheduled { .. } => 10,
                MqttEventFFI::StreamClosed { .. } => 11,
                MqttEventFFI::StreamReset { .. } => 12,
                MqttEventFFI::StreamStopped { .. } => 13,
                MqttEventFFI::AuthReceived(_) => 14,
                MqttEventFFI::PublishReceived { .. } => 15,
                MqttEventFFI::PubRelReceived { .. } => 16,
                MqttEventFFI::TransportClosed { .. } => 17,
                MqttEventFFI::ZeroRttStatusChanged { .. } => 18,
            }
        } else {
            0
        }
    } else {
        0
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_connected_rc(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> u8 {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::Connected(res)) = list.events.get(index) {
            res.reason_code
        } else {
            0
        }
    } else {
        0
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`
/// and returns an allocated `c_char` pointer that must be freed using `mqtt_engine_free_string`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_message_topic(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> *mut c_char {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::MessageReceived(msg)) = list.events.get(index) {
            return CString::new(msg.topic.clone()).unwrap().into_raw();
        }
    }
    std::ptr::null_mut()
}

/// # Safety
///
/// This function is unsafe because it dereferences raw pointers for `ptr` and `out_len`,
/// and returns an allocated `u8` pointer that must be freed with `mqtt_engine_free_bytes`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_message_payload(
    ptr: *const MqttEventListFFI,
    index: usize,
    out_len: *mut usize,
) -> *mut u8 {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::MessageReceived(msg)) = list.events.get(index) {
            if !out_len.is_null() {
                *out_len = msg.payload.len();
            }
            let mut b = msg.payload.clone().into_boxed_slice();
            let p = b.as_mut_ptr();
            std::mem::forget(b);
            return p;
        }
    }
    std::ptr::null_mut()
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_published_pid(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> i32 {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::Published(res)) = list.events.get(index) {
            return res.packet_id.map(|id| id as i32).unwrap_or(0);
        }
    }
    -1
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_subscribed_pid(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> i32 {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::Subscribed(res)) = list.events.get(index) {
            return res.packet_id as i32;
        }
    }
    -1
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`
/// and returns an allocated `c_char` pointer that must be freed using `mqtt_engine_free_string`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_error_message(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> *mut c_char {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::Error { message }) = list.events.get(index) {
            return CString::new(message.clone()).unwrap().into_raw();
        }
    }
    std::ptr::null_mut()
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_stream_id(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> u64 {
    if let Some(list) = ptr.as_ref() {
        match list.events.get(index) {
            Some(MqttEventFFI::StreamClosed { stream_id, .. })
            | Some(MqttEventFFI::StreamReset { stream_id, .. })
            | Some(MqttEventFFI::StreamStopped { stream_id, .. }) => *stream_id,
            _ => 0,
        }
    } else {
        0
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_stream_error_code(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> u64 {
    if let Some(list) = ptr.as_ref() {
        match list.events.get(index) {
            Some(MqttEventFFI::StreamReset { error_code, .. })
            | Some(MqttEventFFI::StreamStopped { error_code, .. }) => *error_code,
            _ => 0,
        }
    } else {
        0
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`
/// and returns an allocated `c_char` pointer that must be freed using `mqtt_engine_free_string`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_stream_close_reason(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> *mut c_char {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::StreamClosed { reason, .. }) = list.events.get(index) {
            return CString::new(reason.clone()).unwrap().into_raw();
        }
    }
    std::ptr::null_mut()
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `MqttEventListFFI`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_stream_closed_by_peer(
    ptr: *const MqttEventListFFI,
    index: usize,
) -> i32 {
    if let Some(list) = ptr.as_ref() {
        if let Some(MqttEventFFI::StreamClosed { by_peer, .. }) = list.events.get(index) {
            return i32::from(*by_peer);
        }
    }
    -1
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `QuicMqttEngineFFI`
/// and returns an allocated `MqttEventListFFI` pointer that must be freed with `mqtt_event_list_free`.
#[no_mangle]
#[cfg(feature = "quic")]
pub unsafe extern "C" fn mqtt_quic_engine_take_events_list(
    ptr: *mut QuicMqttEngineFFI,
) -> *mut MqttEventListFFI {
    if let Some(engine) = ptr.as_ref() {
        let events = engine.take_events();
        Box::into_raw(Box::new(MqttEventListFFI { events }))
    } else {
        std::ptr::null_mut()
    }
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer to `TlsMqttEngineFFI`
/// and returns an allocated `MqttEventListFFI` pointer that must be freed with `mqtt_event_list_free`.
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_take_events_list(
    ptr: *mut TlsMqttEngineFFI,
) -> *mut MqttEventListFFI {
    if let Some(engine) = ptr.as_ref() {
        let events = engine.take_events();
        Box::into_raw(Box::new(MqttEventListFFI { events }))
    } else {
        std::ptr::null_mut()
    }
}
