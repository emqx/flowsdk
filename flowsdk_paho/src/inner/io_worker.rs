// SPDX-License-Identifier: MPL-2.0
//! I/O worker thread for the Paho compatibility layer.
//!
//! This thread owns the `MqttEngine` (sans-I/O) and a blocking TCP/TLS socket.
//! It runs a poll-based event loop that multiplexes:
//! - Socket reads (incoming MQTT data)
//! - Command channel (from C caller thread)
//! - Tick timer (keep-alive, retransmissions)

use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError};

use flowsdk::mqtt_client::engine::MqttEngine;
use flowsdk::mqtt_client::error::MqttClientError;
use flowsdk::mqtt_client::opts::MqttClientOptions;

use super::client_state::{AsyncResponse, PahoCommand, SharedState};
use super::event_dispatch;
use super::transport::BlockingTransport;
use crate::common::return_codes::MQTTCLIENT_FAILURE;
use crate::common::uri_parser::ParsedUri;

/// The I/O worker that runs on a dedicated thread.
///
/// Owns the `MqttEngine` exclusively — no locking needed for engine access.
pub struct IoWorker {
    engine: MqttEngine,
    stream: Option<BlockingTransport>,
    command_rx: Receiver<PahoCommand>,
    shared: Arc<SharedState>,
}

//@TODO Prioritize commands/network bytes and protocol events?
impl IoWorker {
    pub fn new(
        options: MqttClientOptions,
        command_rx: Receiver<PahoCommand>,
        shared: Arc<SharedState>,
    ) -> Self {
        Self {
            engine: MqttEngine::new(options),
            stream: None,
            command_rx,
            shared,
        }
    }

