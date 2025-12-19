use crate::mqtt_client::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use crate::mqtt_client::engine::{MqttEngine, MqttEvent};
use crate::mqtt_client::error::MqttClientError;
use crate::mqtt_client::opts::MqttClientOptions;
use std::time::Instant;

/// A pure protocol MQTT client without any I/O.
/// Users are responsible for feeding bytes and taking outgoing bytes.
pub struct NoIoMqttClient {
    engine: MqttEngine,
}

impl NoIoMqttClient {
    /// Create a new NoIoMqttClient with the given options.
    pub fn new(options: MqttClientOptions) -> Self {
        Self {
            engine: MqttEngine::new(options),
        }
    }

    /// Process incoming bytes from the network.
    /// Returns a list of MQTT events generated.
    pub fn handle_incoming(&mut self, data: &[u8]) -> Vec<MqttEvent> {
        self.engine.handle_incoming(data)
    }

    /// Process protocol timer ticks (keep-alive, retransmissions).
    /// Returns a list of MQTT events generated.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        self.engine.handle_tick(now)
    }

    /// Take all pending events from the client.
    pub fn take_events(&mut self) -> Vec<MqttEvent> {
        self.engine.take_events()
    }

    /// Take outgoing bytes to be written to the network.
    pub fn take_outgoing(&mut self) -> Vec<u8> {
        self.engine.take_outgoing()
    }

    /// Get the next time when a protocol tick is expected.
    pub fn next_tick_at(&self) -> Option<Instant> {
        self.engine.next_tick_at()
    }

    /// Initiate a connection (enqueues CONNECT packet).
    pub fn connect(&mut self) {
        self.engine.connect();
    }

    /// Publish a message.
    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.engine.publish(command)
    }

    /// Subscribe to topics.
    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.engine.subscribe(command)
    }

    /// Unsubscribe from topics.
    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.engine.unsubscribe(command)
    }

    /// Send a ping.
    pub fn ping(&mut self) {
        self.engine.send_ping();
    }

    /// Send a disconnect.
    pub fn disconnect(&mut self) {
        self.engine.disconnect();
    }

    /// Check if the client is logically connected.
    pub fn is_connected(&self) -> bool {
        self.engine.is_connected()
    }

    /// Get the client options.
    pub fn options(&self) -> &MqttClientOptions {
        self.engine.options()
    }

    /// Handle connection lost state.
    pub fn handle_connection_lost(&mut self) {
        self.engine.handle_connection_lost();
    }
}
