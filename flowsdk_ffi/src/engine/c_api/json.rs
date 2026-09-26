// SPDX-License-Identifier: MPL-2.0
//! Additive, checked C API. The JSON contract is documented in C_API.md.
//! All pointers must be valid for their stated lengths. Out parameters must be
//! writable and must not alias inputs. Engine handles must remain alive for each
//! call. Returned allocations use the existing free_bytes/free_string functions.
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    version: u32,
    connect: MqttConnectOptionsFFI,
    #[serde(default)]
    runtime: MqttRuntimeOptionsFFI,
    #[serde(default)]
    tls: MqttTlsOptionsFFI,
    #[serde(default)]
    server_name: String,
}

#[derive(Deserialize)]
#[cfg_attr(not(feature = "quic"), allow(dead_code))]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Connect,
    ResetTransport,
    SetReconnect {
        enabled: bool,
    },
    ScheduleReconnect {
        now_ms: u64,
    },
    Ping,
    Auth {
        reason_code: u8,
        #[serde(default)]
        properties: Vec<MqttPropertyFFI>,
    },
    Disconnect {
        #[serde(default)]
        options: MqttDisconnectOptionsFFI,
    },
    Publish {
        topic: String,
        payload: Vec<u8>,
        #[serde(default)]
        options: MqttPublishOptionsFFI,
    },
    Subscribe {
        options: MqttSubscribeOptionsFFI,
    },
    Unsubscribe {
        options: MqttUnsubscribeOptionsFFI,
    },
    Acknowledge {
        kind: MqttAcknowledgementFFI,
        packet_id: u16,
        #[serde(default)]
        reason_code: u8,
        #[serde(default)]
        properties: Vec<MqttPropertyFFI>,
        #[serde(default)]
        stream_id: Option<u64>,
    },
    ConnectQuic {
        server_addr: String,
        server_name: String,
        #[serde(default)]
        tls: MqttTlsOptionsFFI,
        now_ms: u64,
        #[serde(default)]
        zero_rtt: Option<QuicZeroRttOptionsFFI>,
    },
    Reconnect {
        now_ms: u64,
    },
    SubscribeOnControl {
        options: MqttSubscribeOptionsFFI,
    },
    UnsubscribeOnControl {
        options: MqttUnsubscribeOptionsFFI,
    },
}

fn unsupported() -> MqttErrorFFI {
    MqttErrorFFI::Unsupported {
        detail: "Command is not supported by this transport/build".into(),
    }
}

unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], MqttErrorFFI> {
    if len > isize::MAX as usize || (ptr.is_null() && len != 0) {
        return Err(properties::invalid("Invalid input pointer/length"));
    }
    Ok(if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    })
}

unsafe fn json<T: serde::de::DeserializeOwned>(
    ptr: *const u8,
    len: usize,
) -> Result<T, MqttErrorFFI> {
    serde_json::from_slice(bytes(ptr, len)?)
        .map_err(|_| properties::invalid("Malformed or unsupported JSON input"))
}

unsafe fn configuration(ptr: *const u8, len: usize) -> Result<Configuration, MqttErrorFFI> {
    let config: Configuration = json(ptr, len)?;
    if config.version != 1 {
        return Err(properties::invalid("Unsupported C configuration version"));
    }
    Ok(config)
}

unsafe fn status(result: Result<(), MqttErrorFFI>, error: *mut *mut c_char) -> c_int {
    if let Some(out) = error.as_mut() {
        *out = std::ptr::null_mut();
    }
    match result {
        Ok(()) => 0,
        Err(err) => {
            let code = match &err {
                MqttErrorFFI::InvalidArgument { .. } => 1,
                MqttErrorFFI::Configuration { .. } => 2,
                MqttErrorFFI::Engine { .. } => 3,
                MqttErrorFFI::Unsupported { .. } => 4,
            };
            if let Some(out) = error.as_mut() {
                *out = CString::new(err.to_string().replace('\0', "\\0"))
                    .unwrap()
                    .into_raw();
            }
            code
        }
    }
}

trait CheckedCommands {
    fn command(&self, command: Command) -> Result<Option<u16>, MqttErrorFFI>;
}

