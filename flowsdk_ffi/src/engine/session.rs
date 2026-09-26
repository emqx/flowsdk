// SPDX-License-Identifier: MPL-2.0
use super::*;
use flowsdk::mqtt_session::ClientSessionState;
use std::sync::atomic::{AtomicBool, Ordering};

/// Startup is tracked separately from MQTT state: TLS/QUIC may already have
/// emitted handshake bytes before the first MQTT CONNECT.
pub(super) struct SessionAccess {
    explicit_peer: bool,
    used: AtomicBool,
    restored: AtomicBool,
}

impl SessionAccess {
    pub(super) fn new(explicit_peer: bool) -> Self {
        Self {
            explicit_peer,
            used: AtomicBool::new(false),
            restored: AtomicBool::new(false),
        }
    }
    pub(super) fn started(&self) {
        self.used.store(true, Ordering::SeqCst);
    }
    fn check_identity(&self) -> Result<(), MqttErrorFFI> {
        if !self.explicit_peer {
            return Err(MqttErrorFFI::Configuration {
                detail: "Persistence requires an explicit runtime peer identity".into(),
            });
        }
        Ok(())
    }
    // Call with the engine lock held. No foreign code or storage I/O runs here.
    fn restore(
        &self,
        engine: &mut MqttEngine,
        saved: ClientSessionState,
    ) -> Result<(), MqttErrorFFI> {
        self.check_identity()?;
        if self.used.load(Ordering::SeqCst) || self.restored.load(Ordering::SeqCst) {
            return Err(MqttErrorFFI::Engine {
                detail: "Restore requires a fresh engine before transport startup".into(),
            });
        }
        engine
            .restore_session_state(saved)
            .map_err(|error| match error {
                flowsdk::mqtt_client::error::MqttClientError::InvalidConfiguration { .. } => {
                    MqttErrorFFI::Configuration {
                        detail: error.to_string(),
                    }
                }
                _ => error.into(),
            })?;
        self.restored.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn decode(bytes: &[u8]) -> Result<ClientSessionState, MqttErrorFFI> {
    // Do not include the decoder's input or checkpoint contents in errors.
    let saved: ClientSessionState = serde_json::from_slice(bytes)
        .map_err(|_| properties::invalid("Malformed session checkpoint"))?;
    if saved.version() != ClientSessionState::VERSION {
        return Err(MqttErrorFFI::Configuration {
            detail: "Unsupported session checkpoint version".into(),
        });
    }
    Ok(saved)
}

fn encode(saved: ClientSessionState) -> Result<Vec<u8>, MqttErrorFFI> {
    serde_json::to_vec(&saved).map_err(|_| MqttErrorFFI::Engine {
        detail: "Session encoding failed".into(),
    })
}

/// Inspects metadata only; restoration performs identity, lifecycle and packet validation.
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
pub fn inspect_session_state(state: Vec<u8>) -> Result<MqttSessionInfoFFI, MqttErrorFFI> {
    let state = decode(&state)?;
    Ok(MqttSessionInfoFFI {
        version: state.version(),
        peer: state.peer().into(),
        client_id: state.client_id().into(),
        mqtt_version: state.mqtt_version(),
        has_session: state.has_session(),
        session_expiry_interval: state.session_expiry_interval(),
    })
}

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl MqttEngineFFI {
    pub fn snapshot_session(&self) -> Result<Vec<u8>, MqttErrorFFI> {
        self.session.check_identity()?;
        let saved = self.engine.lock().unwrap().snapshot_session()?;
        encode(saved)
    }
    pub fn restore_session_state(&self, state: Vec<u8>) -> Result<(), MqttErrorFFI> {
        let saved = decode(&state)?;
        self.session
            .restore(&mut self.engine.lock().unwrap(), saved)
    }
}

#[cfg(feature = "tls")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl TlsMqttEngineFFI {
    pub fn snapshot_session(&self) -> Result<Vec<u8>, MqttErrorFFI> {
        self.session.check_identity()?;
        let saved = self.engine.lock().unwrap().engine().snapshot_session()?;
        encode(saved)
    }
    pub fn restore_session_state(&self, state: Vec<u8>) -> Result<(), MqttErrorFFI> {
        let saved = decode(&state)?;
        self.session
            .restore(self.engine.lock().unwrap().engine_mut(), saved)
    }
}

#[cfg(feature = "quic")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl QuicMqttEngineFFI {
    pub fn snapshot_session(&self) -> Result<Vec<u8>, MqttErrorFFI> {
        self.session.check_identity()?;
        let saved = self.engine.lock().unwrap().engine().snapshot_session()?;
        encode(saved)
    }
    pub fn restore_session_state(&self, state: Vec<u8>) -> Result<(), MqttErrorFFI> {
        let saved = decode(&state)?;
        self.session
            .restore(self.engine.lock().unwrap().engine_mut(), saved)
    }
}

#[cfg(not(feature = "tls"))]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl TlsMqttEngineFFI {
    pub fn snapshot_session(&self) -> Result<Vec<u8>, MqttErrorFFI> {
        Err(runtime::tls_disabled())
    }
    pub fn restore_session_state(&self, _state: Vec<u8>) -> Result<(), MqttErrorFFI> {
        Err(runtime::tls_disabled())
    }
}
