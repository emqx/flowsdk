// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_session::ClientSession;

use crate::mqtt_serde::control_packet::MqttControlPacket;
use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::connectv5;
use crate::mqtt_serde::mqttv5::disconnectv5;
use crate::mqtt_serde::mqttv5::pingreqv5;
use crate::mqtt_serde::mqttv5::publishv5;
use crate::mqtt_serde::mqttv5::pubrelv5;
use crate::mqtt_serde::mqttv5::subscribev5;
use crate::mqtt_serde::mqttv5::unsubscribev5;
use crate::mqtt_serde::mqttv5::will as willv5;

use crate::mqtt_serde::mqttv3::connectv3;
use crate::mqtt_serde::mqttv3::disconnectv3;
use crate::mqtt_serde::mqttv3::pingreqv3;
use crate::mqtt_serde::mqttv3::publishv3;
use crate::mqtt_serde::mqttv3::pubrelv3;
use crate::mqtt_serde::mqttv3::subscribev3;
use crate::mqtt_serde::mqttv3::unsubscribev3;

use crate::mqtt_serde::MqttStream;

use super::MqttClientOptions;
use std::collections::HashMap;
use std::io;
use std::net::TcpStream;

pub use super::types::*;

pub struct Context {
    peer: String,
    session: Option<ClientSession>,
    mqtt_stream: Option<MqttStream<TcpStream>>,
    // Update when subscribed to topics and session is not None
    #[allow(dead_code)]
    subscribed_topics: Vec<Subscription>,
    session_present: bool,
    #[allow(dead_code)]
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

    // Helper methods for protocol version
    fn is_v3(&self) -> bool {
        self.options.mqtt_version == 3 || self.options.mqtt_version == 4
    }

