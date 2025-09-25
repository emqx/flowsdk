use crate::mqtt_session::ClientSession;

use crate::mqtt_serde::control_packet::MqttControlPacket;
use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::connectv5;
use crate::mqtt_serde::mqttv5::disconnectv5;
use crate::mqtt_serde::mqttv5::pingreqv5;
use crate::mqtt_serde::mqttv5::publishv5;
use crate::mqtt_serde::mqttv5::pubrelv5;
use crate::mqtt_serde::mqttv5::subscribev5;

use crate::mqtt_serde::MqttStream;

use super::MqttClientOptions;
use std::io;
use std::net::TcpStream;

pub struct Subscription {
    pub topic: String,
    pub qos: u8,
}
pub struct Context {
    peer: String,
    session: Option<ClientSession>,
    mqtt_stream: Option<MqttStream<TcpStream>>,
    // Update when subscribed to topics and session is not None
    subscribed_topics: Vec<Subscription>,
    session_present: bool,
    mqtt_buffer: Vec<MqttPacket>,
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
        }
    }
}

pub struct MqttClient {
    context: Context,
    options: MqttClientOptions,
}

impl MqttClient {
    pub fn new(endpoint: String, options: MqttClientOptions) -> Self {
        MqttClient {
            context: Context::new(endpoint),
            options,
        }
    }

    pub fn new_with_sess(peer: String, options: MqttClientOptions, session: ClientSession) -> Self {
        MqttClient {
            context: Context::new_with_sess(peer, session),
            options,
        }
    }

    // Connect to the MQTT broker and wait for CONNACK
    pub fn connected(&mut self) -> io::Result<()> {
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
                        if connack.reason_code == 0 {
                            self.context.session_present = connack.session_present;
                            // If not session present, clear session state
                            if !self.context.session_present {
                                if let Some(sess) = &mut self.context.session {
                                    sess.clear();
                                }
                            }
                        } else {
                            return Err(io::Error::new(
                                io::ErrorKind::Other,
                                format!("Connection refused: {}", connack.reason_code),
                            ));
                        }
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
        Ok(())
    }

    // Subscribe to a topic and wait for SUBACK
    pub fn subscribed(&mut self, topic: &str, qos: u8) -> io::Result<()> {
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
                    MqttPacket::SubAck5(_suback) => {
                        // Process suback.reason_codes as needed
                    }
                    _ => unreachable!(), // receive_for guarantees we get the right packet type
                }
            }
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "No active session"));
        }
        Ok(())
    }

    pub fn published(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: u8,
        retain: bool,
    ) -> io::Result<()> {
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
                    let expected_packet_id = packet_id.unwrap();
                    if let Some(packet) = self.recv_for_packet(|p| {
                        matches!(p, MqttPacket::PubAck5(puback) if puback.packet_id == expected_packet_id)
                    })? {
                        match packet {
                            MqttPacket::PubAck5(_puback) => {
                                // Process puback.reason_code as needed
                            }
                            _ => unreachable!(), // receive_for guarantees we get the right packet type
                        }
                    }
                }
                2 => {
                    // Handle QoS 2 flow (PUBREC, PUBREL, PUBCOMP)
                    let expected_packet_id = packet_id.unwrap();

                    // Wait for PUBREC
                    if let Some(packet) = self.recv_for_packet(|p| {
                        matches!(p, MqttPacket::PubRec5(pubrec) if pubrec.packet_id == expected_packet_id)
                    })? {
                        match packet {
                            MqttPacket::PubRec5(pubrec) => {
                                // Send PUBREL
                                let pubrel_packet = pubrelv5::MqttPubRel::new(
                                    pubrec.packet_id,
                                    0,      // reason code
                                    vec![], // properties
                                );
                                self.send_packet(pubrel_packet)?;

                                // Wait for PUBCOMP
                                if let Some(packet) = self.recv_for_packet(|p| {
                                    matches!(p, MqttPacket::PubComp5(pubcomp) if pubcomp.packet_id == expected_packet_id)
                                })? {
                                    match packet {
                                        MqttPacket::PubComp5(_pubcomp) => {
                                            // Process pubcomp.reason_code as needed
                                        }
                                        _ => unreachable!(), // receive_for guarantees we get the right packet type
                                    }
                                }
                            }
                            _ => unreachable!(), // receive_for guarantees we get the right packet type
                        }
                    }
                }
                _ => {} // QoS 0, no acknowledgment needed
            }
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "No active session"));
        }
        Ok(())
    }

    pub fn disconnected(&mut self) -> io::Result<()> {
        let disconnect_packet = disconnectv5::MqttDisconnect::new(0, vec![]); // reason code 0, no properties
        self.send_packet(disconnect_packet)
    }

    pub fn pingd(&mut self) -> io::Result<()> {
        let pingreq_packet = pingreqv5::MqttPingReq::new();
        self.send_packet(pingreq_packet)?;
        // Wait for PINGRESP
        if let Some(packet) = self.recv_for_packet(|p| matches!(p, MqttPacket::PingResp5(_)))? {
            match packet {
                MqttPacket::PingResp5(_) => {
                    // Successfully received PINGRESP
                }
                _ => unreachable!(), // receive_for guarantees we get the right packet type
            }
        }

        Ok(())
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
                Ok(_) => None,
                Err(parse_err) => Some(Err(io::Error::new(io::ErrorKind::InvalidData, parse_err))),
            })
            .transpose() // Convert Option<Result<T, E>> to Result<Option<T>, E>
    }
}
