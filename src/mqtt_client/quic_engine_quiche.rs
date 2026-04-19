// SPDX-License-Identifier: MPL-2.0
//! Quiche-backed sans-I/O MQTT-over-QUIC engine.
//!
//! Uses Cloudflare's `quiche` library which can be linked against the system OpenSSL
//! (via `features = ["openssl"]`) rather than vendoring BoringSSL, keeping the
//! compiled library footprint small.
//!
//! The public API mirrors [`super::engine::QuicMqttEngine`] so FFI and higher-level
//! code can switch backends via a feature flag.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Instant;

use super::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use super::engine::{MqttEngine, MqttEvent};
use super::error::MqttClientError;
use super::opts::MqttClientOptions;

/// Maximum UDP datagram size we read/write (standard Ethernet MTU minus overhead).
const MAX_DATAGRAM_SIZE: usize = 1350;

/// The QUIC stream ID used for the MQTT control channel.
/// Client-initiated bidirectional streams start at 0 and increment by 4.
const MQTT_STREAM_ID: u64 = 0;

/// A "Sans-I/O" MQTT over QUIC engine backed by Cloudflare's `quiche` library.
///
/// Using quiche with the `openssl` feature flag allows dynamic linking against
/// the system's OpenSSL/libcrypto, avoiding the ~400 KB of vendored crypto code
/// that the default BoringSSL backend adds to the shared library.
///
/// # Usage
///
/// Identical to `QuicMqttEngine` — feed datagrams in, pull datagrams out.
pub struct QuicMqttEngineQuiche {
    mqtt_engine: MqttEngine,
    connection: Option<Box<quiche::Connection>>,
    local_addr: Option<SocketAddr>,
    peer_addr: Option<SocketAddr>,
    outgoing_datagrams: VecDeque<(SocketAddr, Vec<u8>)>,
    mqtt_stream_open: bool,
}

impl QuicMqttEngineQuiche {
    pub fn new(options: MqttClientOptions) -> Result<Self, MqttClientError> {
        Ok(Self {
            mqtt_engine: MqttEngine::new(options),
            connection: None,
            local_addr: None,
            peer_addr: None,
            outgoing_datagrams: VecDeque::new(),
            mqtt_stream_open: false,
        })
    }

    /// Initiate a QUIC connection to `server_addr`.
    ///
    /// # Parameters
    /// - `ca_cert_path`: path to a PEM CA certificate file; if `None` and
    ///   `insecure_skip_verify` is false, the system/default roots are used via
    ///   quiche's default verification.
    /// - `insecure_skip_verify`: disable peer certificate verification (for
    ///   testing only).
    /// - `alpn`: ALPN protocol IDs; defaults to `["mqtt"]`.
    pub fn connect(
        &mut self,
        server_addr: SocketAddr,
        server_name: &str,
        ca_cert_path: Option<&str>,
        insecure_skip_verify: bool,
        alpn: Vec<Vec<u8>>,
    ) -> Result<(), MqttClientError> {
        let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).map_err(|e| {
            MqttClientError::InternalError {
                message: format!("quiche Config::new failed: {e}"),
            }
        })?;

        // ALPN
        let alpn_refs: Vec<&[u8]> = if alpn.is_empty() {
            vec![b"mqtt"]
        } else {
            alpn.iter().map(|v| v.as_slice()).collect()
        };
        config
            .set_application_protos(&alpn_refs)
            .map_err(|e| MqttClientError::InternalError {
                message: format!("quiche set_application_protos failed: {e}"),
            })?;

        // TLS verification
        config.verify_peer(!insecure_skip_verify);
        if let Some(path) = ca_cert_path {
            config.load_verify_locations_from_file(path).map_err(|e| {
                MqttClientError::InternalError {
                    message: format!("quiche load_verify_locations_from_file failed: {e}"),
                }
            })?;
        }

        // Transport parameters — generous limits for MQTT workloads
        config.set_max_idle_timeout(120_000);
        config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
        config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
        config.set_initial_max_data(10_000_000);
        config.set_initial_max_stream_data_bidi_local(1_024_000);
        config.set_initial_max_stream_data_bidi_remote(1_024_000);
        config.set_initial_max_streams_bidi(100);
        config.set_disable_active_migration(true);

        // Pick a random source connection ID
        let mut scid_bytes = [0u8; quiche::MAX_CONN_ID_LEN];
        for b in &mut scid_bytes {
            *b = rand_byte();
        }
        let scid = quiche::ConnectionId::from_ref(&scid_bytes);

