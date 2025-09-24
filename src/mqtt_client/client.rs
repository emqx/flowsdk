use crate::mqtt_session::ClientSession;

use crate::mqtt_serde::control_packet::MqttControlPacket;
use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::connectv5;
use crate::mqtt_serde::mqttv5::disconnectv5;
use crate::mqtt_serde::mqttv5::pingreqv5;
use crate::mqtt_serde::mqttv5::pingrespv5;
use crate::mqtt_serde::mqttv5::pubackv5;
use crate::mqtt_serde::mqttv5::pubcompv5;
use crate::mqtt_serde::mqttv5::publishv5;
use crate::mqtt_serde::mqttv5::pubrecv5;
use crate::mqtt_serde::mqttv5::pubrelv5;
use crate::mqtt_serde::mqttv5::subscribev5;
use crate::mqtt_serde::mqttv5::unsubscribev5;

use super::MqttClientOptions;
use crate::mqtt_serde::parser::stream::MqttParser;
use std::io;
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct Subscription {
    pub topic: String,
    pub qos: u8,
}
pub struct Context {
    peer: String,
    stream: Option<TcpStream>,
    session: Option<ClientSession>,
    parser: MqttParser,
    // Update when subscribed to topics and session is not None
    subscribed_topics: Vec<Subscription>,
    session_present: bool,
}

impl Context {
    pub fn new(peer: String) -> Self {
        Context {
            peer,
            stream: None,
            session: None,
            session_present: false,
            subscribed_topics: Vec::new(),
            parser: MqttParser::new(16384, 5),
        }
    }

