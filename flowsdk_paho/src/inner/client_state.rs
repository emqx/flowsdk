// SPDX-License-Identifier: MPL-2.0
//! Paho client state types.
//!
//! `PahoClientInner` is the opaque object behind each `MQTTClient` / `MQTTAsync` handle.
//! `SharedState` is the `Arc`-shared state between the C caller thread and the I/O thread.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;

use std::collections::HashMap;

use flowsdk::mqtt_client::client::ConnectionResult;
use flowsdk::mqtt_client::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use flowsdk::mqtt_client::error::MqttClientError;

use super::event_dispatch::{CallbackState, ReceivedMessage};
use super::token_tracker::TokenTracker;
use crate::common::uri_parser::ParsedUri;

// ─── Commands sent from C caller thread to I/O thread ────────────────────

/// Commands that the C API functions send to the I/O worker thread.
pub enum PahoCommand {
    /// Establish connection to the broker.
    Connect {
        uri: ParsedUri,
        connect_timeout_secs: u64,
    },
    /// Establish connection and wait for CONNACK.
    ConnectSync {
        uri: ParsedUri,
        connect_timeout_secs: u64,
        response_tx: std::sync::mpsc::Sender<Result<ConnectionResult, MqttClientError>>,
    },
    /// Publish a message (fire-and-forget, QoS 0 only).
    Publish(PublishCommand),
    /// Publish a message and return the assigned packet ID (for QoS 1/2 delivery tokens).
    PublishSync {
        command: PublishCommand,
        response_tx: std::sync::mpsc::Sender<Result<Option<u16>, MqttClientError>>,
    },
    /// Subscribe to topics (fire-and-forget, no sync wait).
    Subscribe(SubscribeCommand),
    /// Subscribe to topics, blocking until SUBACK.
    SubscribeSync {
        command: SubscribeCommand,
        response_tx: std::sync::mpsc::Sender<Result<(), MqttClientError>>,
    },
    /// Unsubscribe from topics (fire-and-forget).
    Unsubscribe(UnsubscribeCommand),
    /// Unsubscribe from topics, blocking until UNSUBACK.
    UnsubscribeSync {
        command: UnsubscribeCommand,
        response_tx: std::sync::mpsc::Sender<Result<(), MqttClientError>>,
    },
    /// Disconnect from the broker.
    Disconnect,
    /// Shutdown the I/O thread and exit.
    Shutdown,
}

// ─── Shared state between C caller thread and I/O thread ─────────────────

/// State shared between the C caller thread(s) and the I/O worker thread.
///
/// Access patterns:
/// - `connected`: AtomicBool — lock-free reads from any thread
/// - `callbacks`: Mutex — written from C caller (setCallbacks), read from I/O thread
/// - `token_tracker`: lock-free (internal Mutex + Condvar)
/// - `message_tx/rx`: crossbeam channel — lock-free MPSC
/// - `connect_waiter`: Mutex<Option> — set by sync connect, consumed by event dispatch
pub struct SharedState {
    /// Whether the client is currently connected.
    pub connected: AtomicBool,

    /// Paho C callback function pointers.
    pub callbacks: Mutex<CallbackState>,

    /// Delivery token tracker for `waitForCompletion`.
    pub token_tracker: TokenTracker,

    /// Sender side of the received message queue (I/O thread → MQTTClient_receive).
    pub message_tx: crossbeam_channel::Sender<ReceivedMessage>,

    /// Flag set by event dispatch when `MqttEvent::ReconnectNeeded` fires.
    pub reconnect_needed: AtomicBool,

    /// One-shot channel for synchronous connect: I/O thread sends the result here.
    pub connect_waiter:
        Mutex<Option<std::sync::mpsc::Sender<Result<ConnectionResult, MqttClientError>>>>,

    /// Pending SUBACK waiters: packet_id → response channel.
    pub subscribe_waiters:
        Mutex<HashMap<u16, std::sync::mpsc::Sender<Result<(), MqttClientError>>>>,

    /// Pending UNSUBACK waiters: packet_id → response channel.
    pub unsubscribe_waiters:
        Mutex<HashMap<u16, std::sync::mpsc::Sender<Result<(), MqttClientError>>>>,
}

impl SharedState {
    pub fn new(message_tx: crossbeam_channel::Sender<ReceivedMessage>) -> Self {
        Self {
            connected: AtomicBool::new(false),
            callbacks: Mutex::new(CallbackState::default()),
            token_tracker: TokenTracker::new(),
            message_tx,
            reconnect_needed: AtomicBool::new(false),
            connect_waiter: Mutex::new(None),
            subscribe_waiters: Mutex::new(HashMap::new()),
            unsubscribe_waiters: Mutex::new(HashMap::new()),
        }
    }
}

// ─── PahoClientInner — the opaque handle ─────────────────────────────────

/// The opaque object behind each `MQTTClient` / `MQTTAsync` handle.
///
/// Allocated on the heap with `Box::into_raw` at `MQTTClient_create` time,
/// recovered with `Box::from_raw` at `MQTTClient_destroy` time.
pub struct PahoClientInner {
    /// Channel to send commands to the I/O worker thread.
    pub command_tx: Sender<PahoCommand>,

    /// Handle to the I/O worker thread (joined on destroy).
    pub io_thread: Option<JoinHandle<()>>,

    /// Shared state between this handle and the I/O thread.
    pub shared: Arc<SharedState>,

    /// Receiver side of the message queue (for `MQTTClient_receive`).
    pub message_rx: Receiver<ReceivedMessage>,

    /// The server URI from `MQTTClient_create`.
    pub server_uri: String,

    /// The client ID from `MQTTClient_create`.
    pub client_id: String,

    /// MQTT version (set at create time, may be overridden at connect time).
    pub mqtt_version: u8,
}

impl PahoClientInner {
    /// Check if the client is currently connected (lock-free).
    pub fn is_connected(&self) -> bool {
        self.shared
            .connected
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Send a command to the I/O worker thread.
    pub fn send_command(
        &self,
        cmd: PahoCommand,
    ) -> Result<(), crossbeam_channel::SendError<PahoCommand>> {
        self.command_tx.send(cmd)
    }
}

impl Drop for PahoClientInner {
    fn drop(&mut self) {
        // Signal the I/O thread to shut down
        let _ = self.command_tx.send(PahoCommand::Shutdown);
        // Wait for the I/O thread to finish
        if let Some(handle) = self.io_thread.take() {
            let _ = handle.join();
        }
    }
}