// Common commands call precisely the same typed APIs as UniFFI.
macro_rules! common_commands {
    ($engine:expr, $command:expr, $other:ident => $fallback:expr) => {
        match $command {
            Command::Ping => $engine.ping().map(|()| None),
            Command::Auth {
                reason_code,
                properties,
            } => $engine
                .auth_with_properties(reason_code, properties)
                .map(|()| None),
            Command::Disconnect { options } => {
                $engine.disconnect_with_options(options).map(|()| None)
            }
            Command::Publish {
                topic,
                payload,
                options,
            } => $engine.publish_with_options(topic, payload, options),
            Command::Subscribe { options } => $engine.subscribe_with_options(options).map(Some),
            Command::Unsubscribe { options } => $engine.unsubscribe_with_options(options).map(Some),
            Command::Acknowledge {
                kind,
                packet_id,
                reason_code,
                properties,
                stream_id,
            } => $engine
                .acknowledge(kind, packet_id, reason_code, properties, stream_id)
                .map(|()| None),
            Command::SetReconnect { enabled } => {
                $engine.set_reconnect(enabled);
                Ok(None)
            }
            $other => $fallback,
        }
    };
}

impl CheckedCommands for MqttEngineFFI {
    fn command(&self, command: Command) -> Result<Option<u16>, MqttErrorFFI> {
        common_commands!(self, command, other => match other {
            Command::Connect => self.connect_checked().map(|()| None),
            Command::ResetTransport => { self.reset_for_new_transport(); Ok(None) },
            Command::ScheduleReconnect { now_ms } => self.schedule_reconnect(now_ms).map(|()| None),
            _ => Err(unsupported()),
        })
    }
}

#[cfg(feature = "tls")]
impl CheckedCommands for TlsMqttEngineFFI {
    fn command(&self, command: Command) -> Result<Option<u16>, MqttErrorFFI> {
        common_commands!(self, command, other => match other {
            Command::Connect => self.connect_checked().map(|()| None),
            Command::ResetTransport => self.reset_for_new_transport().map(|()| None),
            _ => Err(unsupported()),
        })
    }
}

#[cfg(not(feature = "tls"))]
impl CheckedCommands for TlsMqttEngineFFI {
    fn command(&self, _command: Command) -> Result<Option<u16>, MqttErrorFFI> {
        Err(unsupported())
    }
}

#[cfg(feature = "quic")]
impl CheckedCommands for QuicMqttEngineFFI {
    fn command(&self, command: Command) -> Result<Option<u16>, MqttErrorFFI> {
        common_commands!(self, command, other => match other {
            Command::ConnectQuic { server_addr, server_name, tls, now_ms, zero_rtt } => {
                match zero_rtt {
                    Some(options) => self.connect_with_zero_rtt(server_addr, server_name, tls, options, now_ms),
                    None => self.connect(server_addr, server_name, tls, now_ms),
                }.map(|()| None)
            },
            Command::Reconnect { now_ms } => self.reconnect(now_ms).map(|()| None),
            Command::SubscribeOnControl { options } => self.subscribe_on_control(options).map(Some),
            Command::UnsubscribeOnControl { options } => self.unsubscribe_on_control(options).map(Some),
            _ => Err(unsupported()),
        })
    }
}

