// SPDX-License-Identifier: MPL-2.0

//! Explicit-time entry points; all nested transitions use one accepted time.

use super::*;

impl<T: TimePoint> MqttEngine<T> {
    /// Validate configuration and time before allocating the parser.
    pub fn try_new_at(options: MqttClientOptions, now: T) -> Result<Self, MqttClientError> {
        if !matches!(options.mqtt_version, 3..=5)
            || options.parser_buffer_size == 0
            || options.max_event_count == 0
            || options.max_outgoing_packet_count == 0
        {
            return Err(MqttClientError::InvalidConfiguration {
                field: "engine options".into(),
                reason:
                    "MQTT version must be 3, 4 or 5 and parser/event/packet limits must be nonzero"
                        .into(),
            });
        }
        Self::check_time_range(&options, now)?;
        Ok(Self::new_at(options, now))
    }

    fn clock_error(reason: &str) -> MqttClientError {
        MqttClientError::InvalidConfiguration {
            field: "monotonic_time".into(),
            reason: reason.into(),
        }
    }

    fn check_time_range(options: &MqttClientOptions, now: T) -> Result<(), MqttClientError> {
        // CONNACK may negotiate any u16 keepalive. Check the entire possible
        // horizon before processing input, so a peer cannot overflow a deadline.
        let keepalive =
            Duration::from_secs(u16::MAX as u64) * options.ping_timeout_multiplier.max(1);
        let timeouts = options.operation_timeouts;
        let durations = [
            Some(keepalive),
            Some(Duration::from_millis(options.retransmission_timeout_ms)),
            Some(Duration::from_millis(options.reconnect_base_delay_ms)),
            Some(Duration::from_millis(options.reconnect_max_delay_ms)),
            timeouts.connect,
            timeouts.publish,
            timeouts.subscribe,
            timeouts.unsubscribe,
        ];
        if durations
            .into_iter()
            .flatten()
            .any(|duration| now.checked_add(duration).is_none())
        {
            return Err(Self::clock_error(
                "timestamp cannot represent the configured deadline horizon",
            ));
        }
        Ok(())
    }

    pub(super) fn begin_update(&mut self, now: T) -> Result<(), MqttClientError> {
        if now < self.now {
            return Err(Self::clock_error("timestamp moved backwards"));
        }
        Self::check_time_range(&self.options, now)?;
        self.now = now;
        Ok(())
    }

    pub(super) fn deadline_from(&self, start: T, duration: Duration) -> T {
        // All stored starts precede the checked operation timestamp. Durations
        // are bounded by check_time_range, including peer-negotiated keepalive.
        start
            .checked_add(duration)
            .expect("validated engine deadline horizon")
    }

