// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::mqtt_client::engine::OperationKind;
#[cfg(feature = "rustls-tls")]
use crate::mqtt_client::transport::{RustlsTlsConfig, RustlsTlsTransport};
use std::collections::HashMap;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::{Context, Poll};

type ConnectFuture = Pin<Box<dyn Future<Output = Result<BoxedTransport, MqttClientError>> + Send>>;

impl TokioClientWorker {
    fn create_transport(&self) -> ConnectFuture {
        let peer = self.engine.options().peer.clone();
        let config = self.config.clone();
        #[cfg(any(feature = "tls", feature = "rustls-tls"))]
        let tls_backend = self.engine.options().tls_backend;
        #[cfg(feature = "tls")]
        let tls_config = self.engine.options().tls_config.clone().unwrap_or_default();
        #[cfg(feature = "rustls-tls")]
        let rustls_config = self.engine.options().rustls_tls_config.clone();
        Box::pin(async move {
            let connect = async {
                let peer = peer.as_str();
                // Parse URL scheme
                if peer.starts_with("mqtts://") {
                    // Decide which TLS backend to use based on options.tls_backend
                    #[cfg(any(feature = "tls", feature = "rustls-tls"))]
                    {
                        let addr = peer.strip_prefix("mqtts://").unwrap_or(peer);
                        match tls_backend {
                            #[cfg(feature = "rustls-tls")]
                            Some(crate::mqtt_client::opts::TlsBackend::Rustls) => {
                                let mut rustls_cfg = rustls_config.unwrap_or_else(|| {
                                    RustlsTlsConfig::builder().use_system_roots(true).build()
                                });
                                if config.tls_enable_key_log {
                                    rustls_cfg.enable_key_log = true;
                                }
                                let transport =
                                    RustlsTlsTransport::connect_with_config(addr, rustls_cfg)
                                        .await
                                        .map_err(|e| MqttClientError::ConnectionLost {
                                            reason: format!(
                                                "Rustls TLS connection failed to {peer}: {e}"
                                            ),
                                        })?;
                                Ok(Box::new(transport) as BoxedTransport)
                            }
                            #[cfg(feature = "tls")]
                            Some(crate::mqtt_client::opts::TlsBackend::Native) => {
                                let transport =
                                    TlsTransport::connect_with_config(addr, &tls_config)
                                        .await
                                        .map_err(|e| MqttClientError::ConnectionLost {
                                            reason: format!(
                                                "TLS connection failed to {}: {}",
                                                peer, e
                                            ),
                                        })?;
                                Ok(Box::new(transport) as BoxedTransport)
                            }
                            #[cfg(all(feature = "tls", not(feature = "rustls-tls")))]
                            Some(crate::mqtt_client::opts::TlsBackend::Rustls) => {
                                Err(MqttClientError::ProtocolViolation {
                                    message: "Rustls TLS backend requires the 'rustls-tls' feature"
                                        .into(),
                                })
                            }
                            #[cfg(all(feature = "rustls-tls", not(feature = "tls")))]
                            Some(crate::mqtt_client::opts::TlsBackend::Native) => {
                                Err(MqttClientError::ProtocolViolation {
                                    message: "Native TLS backend requires the 'tls' feature".into(),
                                })
                            }
                            None => {
                                // Backward compatibility: default to Native if available
                                #[cfg(feature = "tls")]
                                {
                                    let transport =
                                        TlsTransport::connect_with_config(addr, &tls_config)
                                            .await
                                            .map_err(|e| MqttClientError::ConnectionLost {
                                                reason: format!(
                                                    "TLS connection failed to {}: {}",
                                                    peer, e
                                                ),
                                            })?;
                                    Ok(Box::new(transport) as BoxedTransport)
                                }
                                #[cfg(not(feature = "tls"))]
                                {
                                    Err(MqttClientError::ProtocolViolation {
                                        message: "Select the rustls TLS backend when native TLS is unavailable".into(),
                                    })
                                }
                            }
                            #[allow(unreachable_patterns)]
                            _ => Err(MqttClientError::ProtocolViolation {
                                message: "Unsupported TLS backend configuration".to_string(),
                            }),
                        }
                    }
                    #[cfg(not(any(feature = "tls", feature = "rustls-tls")))]
                    {
                        Err(MqttClientError::ProtocolViolation {
                            message: "TLS transport requires the 'tls' or 'rustls-tls' feature"
                                .into(),
                        })
                    }
                } else if peer.starts_with("quic://") {
                    #[cfg(feature = "quic")]
                    {
                        let addr = peer.strip_prefix("quic://").unwrap_or(peer);

                        // Build QUIC config from TokioAsyncClientConfig
                        let mut builder = QuicConfig::builder()
                            .alpn(b"mqtt")
                            .enable_0rtt(config.quic_enable_0rtt);

                        // Apply insecure skip verify if configured
                        if config.quic_insecure_skip_verify {
                            builder = builder.insecure_skip_verify(true);
                        }

                        // Apply custom root CA if configured
                        if let Some(ref ca_pem) = config.quic_custom_root_ca_pem {
                            builder =
                                builder
                                    .custom_roots_from_pem(ca_pem.as_bytes())
                                    .map_err(|e| MqttClientError::ConnectionLost {
                                        reason: format!(
                                            "Failed to load custom root CA for QUIC: {}",
                                            e
                                        ),
                                    })?;
                        }

                        // Apply client cert and key for mTLS if configured
                        if let (Some(ref cert_pem), Some(ref key_pem)) =
                            (&config.quic_client_cert_pem, &config.quic_client_key_pem)
                        {
                            builder = builder
                                .client_cert_chain_from_pem(cert_pem.as_bytes())
                                .map_err(|e| MqttClientError::ConnectionLost {
                                    reason: format!(
                                        "Failed to load client certificate for QUIC: {}",
                                        e
                                    ),
                                })?
                                .client_private_key_from_pem(key_pem.as_bytes())
                                .map_err(|e| MqttClientError::ConnectionLost {
                                    reason: format!(
                                        "Failed to load client private key for QUIC: {}",
                                        e
                                    ),
                                })?;
                        }

                        if config.quic_datagram_receive_buffer_size > 0 {
                            builder = builder.datagram_receive_buffer_size(
                                config.quic_datagram_receive_buffer_size,
                            );
                        }

                        if config.quic_enable_key_log {
                            builder = builder.enable_key_log(true);
                        }

                        if let Some(local_bind_addr) = config.quic_local_bind_addr {
                            builder = builder.local_bind_addr(local_bind_addr);
                        }
                        let cfg = builder.build();

                        let transport = QuicTransport::connect_with_config(addr, cfg)
                            .await
                            .map_err(|e| MqttClientError::ConnectionLost {
                                reason: format!("QUIC connection failed to {}: {}", peer, e),
                            })?;
                        Ok(Box::new(transport) as BoxedTransport)
                    }
                    #[cfg(not(feature = "quic"))]
                    {
                        Err(MqttClientError::ProtocolViolation {
                            message: "QUIC transport requires the 'quic' feature".into(),
                        })
                    }
                } else {
                    // Default to TCP for mqtt:// or plain addresses
                    let addr = peer.strip_prefix("mqtt://").unwrap_or(peer);

                    let transport = TcpTransport::connect(addr).await.map_err(|e| {
                        MqttClientError::ConnectionLost {
                            reason: format!("TCP connection failed to {}: {}", peer, e),
                        }
                    })?;

                    Ok(Box::new(transport) as BoxedTransport)
                }
            };
            match config.connect_timeout_ms {
                Some(duration) => tokio::time::timeout(Duration::from_millis(duration), connect)
                    .await
                    .map_err(|_| MqttClientError::OperationTimeout {
                        operation: "transport connect".into(),
                        timeout_ms: duration,
                    })?,
                None => connect.await,
            }
        })
    }
}