macro_rules! checked_api {
    ($ty:ty, $new:ident, $command:ident, $snapshot:ident, $restore:ident, $construct:expr) => {
        /// # Safety
        /// See this module's pointer, handle and allocation contract.
        #[no_mangle]
        pub unsafe extern "C" fn $new(
            input: *const u8,
            len: usize,
            out: *mut *mut $ty,
            error: *mut *mut c_char,
        ) -> c_int {
            if let Some(out) = out.as_mut() {
                *out = std::ptr::null_mut();
            }
            status(
                (|| {
                    let out = out
                        .as_mut()
                        .ok_or_else(|| properties::invalid("Null engine output"))?;
                    let config = configuration(input, len)?;
                    *out = Box::into_raw(Box::new(($construct)(config)?));
                    Ok(())
                })(),
                error,
            )
        }
        /// # Safety
        /// See this module's pointer, handle and allocation contract. packet_id may be null.
        #[no_mangle]
        pub unsafe extern "C" fn $command(
            engine: *const $ty,
            input: *const u8,
            len: usize,
            packet_id: *mut u16,
            error: *mut *mut c_char,
        ) -> c_int {
            if let Some(out) = packet_id.as_mut() {
                *out = 0;
            }
            status(
                (|| {
                    let engine = engine
                        .as_ref()
                        .ok_or_else(|| properties::invalid("Null engine"))?;
                    let id = engine.command(json(input, len)?)?;
                    if let Some(out) = packet_id.as_mut() {
                        *out = id.unwrap_or(0);
                    }
                    Ok(())
                })(),
                error,
            )
        }
        /// # Safety
        /// See this module's pointer, handle and allocation contract. Free the returned bytes.
        #[cfg(feature = "durable-session")]
        #[no_mangle]
        pub unsafe extern "C" fn $snapshot(
            engine: *const $ty,
            out: *mut *mut u8,
            len: *mut usize,
            error: *mut *mut c_char,
        ) -> c_int {
            if let Some(out) = out.as_mut() {
                *out = std::ptr::null_mut();
            }
            if let Some(len) = len.as_mut() {
                *len = 0;
            }
            status(
                (|| {
                    let out = out
                        .as_mut()
                        .ok_or_else(|| properties::invalid("Null byte output"))?;
                    let len = len
                        .as_mut()
                        .ok_or_else(|| properties::invalid("Null length output"))?;
                    let saved = engine
                        .as_ref()
                        .ok_or_else(|| properties::invalid("Null engine"))?
                        .snapshot_session()?
                        .into_boxed_slice();
                    *len = saved.len();
                    *out = Box::into_raw(saved) as *mut u8;
                    Ok(())
                })(),
                error,
            )
        }
        /// # Safety
        /// See this module's pointer, handle and allocation contract. Input is borrowed only.
        #[cfg(feature = "durable-session")]
        #[no_mangle]
        pub unsafe extern "C" fn $restore(
            engine: *const $ty,
            input: *const u8,
            len: usize,
            error: *mut *mut c_char,
        ) -> c_int {
            status(
                (|| {
                    engine
                        .as_ref()
                        .ok_or_else(|| properties::invalid("Null engine"))?
                        .restore_session_state(bytes(input, len)?.to_vec())
                })(),
                error,
            )
        }
    };
}

checked_api!(
    MqttEngineFFI,
    mqtt_engine_new_v1,
    mqtt_engine_command_v1,
    mqtt_engine_snapshot_session,
    mqtt_engine_restore_session_state,
    |c: Configuration| MqttEngineFFI::new_with_runtime_options(c.connect, c.runtime)
);
checked_api!(
    TlsMqttEngineFFI,
    mqtt_tls_engine_new_v1,
    mqtt_tls_engine_command_v1,
    mqtt_tls_engine_snapshot_session,
    mqtt_tls_engine_restore_session_state,
    |c: Configuration| TlsMqttEngineFFI::new_with_runtime_options(
        c.connect,
        c.runtime,
        c.tls,
        c.server_name
    )
);
#[cfg(feature = "quic")]
checked_api!(
    QuicMqttEngineFFI,
    mqtt_quic_engine_new_v1,
    mqtt_quic_engine_command_v1,
    mqtt_quic_engine_snapshot_session,
    mqtt_quic_engine_restore_session_state,
    |c: Configuration| QuicMqttEngineFFI::new_with_runtime_options(c.connect, c.runtime)
);

/// # Safety
/// Input is borrowed for len bytes; out/error are writable. Free strings with mqtt_engine_free_string.
#[cfg(feature = "durable-session")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_session_inspect(
    input: *const u8,
    len: usize,
    out: *mut *mut c_char,
    error: *mut *mut c_char,
) -> c_int {
    if let Some(out) = out.as_mut() {
        *out = std::ptr::null_mut();
    }
    status(
        (|| {
            let out = out
                .as_mut()
                .ok_or_else(|| properties::invalid("Null metadata output"))?;
            let info = inspect_session_state(bytes(input, len)?.to_vec())?;
            *out = CString::new(serde_json::to_string(&info).unwrap())
                .unwrap()
                .into_raw();
            Ok(())
        })(),
        error,
    )
}

/// # Safety
/// The event list must remain alive; out/error must be writable. Returns owned JSON including failure metadata.
#[no_mangle]
pub unsafe extern "C" fn mqtt_event_list_get_json(
    ptr: *const MqttEventListFFI,
    index: usize,
    out: *mut *mut c_char,
    error: *mut *mut c_char,
) -> c_int {
    if let Some(out) = out.as_mut() {
        *out = std::ptr::null_mut();
    }
    status(
        (|| {
            let out = out
                .as_mut()
                .ok_or_else(|| properties::invalid("Null event output"))?;
            let event = ptr
                .as_ref()
                .and_then(|list| list.events.get(index))
                .ok_or_else(|| properties::invalid("Invalid event list/index"))?;
            *out = CString::new(serde_json::to_string(event).unwrap())
                .unwrap()
                .into_raw();
            Ok(())
        })(),
        error,
    )
}
