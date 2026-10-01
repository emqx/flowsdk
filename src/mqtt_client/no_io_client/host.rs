// SPDX-License-Identifier: MPL-2.0

use super::*;

impl NoIoMqttClient<Instant> {
    /// Create a new Sans-I/O MQTT client with the given options.
    ///
    /// # Arguments
    ///
    /// * `options` - MQTT client configuration
    ///
    /// # Example
    ///
    /// ```no_run
    /// use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    ///
    /// let options = MqttClientOptions::builder()
    ///     .peer("broker.example.com:1883")
    ///     .client_id("my_client")
    ///     .keep_alive(60)
    ///     .clean_start(true)
    ///     .build();
    ///
    /// let client = NoIoMqttClient::new(options);
    /// ```
    pub fn new(options: MqttClientOptions) -> Self {
        Self {
            engine: MqttEngine::new(options),
        }
    }

    // ========================================================================
    // I/O Interface
    // ========================================================================

    /// Process incoming bytes from the network.
    ///
    /// Feed raw bytes received from your socket into this method. The client will
    /// parse MQTT packets and return a list of events.
    ///
    /// # Arguments
    ///
    /// * `data` - Raw bytes from the network
    ///
    /// # Returns
    ///
    /// A vector of [`MqttEvent`]s generated from the incoming data.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, MqttEvent};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// // Bytes received from socket
    /// let incoming_bytes = vec![0x20, 0x02, 0x00, 0x00]; // CONNACK
    ///
    /// let events = client.handle_incoming(&incoming_bytes);
    /// for event in events {
    ///     match event {
    ///         MqttEvent::Connected(result) => {
    ///             println!("Connected! Reason code: {}", result.reason_code);
    ///         }
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub fn handle_incoming(&mut self, data: &[u8]) -> Vec<MqttEvent> {
        self.engine.handle_incoming(data)
    }

    /// Take outgoing bytes to be written to the network.
    ///
    /// Call this method after any operation that might generate outgoing packets
    /// (connect, publish, subscribe, etc.) and write the returned bytes to your socket.
    ///
    /// # Returns
    ///
    /// A vector of bytes to send to the network. Returns an empty vector if there's
    /// nothing to send.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.connect().unwrap();
    ///
    /// let outgoing = client.take_outgoing();
    /// if !outgoing.is_empty() {
    ///     // Write to your socket
    ///     // socket.write_all(&outgoing)?;
    /// }
    /// ```
    pub fn take_outgoing(&mut self) -> Vec<u8> {
        self.engine.take_outgoing()
    }