fn invalid(field: &str, reason: &str) -> MqttClientError {
    MqttClientError::InvalidConfiguration {
        field: field.into(),
        reason: reason.into(),
    }
}

pub(super) fn resolve_options(
    options: &mut MqttClientOptions,
    config: &mut TokioAsyncClientConfig,
) -> Result<(), MqttClientError> {
    if config.command_queue_size == 0 || config.default_operation_timeout_ms == 0 {
        return Err(invalid(
            "async configuration",
            "Command capacity and default operation timeout must be nonzero",
        ));
    }
    if let Some(maximum) = config.receive_maximum {
        if maximum == 0
            || options
                .incoming_receive_maximum
                .is_some_and(|value| value != maximum)
        {
            return Err(invalid(
                "receive_maximum",
                "Receive Maximum is zero or conflicts with core options",
            ));
        }
        merge_property(options, Property::ReceiveMaximum(maximum))?;
        options.incoming_receive_maximum = Some(maximum);
    }
    if let Some(maximum) = config.topic_alias_maximum {
        merge_property(options, Property::TopicAliasMaximum(maximum))?;
    }
    config.auto_reconnect &= options.reconnect;
    options.reconnect = config.auto_reconnect;
    options.reconnect_max_delay_ms = config.max_reconnect_delay_ms;
    options.max_reconnect_attempts = config.max_reconnect_attempts;
    options.operation_timeouts.connect = options
        .operation_timeouts
        .connect
        .or(config.connect_timeout_ms.map(Duration::from_millis));
    if config.priority_queue_enabled {
        if config.priority_queue_limit == 0 {
            return Err(invalid(
                "priority_queue_limit",
                "Priority queue capacity must be nonzero",
            ));
        }
        options.max_outgoing_packet_count = options
            .max_outgoing_packet_count
            .min(config.priority_queue_limit);
    }
    Ok(())
}