    // Convert v5 Will to v3 Will (strip properties)
    fn convert_will_v5_to_v3(will_v5: &willv5::Will) -> connectv3::Will {
        connectv3::Will {
            retain: will_v5.will_retain,
            qos: will_v5.will_qos,
            topic: will_v5.will_topic.clone(),
            message: will_v5.will_message.clone(),
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

            // Create CONNECT packet based on protocol version
            if self.is_v3() {
                // MQTT v3.1.1
                let mut connect_packet = connectv3::MqttConnect::new(
                    self.options.client_id.clone(),
                    self.options.keep_alive,
                    self.options.clean_start, // v3 uses clean_session
                );
                connect_packet.username = self.options.username.clone();
                connect_packet.password = self.options.password.clone();

                // Convert v5 Will to v3 Will if present
                if let Some(will_v5) = &self.options.will {
                    connect_packet.will = Some(Self::convert_will_v5_to_v3(will_v5));
                }

                // stream used, now save it
                let mqtt_stream = MqttStream::new(stream, 16384, self.options.mqtt_version);
                self.context.mqtt_stream = Some(mqtt_stream);

                // Send CONNECT packet
                self.send_packet(connect_packet)?;

                if let Some(packet) = self.recv_packet()? {
                    match packet {
                        MqttPacket::ConnAck3(connack) => {
                            // Always update session_present in context
                            self.context.session_present = connack.session_present;

                            // If connection successful and session not present, clear session state
                            // v3 return code 0 = Connection Accepted
                            if connack.return_code == 0 && !connack.session_present {
                                if let Some(sess) = &mut self.context.session {
                                    sess.clear();
                                }
                                // Also clear pending operations since session is reset
                                self.clear_pending_operations();
                            }

                            // Return the connection result
                            return Ok(ConnectionResult {
                                reason_code: connack.return_code,
                                session_present: connack.session_present,
                                properties: None, // v3 doesn't have properties
                            });
                        }
                        _ => {
                            return Err(io::Error::other("Expected CONNACK packet"));
                        }
                    }
                }
            } else {
                // MQTT v5
                let connect_packet = connectv5::MqttConnect::new(
                    self.options.client_id.clone(),
                    self.options.username.clone(),
                    self.options.password.clone(),
                    self.options.will.clone(),
                    self.options.keep_alive,
                    self.options.clean_start,
                    self.options.connect_properties.clone(),
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
                            return Err(io::Error::other("Expected CONNACK packet"));
                        }
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
            let packet_id = session.next_packet_id();

            if self.is_v3() {
                // MQTT v3.1.1
                let subscribe_packet = subscribev3::MqttSubscribe::new(
                    packet_id,
                    vec![subscribev3::SubscriptionTopic {
                        topic_filter: topic.to_string(),
                        qos,
                    }],
                );
                self.send_packet(subscribe_packet)?;

                if let Some(packet) = self.recv_for_packet(
                    |p| matches!(p, MqttPacket::SubAck3(suback) if suback.message_id == packet_id),
                )? {
                    match packet {
                        MqttPacket::SubAck3(suback) => {
                            return Ok(SubscribeResult {
                                packet_id: suback.message_id,
                                reason_codes: suback.return_codes,
                                properties: vec![], // v3 doesn't have properties
                            });
                        }
                        _ => {
                            self.context.unhandled_packets.push(packet);
                        }
                    }
                }
            } else {
                // MQTT v5
                let subscription = subscribev5::TopicSubscription {
                    topic_filter: topic.to_string(),
                    qos,
                    no_local: false,
                    retain_as_published: false,
                    retain_handling: 0,
                };
                let subscribe_packet =
                    subscribev5::MqttSubscribe::new(packet_id, vec![subscription], vec![]);
                self.send_packet(subscribe_packet)?;

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
                            self.context.unhandled_packets.push(packet);
                        }
                    }
                }
            }
        } else {
            return Err(io::Error::other("No active session"));
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

            if self.is_v3() {
                // MQTT v3.1.1
                let unsubscribe_packet =
                    unsubscribev3::MqttUnsubscribe::new(packet_id, topic_filters.clone());
                self.send_packet(unsubscribe_packet)?;

                if let Some(packet) = self.recv_for_packet(|p| {
                    matches!(p, MqttPacket::UnsubAck3(unsuback) if unsuback.message_id == packet_id)
                })? {
                    match packet {
                        MqttPacket::UnsubAck3(unsuback) => {
                            // v3 UNSUBACK doesn't have return codes, just message_id
                            // Return success (0) for all topics
                            return Ok(UnsubscribeResult {
                                packet_id: unsuback.message_id,
                                reason_codes: vec![0; topic_filters.len()],
                                properties: vec![],
                            });
                        }
                        _ => {
                            self.context.unhandled_packets.push(packet);
                        }
                    }
                }
            } else {
                // MQTT v5
                let unsubscribe_packet =
                    unsubscribev5::MqttUnsubscribe::new(packet_id, topic_filters, vec![]);
                self.send_packet(unsubscribe_packet)?;

                if let Some(packet) = self.recv_for_packet(|p| {
                    matches!(p, MqttPacket::UnsubAck5(unsuback) if unsuback.packet_id == packet_id)
                })? {
                    match packet {
                        MqttPacket::UnsubAck5(unsuback) => {
                            return Ok(UnsubscribeResult {
                                packet_id: unsuback.packet_id,
                                reason_codes: unsuback.reason_codes,
                                properties: unsuback.properties,
                            });
                        }
                        _ => {
                            self.context.unhandled_packets.push(packet);
                        }
                    }
                }
            }
        } else {
            return Err(io::Error::other("No active session"));
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

            if self.is_v3() {
                // MQTT v3.1.1
                let publish_packet = publishv3::MqttPublish::new(
                    topic.to_string(),
                    qos,
                    payload.to_vec(),
                    packet_id,
                    retain,
                    false, // dup
                );
                self.send_packet(publish_packet)?;
            } else {
                // MQTT v5
                let publish_packet = publishv5::MqttPublish::new(
                    qos,
                    topic.to_string(),
                    packet_id,
                    payload.to_vec(),
                    retain,
                    false, // dup
                );
                self.send_packet(publish_packet)?;
            }

            // Handle PUBACK/PUBREC response for QoS 1/2 if needed
            match qos {
                1 => self.receive_for_puback(packet_id),
                2 => self.handle_qos2(packet_id),
                _ => {
                    // QoS 0, no acknowledgment needed
                    Ok(PublishResult {
                        packet_id,
                        reason_code: None,
                        properties: None,
                        qos,
                    })
                }
            }
        } else {
            Err(io::Error::other("No active session"))
        }
    }

    fn handle_qos2(&mut self, packet_id: Option<u16>) -> Result<PublishResult, io::Error> {
        let expected_packet_id = packet_id.unwrap();

        if self.is_v3() {
            // MQTT v3.1.1
            self.recv_for_packet(|p| {
                matches!(p, MqttPacket::PubRec3(pubrec) if pubrec.message_id == expected_packet_id)
            })?;

            self.send_packet(pubrelv3::MqttPubRel::new(expected_packet_id))?;
            match self.recv_for_packet(|p| {
                matches!(p, MqttPacket::PubComp3(pubcomp) if pubcomp.message_id == expected_packet_id)
            })? {
                Some(MqttPacket::PubComp3(pubcomp)) => {
                    Ok(PublishResult {
                        packet_id: Some(pubcomp.message_id),
                        reason_code: Some(0), // v3 doesn't have reason codes, assume success
                        properties: None,
                        qos: 2,
                    })
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "No PUBCOMP received from broker",
                )),
            }
        } else {
            // MQTT v5
            self.recv_for_packet(|p| {
                matches!(p, MqttPacket::PubRec5(pubrec) if pubrec.packet_id == expected_packet_id)
            })?;

            self.send_packet(pubrelv5::MqttPubRel::new(expected_packet_id, 0, vec![]))?;
            match self.recv_for_packet(|p| {
                matches!(p, MqttPacket::PubComp5(pubcomp) if pubcomp.packet_id == expected_packet_id)
            })? {
                Some(MqttPacket::PubComp5(pubcomp)) => {
                    Ok(PublishResult {
                        packet_id: Some(pubcomp.packet_id),
                        reason_code: Some(pubcomp.reason_code),
                        properties: Some(pubcomp.properties),
                        qos: 2,
                    })
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "No PUBCOMP received from broker",
                )),
            }
        }
    }

    fn receive_for_puback(&mut self, packet_id: Option<u16>) -> Result<PublishResult, io::Error> {
        let expected_packet_id = packet_id.unwrap();

        if self.is_v3() {
            // MQTT v3.1.1
            match self.recv_for_packet(|p| {
                matches!(p, MqttPacket::PubAck3(puback) if puback.message_id == expected_packet_id)
            })? {
                Some(MqttPacket::PubAck3(puback)) => {
                    Ok(PublishResult {
                        packet_id: Some(puback.message_id),
                        reason_code: Some(0), // v3 doesn't have reason codes, assume success
                        properties: None,
                        qos: 1,
                    })
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "No PUBACK received from broker",
                )),
            }
        } else {
            // MQTT v5
            match self.recv_for_packet(|p| {
                matches!(p, MqttPacket::PubAck5(puback) if puback.packet_id == expected_packet_id)
            })? {
                Some(MqttPacket::PubAck5(puback)) => {
                    Ok(PublishResult {
                        packet_id: Some(puback.packet_id),
                        reason_code: Some(puback.reason_code),
                        properties: Some(puback.properties),
                        qos: 1,
                    })
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "No PUBACK received from broker",
                )),
            }
        }
    }

    pub fn disconnected(&mut self, reason_code: u8) -> io::Result<()> {
        if self.is_v3() {
            // MQTT v3.1.1 - DISCONNECT has no variable header or payload
            let disconnect_packet = disconnectv3::MqttDisconnect::new();
            self.send_packet(disconnect_packet)?;
        } else {
            // MQTT v5
            let disconnect_packet = disconnectv5::MqttDisconnect::new(reason_code, vec![]);
            self.send_packet(disconnect_packet)?;
        }
        Ok(())
    }

    pub fn pingd(&mut self) -> io::Result<PingResult> {
        if self.is_v3() {
            // MQTT v3.1.1
            let pingreq_packet = pingreqv3::MqttPingReq::new();
            self.send_packet(pingreq_packet)?;

            if let Some(packet) = self.recv_for_packet(|p| matches!(p, MqttPacket::PingResp3(_)))? {
                match packet {
                    MqttPacket::PingResp3(_) => {
                        return Ok(PingResult { success: true });
                    }
                    _ => {
                        self.context.unhandled_packets.push(packet);
                    }
                }
            }
        } else {
            // MQTT v5
            let pingreq_packet = pingreqv5::MqttPingReq::new();
            self.send_packet(pingreq_packet)?;

            if let Some(packet) = self.recv_for_packet(|p| matches!(p, MqttPacket::PingResp5(_)))? {
                match packet {
                    MqttPacket::PingResp5(_) => {
                        return Ok(PingResult { success: true });
                    }
                    _ => {
                        self.context.unhandled_packets.push(packet);
                    }
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
        let stream = TcpStream::connect(&self.context.peer)?;
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
            self.options.connect_properties.clone(),
        );

        // Stream used, now save it
        let mqtt_stream = MqttStream::new(stream, 16384, 5);
        self.context.mqtt_stream = Some(mqtt_stream);

        // Send CONNECT packet without waiting for CONNACK
        self.send_packet(connect_packet)?;
        Ok(())
    }

    /// Bound a blocking receive, allowing event loops to process queued commands.
    pub fn set_read_timeout(&mut self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.context
            .mqtt_stream
            .as_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "No active MQTT stream connection",
                )
            })?
            .mut_stream()
            .set_read_timeout(timeout)
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
            Err(io::Error::other("No active session"))
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
            Err(io::Error::other("No active session"))
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
            Err(io::Error::other("No active session"))
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

        // MqttStream version is set at creation time, so next() will parse with correct version
        match stream.next() {
            Some(Ok(packet)) => Ok(Some(packet)),
            Some(Err(crate::mqtt_serde::parser::ParseError::IoError(error))) => Err(error),
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
