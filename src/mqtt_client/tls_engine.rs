// SPDX-License-Identifier: MPL-2.0

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Instant;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection};

use super::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use super::engine::{MqttEngine, MqttEvent};
use super::error::MqttClientError;
use super::opts::MqttClientOptions;

/// A "Sans-I/O" MQTT over TLS protocol engine.
///
/// This engine combines the `MqttEngine` (MQTT state machine) with `rustls` (TLS state machine)
/// to provide a complete MQTT-over-TLS implementation that does not perform any direct I/O.
pub struct TlsMqttEngine {
    mqtt_engine: MqttEngine,
    tls_connection: ClientConnection,
    tls_config: Arc<ClientConfig>,
    server_name: ServerName<'static>,

    // Buffer for plaintext data from TLS engine to be fed into MQTT engine
    incoming_plaintext: Vec<u8>,
    outgoing_plaintext: Vec<u8>,
}

impl TlsMqttEngine {
    pub fn new(
        options: MqttClientOptions,
        server_name: &str,
        config: Arc<ClientConfig>,
    ) -> Result<Self, MqttClientError> {
        let mqtt_engine = MqttEngine::new(options);

        let server_name = ServerName::try_from(server_name)
            .map_err(|e| MqttClientError::InternalError {
                message: format!("Invalid server name: {}", e),
            })?
            .to_owned();

        let tls_connection =
            ClientConnection::new(config.clone(), server_name.clone()).map_err(|e| {
                MqttClientError::InternalError {
                    message: format!("Failed to create TLS connection: {}", e),
                }
            })?;

        Ok(Self {
            mqtt_engine,
            tls_connection,
            tls_config: config,
            server_name,
            incoming_plaintext: Vec::new(),
            outgoing_plaintext: Vec::new(),
        })
    }

    /// Start a fresh TLS handshake while retaining the MQTT session/inflight state.
    pub fn reset_for_new_transport(&mut self) -> Result<(), MqttClientError> {
        self.tls_connection =
            ClientConnection::new(self.tls_config.clone(), self.server_name.clone()).map_err(
                |e| MqttClientError::InternalError {
                    message: e.to_string(),
                },
            )?;
        self.incoming_plaintext.clear();
        self.outgoing_plaintext.clear();
        self.mqtt_engine.reset_for_new_transport();
        Ok(())
    }

    pub fn handle_connection_lost(&mut self) {
        self.mqtt_engine.handle_connection_lost();
    }

    /// Feed encrypted data received from the socket into the TLS engine.
    pub fn handle_socket_data(&mut self, mut data: &[u8]) -> Result<(), MqttClientError> {
        while !data.is_empty() {
            let n = self.tls_connection.read_tls(&mut data).map_err(|e| {
                MqttClientError::InternalError {
                    message: format!("TLS read error: {}", e),
                }
            })?;
            if n == 0 {
                break;
            }

            self.tls_connection.process_new_packets().map_err(|e| {
                MqttClientError::InternalError {
                    message: format!("TLS process error: {}", e),
                }
            })?;
            self.drain_plaintext()?;
        }
        Ok(())
    }

    fn drain_plaintext(&mut self) -> Result<(), MqttClientError> {
        let mut buf = vec![0u8; 4096];
        loop {
            match self.tls_connection.reader().read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let events = self.mqtt_engine.handle_incoming(&buf[..n]);
                    self.mqtt_engine.defer_events(events);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    return Err(MqttClientError::InternalError {
                        message: format!("Plaintext read error: {}", e),
                    })
                }
            }
        }

        Ok(())
    }

    /// Take encrypted data from the TLS engine to be sent to the socket.
    pub fn take_socket_data(&mut self) -> Vec<u8> {
        let mut buf = Vec::new();
        let _ = self.tls_connection.write_tls(&mut buf);
        buf
    }

    /// Drive both engines.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        let mut mqtt_events = Vec::new();

        // 1. Process internal plaintext from TLS -> MQTT
        mqtt_events.extend(self.mqtt_engine.handle_incoming(&self.incoming_plaintext));
        self.incoming_plaintext.clear();

        mqtt_events.extend(self.mqtt_engine.handle_tick(now));

        // 2. Process outgoing plaintext from MQTT -> TLS
        if self.outgoing_plaintext.is_empty() {
            self.outgoing_plaintext = self.mqtt_engine.take_outgoing();
        }
        if !self.outgoing_plaintext.is_empty() {
            match self.tls_connection.writer().write(&self.outgoing_plaintext) {
                Ok(written) => {
                    self.outgoing_plaintext.drain(..written);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => mqtt_events.push(MqttEvent::Error(MqttClientError::InternalError {
                    message: format!("TLS plaintext write failed: {error}"),
                })),
            }
        }

        mqtt_events.extend(self.mqtt_engine.take_events());

        mqtt_events
    }

    /// Initiate the MQTT connection (sends CONNECT packet).
    /// This should be called after the TLS handshake is potentially complete,
    /// or just to kick off the MQTT level.
    pub fn connect(&mut self) -> Result<(), MqttClientError> {
        self.mqtt_engine.connect()
    }

    pub fn engine(&self) -> &MqttEngine {
        &self.mqtt_engine
    }

    pub fn engine_mut(&mut self) -> &mut MqttEngine {
        &mut self.mqtt_engine
    }

    // Delegation methods
    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.mqtt_engine.publish(command)
    }

    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.mqtt_engine.subscribe(command)
    }

    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.mqtt_engine.unsubscribe(command)
    }

    pub fn disconnect(&mut self) -> Result<(), MqttClientError> {
        self.mqtt_engine.disconnect()
    }

    pub fn try_disconnect(&mut self) -> Result<(), MqttClientError> {
        self.mqtt_engine.try_disconnect()
    }

    pub fn try_disconnect_with(
        &mut self,
        reason_code: u8,
        properties: Vec<crate::mqtt_serde::mqttv5::common::properties::Property>,
    ) -> Result<(), MqttClientError> {
        self.mqtt_engine
            .try_disconnect_with(reason_code, properties)
    }

    /// All queued MQTT plaintext and encrypted TLS records have been drained.
    pub fn disconnect_complete(&self) -> bool {
        !self.is_connected()
            && self.outgoing_plaintext.is_empty()
            && !self.mqtt_engine.has_pending_output()
            && !self.tls_connection.wants_write()
    }

    pub fn is_connected(&self) -> bool {
        self.mqtt_engine.is_connected()
    }

    pub fn mqtt_version(&self) -> u8 {
        self.mqtt_engine.mqtt_version()
    }

    pub fn try_send_ping(&mut self) -> Result<(), MqttClientError> {
        self.mqtt_engine.try_send_ping()
    }

    pub fn try_auth(
        &mut self,
        reason_code: u8,
        properties: Vec<crate::mqtt_serde::mqttv5::common::properties::Property>,
    ) -> Result<(), MqttClientError> {
        self.mqtt_engine.try_auth(reason_code, properties)
    }

    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        self.mqtt_engine.take_events()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::RootCertStore;

    #[test]
    fn test_tls_engine_creation() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let options = MqttClientOptions::builder()
            .client_id("test_client")
            .build();

        let mut roots = RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().expect("could not load platform certs")
        {
            roots.add(cert).unwrap();
        }

        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        let engine = TlsMqttEngine::new(options, "localhost", Arc::new(config));
        assert!(engine.is_ok());

        let engine = engine.unwrap();
        assert!(!engine.is_connected());
    }
}