fn merge_property(options: &mut MqttClientOptions, value: Property) -> Result<(), MqttClientError> {
    let mut found = false;
    for property in &options.connect_properties {
        if std::mem::discriminant(property) == std::mem::discriminant(&value) {
            if property != &value {
                return Err(invalid(
                    "CONNECT properties",
                    "Async configuration conflicts with core CONNECT properties",
                ));
            }
            found = true;
        }
    }
    if !found {
        options.connect_properties.push(value);
    }
    Ok(())
}

#[derive(Default)]
enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected(ConnectionResult),
    Disconnecting,
}

#[derive(Default)]
struct Pending {
    connect: Vec<Response<ConnectionResult>>,
    publish: HashMap<u16, Response<PublishResult>>,
    subscribe: HashMap<u16, Response<SubscribeResult>>,
    unsubscribe: HashMap<u16, Response<UnsubscribeResult>>,
    ping: Vec<Response<PingResult>>,
}

impl Pending {
    fn fail_connect(&mut self, error: &MqttClientError) {
        for tx in self.connect.drain(..) {
            let _ = tx.send(Err(error.clone()));
        }
    }

    fn fail_ping(&mut self, error: &MqttClientError) {
        for tx in self.ping.drain(..) {
            let _ = tx.send(Err(error.clone()));
        }
    }

    fn fail_all(&mut self, error: &MqttClientError) {
        self.fail_connect(error);
        self.fail_ping(error);
        for (_, tx) in self.publish.drain() {
            let _ = tx.send(Err(error.clone()));
        }
        for (_, tx) in self.subscribe.drain() {
            let _ = tx.send(Err(error.clone()));
        }
        for (_, tx) in self.unsubscribe.drain() {
            let _ = tx.send(Err(error.clone()));
        }
    }

    fn prune(&mut self) {
        self.connect.retain(|tx| !tx.is_closed());
        self.ping.retain(|tx| !tx.is_closed());
        self.publish.retain(|_, tx| !tx.is_closed());
        self.subscribe.retain(|_, tx| !tx.is_closed());
        self.unsubscribe.retain(|_, tx| !tx.is_closed());
    }