    /// Main event loop. Runs until a `Shutdown` command is received or the channel closes.
    pub fn run(mut self) {
        // Read buffer for incoming data
        let mut buf = vec![0u8; 8192];

        loop {
            // Calculate how long to wait before the next tick
            let poll_timeout = self.next_poll_timeout();

            // If we have a connected socket, set a short read timeout for polling
            if let Some(ref stream) = self.stream {
                let _ = stream.set_read_timeout(Some(poll_timeout));
            }

            // 1. Try to read from socket (non-blocking via read timeout)
            if self.stream.is_some() {
                match self.stream.as_mut().unwrap().read(&mut buf) {
                    Ok(0) => {
                        // Connection closed by server
                        self.handle_connection_lost();
                    }
                    Ok(n) => {
                        let events = self.engine.handle_incoming(&buf[..n]);
                        event_dispatch::dispatch_events(events, &self.shared);
                        self.flush_outgoing();
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        // No data available — not an error
                    }
                    Err(_e) => {
                        // I/O error — connection lost
                        self.handle_connection_lost();
                    }
                }
            } else {
                // No socket — just sleep for the poll timeout to avoid busy-waiting
                std::thread::sleep(poll_timeout);
            }

            // 2. Process all pending commands
            loop {
                match self.command_rx.try_recv() {
                    Ok(cmd) => {
                        if !self.handle_command(cmd) {
                            return; // Shutdown
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return, // Channel closed
                }
            }

            // 3. Handle tick timer (keep-alive, retransmissions)
            if let Some(next_tick) = self.engine.next_tick_at() {
                if Instant::now() >= next_tick {
                    let events = self.engine.handle_tick(Instant::now());
                    event_dispatch::dispatch_events(events, &self.shared);
                    self.flush_outgoing();
                }
            }

            // 4. Check if reconnection is needed
            if self.shared.reconnect_needed.swap(false, Ordering::SeqCst) {
                // Engine requested reconnection — will be handled on next connect command
                // or via auto-reconnect logic
            }
        }
    }

    /// Handle a command from the C caller thread.
    /// Returns `false` if the worker should shut down.
    fn handle_command(&mut self, cmd: PahoCommand) -> bool {
        match cmd {
            PahoCommand::Connect {
                uri,
                connect_timeout_secs,
            } => {
                self.do_connect(&uri, connect_timeout_secs);
                true
            }
            PahoCommand::ConnectSync {
                uri,
                connect_timeout_secs,
                response_tx,
            } => {
                // Store the response channel in shared state
                *self.shared.connect_waiter.lock() = Some(response_tx.clone());
                self.do_connect(&uri, connect_timeout_secs);

                // If connect failed at the transport level, send error immediately
                if self.stream.is_none() {
                    if let Some(tx) = self.shared.connect_waiter.lock().take() {
                        let _ = tx.send(Err(MqttClientError::ConnectionLost {
                            reason: "Transport connection failed".to_string(),
                        }));
                    }
                }
                true
            }
            PahoCommand::Publish(cmd) => {
                match self.engine.publish(cmd) {
                    Ok(packet_id) => {
                        if let Some(pid) = packet_id {
                            self.shared.token_tracker.mark_pending(pid as i32);
                        }
                    }
                    Err(_e) => { /* Error will surface via return code */ }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::PublishSync {
                command,
                response_tx,
            } => {
                match self.engine.publish(command) {
                    Ok(packet_id) => {
                        if let Some(pid) = packet_id {
                            self.shared.token_tracker.mark_pending(pid as i32);
                        }
                        let _ = response_tx.send(Ok(packet_id));
                    }
                    Err(e) => {
                        let _ = response_tx.send(Err(e));
                    }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::Subscribe(cmd) => {
                match self.engine.subscribe(cmd) {
                    Ok(packet_id) => {
                        self.shared.token_tracker.mark_pending(packet_id as i32);
                    }
                    Err(_e) => { /* Error will surface via return code */ }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::SubscribeSync {
                command,
                response_tx,
            } => {
                match self.engine.subscribe(command) {
                    Ok(packet_id) => {
                        self.shared.token_tracker.mark_pending(packet_id as i32);
                        self.shared
                            .subscribe_waiters
                            .lock()
                            .insert(packet_id as u16, response_tx);
                    }
                    Err(e) => {
                        let _ = response_tx.send(Err(e));
                    }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::Unsubscribe(cmd) => {
                match self.engine.unsubscribe(cmd) {
                    Ok(packet_id) => {
                        self.shared.token_tracker.mark_pending(packet_id as i32);
                    }
                    Err(_e) => { /* Error will surface via return code */ }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::UnsubscribeSync {
                command,
                response_tx,
            } => {
                match self.engine.unsubscribe(command) {
                    Ok(packet_id) => {
                        self.shared.token_tracker.mark_pending(packet_id as i32);
                        self.shared
                            .unsubscribe_waiters
                            .lock()
                            .insert(packet_id as u16, response_tx);
                    }
                    Err(e) => {
                        let _ = response_tx.send(Err(e));
                    }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::Disconnect => {
                self.engine.disconnect();
                self.flush_outgoing();
                // Close the transport
                if let Some(ref stream) = self.stream {
                    let _ = stream.shutdown();
                }
                self.stream = None;
                self.shared.connected.store(false, Ordering::SeqCst);
                self.shared.token_tracker.clear();
                true
            }
            PahoCommand::ConnectAsync {
                uri,
                connect_timeout_secs,
                response,
            } => {
                *self.shared.async_connect_response.lock() = Some(response);
                self.do_connect(&uri, connect_timeout_secs);
                // If the transport failed to connect, fire onFailure right away.
                if self.stream.is_none() {
                    if let Some(resp) = self.shared.async_connect_response.lock().take() {
                        event_dispatch::fire_failure(
                            &resp,
                            MQTTCLIENT_FAILURE,
                            "Transport connection failed",
                        );
                    }
                }
                true
            }
            PahoCommand::PublishAsync {
                command,
                mut response,
                token_tx,
            } => {
                match self.engine.publish(command) {
                    Ok(Some(pid)) => {
                        let token = pid as i32;
                        response.token = token;
                        self.shared.token_tracker.mark_pending(token);
                        self.shared.async_responses.lock().insert(token, response);
                        let _ = token_tx.send(token);
                    }
                    Ok(None) => {
                        // QoS 0: no packet id. Token 0; onSuccess fires once written.
                        response.token = 0;
                        let _ = token_tx.send(0);
                        self.flush_outgoing();
                        event_dispatch::fire_success_publish(&response, "");
                    }
                    Err(_e) => {
                        let _ = token_tx.send(0);
                    }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::SubscribeAsync {
                command,
                mut response,
                token_tx,
            } => {
                match self.engine.subscribe(command) {
                    Ok(packet_id) => {
                        let token = packet_id as i32;
                        response.token = token;
                        self.shared.token_tracker.mark_pending(token);
                        self.shared.async_responses.lock().insert(token, response);
                        let _ = token_tx.send(token);
                    }
                    Err(_e) => {
                        let _ = token_tx.send(0);
                    }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::UnsubscribeAsync {
                command,
                mut response,
                token_tx,
            } => {
                match self.engine.unsubscribe(command) {
                    Ok(packet_id) => {
                        let token = packet_id as i32;
                        response.token = token;
                        self.shared.token_tracker.mark_pending(token);
                        self.shared.async_responses.lock().insert(token, response);
                        let _ = token_tx.send(token);
                    }
                    Err(_e) => {
                        let _ = token_tx.send(0);
                    }
                }
                self.flush_outgoing();
                true
            }
            PahoCommand::DisconnectAsync { response } => {
                self.engine.disconnect();
                self.flush_outgoing();
                if let Some(ref stream) = self.stream {
                    let _ = stream.shutdown();
                }
                self.stream = None;
                self.shared.connected.store(false, Ordering::SeqCst);
                self.fail_pending_async("Disconnected");
                self.shared.token_tracker.clear();
                event_dispatch::fire_success_simple(&response);
                true
            }
            PahoCommand::Shutdown => {
                // Disconnect if connected
                if self.stream.is_some() {
                    self.engine.disconnect();
                    self.flush_outgoing();
                    if let Some(ref stream) = self.stream {
                        let _ = stream.shutdown();
                    }
                }
                false
            }
        }
    }

    /// Establish TCP/TLS connection and send MQTT CONNECT packet.
    fn do_connect(&mut self, uri: &ParsedUri, connect_timeout_secs: u64) {
        let timeout = Duration::from_secs(connect_timeout_secs);

        // Close existing connection if any
        if let Some(ref stream) = self.stream {
            let _ = stream.shutdown();
        }
        self.stream = None;

        // Establish transport connection
        match BlockingTransport::connect(uri, timeout) {
            Ok(transport) => {
                self.stream = Some(transport);

                // Send MQTT CONNECT packet
                self.engine.connect();
                self.flush_outgoing();
            }
            Err(e) => {
                // Transport connection failed
                self.shared.connected.store(false, Ordering::SeqCst);
                // If there's a sync connect waiter, notify it
                if let Some(tx) = self.shared.connect_waiter.lock().take() {
                    let _ = tx.send(Err(MqttClientError::NetworkError {
                        kind: e.kind(),
                        message: e.to_string(),
                    }));
                }
            }
        }
    }

    /// Take outgoing bytes from the engine and write them to the transport.
    fn flush_outgoing(&mut self) {
        let bytes = self.engine.take_outgoing();
        if bytes.is_empty() {
            return;
        }

        let write_failed = if let Some(ref mut stream) = self.stream {
            stream.write_all(&bytes).is_err()
        } else {
            false
        };

        if write_failed {
            self.handle_connection_lost();
        }
    }

    /// Handle a lost connection: notify engine, update state, invoke callbacks.
    fn handle_connection_lost(&mut self) {
        self.stream = None;
        self.engine.handle_connection_lost();
        let events = self.engine.take_events();
        event_dispatch::dispatch_events(events, &self.shared);
        self.shared.connected.store(false, Ordering::SeqCst);
        self.fail_pending_async("Connection lost");
    }

    /// Fire `onFailure` for every outstanding async operation (connect + ops).
    fn fail_pending_async(&self, reason: &str) {
        if let Some(resp) = self.shared.async_connect_response.lock().take() {
            event_dispatch::fire_failure(&resp, MQTTCLIENT_FAILURE, reason);
        }
        let pending: Vec<AsyncResponse> =
            self.shared.async_responses.lock().drain().map(|(_, r)| r).collect();
        for resp in pending {
            event_dispatch::fire_failure(&resp, MQTTCLIENT_FAILURE, reason);
        }
    }

    /// Calculate the timeout for the next poll iteration.
    fn next_poll_timeout(&self) -> Duration {
        if let Some(next_tick) = self.engine.next_tick_at() {
            let now = Instant::now();
            if next_tick > now {
                // Cap at 100ms to stay responsive to commands
                std::cmp::min(next_tick.duration_since(now), Duration::from_millis(100))
            } else {
                Duration::from_millis(1) // Tick overdue, process immediately
            }
        } else if self.stream.is_some() {
            Duration::from_millis(100) // Connected but no tick scheduled
        } else {
            Duration::from_millis(200) // Not connected, check commands less frequently
        }
    }
}
