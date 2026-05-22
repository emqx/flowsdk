// SPDX-License-Identifier: MPL-2.0

use flowsdk::mqtt_client::{MqttClientOptions, MqttEvent, NoIoMqttClient, PublishCommand};
#[cfg(feature = "quic")]
use flowsdk::mqtt_client::engine::QuicMqttEngine;
use std::collections::VecDeque;
#[cfg(feature = "quic")]
use std::net::SocketAddr;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use crate::config::BenchConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    TcpConnecting,
    MqttConnecting,
    Publishing,
    Draining,
    Disconnecting,
    Done,
    Failed,
}

pub struct Connection {
    pub fd: RawFd,
    pub mqtt: NoIoMqttClient,
    pub state: ConnState,
    pub send_buf: Vec<u8>,
    pub send_offset: usize,
    pub recv_buf: Vec<u8>,
    pub messages_sent: u64,
    pub messages_acked: u64,
    pub messages_target: u64,
    pub latency_pending: VecDeque<Instant>,
    pub latency_samples: Vec<Duration>,
    pub next_publish_at: Option<Instant>,
    pub recv_pending: bool,
    pub send_pending: bool,
    pub client_index: usize,
    pub drain_deadline: Option<Instant>,
}

impl Connection {
    pub fn new(fd: RawFd, client_index: usize, config: &BenchConfig) -> Self {
        let peer = format!("{}:{}", config.host, config.port);
        let client_id = format!("mqtt_ring_bench_{}", client_index);
        let options = MqttClientOptions::builder()
            .peer(&peer)
            .client_id(&client_id)
            .keep_alive(config.keep_alive)
            .mqtt_version(config.mqtt_version)
            .clean_start(true)
            .reconnect(false)
            .auto_ack(true)
            .parser_buffer_size(config.parser_buf)
            .max_outgoing_packet_count(config.messages.min(10_000) as usize)
            .max_event_count(1000)
            .build();

        Self {
            fd,
            mqtt: NoIoMqttClient::new(options),
            state: ConnState::TcpConnecting,
            send_buf: Vec::new(),
            send_offset: 0,
            recv_buf: vec![0u8; config.parser_buf.max(1500)],
            messages_sent: 0,
            messages_acked: 0,
            messages_target: config.messages,
            latency_pending: VecDeque::new(),
            latency_samples: Vec::new(),
            next_publish_at: None,
            recv_pending: false,
            send_pending: false,
            client_index,
            drain_deadline: None,
        }
    }

    pub fn initiate_mqtt_connect(&mut self) {
        self.mqtt.connect();
        self.take_outgoing();
        self.state = ConnState::MqttConnecting;
    }

    pub fn handle_incoming(&mut self, data: &[u8]) -> Vec<MqttEvent> {
        self.mqtt.handle_incoming(data)
    }

    pub fn try_publish(&mut self, config: &BenchConfig) -> bool {
        if self.state != ConnState::Publishing {
            return false;
        }
        if self.messages_sent >= self.messages_target {
            return false;
        }
        if self.send_offset < self.send_buf.len() {
            return false;
        }
        if let Some(next) = self.next_publish_at {
            if Instant::now() < next {
                return false;
            }
        }

        let topic = format!("{}/{}", config.topic, self.client_index);
        let payload = vec![0x42u8; config.payload_size];
        let cmd = PublishCommand::builder()
            .topic(&topic)
            .payload(payload)
            .qos(config.qos)
            .build();
        let cmd = match cmd {
            Ok(c) => c,
            Err(_) => return false,
        };

        if config.qos > 0 {
            self.latency_pending.push_back(Instant::now());
        }

        match self.mqtt.publish(cmd) {
            Ok(_) => {
                self.messages_sent += 1;
                self.take_outgoing();
                if config.interval_ms > 0 {
                    self.next_publish_at =
                        Some(Instant::now() + Duration::from_millis(config.interval_ms));
                }
                true
            }
            Err(_) => {
                if config.qos > 0 {
                    self.latency_pending.pop_back();
                }
                false
            }
        }
    }

    pub fn process_events(&mut self, events: Vec<MqttEvent>) -> EventOutcome {
        let mut outcome = EventOutcome::default();
        for event in events {
            match event {
                MqttEvent::Connected(result) => {
                    if result.is_success() {
                        self.state = ConnState::Publishing;
                        outcome.connected = true;
                    } else {
                        self.state = ConnState::Failed;
                        outcome.error = true;
                    }
                }
                MqttEvent::Published(result) => {
                    if result.is_success() {
                        self.messages_acked += 1;
                        outcome.acked += 1;
                        if let Some(sent_at) = self.latency_pending.pop_front() {
                            self.latency_samples.push(sent_at.elapsed());
                        }
                    } else {
                        outcome.error = true;
                    }
                }
                MqttEvent::Error(_) => {
                    outcome.error = true;
                }
                MqttEvent::Disconnected(_) | MqttEvent::ReconnectNeeded => {
                    if self.state != ConnState::Disconnecting && self.state != ConnState::Done {
                        self.state = ConnState::Failed;
                        outcome.error = true;
                    }
                }
                _ => {}
            }
        }
        outcome
    }