    fn poll_cancelled(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let mut closed = false;
        for tx in &mut self.connect {
            closed |= tx.poll_closed(cx).is_ready();
        }
        for tx in &mut self.ping {
            closed |= tx.poll_closed(cx).is_ready();
        }
        for tx in self.publish.values_mut() {
            closed |= tx.poll_closed(cx).is_ready();
        }
        for tx in self.subscribe.values_mut() {
            closed |= tx.poll_closed(cx).is_ready();
        }
        for tx in self.unsubscribe.values_mut() {
            closed |= tx.poll_closed(cx).is_ready();
        }
        if closed {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

pub(super) struct TokioClientWorker {
    pub(super) engine: MqttEngine,
    pub(super) stream: Option<BoxedTransport>,
    state: ConnectionState,
    connecting: Option<ConnectFuture>,
    intentional_stop: bool,
    pending: Pending,
    event_handler: Box<dyn TokioMqttEventHandler>,
    command_rx: mpsc::Receiver<TokioClientCommand>,
    config: TokioAsyncClientConfig,
}

impl TokioClientWorker {
    pub(super) fn new(
        options: MqttClientOptions,
        event_handler: Box<dyn TokioMqttEventHandler>,
        command_rx: mpsc::Receiver<TokioClientCommand>,
        config: TokioAsyncClientConfig,
    ) -> Self {
        Self {
            engine: MqttEngine::new(options),
            event_handler,
            command_rx,
            config,
            stream: None,
            state: ConnectionState::Disconnected,
            connecting: None,
            intentional_stop: false,
            pending: Pending::default(),
        }
    }

    pub(super) async fn run(mut self) -> Result<(), MqttClientError> {
        let mut read_buffer = [0; 4096];
        loop {
            self.pending.prune();
            let deadline = self.engine.next_tick_at();
            tokio::select! {
                command = self.command_rx.recv() => {
                    match command {
                        Some(TokioClientCommand::Shutdown) | None => {
                            self.command_rx.close();
                            let result = self.disconnect(0, Vec::new()).await;
                            if let Err(error) = &result {
                                self.engine.set_reconnect(false);
                                self.connecting = None;
                                self.stream = None;
                                self.pending.fail_all(error);
                                if !matches!(self.state, ConnectionState::Disconnected) {
                                    self.event_handler.on_disconnected(None).await;
                                }
                            }
                            return result;
                        }
                        Some(command) => self.handle_command(command).await,
                    }
                }
                result = async {
                    match &mut self.connecting {
                        Some(connecting) => connecting.await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.connecting = None;
                    self.transport_connected(result).await;
                }
                result = async {
                    match &mut self.stream {
                        Some(stream) => stream.read(&mut read_buffer).await,
                        None => std::future::pending().await,
                    }
                } => {
                    match result {
                        Ok(len) if len > 0 => {
                            let events = self.engine.handle_incoming(&read_buffer[..len]);
                            self.dispatch_events(events).await;
                        }
                        result => {
                            let error = match result {
                                Err(error) => MqttClientError::from_io_error(error, "transport read"),
                                _ => MqttClientError::ConnectionLost { reason: "Connection closed by server".into() },
                            };
                            self.event_handler.on_error(&error).await;
                            self.connection_lost(None).await;
                        }
                    }
                }
                _ = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                        None => std::future::pending().await,
                    }
                } => {
                    let events = self.engine.handle_tick(Instant::now());
                    self.dispatch_events(events).await;
                }
                _ = poll_fn(|cx| self.pending.poll_cancelled(cx)) => {}
            }
            if let Err(error) = self.handle_outgoing().await {
                self.event_handler.on_error(&error).await;
                self.connection_lost(None).await;
            }
        }
    }

    fn start_connect(&mut self) {
        if !matches!(self.state, ConnectionState::Disconnected) {
            return;
        }
        self.intentional_stop = false;
        self.engine.set_reconnect(self.config.auto_reconnect);
        self.engine.reset_reconnect_state();
        self.state = ConnectionState::Connecting;
        self.connecting = Some(self.create_transport());
    }

    async fn transport_connected(&mut self, result: Result<BoxedTransport, MqttClientError>) {
        let result = match result {
            Ok(transport) => {
                if self.config.tcp_nodelay {
                    if let Err(error) = transport.set_nodelay(true) {
                        self.event_handler
                            .on_error(&MqttClientError::NetworkError {
                                kind: io::ErrorKind::Other,
                                message: format!("TCP_NODELAY: {error}"),
                            })
                            .await;
                    }
                }
                self.stream = Some(transport);
                self.engine.reset_for_new_transport();
                self.engine.connect()
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.pending.fail_connect(&error);
            self.event_handler.on_error(&error).await;
            self.connection_lost(None).await;
        }
    }

    pub(super) async fn dispatch_events(&mut self, events: Vec<MqttEvent>) {
        for event in events {
            match event {
                MqttEvent::Connected(result) => {
                    self.state = ConnectionState::Connected(result.clone());
                    for tx in self.pending.connect.drain(..) {
                        let _ = tx.send(Ok(result.clone()));
                    }
                    self.event_handler.on_connected(&result).await;
                    if !result.is_success() {
                        self.connection_lost(Some(result.reason_code)).await;
                    }
                }
                MqttEvent::Disconnected(reason) => self.connection_lost(reason).await,
                MqttEvent::DisconnectReceived {
                    reason_code,
                    properties,
                } => {
                    self.event_handler
                        .on_disconnect_received(reason_code, &properties)
                        .await;
                    self.connection_lost(Some(reason_code)).await;
                }
                MqttEvent::Published(result) => {
                    if let Some(tx) = result
                        .packet_id
                        .and_then(|id| self.pending.publish.remove(&id))
                    {
                        let _ = tx.send(Ok(result.clone()));
                    }
                    self.event_handler.on_published(&result).await;
                }
                MqttEvent::Subscribed(result) => {
                    if let Some(tx) = self.pending.subscribe.remove(&result.packet_id) {
                        let _ = tx.send(Ok(result.clone()));
                    }
                    self.event_handler.on_subscribed(&result).await;
                }
                MqttEvent::Unsubscribed(result) => {
                    if let Some(tx) = self.pending.unsubscribe.remove(&result.packet_id) {
                        let _ = tx.send(Ok(result.clone()));
                    }
                    self.event_handler.on_unsubscribed(&result).await;
                }
                MqttEvent::MessageReceived(message) => {
                    self.event_handler.on_message_received(&message).await
                }
                MqttEvent::PubRelReceived { packet_id, .. } => {
                    self.event_handler.on_pubrel_received(packet_id).await
                }
                MqttEvent::PingResponse(result) => {
                    for tx in self.pending.ping.drain(..) {
                        let _ = tx.send(Ok(result.clone()));
                    }
                    self.event_handler.on_ping_response(&result).await;
                }
                MqttEvent::AuthReceived(result) => {
                    self.event_handler.on_auth_received(&result).await
                }
                MqttEvent::OperationFailed {
                    operation,
                    packet_id,
                    error,
                } => {
                    match operation {
                        OperationKind::Connect => self.pending.fail_connect(&error),
                        OperationKind::Publish => {
                            if let Some(tx) =
                                packet_id.and_then(|id| self.pending.publish.remove(&id))
                            {
                                let _ = tx.send(Err(error.clone()));
                            }
                        }
                        OperationKind::Subscribe => {
                            if let Some(tx) =
                                packet_id.and_then(|id| self.pending.subscribe.remove(&id))
                            {
                                let _ = tx.send(Err(error.clone()));
                            }
                        }
                        OperationKind::Unsubscribe => {
                            if let Some(tx) =
                                packet_id.and_then(|id| self.pending.unsubscribe.remove(&id))
                            {
                                let _ = tx.send(Err(error.clone()));
                            }
                        }
                    }
                    self.event_handler.on_error(&error).await;
                    if operation == OperationKind::Connect {
                        self.connection_lost(None).await;
                    }
                }
                MqttEvent::Error(error) => {
                    if matches!(self.state, ConnectionState::Connecting) {
                        self.pending.fail_connect(&error);
                    }
                    self.event_handler.on_error(&error).await;
                }
                MqttEvent::ReconnectNeeded
                    if self.config.auto_reconnect && !self.intentional_stop =>
                {
                    if self.stream.is_some() {
                        self.connection_lost(None).await;
                    } else if matches!(self.state, ConnectionState::Disconnected) {
                        // Preserve retry counters across attempts; CONNACK resets them.
                        self.state = ConnectionState::Connecting;
                        self.connecting = Some(self.create_transport());
                    }
                }
                MqttEvent::ReconnectScheduled { attempt, .. }
                    if !self.intentional_stop && self.config.auto_reconnect =>
                {
                    self.event_handler.on_reconnect_attempt(attempt).await;
                }
                // Full messages carry publish metadata. This wrapper owns one byte-stream transport.
                MqttEvent::PublishReceived { .. }
                | MqttEvent::TransportClosed { .. }
                | MqttEvent::StreamClosed { .. }
                | MqttEvent::StreamReset { .. }
                | MqttEvent::StreamStopped { .. }
                | MqttEvent::ZeroRttStatusChanged { .. }
                | MqttEvent::ReconnectNeeded
                | MqttEvent::ReconnectScheduled { .. } => {}
            }
        }
    }

    async fn write_output(&mut self) -> Result<(), MqttClientError> {
        let bytes = self.engine.take_outgoing();
        if bytes.is_empty() {
            return Ok(());
        }
        let stream = self.stream.as_mut().ok_or(MqttClientError::NotConnected)?;
        let duration = self.config.default_operation_timeout_ms;
        tokio::time::timeout(Duration::from_millis(duration), stream.write_all(&bytes))
            .await
            .map_err(|_| MqttClientError::OperationTimeout {
                operation: "transport write".into(),
                timeout_ms: duration,
            })?
            .map_err(|error| MqttClientError::from_io_error(error, "transport write"))
    }

    pub(super) async fn handle_outgoing(&mut self) -> Result<(), MqttClientError> {
        let result = self.write_output().await;
        let events = self.engine.take_events();
        self.dispatch_events(events).await;
        result
    }

    fn publish(&mut self, mut command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        if !self.engine.is_connected() {
            if !self.config.buffer_messages || self.intentional_stop {
                return Err(MqttClientError::NotConnected);
            }
            if self.engine.pending_publish_count() >= self.config.max_buffer_size {
                return Err(MqttClientError::BufferFull {
                    buffer_type: "offline publish queue".into(),
                    capacity: self.config.max_buffer_size,
                });
            }
        }
        if !self.config.priority_queue_enabled {
            command.priority = 128;
        }
        self.engine.publish(command)
    }

    fn acknowledge(
        &mut self,
        kind: u8,
        id: u16,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        match kind {
            4 => self.engine.puback(id, reason, properties),
            5 => self.engine.pubrec(id, reason, properties),
            7 => self.engine.pubcomp(id, reason, properties),
            _ => unreachable!("internal acknowledgement kind"),
        }
    }

    fn ping(&mut self) -> Result<(), MqttClientError> {
        if !self.engine.is_connected() {
            return Err(MqttClientError::NotConnected);
        }
        self.engine.send_ping()
    }

    async fn handle_command(&mut self, command: TokioClientCommand) {
        let result = match command {
            TokioClientCommand::Connect => {
                self.start_connect();
                Ok(())
            }
            TokioClientCommand::ConnectSync { response_tx } => {
                if let ConnectionState::Connected(result) = &self.state {
                    let _ = response_tx.send(Ok(result.clone()));
                } else {
                    self.pending.connect.push(response_tx);
                    self.start_connect();
                }
                Ok(())
            }
            TokioClientCommand::Publish(command) => self.publish(command).map(|_| ()),
            TokioClientCommand::PublishSync {
                command,
                response_tx,
            } => {
                match self.publish(command) {
                    Ok(Some(id)) => {
                        self.pending.publish.insert(id, response_tx);
                    }
                    Ok(None) => {
                        let _ = response_tx.send(Ok(PublishResult {
                            packet_id: None,
                            reason_code: None,
                            properties: None,
                            qos: 0,
                        }));
                    }
                    Err(error) => {
                        let _ = response_tx.send(Err(error.clone()));
                        self.event_handler.on_error(&error).await;
                    }
                }
                Ok(())
            }
            TokioClientCommand::Subscribe(command) => self.engine.subscribe(command).map(|_| ()),
            TokioClientCommand::SubscribeSync {
                command,
                response_tx,
            } => {
                match self.engine.subscribe(command) {
                    Ok(id) => {
                        self.pending.subscribe.insert(id, response_tx);
                    }
                    Err(error) => {
                        let _ = response_tx.send(Err(error.clone()));
                        self.event_handler.on_error(&error).await;
                    }
                }
                Ok(())
            }
            TokioClientCommand::Unsubscribe(command) => {
                self.engine.unsubscribe(command).map(|_| ())
            }
            TokioClientCommand::UnsubscribeSync {
                command,
                response_tx,
            } => {
                match self.engine.unsubscribe(command) {
                    Ok(id) => {
                        self.pending.unsubscribe.insert(id, response_tx);
                    }
                    Err(error) => {
                        let _ = response_tx.send(Err(error.clone()));
                        self.event_handler.on_error(&error).await;
                    }
                }
                Ok(())
            }
            TokioClientCommand::Ping => self.ping(),
            TokioClientCommand::PingSync { response_tx } => {
                if self.pending.ping.is_empty() {
                    if let Err(error) = self.ping() {
                        let _ = response_tx.send(Err(error.clone()));
                        self.event_handler.on_error(&error).await;
                        return;
                    }
                }
                self.pending.ping.push(response_tx);
                Ok(())
            }
            TokioClientCommand::Acknowledge {
                kind,
                packet_id,
                reason_code,
                properties,
                response_tx,
            } => {
                let result = self.acknowledge(kind, packet_id, reason_code, properties);
                let _ = response_tx.send(result.clone());
                result
            }
            TokioClientCommand::Disconnect {
                reason_code,
                properties,
                response_tx,
            } => {
                let result = self.disconnect(reason_code, properties).await;
                if let Some(tx) = response_tx {
                    let _ = tx.send(result.clone());
                }
                result
            }
            TokioClientCommand::SetAutoReconnect { enabled } => {
                self.config.auto_reconnect = enabled;
                self.engine.set_reconnect(enabled && !self.intentional_stop);
                if !enabled && matches!(self.state, ConnectionState::Disconnected) {
                    self.pending.fail_all(&MqttClientError::NotConnected);
                } else if matches!(self.state, ConnectionState::Disconnected)
                    && !self.intentional_stop
                {
                    self.engine.schedule_reconnect(Instant::now());
                }
                Ok(())
            }
            TokioClientCommand::Auth {
                reason_code,
                properties,
            } => self.engine.auth(reason_code, properties),
            TokioClientCommand::SendPacket(packet) => match packet {
                MqttPacket::PubAck5(p) => {
                    self.engine.puback(p.packet_id, p.reason_code, p.properties)
                }
                MqttPacket::PubRec5(p) => {
                    self.engine.pubrec(p.packet_id, p.reason_code, p.properties)
                }
                MqttPacket::PubComp5(p) => {
                    self.engine
                        .pubcomp(p.packet_id, p.reason_code, p.properties)
                }
                MqttPacket::PubAck3(p) => self.engine.puback(p.message_id, 0, Vec::new()),
                MqttPacket::PubRec3(p) => self.engine.pubrec(p.message_id, 0, Vec::new()),
                MqttPacket::PubComp3(p) => self.engine.pubcomp(p.message_id, 0, Vec::new()),
                MqttPacket::Disconnect5(p) => self.disconnect(p.reason_code, p.properties).await,
                MqttPacket::Disconnect3(_) => self.disconnect(0, Vec::new()).await,
                MqttPacket::Auth(p) => self.engine.auth(p.reason_code, p.properties),
                packet => self.engine.enqueue_packet(packet),
            },
            TokioClientCommand::Shutdown => unreachable!("shutdown is handled by the run loop"),
        };
        if let Err(error) = result {
            self.event_handler.on_error(&error).await;
        }
    }

    async fn disconnect(
        &mut self,
        reason: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.engine.try_disconnect_with(reason, properties)?;
        let active = !matches!(self.state, ConnectionState::Disconnected);
        self.state = ConnectionState::Disconnecting;
        self.intentional_stop = true;
        self.engine.set_reconnect(false);
        self.connecting = None;
        self.pending.fail_all(&MqttClientError::OperationCancelled {
            operation: "disconnect".into(),
        });
        let write = self.write_output().await;
        let close = if let Some(mut stream) = self.stream.take() {
            let duration = self.config.default_operation_timeout_ms;
            tokio::time::timeout(Duration::from_millis(duration), stream.close())
                .await
                .map_err(|_| MqttClientError::OperationTimeout {
                    operation: "transport close".into(),
                    timeout_ms: duration,
                })
                .and_then(|result| {
                    result.map_err(|error| MqttClientError::NetworkError {
                        kind: io::ErrorKind::Other,
                        message: error.to_string(),
                    })
                })
        } else {
            Ok(())
        };
        self.engine.reset_for_new_transport();
        self.state = ConnectionState::Disconnected;
        if active {
            self.event_handler.on_disconnected(Some(reason)).await;
        }
        write.and(close)
    }

    async fn connection_lost(&mut self, reason: Option<u8>) {
        if matches!(self.state, ConnectionState::Disconnected) {
            return;
        }
        self.stream = None;
        self.connecting = None;
        self.state = ConnectionState::Disconnected;
        self.engine.handle_connection_lost();
        let error = MqttClientError::ConnectionLost {
            reason: "MQTT transport closed".into(),
        };
        self.pending.fail_connect(&error);
        self.pending.fail_ping(&error);
        if !self.config.auto_reconnect
            || self.intentional_stop
            || self.engine.next_tick_at().is_none()
        {
            self.pending.fail_all(&error);
        }
        self.event_handler.on_disconnected(reason).await;
        self.event_handler.on_connection_lost().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_client::transport::TransportError;
    use crate::mqtt_serde::mqttv5::publishv5::MqttPublish;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    struct Handler(Arc<AtomicUsize>);

    #[async_trait]
    impl TokioMqttEventHandler for Handler {
        async fn on_disconnected(&mut self, _: Option<u8>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn connected_worker(
        options: MqttClientOptions,
    ) -> (
        TokioClientWorker,
        mpsc::Sender<TokioClientCommand>,
        Arc<AtomicUsize>,
    ) {
        let disconnected = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel(4);
        let mut worker = TokioClientWorker::new(
            options,
            Box::new(Handler(disconnected.clone())),
            rx,
            TokioAsyncClientConfig::builder()
                .default_operation_timeout_ms(20)
                .build(),
        );
        worker.engine.connect().unwrap();
        worker.engine.take_outgoing();
        let bytes: &[u8] = if worker.engine.mqtt_version() == 5 {
            &[0x20, 3, 0, 0, 0]
        } else {
            &[0x20, 2, 0, 0]
        };
        let events = worker.engine.handle_incoming(bytes);
        worker.dispatch_events(events).await;
        (worker, tx, disconnected)
    }

    async fn acknowledge(worker: &mut TokioClientWorker, kind: u8) -> Result<(), MqttClientError> {
        let (tx, rx) = oneshot::channel();
        worker
            .handle_command(TokioClientCommand::Acknowledge {
                kind,
                packet_id: 42,
                reason_code: 0,
                properties: Vec::new(),
                response_tx: tx,
            })
            .await;
        rx.await.unwrap()
    }

    #[tokio::test]
    async fn manual_ack_can_retry_after_output_buffer_rejection() {
        for version in [3, 4, 5] {
            for kind in [4, 5, 7] {
                let (mut worker, _tx, _) = connected_worker(
                    MqttClientOptions::builder()
                        .mqtt_version(version)
                        .auto_ack(false)
                        .keep_alive(0)
                        .max_outgoing_packet_count(1)
                        .build(),
                )
                .await;
                let mut packet = MqttPacket::Publish5(MqttPublish::new(
                    if kind == 4 { 1 } else { 2 },
                    "t".into(),
                    Some(42),
                    vec![],
                    false,
                    false,
                ));
                if version != 5 {
                    packet = MqttPacket::Publish3(
                        crate::mqtt_serde::mqttv3::publishv3::MqttPublish::new(
                            "t".into(),
                            if kind == 4 { 1 } else { 2 },
                            vec![],
                            Some(42),
                            false,
                            false,
                        ),
                    );
                }
                worker.engine.handle_incoming(&packet.to_bytes().unwrap());
                if kind == 7 {
                    acknowledge(&mut worker, 5).await.unwrap();
                    worker.engine.take_outgoing();
                    worker.engine.handle_incoming(&[0x62, 2, 0, 42]);
                }
                worker.engine.send_ping().unwrap();
                assert!(matches!(
                    acknowledge(&mut worker, kind).await,
                    Err(MqttClientError::BufferFull { .. })
                ));
                assert_eq!(worker.engine.take_outgoing(), [0xc0, 0]);
                acknowledge(&mut worker, kind).await.unwrap();
                let bytes = worker.engine.take_outgoing();
                assert_eq!(bytes[0], kind << 4);
                if kind == 5 {
                    worker.engine.handle_incoming(&[0x62, 2, 0, 42]);
                    acknowledge(&mut worker, 7).await.unwrap();
                } else {
                    assert!(acknowledge(&mut worker, kind).await.is_err());
                }
            }
        }
    }

    struct StalledTransport {
        write: bool,
        close: bool,
        dropped: Arc<AtomicBool>,
    }

    impl Drop for StalledTransport {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl AsyncRead for StalledTransport {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for StalledTransport {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.write {
                Poll::Pending
            } else {
                Poll::Ready(Ok(bytes.len()))
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.close {
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        }
    }

    #[async_trait]
    impl Transport for StalledTransport {
        async fn connect(_: &str) -> Result<Self, TransportError> {
            unreachable!()
        }

        async fn close(&mut self) -> Result<(), TransportError> {
            self.shutdown().await.map_err(TransportError::from)
        }

        fn peer_addr(&self) -> Result<String, TransportError> {
            Ok("test".into())
        }

        fn local_addr(&self) -> Result<String, TransportError> {
            Ok("test".into())
        }
    }

    #[tokio::test]
    async fn shutdown_releases_transport_and_waiters_when_io_stalls_or_buffer_is_full() {
        for failure in ["transport write", "transport close", "buffer full"] {
            let (mut worker, tx, disconnected) = connected_worker(
                MqttClientOptions::builder()
                    .keep_alive(0)
                    .max_outgoing_packet_count(1)
                    .build(),
            )
            .await;
            let dropped = Arc::new(AtomicBool::new(false));
            worker.stream = Some(Box::new(StalledTransport {
                write: failure == "transport write",
                close: failure == "transport close",
                dropped: dropped.clone(),
            }));
            let (reply, result) = oneshot::channel();
            worker
                .handle_command(TokioClientCommand::PublishSync {
                    command: PublishCommand::simple("t", vec![], 1, false),
                    response_tx: reply,
                })
                .await;
            if failure != "buffer full" {
                worker.engine.take_outgoing();
            }
            tx.send(TokioClientCommand::Shutdown).await.unwrap();
            let error = tokio::time::timeout(Duration::from_secs(1), worker.run())
                .await
                .expect("shutdown must finish even when I/O cannot progress")
                .unwrap_err();
            if failure == "buffer full" {
                assert!(matches!(error, MqttClientError::BufferFull { .. }));
            } else {
                assert!(matches!(
                    error, MqttClientError::OperationTimeout { operation, .. } if operation == failure
                ));
            }
            assert!(result.await.unwrap().is_err());
            assert!(dropped.load(Ordering::SeqCst));
            assert!(tx.is_closed());
            assert_eq!(disconnected.load(Ordering::SeqCst), 1);
        }
    }
}
