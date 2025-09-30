use crate::mqtt_serde::mqttv3::puback;
use crate::mqtt_session::ClientSession;

use crate::mqtt_serde::control_packet::MqttControlPacket;
use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::common::properties::Property;
use crate::mqtt_serde::mqttv5::connectv5;
use crate::mqtt_serde::mqttv5::disconnectv5;
use crate::mqtt_serde::mqttv5::pingreqv5;
use crate::mqtt_serde::mqttv5::publishv5;
use crate::mqtt_serde::mqttv5::pubrelv5;
use crate::mqtt_serde::mqttv5::subscribev5;
use crate::mqtt_serde::mqttv5::unsubscribev5;

use crate::mqtt_serde::MqttStream;

use super::MqttClientOptions;
use std::collections::HashMap;
use std::io;
use std::net::TcpStream;

pub struct Subscription {
    pub topic: String,
    pub qos: u8,
}

#[derive(Debug, Clone)]
pub struct ConnectionResult {
    pub reason_code: u8,
    pub session_present: bool,
    pub properties: Option<Vec<Property>>,
}

impl ConnectionResult {
    /// Returns true if the connection was successful (reason code 0)
    pub fn is_success(&self) -> bool {
        self.reason_code == 0
    }

    /// Returns true if the connection failed
    pub fn is_failure(&self) -> bool {
        self.reason_code != 0
    }

