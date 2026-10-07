// SPDX-License-Identifier: MPL-2.0

use super::*;

impl MqttEngine<Instant> {
    pub fn new(options: MqttClientOptions) -> Self {
        Self::new_at(options, Instant::now())
    }

    fn host_now(&self) -> Instant {
        Instant::now().max(self.now)
    }
    pub fn handle_connection_lost(&mut self) {
        if let Err(error) = self.handle_connection_lost_at(self.host_now()) {
            self.events.push(MqttEvent::Error(error));
        }
    }

    pub fn reset_for_new_transport(&mut self) {
        if let Err(error) = self.reset_for_new_transport_at(self.host_now()) {
            self.events.push(MqttEvent::Error(error));
        }
    }

    /// Schedule from at least the latest accepted time, tolerating a stale host sample.
    pub fn schedule_reconnect(&mut self, now: Instant) {
        if let Err(error) = self.schedule_reconnect_at(now.max(self.now)) {
            self.events.push(MqttEvent::Error(error));
        }
    }

    pub fn handle_incoming(&mut self, data: &[u8]) -> Vec<MqttEvent> {
        self.handle_incoming_at(data, self.host_now())
            .unwrap_or_else(|error| vec![MqttEvent::Error(error)])
    }

    /// Drive timers without moving time backwards when host clock samples are stale.
    ///
    /// Convenience calls may have already read a later clock value, and FFI
    /// callers may round ticks to milliseconds. Use `handle_tick_at` when clock
    /// regression should instead be reported as an error.
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        self.handle_tick_at(now.max(self.now))
            .unwrap_or_else(|error| vec![MqttEvent::Error(error)])
    }

    pub fn take_outgoing(&mut self) -> Vec<u8> {
        match self.take_outgoing_at(self.host_now()) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.events.push(MqttEvent::Error(error));
                Vec::new()
            }
        }
    }

    pub fn connect(&mut self) -> Result<(), MqttClientError> {
        self.connect_at(self.host_now())
    }

    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.publish_at(command, self.host_now())
    }

    pub fn publish_encoded(
        &mut self,
        command: PublishCommand,
        stream: Option<u64>,
    ) -> Result<(Option<u16>, Vec<u8>), MqttClientError> {
        self.publish_encoded_at(command, stream, self.host_now())
    }

    pub fn ingest_stream_packet(
        &mut self,
        packet: MqttPacket,
        stream: u64,
    ) -> (Vec<MqttEvent>, Vec<u8>) {
        self.ingest_stream_packet_at(packet, stream, self.host_now())
            .unwrap_or_else(|error| (vec![MqttEvent::Error(error)], Vec::new()))
    }

    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.subscribe_at(command, self.host_now())
    }

    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.unsubscribe_at(command, self.host_now())
    }

    pub fn subscribe_encoded(
        &mut self,
        command: SubscribeCommand,
        stream: Option<u64>,
    ) -> Result<(u16, Vec<u8>), MqttClientError> {
        self.subscribe_encoded_at(command, stream, self.host_now())
    }

    pub fn unsubscribe_encoded(
        &mut self,
        command: UnsubscribeCommand,
        stream: Option<u64>,
    ) -> Result<(u16, Vec<u8>), MqttClientError> {
        self.unsubscribe_encoded_at(command, stream, self.host_now())
    }

    pub fn disconnect(&mut self) -> Result<(), MqttClientError> {
        self.disconnect_at(self.host_now())
    }

    pub fn try_disconnect(&mut self) -> Result<(), MqttClientError> {
        self.try_disconnect_at(self.host_now())
    }

    pub fn try_disconnect_with(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.try_disconnect_with_at(reason_code, properties, self.host_now())
    }

    pub fn auth(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.auth_at(reason_code, properties, self.host_now())
    }

    pub fn try_auth(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.try_auth_at(reason_code, properties, self.host_now())
    }

    pub fn send_ping(&mut self) -> Result<(), MqttClientError> {
        self.send_ping_at(self.host_now())
    }

    pub fn try_send_ping(&mut self) -> Result<(), MqttClientError> {
        self.try_send_ping_at(self.host_now())
    }

    pub fn enqueue_packet(&mut self, packet: MqttPacket) -> Result<(), MqttClientError> {
        self.enqueue_packet_at(packet, self.host_now())
    }

    pub fn puback(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.puback_at(id, reason, properties, self.host_now())
    }

    pub fn pubrec(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.pubrec_at(id, reason, properties, self.host_now())
    }

    pub fn pubcomp(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.pubcomp_at(id, reason, properties, self.host_now())
    }
}