        // Use an unspecified local address — the actual UDP socket is managed externally.
        let local_addr: SocketAddr = if server_addr.is_ipv6() {
            "[::]:0".parse().unwrap()
        } else {
            "0.0.0.0:0".parse().unwrap()
        };

        let conn = quiche::connect(
            Some(server_name),
            &scid,
            local_addr,
            server_addr,
            &mut config,
        )
        .map_err(|e| MqttClientError::InternalError {
            message: format!("quiche::connect failed: {e}"),
        })?;

        self.connection = Some(Box::new(conn));
        self.local_addr = Some(local_addr);
        self.peer_addr = Some(server_addr);

        // Drain the initial handshake packets into outgoing_datagrams so the
        // caller can send them on the first poll.
        self.drain_outgoing();

        Ok(())
    }

    /// Feed an incoming UDP datagram.
    pub fn handle_datagram(&mut self, mut data: Vec<u8>, remote_addr: SocketAddr, _now: Instant) {
        let local = self
            .local_addr
            .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());
        if let Some(conn) = &mut self.connection {
            let recv_info = quiche::RecvInfo {
                from: remote_addr,
                to: local,
            };
            // Ignore receive errors (e.g. Unknown Connection ID during handshake)
            let _ = conn.recv(&mut data, recv_info);
        }
    }

    /// Drive the state machine, produce MQTT events, and queue outgoing datagrams.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        let mut mqtt_events = Vec::new();

        if let Some(conn) = &mut self.connection {
            // Drive QUIC timeouts
            conn.on_timeout();

            // Check handshake completion
            if conn.is_established() && !self.mqtt_stream_open {
                self.mqtt_stream_open = true;
                // Signal MQTT engine to send CONNECT
                self.mqtt_engine.connect();
            }

            // Closed connection
            if conn.is_closed() {
                self.mqtt_engine.handle_connection_lost();
                mqtt_events.push(MqttEvent::Disconnected(None));
            }

            // Transfer QUIC stream data → MQTT engine
            // We only run this after the stream has been opened
            if self.mqtt_stream_open {
                let mut buf = vec![0u8; 65536];
                loop {
                    match conn.stream_recv(MQTT_STREAM_ID, &mut buf) {
                        Ok((len, _fin)) => {
                            mqtt_events.extend(self.mqtt_engine.handle_incoming(&buf[..len]));
                        }
                        Err(quiche::Error::Done) => break,
                        Err(_) => break,
                    }
                }

                // Transfer MQTT engine outgoing → QUIC stream
                let outgoing = self.mqtt_engine.take_outgoing();
                if !outgoing.is_empty() {
                    let _ = conn.stream_send(MQTT_STREAM_ID, &outgoing, false);
                }
            }
        }

        // Drive MQTT engine tick
        let tick_events = self.mqtt_engine.handle_tick(now);
        mqtt_events.extend(tick_events);

        // Drain any newly produced outgoing QUIC datagrams
        self.drain_outgoing();

        mqtt_events
    }

    /// Drain all pending outgoing datagrams from the quiche connection.
    fn drain_outgoing(&mut self) {
        let peer = match self.peer_addr {
            Some(a) => a,
            None => return,
        };
        let conn = match &mut self.connection {
            Some(c) => c,
            None => return,
        };
        let mut out = vec![0u8; MAX_DATAGRAM_SIZE];
        loop {
            match conn.send(&mut out) {
                Ok((len, _send_info)) => {
                    self.outgoing_datagrams
                        .push_back((peer, out[..len].to_vec()));
                }
                Err(quiche::Error::Done) => break,
                Err(_) => break,
            }
        }
    }

    pub fn take_outgoing_datagrams(&mut self) -> VecDeque<(SocketAddr, Vec<u8>)> {
        std::mem::take(&mut self.outgoing_datagrams)
    }

    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        self.mqtt_engine.take_events()
    }

    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.mqtt_engine.publish(command)
    }

    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.mqtt_engine.subscribe(command)
    }

    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.mqtt_engine.unsubscribe(command)
    }

    pub fn disconnect(&mut self) {
        self.mqtt_engine.disconnect();
    }

    pub fn is_connected(&self) -> bool {
        self.mqtt_engine.is_connected()
    }
}

/// Cheap non-cryptographic random byte for connection ID generation.
/// Real deployments should use `rand`; this avoids adding a dependency.
fn rand_byte() -> u8 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;
    let mut h = DefaultHasher::new();
    SystemTime::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    h.finish() as u8
}