    pub fn new_with_sess(peer: String, session: ClientSession) -> Self {
        Context {
            peer,
            stream: None,
            session: Some(session),
            session_present: false,
            subscribed_topics: Vec::new(),
            parser: MqttParser::new(16384, 5),
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
        if let Ok(mut stream) = TcpStream::connect(self.context.peer.clone()) {
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

            let conn_bytes = connect_packet.to_bytes().unwrap();

            stream.write(&conn_bytes).unwrap();

            // Wait for CONNACK
            let mut buffer = [0; 4096];
            let n = stream.read(&mut buffer).unwrap();

            // stream used, now save it
            self.context.stream = Some(stream);

            self.context.parser.feed(&buffer[..n]);

            if let Some(packet) = self.context.parser.next_packet().unwrap() {
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
            let stream = match &mut self.context.stream {
                Some(s) => s,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "No active connection",
                    ));
                }
            };
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
            // Assume stream is available here
            stream.write(&subscribe_packet.to_bytes().unwrap()).unwrap();
            // Handle SUBACK response similarly to CONNACK
            let mut buf = [0; 4096];
            let n: usize = stream.read(&mut buf).unwrap();

            self.context.parser.feed(&buf[..n]);

            if let Some(packet) = self.context.parser.next_packet().unwrap() {
                match packet {
                    MqttPacket::SubAck5(suback) => {
                        if suback.packet_id != packet_id {
                            return Err(io::Error::new(
                                io::ErrorKind::Other,
                                "Mismatched SUBACK packet ID",
                            ));
                        }
                        // Process suback.reason_codes as needed
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "Expected SUBACK packet",
                        ));
                    }
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
            let stream = match &mut self.context.stream {
                Some(s) => s,
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "No active connection",
                    ));
                }
            };
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
            stream.write(&publish_packet.to_bytes().unwrap()).unwrap();
            // Handle PUBACK/PUBREC response for QoS 1/2 if needed
            match qos {
                1 => {
                    let mut buf = [0; 4096];
                    let n: usize = stream.read(&mut buf).unwrap();
                    self.context.parser.feed(&buf[..n]);
                    if let Some(packet) = self.context.parser.next_packet().unwrap() {
                        match packet {
                            MqttPacket::PubAck5(puback) => {
                                // @TODO may be packet_id is different, we should wait for more.
                                if packet_id.unwrap() != puback.packet_id {
                                    return Err(io::Error::new(
                                        io::ErrorKind::Other,
                                        "Mismatched PUBACK packet ID",
                                    ));
                                }
                                // Process puback.reason_code as needed
                            }
                            _ => {
                                println!("Received unexpected packet: {:?}", packet);
                                return Err(io::Error::new(
                                    io::ErrorKind::Other,
                                    "Expected PUBACK packet",
                                ));
                            }
                        }
                    }
                }
                2 => {
                    // Handle QoS 2 flow (PUBREC, PUBREL, PUBCOMP)
                    // This is a simplified version and may need more robust handling
                    let mut buf = [0; 4096];
                    let n: usize = stream.read(&mut buf).unwrap();
                    self.context.parser.feed(&buf[..n]);
                    if let Some(packet) = self.context.parser.next_packet().unwrap() {
                        match packet {
                            MqttPacket::PubRec5(pubrec) => {
                                if pubrec.packet_id != packet_id.unwrap() {
                                    return Err(io::Error::new(
                                        io::ErrorKind::Other,
                                        "Mismatched PUBREC packet ID",
                                    ));
                                }
                                // Send PUBREL
                                let pubrel_packet = pubrelv5::MqttPubRel::new(
                                    pubrec.packet_id,
                                    0,      // reason code
                                    vec![], // properties
                                );

                                // @TODO: check return type
                                stream.write(&pubrel_packet.to_bytes().unwrap()).unwrap();

                                // Wait for PUBCOMP
                                let n: usize = stream.read(&mut buf).unwrap();
                                self.context.parser.feed(&buf[..n]);
                                if let Some(packet) = self.context.parser.next_packet().unwrap() {
                                    match packet {
                                        MqttPacket::PubComp5(pubcomp) => {
                                            if pubcomp.packet_id != packet_id.unwrap() {
                                                return Err(io::Error::new(
                                                    io::ErrorKind::Other,
                                                    "Mismatched PUBCOMP packet ID",
                                                ));
                                            }
                                            // Process pubcomp.reason_code as needed
                                        }
                                        _ => {
                                            return Err(io::Error::new(
                                                io::ErrorKind::Other,
                                                "Expected PUBCOMP packet",
                                            ));
                                        }
                                    }
                                }
                            }
                            _ => {
                                return Err(io::Error::new(
                                    io::ErrorKind::Other,
                                    "Expected PUBREC packet",
                                ));
                            }
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
        if let Some(stream) = &mut self.context.stream {
            let disconnect_packet = disconnectv5::MqttDisconnect::new(0, vec![]); // reason code 0, no properties
            stream
                .write(&disconnect_packet.to_bytes().unwrap())
                .unwrap();
        }
        Ok(())
    }

    pub fn pingd(&mut self) -> io::Result<()> {
        if let Some(stream) = &mut self.context.stream {
            let pingreq_packet = pingreqv5::MqttPingReq::new();
            stream.write(&pingreq_packet.to_bytes().unwrap()).unwrap();
            // Wait for PINGRESP
            let mut buf = [0; 4096];
            let n: usize = stream.read(&mut buf).unwrap();
            self.context.parser.feed(&buf[..n]);
            if let Some(packet) = self.context.parser.next_packet().unwrap() {
                match packet {
                    MqttPacket::PingResp5(_) => {
                        // Successfully received PINGRESP
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!("Expected PINGRESP packet, got {:?}", packet),
                        ));
                    }
                }
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "No active connection",
            ));
        }
        Ok(())
    }

    pub fn recv_packet(&mut self) -> io::Result<Option<MqttPacket>> {
        if let Some(stream) = &mut self.context.stream {
            let mut buf = [0; 4096];
            let n: usize = stream.read(&mut buf)?;
            if n == 0 {
                // Connection closed
                self.context.stream = None;
                return Ok(None);
            }
            self.context.parser.feed(&buf[..n]);
            if let Some(packet) = self.context.parser.next_packet().unwrap() {
                return Ok(Some(packet));
            }
        }
        Ok(None)
    }
}
