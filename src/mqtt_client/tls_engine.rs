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

    // Buffer for plaintext data from TLS engine to be fed into MQTT engine
    incoming_plaintext: Vec<u8>,
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

        let tls_connection = ClientConnection::new(config, server_name).map_err(|e| {
            MqttClientError::InternalError {
                message: format!("Failed to create TLS connection: {}", e),
            }
        })?;

        Ok(Self {
            mqtt_engine,
            tls_connection,
            incoming_plaintext: Vec::new(),
        })
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
        }

        // After processing packets, check if there is any plaintext available
        let mut buf = vec![0u8; 4096];
        loop {
            match self.tls_connection.reader().read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.incoming_plaintext.extend_from_slice(&buf[..n]);
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
        if !self.incoming_plaintext.is_empty() {
            let events = self.mqtt_engine.handle_incoming(&self.incoming_plaintext);
            mqtt_events.extend(events);
            self.incoming_plaintext.clear();
        }

        // 2. Process outgoing plaintext from MQTT -> TLS
        let outgoing = self.mqtt_engine.take_outgoing();
        if !outgoing.is_empty() {
            let _ = self.tls_connection.writer().write_all(&outgoing);
        }

        // 3. Drive MQTT engine tick
        mqtt_events.extend(self.mqtt_engine.handle_tick(now));

        mqtt_events
    }

    /// Initiate the MQTT connection (sends CONNECT packet).
    /// This should be called after the TLS handshake is potentially complete,
    /// or just to kick off the MQTT level.
    pub fn connect(&mut self) {
        self.mqtt_engine.connect();
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

    pub fn disconnect(&mut self) {
        self.mqtt_engine.disconnect();
    }

    pub fn is_connected(&self) -> bool {
        self.mqtt_engine.is_connected()
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
