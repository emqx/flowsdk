// SPDX-License-Identifier: MPL-2.0

//! Application-owned storage for MQTT client sessions.

use crate::mqtt_serde::control_packet::MqttPacket;
use serde::{Deserialize, Serialize};

/// A versioned, serializable checkpoint of a client's MQTT session.
///
/// Obtain this from `MqttEngine::snapshot_session` or
/// `NoIoMqttClient::snapshot_session`. Restore it before CONNECT with
/// `restore_session_state`. Deserialization alone does not validate the state;
/// restoration checks its version, identity, packets and configured limits.
///
/// Contains queued publications, outstanding operations, incoming QoS 2 stages
/// and the packet ID allocator. Socket bytes, timers, negotiated connection
/// limits, credentials and application events are not persisted. The broker's
/// next CONNACK determines whether the MQTT session still exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSessionState {
    pub(crate) version: u32,
    pub(crate) peer: String,
    pub(crate) client_id: String,
    pub(crate) mqtt_version: u8,
    pub(crate) has_session: bool,
    pub(crate) session_expiry_interval: u32,
    pub(crate) packet_id_counter: u16,
    pub(crate) outbound: Vec<(u16, MqttPacket)>,
    pub(crate) queued: Vec<(u8, MqttPacket)>,
    pub(crate) received: Vec<ReceivedExchange>,
}

impl ClientSessionState {
    /// Storage schema version understood by this SDK.
    pub const VERSION: u32 = 1;

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn peer(&self) -> &str {
        &self.peer
    }

    /// Includes a broker-assigned identifier, if one was negotiated.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn mqtt_version(&self) -> u8 {
        self.mqtt_version
    }

    /// Whether a successful CONNACK established the saved session.
    pub fn has_session(&self) -> bool {
        self.has_session
    }

    /// Last negotiated MQTT 5 session expiry interval, in seconds.
    /// This is not a remaining lifetime or an offline expiry timestamp.
    pub fn session_expiry_interval(&self) -> u32 {
        self.session_expiry_interval
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReceivedExchange {
    // Old stream identity is only a correlation key. On restore the exchange
    // is unbound, just as it is after an ordinary transport reset.
    pub stream: Option<u64>,
    pub packet_id: u16,
    pub stage: ReceivedStage,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum ReceivedStage {
    Publish,
    PubRec,
    PubRel,
}

/// Pluggable storage for checkpoints across client process restarts.
///
/// The application owns and calls the store; the sans-I/O engine never performs
/// storage I/O. Keys are application-defined and should identify a broker/client
/// pair (and account, where applicable). Use one writer per key, or implement
/// equivalent locking in the backend. `resume` loads without deleting the record.
///
/// A durable implementation must commit complete snapshots atomically and return
/// success only after the requested durability boundary is reached. An error must
/// not expose a partial record. Propagate storage errors instead of silently
/// starting a new session. Backend-specific errors remain available to callers.
///
/// For a planned restart, stop driving the client, checkpoint its latest state,
/// then discard the old transport. For crash recovery, commit after each protocol
/// transition and before releasing its wire output or application events. Output
/// draining can itself advance protocol state: checkpoint again after draining
/// and before writing the returned bytes. External message processing requires
/// application transactions/manual ACKs; a checkpoint does not make side effects
/// exactly-once. Application events and completion notifications are not stored
/// in the checkpoint.
pub trait ClientSessionStore {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Insert a new record. Return an error if the key already exists.
    fn create(&mut self, key: &str, state: &ClientSessionState) -> Result<(), Self::Error>;

    /// Load a checkpoint, returning `None` only when the key does not exist.
    fn resume(&mut self, key: &str) -> Result<Option<ClientSessionState>, Self::Error>;

    /// Atomically replace an existing record. Return an error if it is missing.
    fn update(&mut self, key: &str, state: &ClientSessionState) -> Result<(), Self::Error>;

    /// Delete a record. Deleting a missing key succeeds.
    /// This only deletes local storage; it does not delete the broker's session.
    fn delete(&mut self, key: &str) -> Result<(), Self::Error>;
}
