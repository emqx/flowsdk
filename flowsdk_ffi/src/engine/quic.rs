// SPDX-License-Identifier: MPL-2.0
use super::*;

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl QuicMqttEngineFFI {
    pub fn connect_with_zero_rtt(
        &self,
        server_addr: String,
        server_name: String,
        tls_opts: MqttTlsOptionsFFI,
        options: QuicZeroRttOptionsFFI,
        now_ms: u64,
    ) -> Result<(), MqttErrorFFI> {
        #[cfg(feature = "durable-session")]
        self.session.started();
        if options.session_cache_size == 0 {
            return Err(properties::invalid("Session cache size must be positive"));
        }
        let addr = server_addr
            .parse()
            .map_err(|e: std::net::AddrParseError| properties::invalid(e.to_string()))?;
        let config = tls_config::client_config(&tls_opts)?;
        let mut engine = self.engine.lock().unwrap();
        if !engine.disconnect_complete() {
            return Err(MqttErrorFFI::Engine {
                detail: "QUIC transport is already active; close it before connecting".into(),
            });
        }
        engine
            .connect_with_zero_rtt(
                addr,
                &server_name,
                config,
                flowsdk::mqtt_client::engine::QuicZeroRttConfig {
                    session_cache_size: options.session_cache_size as usize,
                    replay_on_reject: options.replay_on_reject,
                },
                runtime::instant_at(self.start_time, now_ms)?,
            )
            .map_err(Into::into)
    }

    pub fn zero_rtt_status(&self) -> QuicZeroRttStatusFFI {
        self.engine.lock().unwrap().zero_rtt_status().into()
    }

    pub fn clear_session_cache(&self) {
        self.engine.lock().unwrap().clear_quic_session_cache();
    }

    pub fn reconnect(&self, now_ms: u64) -> Result<(), MqttErrorFFI> {
        #[cfg(feature = "durable-session")]
        self.session.started();
        self.engine
            .lock()
            .unwrap()
            .reconnect(runtime::instant_at(self.start_time, now_ms)?)
            .map_err(Into::into)
    }

    pub fn notify_local_address_changed(&self) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .notify_local_address_changed()
            .map_err(Into::into)
    }

    pub fn quic_ping(&self) -> Result<(), MqttErrorFFI> {
        self.engine.lock().unwrap().quic_ping().map_err(Into::into)
    }

    pub fn open_data_stream(&self) -> Result<u64, MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .open_data_stream()
            .map_err(Into::into)
    }

    pub fn control_stream_id(&self) -> Option<u64> {
        self.engine.lock().unwrap().control_stream_id()
    }

    pub fn data_stream_count(&self) -> u64 {
        self.engine.lock().unwrap().data_stream_count() as u64
    }

    pub fn set_stream_priority(&self, stream_id: u64, priority: u8) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .set_stream_priority(stream_id, priority)
            .map_err(Into::into)
    }

    pub fn finish_stream(&self, stream_id: u64) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .finish_stream(stream_id)
            .map_err(Into::into)
    }

    pub fn reset_stream(&self, stream_id: u64, error_code: u64) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .reset_stream(stream_id, error_code)
            .map_err(Into::into)
    }

    pub fn stop_stream(&self, stream_id: u64, error_code: u64) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .stop_stream(stream_id, error_code)
            .map_err(Into::into)
    }

    pub fn close_transport(&self, error_code: u64, reason: Vec<u8>) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .close(error_code, &reason)
            .map_err(Into::into)
    }

    pub fn close_silent(&self) {
        self.engine.lock().unwrap().close_silent();
    }

    pub fn publish_on(
        &self,
        stream_id: u64,
        topic: String,
        payload: Vec<u8>,
        options: MqttPublishOptionsFFI,
    ) -> Result<Option<u16>, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let priority = options.priority;
        let command = options.command(topic, payload, engine.engine().mqtt_version(), true)?;
        if let Some(priority) = priority {
            engine.set_stream_priority(stream_id, priority)?;
        }
        engine.publish_on(stream_id, command).map_err(Into::into)
    }

    pub fn subscribe_on(
        &self,
        stream_id: u64,
        options: MqttSubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.engine().mqtt_version())?;
        engine.subscribe_on(stream_id, command).map_err(Into::into)
    }

    pub fn unsubscribe_on(
        &self,
        stream_id: u64,
        options: MqttUnsubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.engine().mqtt_version())?;
        engine
            .unsubscribe_on(stream_id, command)
            .map_err(Into::into)
    }
    pub fn subscribe_on_control(
        &self,
        options: MqttSubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.engine().mqtt_version())?;
        engine.subscribe_on_control(command).map_err(Into::into)
    }

    pub fn unsubscribe_on_control(
        &self,
        options: MqttUnsubscribeOptionsFFI,
    ) -> Result<u16, MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        let command = options.command(engine.engine().mqtt_version())?;
        engine.unsubscribe_on_control(command).map_err(Into::into)
    }
}