    /// Process protocol timer ticks for keep-alive and retransmissions.
    ///
    /// Call this method periodically based on the time returned by [`next_tick_at()`](Self::next_tick_at).
    /// This handles:
    ///
    /// - Keep-alive PING packets
    /// - Connection timeout detection
    /// - Opt-in operation deadlines and MQTT 3.1.1 retransmissions
    ///
    /// # Arguments
    ///
    /// * `now` - Current time
    ///
    /// # Returns
    ///
    /// A vector of [`MqttEvent`]s generated (e.g., `ReconnectNeeded` when backoff expires).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # use std::time::Instant;
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let now = Instant::now();
    ///
    /// if let Some(next_tick) = client.next_tick_at() {
    ///     if now >= next_tick {
    ///         let events = client.handle_tick(now);
    ///         // Process events...
    ///     }
    /// }
    /// ```
    pub fn handle_tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        self.engine.handle_tick(now)
    }

    // ========================================================================
    // MQTT Operations
    // ========================================================================

    /// Initiate a connection to the MQTT broker.
    ///
    /// This enqueues a CONNECT packet. Call [`take_outgoing()`](Self::take_outgoing)
    /// to get the bytes to send to the network.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.connect().unwrap();
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn connect(&mut self) -> Result<(), MqttClientError> {
        self.engine.connect()
    }

    /// Publish a message to a topic.
    ///
    /// # Arguments
    ///
    /// * `command` - Publish command with topic, payload, QoS, etc.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(packet_id))` for QoS 1/2 messages
    /// - `Ok(None)` for QoS 0 messages
    /// - `Err(...)` if not connected or packet ID allocation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, PublishCommand};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let cmd = PublishCommand::builder()
    ///     .topic("sensors/temperature")
    ///     .payload(b"23.5")
    ///     .qos(1)
    ///     .build()
    ///     .unwrap();
    ///
    /// match client.publish(cmd) {
    ///     Ok(Some(packet_id)) => println!("Published with ID: {}", packet_id),
    ///     Ok(None) => println!("Published (QoS 0)"),
    ///     Err(e) => eprintln!("Publish failed: {}", e),
    /// }
    ///
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn publish(&mut self, command: PublishCommand) -> Result<Option<u16>, MqttClientError> {
        self.engine.publish(command)
    }

    /// Subscribe to one or more topics.
    ///
    /// # Arguments
    ///
    /// * `command` - Subscribe command with topics and QoS levels
    ///
    /// # Returns
    ///
    /// - `Ok(packet_id)` on success
    /// - `Err(...)` if not connected or packet ID allocation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, SubscribeCommand};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let cmd = SubscribeCommand::builder()
    ///     .add_topic("sensors/+/temperature", 1)
    ///     .add_topic("alerts/#", 2)
    ///     .build()
    ///     .unwrap();
    ///
    /// match client.subscribe(cmd) {
    ///     Ok(packet_id) => println!("Subscribe packet ID: {}", packet_id),
    ///     Err(e) => eprintln!("Subscribe failed: {}", e),
    /// }
    ///
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn subscribe(&mut self, command: SubscribeCommand) -> Result<u16, MqttClientError> {
        self.engine.subscribe(command)
    }

    /// Unsubscribe from one or more topics.
    ///
    /// # Arguments
    ///
    /// * `command` - Unsubscribe command with topics
    ///
    /// # Returns
    ///
    /// - `Ok(packet_id)` on success
    /// - `Err(...)` if not connected or packet ID allocation fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions, UnsubscribeCommand};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// let cmd = UnsubscribeCommand::from_topics(vec!["sensors/+/temperature".to_string()]);
    ///
    /// match client.unsubscribe(cmd) {
    ///     Ok(packet_id) => println!("Unsubscribe packet ID: {}", packet_id),
    ///     Err(e) => eprintln!("Unsubscribe failed: {}", e),
    /// }
    ///
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn unsubscribe(&mut self, command: UnsubscribeCommand) -> Result<u16, MqttClientError> {
        self.engine.unsubscribe(command)
    }

    /// Send a PINGREQ packet to the broker.
    ///
    /// Normally you don't need to call this manually as [`handle_tick()`](Self::handle_tick)
    /// will send pings automatically based on the keep-alive interval.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.ping().unwrap();
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn ping(&mut self) -> Result<(), MqttClientError> {
        self.engine.send_ping()
    }

    /// Send a DISCONNECT packet to the broker.
    ///
    /// This gracefully closes the connection. After calling this, you should
    /// send the outgoing bytes and close your socket.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// client.disconnect().unwrap();
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker, then close socket
    /// ```
    pub fn disconnect(&mut self) -> Result<(), MqttClientError> {
        self.engine.disconnect()
    }

    /// Send an AUTH packet for enhanced authentication (MQTT v5 only).
    ///
    /// Used for multi-step authentication flows like SCRAM, OAuth, etc.
    ///
    /// # Arguments
    ///
    /// * `reason_code` - Authentication reason code
    /// * `properties` - Authentication properties
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// // Send authentication response
    /// client.auth(0x18, vec![/* auth properties */]).unwrap();
    /// let bytes = client.take_outgoing();
    /// // Send `bytes` to broker...
    /// ```
    pub fn auth(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.engine.auth(reason_code, properties)
    }

    /// Discard old transport bytes and aliases while retaining eligible session state.
    /// Call before CONNECT on a fresh transport.
    pub fn reset_for_new_transport(&mut self) {
        self.engine.reset_for_new_transport();
    }

    /// Schedule a backoff deadline unless one is already pending.
    pub fn schedule_reconnect(&mut self, now: Instant) {
        self.engine.schedule_reconnect(now);
    }

    /// Queue DISCONNECT with MQTT 5 reason/properties; errors leave state unchanged.
    pub fn disconnect_with(
        &mut self,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.engine.try_disconnect_with(reason_code, properties)
    }

    /// Acknowledge a received QoS 1 message when automatic acknowledgments are disabled.
    /// On failure, receive state remains available for retry.
    pub fn puback(
        &mut self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.engine.puback(packet_id, reason_code, properties)
    }

    /// Accept a received QoS 2 message. Use reason zero and no properties for MQTT 3.
    /// On failure, receive state remains available for retry.
    pub fn pubrec(
        &mut self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.engine.pubrec(packet_id, reason_code, properties)
    }

    /// Complete a QoS 2 exchange after receiving `PubRelReceived`.
    /// On failure, receive state remains available for retry.
    pub fn pubcomp(
        &mut self,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    ) -> Result<(), MqttClientError> {
        self.engine.pubcomp(packet_id, reason_code, properties)
    }

    // ========================================================================
    // Connection Management
    // ========================================================================

    /// Handle connection lost state.
    ///
    /// Call this when your socket disconnects or encounters an error.
    /// Discards transport bytes and retains eligible MQTT session state. When
    /// reconnection is enabled, schedules one retry using exponential backoff.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::{NoIoMqttClient, MqttClientOptions};
    /// # let mut client = NoIoMqttClient::new(MqttClientOptions::default());
    /// // Socket error detected
    /// client.handle_connection_lost();
    ///
    /// // Later, reconnect
    /// // ... create new socket ...
    /// client.connect().unwrap();
    /// ```
    pub fn handle_connection_lost(&mut self) {
        self.engine.handle_connection_lost();
    }
}