    /// Run `handle_connection_lost` using caller-supplied monotonic time.
    pub fn handle_connection_lost_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.handle_connection_lost_inner();
        Ok(())
    }

    /// Run `reset_for_new_transport` using caller-supplied monotonic time.
    pub fn reset_for_new_transport_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.reset_for_new_transport_inner();
        Ok(())
    }

    /// Run `schedule_reconnect` using caller-supplied monotonic time.
    pub fn schedule_reconnect_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.schedule_reconnect_inner(now);
        Ok(())
    }

    /// Run `handle_incoming` using caller-supplied monotonic time.
    pub fn handle_incoming_at(
        &mut self,
        data: &[u8],
        now: T,
    ) -> Result<Vec<MqttEvent>, MqttClientError> {
        self.begin_update(now)?;
        Ok(self.handle_incoming_inner(data))
    }

    /// Run `handle_tick` using caller-supplied monotonic time.
    pub fn handle_tick_at(&mut self, now: T) -> Result<Vec<MqttEvent>, MqttClientError> {
        self.begin_update(now)?;
        Ok(self.handle_tick_inner(now))
    }

    /// Run `take_outgoing` using caller-supplied monotonic time.
    pub fn take_outgoing_at(&mut self, now: T) -> Result<Vec<u8>, MqttClientError> {
        self.begin_update(now)?;
        Ok(self.take_outgoing_inner())
    }

    /// Run `connect` using caller-supplied monotonic time.
    pub fn connect_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.connect_inner()
    }

    /// Run `publish` using caller-supplied monotonic time.
    pub fn publish_at(
        &mut self,
        command: PublishCommand,
        now: T,
    ) -> Result<Option<u16>, MqttClientError> {
        self.begin_update(now)?;
        self.publish_inner(command)
    }

    /// Run `publish_encoded` using caller-supplied monotonic time.
    pub fn publish_encoded_at(
        &mut self,
        command: PublishCommand,
        stream: Option<u64>,
        now: T,
    ) -> Result<(Option<u16>, Vec<u8>), MqttClientError> {
        self.begin_update(now)?;
        self.publish_encoded_inner(command, stream)
    }

    /// Run `ingest_stream_packet` using caller-supplied monotonic time.
    pub fn ingest_stream_packet_at(
        &mut self,
        packet: MqttPacket,
        stream: u64,
        now: T,
    ) -> Result<(Vec<MqttEvent>, Vec<u8>), MqttClientError> {
        self.begin_update(now)?;
        Ok(self.ingest_stream_packet_inner(packet, stream))
    }

    /// Run `subscribe` using caller-supplied monotonic time.
    pub fn subscribe_at(
        &mut self,
        command: SubscribeCommand,
        now: T,
    ) -> Result<u16, MqttClientError> {
        self.begin_update(now)?;
        self.subscribe_inner(command)
    }

    /// Run `unsubscribe` using caller-supplied monotonic time.
    pub fn unsubscribe_at(
        &mut self,
        command: UnsubscribeCommand,
        now: T,
    ) -> Result<u16, MqttClientError> {
        self.begin_update(now)?;
        self.unsubscribe_inner(command)
    }

    /// Run `subscribe_encoded` using caller-supplied monotonic time.
    pub fn subscribe_encoded_at(
        &mut self,
        command: SubscribeCommand,
        stream: Option<u64>,
        now: T,
    ) -> Result<(u16, Vec<u8>), MqttClientError> {
        self.begin_update(now)?;
        self.subscribe_encoded_inner(command, stream)
    }

    /// Run `unsubscribe_encoded` using caller-supplied monotonic time.
    pub fn unsubscribe_encoded_at(
        &mut self,
        command: UnsubscribeCommand,
        stream: Option<u64>,
        now: T,
    ) -> Result<(u16, Vec<u8>), MqttClientError> {
        self.begin_update(now)?;
        self.unsubscribe_encoded_inner(command, stream)
    }

    /// Run `disconnect` using caller-supplied monotonic time.
    pub fn disconnect_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.disconnect_inner()
    }

    /// Run `try_disconnect` using caller-supplied monotonic time.
    pub fn try_disconnect_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.try_disconnect_inner()
    }

    /// Run `try_disconnect_with` using caller-supplied monotonic time.
    pub fn try_disconnect_with_at(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.try_disconnect_with_inner(reason_code, properties)
    }

    /// Run `auth` using caller-supplied monotonic time.
    pub fn auth_at(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.auth_inner(reason_code, properties)
    }

    /// Run `try_auth` using caller-supplied monotonic time.
    pub fn try_auth_at(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.try_auth_inner(reason_code, properties)
    }

    /// Run `send_ping` using caller-supplied monotonic time.
    pub fn send_ping_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.send_ping_inner()
    }

    /// Run `try_send_ping` using caller-supplied monotonic time.
    pub fn try_send_ping_at(&mut self, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.try_send_ping_inner()
    }

    /// Run `enqueue_packet` using caller-supplied monotonic time.
    pub fn enqueue_packet_at(&mut self, packet: MqttPacket, now: T) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.enqueue_packet_inner(packet)
    }

    /// Run `puback` using caller-supplied monotonic time.
    pub fn puback_at(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.puback_inner(id, reason, properties)
    }

    /// Run `pubrec` using caller-supplied monotonic time.
    pub fn pubrec_at(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.pubrec_inner(id, reason, properties)
    }

    /// Run `pubcomp` using caller-supplied monotonic time.
    pub fn pubcomp_at(
        &mut self,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.begin_update(now)?;
        self.pubcomp_inner(id, reason, properties)
    }
}