    /// Returns a description of the reason code
    pub fn reason_description(&self) -> &'static str {
        match self.reason_code {
            0 => "Success",
            1 => "Unspecified error",
            2 => "Malformed Packet",
            3 => "Protocol Error",
            4 => "Implementation specific error",
            5 => "Unsupported Protocol Version",
            6 => "Client Identifier not valid",
            7 => "Bad User Name or Password",
            8 => "Not authorized",
            9 => "Server unavailable",
            10 => "Server busy",
            11 => "Banned",
            12 => "Bad authentication method",
            13 => "Keep Alive timeout",
            14 => "Session taken over",
            15 => "Topic Filter invalid",
            16 => "Topic Name invalid",
            17 => "Packet identifier in use",
            18 => "Packet Identifier not found",
            19 => "Receive Maximum exceeded",
            20 => "Topic Alias invalid",
            21 => "Packet too large",
            22 => "Message rate too high",
            23 => "Quota exceeded",
            24 => "Administrative action",
            25 => "Payload format invalid",
            26 => "Retain not supported",
            27 => "QoS not supported",
            28 => "Use another server",
            29 => "Server moved",
            30 => "Shared Subscriptions not supported",
            31 => "Connection rate exceeded",
            32 => "Maximum connect time",
            33 => "Subscription Identifiers not supported",
            34 => "Wildcard Subscriptions not supported",
            _ => "Unknown reason code",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubscribeResult {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<Property>,
}

impl SubscribeResult {
    pub fn is_success(&self) -> bool {
        self.reason_codes.iter().all(|&code| code <= 2) // 0, 1, 2 are success codes
    }

    pub fn successful_subscriptions(&self) -> usize {
        self.reason_codes.iter().filter(|&&code| code <= 2).count()
    }
}

#[derive(Debug, Clone)]
pub struct UnsubscribeResult {
    pub packet_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Vec<Property>,
}

impl UnsubscribeResult {
    pub fn is_success(&self) -> bool {
        self.reason_codes
            .iter()
            .all(|&code| code == 0 || code == 17) // 0 = Success, 17 = No subscription existed
    }
}

#[derive(Debug, Clone)]
pub struct PublishResult {
    pub packet_id: Option<u16>,
    pub reason_code: Option<u8>, // None for QoS 0
    pub properties: Option<Vec<Property>>,
    pub qos: u8,
}

impl PublishResult {
    pub fn is_success(&self) -> bool {
        self.reason_code.map_or(true, |code| code == 0) // QoS 0 or reason code 0
    }
}

#[derive(Debug, Clone)]
pub struct PingResult {
    // PINGRESP has no variable header or payload, just the fact that we received it
    pub success: bool,
}

pub struct Context {
    peer: String,
    session: Option<ClientSession>,
    mqtt_stream: Option<MqttStream<TcpStream>>,
    // Update when subscribed to topics and session is not None
    subscribed_topics: Vec<Subscription>,
    session_present: bool,
    mqtt_buffer: Vec<MqttPacket>,
    // Track pending operations for fire-and-forget methods
    pending_subscribes: std::collections::HashMap<u16, String>, // packet_id -> topic
    pending_unsubscribes: std::collections::HashMap<u16, Vec<String>>, // packet_id -> topics
    pending_publishes: std::collections::HashMap<u16, (String, u8)>, // packet_id -> (topic, qos)

    // Track unhandled incoming packet during a blocking call
    unhandled_packets: Vec<MqttPacket>,
}

impl Context {
    pub fn new(peer: String) -> Self {
        Context {
            peer,
            session: None,
            session_present: false,
            subscribed_topics: Vec::new(),
            mqtt_stream: None,
            mqtt_buffer: Vec::new(),
            pending_subscribes: HashMap::new(),
            pending_unsubscribes: HashMap::new(),
            pending_publishes: HashMap::new(),
            unhandled_packets: Vec::new(),
        }
    }

    pub fn new_with_sess(peer: String, session: ClientSession) -> Self {
        Context {
            peer,
            session: Some(session),
            session_present: false,
            subscribed_topics: Vec::new(),
            mqtt_stream: None,
            mqtt_buffer: Vec::new(),
            pending_subscribes: HashMap::new(),
            pending_unsubscribes: HashMap::new(),
            pending_publishes: HashMap::new(),
            unhandled_packets: Vec::new(),
        }
    }
}

pub struct MqttClient {
    context: Context,
    options: MqttClientOptions,
}

impl MqttClient {
    pub fn new(options: MqttClientOptions) -> Self {
        MqttClient {
            context: Context::new(options.peer.clone()),
            options,
        }
    }

    pub fn new_with_sess(options: MqttClientOptions, session: ClientSession) -> Self {
        MqttClient {
            context: Context::new_with_sess(options.peer.clone(), session),
            options,
        }
    }

    // methods for unhandled packets
    pub fn unhandled_packets_mut(&mut self) -> &mut Vec<MqttPacket> {
        &mut self.context.unhandled_packets
    }

    pub fn peek_unhandled_packets(&self) -> &Vec<MqttPacket> {
        &self.context.unhandled_packets
    }

    pub fn pop_unhandled_packet(&mut self) -> Option<MqttPacket> {
        if !self.context.unhandled_packets.is_empty() {
            Some(self.context.unhandled_packets.remove(0))
        } else {
            None
        }
    }

    pub fn clear_unhandled_packets(&mut self) {
        self.context.unhandled_packets.clear();
    }

    // Connect to the MQTT broker and wait for CONNACK
    pub fn connected(&mut self) -> io::Result<ConnectionResult> {
        // Establish connection to the MQTT broker
        if let Ok(stream) = TcpStream::connect(self.context.peer.clone()) {
            // Connection established

            // Initialize ClientSession
            if self.options.sessionless {
                self.context.session = None;
                self.options.clean_start = true;
            } else if self.context.session.is_none() {
                self.context.session = Some(ClientSession::new());
            }

            // Send CONNECT packet with
            let connect_packet = connectv5::MqttConnect::new(
                self.options.client_id.clone(),
                self.options.username.clone(),
                self.options.password.clone(),
                self.options.will.clone(),
                self.options.keep_alive,
                self.options.clean_start,
                vec![], // Add properties as needed
            );

            // stream used, now save it
            let mqtt_stream = MqttStream::new(stream, 16384, 5);
            self.context.mqtt_stream = Some(mqtt_stream);

            // Send CONNECT packet
            self.send_packet(connect_packet)?;

            if let Some(packet) = self.recv_packet()? {
                match packet {
                    MqttPacket::ConnAck5(connack) => {
                        // Always update session_present in context
                        self.context.session_present = connack.session_present;

                        // If connection successful and session not present, clear session state
                        if connack.reason_code == 0 && !connack.session_present {
                            if let Some(sess) = &mut self.context.session {
                                sess.clear();
                            }
                            // Also clear pending operations since session is reset
                            self.clear_pending_operations();
                        }

                        // Return the connection result regardless of reason code
                        return Ok(ConnectionResult {
                            reason_code: connack.reason_code,
                            session_present: connack.session_present,
                            properties: connack.properties,
                        });
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "Expected CONNACK packet",
                        ));
                    }
                }
            }
        }

        // If we reach here, no CONNACK was received
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "No CONNACK received from broker",
        ))
    }

    // Subscribe to a topic and wait for SUBACK
    pub fn subscribed(&mut self, topic: &str, qos: u8) -> io::Result<SubscribeResult> {
        if let Some(session) = &mut self.context.session {
            let subscription = subscribev5::TopicSubscription {
                topic_filter: topic.to_string(),
                qos,
                no_local: false,
                retain_as_published: false,
                retain_handling: 0, //@TODO maybe 2 (NO handling) is more appropriate
            };

            let packet_id = session.next_packet_id();
            let subscribe_packet = subscribev5::MqttSubscribe::new(
                packet_id,
                vec![subscription],
                vec![], // Add properties as needed
            );
            // Send SUBSCRIBE packet
            self.send_packet(subscribe_packet)?;
            // Wait for SUBACK with matching packet ID
            if let Some(packet) = self.recv_for_packet(
                |p| matches!(p, MqttPacket::SubAck5(suback) if suback.packet_id == packet_id),
            )? {
                match packet {
                    MqttPacket::SubAck5(suback) => {
                        return Ok(SubscribeResult {
                            packet_id: suback.packet_id,
                            reason_codes: suback.reason_codes,
                            properties: suback.properties,
                        });
                    }
                    _ => {
                        // Store unhandled packet for later processing
                        self.context.unhandled_packets.push(packet);
                    }
                }
            }
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "No active session"));
        }

        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "No SUBACK received from broker",
        ))
    }

    // Unsubscribe from topics and wait for UNSUBACK
    pub fn unsubscribed(&mut self, topics: Vec<&str>) -> io::Result<UnsubscribeResult> {
        if let Some(session) = &mut self.context.session {
            let topic_filters: Vec<String> = topics.iter().map(|&s| s.to_string()).collect();
            let packet_id = session.next_packet_id();

            let unsubscribe_packet = unsubscribev5::MqttUnsubscribe::new(
                packet_id,
                topic_filters,
                vec![], // Add properties as needed
            );

            // Send UNSUBSCRIBE packet
            self.send_packet(unsubscribe_packet)?;

            // Wait for UNSUBACK with matching packet ID
            if let Some(packet) = self.recv_for_packet(
                |p| matches!(p, MqttPacket::UnsubAck5(unsuback) if unsuback.packet_id == packet_id),
            )? {
                match packet {
                    MqttPacket::UnsubAck5(unsuback) => {
                        return Ok(UnsubscribeResult {
                            packet_id: unsuback.packet_id,
                            reason_codes: unsuback.reason_codes,
                            properties: unsuback.properties,
                        });
                    }
                    _ => {
                        // Store unhandled packet for later processing
                        self.context.unhandled_packets.push(packet);
                    }
                }
            }
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "No active session"));
        }

        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "No UNSUBACK received from broker",
        ))
    }

    // Convenience method to unsubscribe from a single topic
    pub fn unsubscribed_single(&mut self, topic: &str) -> io::Result<UnsubscribeResult> {
        self.unsubscribed(vec![topic])
    }

    pub fn published(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
    ) -> io::Result<PublishResult> {
        if let Some(session) = &mut self.context.session {
            let packet_id = if qos > 0 {
                Some(session.next_packet_id())
            } else {
                None
            };
            let publish_packet = publishv5::MqttPublish::new(
                qos,
                topic.to_string(),
                packet_id,
                payload.to_vec(),
                retain,
                false,
            );
            // Send PUBLISH packet
            self.send_packet(publish_packet)?;

            // Handle PUBACK/PUBREC response for QoS 1/2 if needed
            match qos {
                1 => {
                    return self.receive_for_puback(packet_id);
                }
                2 => return Ok(self.handle_qos2(packet_id)?),
                _ => {
                    // QoS 0, no acknowledgment needed
                    return Ok(PublishResult {
                        packet_id,
                        reason_code: None,
                        properties: None,
                        qos,
                    });
                }
            }
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "No active session"));
        }
    }

    fn handle_qos2(&mut self, packet_id: Option<u16>) -> Result<PublishResult, io::Error> {
        let expected_packet_id = packet_id.unwrap();
        self.recv_for_packet(
            |p| matches!(p, MqttPacket::PubRec5(pubrec) if pubrec.packet_id == expected_packet_id),
        )?;

        self.send_packet(pubrelv5::MqttPubRel::new(expected_packet_id, 0, vec![]))?;
        match self.recv_for_packet(|p| {
            matches!(p, MqttPacket::PubComp5(pubcomp) if pubcomp.packet_id == expected_packet_id)
        })? {
            Some(MqttPacket::PubComp5(pubcomp)) => {
                // PUBCOMP received
                Ok(PublishResult {
                    packet_id: Some(pubcomp.packet_id),
                    reason_code: Some(pubcomp.reason_code),
                    properties: Some(pubcomp.properties),
                    qos: 2,
                })
            }
            _ => {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "No PUBCOMP received from broker",
                ))
            }
        }
    }

    fn receive_for_puback(&mut self, packet_id: Option<u16>) -> Result<PublishResult, io::Error> {
        let expected_packet_id = packet_id.unwrap();
        match self.recv_for_packet(
            |p| matches!(p, MqttPacket::PubAck5(puback) if puback.packet_id == expected_packet_id),
        )? {
            Some(MqttPacket::PubAck5(puback)) => {
                // PUBACK received
                Ok(PublishResult {
                    packet_id: Some(puback.packet_id),
                    reason_code: Some(puback.reason_code), // PUBACK reason code 0 = Success
                    properties: Some(puback.properties),   // No properties in PUBACK
                    qos: 1,
                })
            }
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "No PUBACK received from broker",
            )),
            _ => unreachable!(),
        }
    }

    pub fn disconnected(&mut self) -> io::Result<()> {
        let disconnect_packet = disconnectv5::MqttDisconnect::new(0, vec![]); // reason code 0, no properties
        self.send_packet(disconnect_packet)
    }

    pub fn pingd(&mut self) -> io::Result<PingResult> {
        let pingreq_packet = pingreqv5::MqttPingReq::new();
        self.send_packet(pingreq_packet)?;
        // Wait for PINGRESP
        if let Some(packet) = self.recv_for_packet(|p| matches!(p, MqttPacket::PingResp5(_)))? {
            match packet {
                MqttPacket::PingResp5(_) => {
                    return Ok(PingResult { success: true });
                }
                _ => {
                    // Store unhandled packet for later processing
                    self.context.unhandled_packets.push(packet);
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "No PINGRESP received from broker",
        ))
    }

    // Fire-and-forget methods (don't wait for responses)

    /// Send CONNECT packet without waiting for CONNACK
    pub fn connect_send(&mut self) -> io::Result<()> {
        // Establish connection to the MQTT broker
        if let Ok(stream) = TcpStream::connect(self.context.peer.clone()) {
            // Initialize ClientSession
            if self.options.sessionless {
                self.context.session = None;
                self.options.clean_start = true;
            } else if self.context.session.is_none() {
                self.context.session = Some(ClientSession::new());
            }

            // Send CONNECT packet
            let connect_packet = connectv5::MqttConnect::new(
                self.options.client_id.clone(),
                self.options.username.clone(),
                self.options.password.clone(),
                self.options.will.clone(),
                self.options.keep_alive,
                self.options.clean_start,
                vec![], // Add properties as needed
            );

            // Stream used, now save it
            let mqtt_stream = MqttStream::new(stream, 16384, 5);
            self.context.mqtt_stream = Some(mqtt_stream);

            // Send CONNECT packet without waiting for CONNACK
            self.send_packet(connect_packet)?;
        }
        Ok(())
    }

    /// Send SUBSCRIBE packet without waiting for SUBACK
    pub fn subscribe_send(&mut self, topic: &str, qos: u8) -> io::Result<u16> {
        if let Some(session) = &mut self.context.session {
            let subscription = subscribev5::TopicSubscription {
                topic_filter: topic.to_string(),
                qos,
                no_local: false,
                retain_as_published: false,
                retain_handling: 0,
            };

            let packet_id = session.next_packet_id();
            let subscribe_packet = subscribev5::MqttSubscribe::new(
                packet_id,
                vec![subscription],
                vec![], // Add properties as needed
            );

            // Send SUBSCRIBE packet without waiting for SUBACK
            self.send_packet(subscribe_packet)?;

            // Update session state - track pending subscription
            self.context
                .pending_subscribes
                .insert(packet_id, topic.to_string());

            Ok(packet_id)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "No active session"))
        }
    }

    /// Send UNSUBSCRIBE packet without waiting for UNSUBACK
    pub fn unsubscribe_send(&mut self, topics: Vec<&str>) -> io::Result<u16> {
        if let Some(session) = &mut self.context.session {
            let topic_filters: Vec<String> = topics.iter().map(|&s| s.to_string()).collect();
            let packet_id = session.next_packet_id();

            let unsubscribe_packet = unsubscribev5::MqttUnsubscribe::new(
                packet_id,
                topic_filters.clone(),
                vec![], // Add properties as needed
            );

            // Send UNSUBSCRIBE packet without waiting for UNSUBACK
            self.send_packet(unsubscribe_packet)?;

            // Update session state - track pending unsubscription
            self.context
                .pending_unsubscribes
                .insert(packet_id, topic_filters);

            Ok(packet_id)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "No active session"))
        }
    }

    /// Send UNSUBSCRIBE packet for single topic without waiting for UNSUBACK
    pub fn unsubscribe_send_single(&mut self, topic: &str) -> io::Result<u16> {
        self.unsubscribe_send(vec![topic])
    }

    /// Send PUBLISH packet without waiting for PUBACK/PUBREC/PUBCOMP
    pub fn publish_send(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
    ) -> io::Result<Option<u16>> {
        if let Some(session) = &mut self.context.session {
            let packet_id = if qos > 0 {
                Some(session.next_packet_id())
            } else {
                None
            };

            let publish_packet = publishv5::MqttPublish::new(
                qos,
                topic.to_string(),
                packet_id,
                payload.to_vec(),
                retain,
                false,
            );

            // Send PUBLISH packet without waiting for acknowledgment
            self.send_packet(publish_packet)?;

            // For QoS 1/2, track the packet_id in session state for retransmission
            if let Some(pid) = packet_id {
                self.context
                    .pending_publishes
                    .insert(pid, (topic.to_string(), qos));
            }

            Ok(packet_id)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "No active session"))
        }
    }

    /// Send PINGREQ packet without waiting for PINGRESP
    pub fn ping_send(&mut self) -> io::Result<()> {
        let pingreq_packet = pingreqv5::MqttPingReq::new();
        self.send_packet(pingreq_packet)
    }

    /// Send DISCONNECT packet without waiting (DISCONNECT has no response)
    pub fn disconnect_send(&mut self) -> io::Result<()> {
        let disconnect_packet = disconnectv5::MqttDisconnect::new(0, vec![]); // reason code 0, no properties
        self.send_packet(disconnect_packet)
    }

    // Session state management methods

    /// Get all pending subscribe packet IDs and their topics
    pub fn get_pending_subscribes(&self) -> &HashMap<u16, String> {
        &self.context.pending_subscribes
    }

    /// Get all pending unsubscribe packet IDs and their topics  
    pub fn get_pending_unsubscribes(&self) -> &HashMap<u16, Vec<String>> {
        &self.context.pending_unsubscribes
    }

    /// Get all pending publish packet IDs and their details
    pub fn get_pending_publishes(&self) -> &HashMap<u16, (String, u8)> {
        &self.context.pending_publishes
    }

    /// Remove a pending subscribe operation (call when SUBACK received)
    pub fn complete_subscribe(&mut self, packet_id: u16) -> Option<String> {
        self.context.pending_subscribes.remove(&packet_id)
    }

    /// Remove a pending unsubscribe operation (call when UNSUBACK received)
    pub fn complete_unsubscribe(&mut self, packet_id: u16) -> Option<Vec<String>> {
        self.context.pending_unsubscribes.remove(&packet_id)
    }

    /// Remove a pending publish operation (call when PUBACK/PUBCOMP received)
    pub fn complete_publish(&mut self, packet_id: u16) -> Option<(String, u8)> {
        self.context.pending_publishes.remove(&packet_id)
    }

    /// Clear all pending operations (useful on reconnect with clean_start=true)
    pub fn clear_pending_operations(&mut self) {
        self.context.pending_subscribes.clear();
        self.context.pending_unsubscribes.clear();
        self.context.pending_publishes.clear();
    }

    fn send_packet<T>(&mut self, packet: T) -> io::Result<()>
    where
        T: MqttControlPacket,
    {
        let stream = match &mut self.context.mqtt_stream {
            Some(s) => s,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "No active MQTT stream connection",
                ));
            }
        };

        let packet_bytes = packet.to_bytes().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to serialize packet: {:?}", e),
            )
        })?;

        stream.write(&packet_bytes).map_err(|e| {
            io::Error::new(
                io::ErrorKind::WriteZero,
                format!("Failed to write packet: {}", e),
            )
        })?;

        Ok(())
    }

    pub fn recv_packet(&mut self) -> io::Result<Option<MqttPacket>> {
        let stream = match &mut self.context.mqtt_stream {
            Some(s) => s,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "No active MQTT stream connection",
                ));
            }
        };

        match stream.next() {
            Some(Ok(packet)) => Ok(Some(packet)),
            Some(Err(parse_error)) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse MQTT packet: {:?}", parse_error),
            )),
            None => Ok(None), // End of stream
        }
    }

    pub fn recv_for_packet<F>(&mut self, mut f: F) -> io::Result<Option<MqttPacket>>
    where
        F: FnMut(&MqttPacket) -> bool,
    {
        let stream = match &mut self.context.mqtt_stream {
            Some(s) => s,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "No active MQTT stream connection",
                ));
            }
        };

        // find_map handles errors and applies the predicate only to successful packets
        stream
            .find_map(|result| match result {
                Ok(packet) if f(&packet) => Some(Ok(packet)),
                Ok(other) => {
                    self.context.unhandled_packets.push(other);
                    None
                }
                Err(parse_err) => Some(Err(io::Error::new(io::ErrorKind::InvalidData, parse_err))),
            })
            .transpose() // Convert Option<Result<T, E>> to Result<Option<T>, E>
    }
}