    pub fn check_publish_complete(&mut self) {
        if self.state == ConnState::Publishing && self.messages_sent >= self.messages_target {
            self.state = ConnState::Draining;
            self.drain_deadline = Some(Instant::now() + Duration::from_secs(10));
        }
    }

    pub fn check_drain_complete(&mut self) -> bool {
        if self.state != ConnState::Draining {
            return false;
        }
        let drained = self.messages_acked >= self.messages_sent
            || self
                .drain_deadline
                .map_or(false, |d| Instant::now() >= d);
        if drained {
            self.mqtt.disconnect();
            self.take_outgoing();
            self.state = ConnState::Disconnecting;
        }
        drained
    }

    pub fn handle_tick(&mut self) -> Vec<MqttEvent> {
        let now = Instant::now();
        if let Some(next) = self.mqtt.next_tick_at() {
            if now >= next {
                let events = self.mqtt.handle_tick(now);
                self.take_outgoing();
                return events;
            }
        }
        Vec::new()
    }

    pub fn take_outgoing(&mut self) {
        let out = self.mqtt.take_outgoing();
        if !out.is_empty() {
            if self.send_offset >= self.send_buf.len() {
                self.send_buf = out;
                self.send_offset = 0;
            } else {
                self.send_buf.extend_from_slice(&out);
            }
        }
    }

    pub fn has_pending_send(&self) -> bool {
        self.send_offset < self.send_buf.len()
    }

    pub fn pending_send_slice(&self) -> &[u8] {
        &self.send_buf[self.send_offset..]
    }

    pub fn advance_send(&mut self, n: usize) {
        self.send_offset += n;
        if self.send_offset >= self.send_buf.len() {
            self.send_buf.clear();
            self.send_offset = 0;
        }
    }
}

#[derive(Default)]
pub struct EventOutcome {
    pub connected: bool,
    pub acked: u64,
    pub error: bool,
}

// ---------------------------------------------------------------------------
// QUIC Connection (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "quic")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuicConnState {
    /// Initial QUIC handshake datagrams being exchanged.
    Handshaking,
    /// MQTT PUBLISH phase.
    Publishing,
    /// All publishes sent; waiting for remaining ACKs.
    Draining,
    /// DISCONNECT sent; finishing up.
    Disconnecting,
    Done,
    Failed,
}

#[cfg(feature = "quic")]
pub struct QuicConnection {
    pub fd: RawFd,
    pub engine: QuicMqttEngine,
    pub state: QuicConnState,
    pub messages_sent: u64,
    pub messages_acked: u64,
    pub messages_target: u64,
    pub latency_pending: VecDeque<Instant>,
    pub latency_samples: Vec<Duration>,
    pub next_publish_at: Option<Instant>,
    pub recv_buf: Vec<u8>,
    pub recv_pending: bool,
    /// Queue of outgoing UDP datagrams waiting to be sent.
    pub send_queue: VecDeque<Vec<u8>>,
    /// True while an io_uring Send SQE is in-flight for this connection.
    pub send_pending: bool,
    pub client_index: usize,
    pub drain_deadline: Option<Instant>,
    pub server_addr: SocketAddr,
}

#[cfg(feature = "quic")]
impl QuicConnection {
    pub fn new(
        fd: RawFd,
        client_index: usize,
        config: &BenchConfig,
        server_addr: SocketAddr,
    ) -> Result<Self, String> {
        let peer = format!("{}:{}", config.host, config.port);
        let client_id = format!("mqtt_ring_bench_{}", client_index);
        let options = MqttClientOptions::builder()
            .peer(&peer)
            .client_id(&client_id)
            .keep_alive(config.keep_alive)
            .mqtt_version(config.mqtt_version)
            .clean_start(true)
            .reconnect(false)
            .auto_ack(true)
            .parser_buffer_size(config.parser_buf)
            .max_outgoing_packet_count(config.messages.min(10_000) as usize)
            .max_event_count(1000)
            .build();

        let engine =
            QuicMqttEngine::new(options).map_err(|e| format!("QuicMqttEngine::new: {}", e))?;

        Ok(Self {
            fd,
            engine,
            state: QuicConnState::Handshaking,
            messages_sent: 0,
            messages_acked: 0,
            messages_target: config.messages,
            latency_pending: VecDeque::new(),
            latency_samples: Vec::new(),
            next_publish_at: None,
            recv_buf: vec![0u8; 2048], // UDP datagrams fit in ~1.5KB
            recv_pending: false,
            send_queue: VecDeque::new(),
            send_pending: false,
            client_index,
            drain_deadline: None,
            server_addr,
        })
    }

