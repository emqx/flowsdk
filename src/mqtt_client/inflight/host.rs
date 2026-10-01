// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::time::Instant;

impl InflightQueue<Instant> {
    fn host_now(&self) -> Instant {
        self.last_update
            .map_or_else(Instant::now, |last| Instant::now().max(last))
    }
    pub fn push(&mut self, id: u16, packet: MqttPacket, qos: u8) -> Result<(), MqttClientError> {
        self.push_at(id, packet, qos, self.host_now())
    }
    pub fn push_with_stream(
        &mut self,
        id: u16,
        packet: MqttPacket,
        qos: u8,
        stream: Option<u64>,
    ) -> Result<(), MqttClientError> {
        self.push_with_stream_at(id, packet, qos, stream, self.host_now())
    }
    pub fn transition_pubrel(&mut self, id: u16, packet: MqttPacket) {
        // Preserve the legacy infallible signature; realistic host times fit.
        let _ = self.transition_pubrel_at(id, packet, self.host_now());
    }
    pub fn resume_on_stream(&mut self, id: u16, stream: u64) {
        let _ = self.resume_on_stream_at(id, stream, self.host_now());
    }
    pub fn get_expired(&mut self, now: Instant) -> Vec<MqttPacket> {
        self.get_expired_at(now).unwrap_or_default()
    }
    pub fn get_expired_with_stream(&mut self, now: Instant) -> Vec<(MqttPacket, Option<u64>)> {
        self.get_expired_with_stream_at(now).unwrap_or_default()
    }
}
