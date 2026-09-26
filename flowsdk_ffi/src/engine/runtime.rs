// SPDX-License-Identifier: MPL-2.0
use super::*;
use flowsdk::mqtt_client::opts::{MqttClientOptions, OperationTimeouts};

pub(super) fn instant_at(origin: Instant, ms: u64) -> Result<Instant, MqttErrorFFI> {
    origin
        .checked_add(Duration::from_millis(ms))
        .ok_or_else(|| properties::invalid("Time is outside the supported clock range"))
}

impl MqttRuntimeOptionsFFI {
    pub(super) fn apply(self, options: &mut MqttClientOptions) -> Result<(), MqttErrorFFI> {
        if let Some(peer) = self.peer {
            if peer.trim().is_empty() || peer.contains('\0') {
                return Err(properties::invalid(
                    "peer must be nonempty and contain no NUL",
                ));
            }
            options.peer = peer;
        }
        if let Some(timeouts) = self.operation_timeouts {
            let duration = |ms: Option<u64>| -> Result<Option<Duration>, MqttErrorFFI> {
                ms.map(|ms| {
                    instant_at(Instant::now(), ms)?;
                    Ok(Duration::from_millis(ms))
                })
                .transpose()
            };
            options.operation_timeouts = OperationTimeouts {
                connect: duration(timeouts.connect_ms)?,
                publish: duration(timeouts.publish_ms)?,
                subscribe: duration(timeouts.subscribe_ms)?,
                unsubscribe: duration(timeouts.unsubscribe_ms)?,
            };
        }
        if let Some(value) = self.incoming_receive_maximum {
            if value == 0 {
                return Err(properties::invalid(
                    "incoming_receive_maximum must be positive",
                ));
            }
            options.incoming_receive_maximum = Some(value);
        }
        let size = |value: Option<u64>| -> Result<Option<usize>, MqttErrorFFI> {
            value
                .map(|n| {
                    usize::try_from(n)
                        .ok()
                        .filter(|n| *n > 0 && *n <= isize::MAX as usize)
                        .ok_or_else(|| {
                            properties::invalid("Byte limit is outside the supported size range")
                        })
                })
                .transpose()
        };
        options.max_incoming_packet_size = size(self.max_incoming_packet_size)?;
        options.max_incoming_buffer_bytes = size(self.max_incoming_buffer_bytes)?;
        options.max_outgoing_buffer_bytes = size(self.max_outgoing_buffer_bytes)?;
        if let Some(enabled) = self.reconnect {
            options.reconnect = enabled;
        }
        Ok(())
    }
}

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl MqttEngineFFI {
    /// Does not reset transport state or discard pending events on error.
    pub fn connect_checked(&self) -> Result<(), MqttErrorFFI> {
        #[cfg(feature = "durable-session")]
        self.session.started();
        self.engine.lock().unwrap().connect().map_err(Into::into)
    }

    pub fn reset_for_new_transport(&self) {
        #[cfg(feature = "durable-session")]
        self.session.started();
        self.engine.lock().unwrap().reset_for_new_transport();
    }

    pub fn has_pending_output(&self) -> bool {
        self.engine.lock().unwrap().has_pending_output()
    }

    pub fn schedule_reconnect(&self, now_ms: u64) -> Result<(), MqttErrorFFI> {
        let now = instant_at(self.start_time, now_ms)?;
        self.engine.lock().unwrap().schedule_reconnect(now);
        Ok(())
    }

    pub fn set_reconnect(&self, enabled: bool) {
        self.engine.lock().unwrap().set_reconnect(enabled);
        if !enabled {
            clear_reconnect_events(&self.events);
        }
    }
}

fn clear_reconnect_events(events: &Mutex<Vec<MqttEventFFI>>) {
    events.lock().unwrap().retain(|e| {
        !matches!(
            e,
            MqttEventFFI::ReconnectNeeded | MqttEventFFI::ReconnectScheduled { .. }
        )
    });
}

#[cfg(feature = "tls")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl TlsMqttEngineFFI {
    pub fn connect_checked(&self) -> Result<(), MqttErrorFFI> {
        #[cfg(feature = "durable-session")]
        self.session.started();
        self.engine.lock().unwrap().connect().map_err(Into::into)
    }

    /// Recreates TLS state as well as the MQTT transport state.
    pub fn reset_for_new_transport(&self) -> Result<(), MqttErrorFFI> {
        #[cfg(feature = "durable-session")]
        self.session.started();
        self.engine
            .lock()
            .unwrap()
            .reset_for_new_transport()
            .map_err(Into::into)
    }

    pub fn set_reconnect(&self, enabled: bool) {
        self.engine
            .lock()
            .unwrap()
            .engine_mut()
            .set_reconnect(enabled);
        if !enabled {
            clear_reconnect_events(&self.events);
        }
    }
}

#[cfg(feature = "quic")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl QuicMqttEngineFFI {
    pub fn set_reconnect(&self, enabled: bool) {
        self.engine
            .lock()
            .unwrap()
            .engine_mut()
            .set_reconnect(enabled);
        if !enabled {
            clear_reconnect_events(&self.events);
        }
    }
}

#[cfg(not(feature = "tls"))]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl TlsMqttEngineFFI {
    #[cfg_attr(feature = "uniffi-bindings", uniffi::constructor)]
    pub fn new_with_runtime_options(
        _opts: MqttConnectOptionsFFI,
        _runtime: MqttRuntimeOptionsFFI,
        _tls_opts: MqttTlsOptionsFFI,
        _server_name: String,
    ) -> Result<Self, MqttErrorFFI> {
        Err(tls_disabled())
    }
    pub fn connect_checked(&self) -> Result<(), MqttErrorFFI> {
        Err(tls_disabled())
    }
    pub fn reset_for_new_transport(&self) -> Result<(), MqttErrorFFI> {
        Err(tls_disabled())
    }
    pub fn set_reconnect(&self, _enabled: bool) -> Result<(), MqttErrorFFI> {
        Err(tls_disabled())
    }
}

#[cfg(not(feature = "tls"))]
pub(super) fn tls_disabled() -> MqttErrorFFI {
    MqttErrorFFI::Unsupported {
        detail: "TLS support is not enabled".into(),
    }
}