    /// Start the QUIC handshake. Must be called after socket creation.
    /// Takes the TLS crypto config and produces initial handshake datagrams.
    pub fn initiate_quic_connect(
        &mut self,
        crypto: rustls::ClientConfig,
        server_name: &str,
        now: Instant,
    ) -> Result<(), String> {
        self.engine
            .connect(self.server_addr, server_name, crypto, now)
            .map_err(|e| format!("QUIC connect: {}", e))?;
        // Drive the engine once to produce the Initial packet.
        let _ = self.engine.handle_tick(now);
        self.drain_outgoing_datagrams();
        Ok(())
    }

    /// Move outgoing datagrams from the engine into our send_queue.
    pub fn drain_outgoing_datagrams(&mut self) {
        let datagrams = self.engine.take_outgoing_datagrams();
        for (_dest, data) in datagrams {
            self.send_queue.push_back(data);
        }
    }

    /// Feed a received UDP datagram into the QUIC engine.
    pub fn handle_datagram(&mut self, data: &[u8], now: Instant) -> Vec<MqttEvent> {
        self.engine
            .handle_datagram(data.to_vec(), self.server_addr, now);
        let events = self.engine.handle_tick(now);
        self.drain_outgoing_datagrams();
        events
    }

    /// Drive the QUIC + MQTT state machines forward.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        let events = self.engine.handle_tick(now);
        self.drain_outgoing_datagrams();
        events
    }

    pub fn try_publish(&mut self, config: &BenchConfig, now: Instant) -> bool {
        if self.state != QuicConnState::Publishing {
            return false;
        }
        if self.messages_sent >= self.messages_target {
            return false;
        }
        if let Some(next) = self.next_publish_at {
            if now < next {
                return false;
            }
        }

        let topic = format!("{}/{}", config.topic, self.client_index);
        let payload = vec![0x42u8; config.payload_size];
        let cmd = PublishCommand::builder()
            .topic(&topic)
            .payload(payload)
            .qos(config.qos)
            .build();
        let cmd = match cmd {
            Ok(c) => c,
            Err(_) => return false,
        };

        if config.qos > 0 {
            self.latency_pending.push_back(now);
        }

        match self.engine.publish(cmd) {
            Ok(_) => {
                self.messages_sent += 1;
                // Drive engine to push MQTT data into QUIC stream and produce datagrams.
                let _ = self.engine.handle_tick(now);
                self.drain_outgoing_datagrams();
                if config.interval_ms > 0 {
                    self.next_publish_at =
                        Some(now + Duration::from_millis(config.interval_ms));
                }
                true
            }
            Err(_) => {
                if config.qos > 0 {
                    self.latency_pending.pop_back();
                }
                false
            }
        }
    }

    pub fn process_events(&mut self, events: Vec<MqttEvent>) -> EventOutcome {
        let mut outcome = EventOutcome::default();
        for event in events {
            match event {
                MqttEvent::Connected(result) => {
                    if result.is_success() {
                        self.state = QuicConnState::Publishing;
                        outcome.connected = true;
                    } else {
                        self.state = QuicConnState::Failed;
                        outcome.error = true;
                    }
                }
                MqttEvent::Published(result) => {
                    if result.is_success() {
                        self.messages_acked += 1;
                        outcome.acked += 1;
                        if let Some(sent_at) = self.latency_pending.pop_front() {
                            self.latency_samples.push(sent_at.elapsed());
                        }
                    } else {
                        outcome.error = true;
                    }
                }
                MqttEvent::Error(_) => {
                    outcome.error = true;
                }
                MqttEvent::Disconnected(_) | MqttEvent::ReconnectNeeded => {
                    if self.state != QuicConnState::Disconnecting
                        && self.state != QuicConnState::Done
                    {
                        self.state = QuicConnState::Failed;
                        outcome.error = true;
                    }
                }
                _ => {}
            }
        }
        outcome
    }

    pub fn check_publish_complete(&mut self) {
        if self.state == QuicConnState::Publishing
            && self.messages_sent >= self.messages_target
        {
            self.state = QuicConnState::Draining;
            self.drain_deadline = Some(Instant::now() + Duration::from_secs(10));
        }
    }

    pub fn check_drain_complete(&mut self, now: Instant) -> bool {
        if self.state != QuicConnState::Draining {
            return false;
        }
        let drained = self.messages_acked >= self.messages_sent
            || self.drain_deadline.map_or(false, |d| now >= d);
        if drained {
            self.engine.disconnect();
            let _ = self.engine.handle_tick(now);
            self.drain_outgoing_datagrams();
            self.state = QuicConnState::Disconnecting;
        }
        drained
    }

    pub fn has_pending_send(&self) -> bool {
        !self.send_queue.is_empty()
    }

    /// Get the front datagram to send. Returns None if queue is empty.
    pub fn front_send_datagram(&self) -> Option<&[u8]> {
        self.send_queue.front().map(|v| v.as_slice())
    }

    /// Pop the front datagram after a successful send.
    pub fn pop_front_datagram(&mut self) {
        self.send_queue.pop_front();
    }
}
