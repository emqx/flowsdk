use dashmap::DashMap;
use mqtt_grpc_duality::mqtt_serde::control_packet::MqttControlPacket;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tonic::{transport::Server, Request, Response, Status};

use mqttv5pb::mqtt_relay_service_server::{MqttRelayService, MqttRelayServiceServer};
use mqttv5pb::{MqttPacket, RelayResponse};

use mqtt_grpc_duality::mqtt_serde;
use mqtt_grpc_duality::mqtt_serde::mqttv5::common::properties::Property;
use mqtt_grpc_duality::mqtt_serde::parser::stream::MqttParser;

use crate::mpsc::Sender;
use tokio::net::TcpSocket;
use tokio::sync::mpsc;

pub mod mqttv5pb {
    tonic::include_proto!("mqttv5"); // The string specified here must match the proto package name
}

#[derive(Debug, Default)]
pub struct MyRelay {
    // Shared state for managing connections
    connections: Arc<DashMap<String, Sender<mqttv5pb::Publish>>>,
}

#[tonic::async_trait]
impl MqttRelayService for MyRelay {
    async fn relay_packet(
        &self,
        request: Request<MqttPacket>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };

        Ok(Response::new(reply))
    }

    async fn mqtt_connect(
        &self,
        request: Request<mqttv5pb::Connect>,
    ) -> Result<Response<mqttv5pb::Connack>, Status> {
        println!("Got a request: {:?}", request);

        let clientid = request.get_ref().client_id.clone();
        let (tx, rx) = mpsc::channel(32);

        let (connack, stream) = match mqtt_connect_to_broker(request).await {
            Ok(s) => s,
            Err(e) => {
                return Err(e);
            }
        };

        self.connections.insert(clientid, tx.clone());

        let _join = tokio::task::spawn(async move {
            mqtt_client_loop(rx, stream).await;
        });

        let m: mqttv5pb::Connack = connack.into();

        Ok(Response::new(m))
    }

    async fn mqtt_publish_qos1(
        &self,
        request: Request<mqttv5pb::Publish>,
    ) -> Result<Response<mqttv5pb::Puback>, Status> {
        println!("Got a request: {:?}", request);
        let msg_id = request.get_ref().message_id;

        let req_meta = request.metadata().get("x-client-id");
        if let Some(client_id) = req_meta.and_then(|v| v.to_str().ok()) {
            println!("Client ID: {}", client_id);

            if let Some(tx) = self.connections.get(client_id) {
                if tx.send(request.into_inner().clone()).await.is_ok() {
                    println!("Publish message sent successfully to proxy channel.");
                } else {
                    eprintln!("Failed to send publish message to proxy channel, receiver dropped.");
                    return Err(Status::internal(
                        "Failed to send publish message, client task may have died.",
                    ));
                }
            } else {
                return Err(Status::not_found(format!(
                    "Client ID '{}' not found or client not connected.",
                    client_id
                )));
            }
        } else {
            // @FIXME: x-client-id header is NOT required, but we need to handle it gracefully
            return Err(Status::invalid_argument("Header 'x-client-id' is required"));
        }

        let m = mqttv5pb::Puback {
            message_id: msg_id,
            reason_code: 0,
            properties: Default::default(),
        };
        Ok(Response::new(m))
    }

    async fn mqtt_subscribe(
        &self,
        request: Request<mqttv5pb::Subscribe>,
    ) -> Result<Response<mqttv5pb::Suback>, Status> {
        println!("Got a request: {:?}", request);

        let m = mqttv5pb::Suback {
            message_id: request.get_ref().message_id,
            reason_codes: vec![0],
            properties: Default::default(),
        };
        Ok(Response::new(m))
    }
}

async fn mqtt_connect_to_broker(
    request: Request<mqttv5pb::Connect>,
) -> Result<
    (
        mqtt_serde::mqttv5::connack::MqttConnAck,
        tokio::net::TcpStream,
    ),
    Status,
> {
    let msg: mqtt_serde::mqttv5::connect::MqttConnect = request.into_inner().into();
    let addr = "127.0.0.1:1883".parse().unwrap();
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    let mut stream = socket.connect(addr).await?;
    let data = msg.to_bytes().unwrap(); //@FIXME unwrap
    println!("Connecting to MQTT broker at {} with data {:?}", addr, data);
    stream
        .write_all(&data)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;
    let connack = wait_for_connack(&mut stream)
        .await
        .map_err(|e| Status::internal(format!("Failed to receive connack from broker: {}", e)))?;
    println!("Connected to MQTT broker at {}", addr);
    Ok((connack, stream))
}

async fn wait_for_connack(
    stream: &mut tokio::net::TcpStream,
) -> Result<mqtt_serde::mqttv5::connack::MqttConnAck, Status> {
    let mut parser = MqttParser::new();

    loop {
        let n = stream
            .read_buf(parser.buffer_mut())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        if n == 0 {
            return Err(Status::unavailable("Connection closed by the broker"));
        }

        match parser.next_packet() {
            Ok(Some(mqtt_grpc_duality::mqtt_serde::control_packet::MqttPacket::ConnAck(
                connack,
            ))) => {
                return Ok(connack);
            }
            Ok(Some(_)) => {
                // Ignore other packets @FIXME: return an error for unexpected packets
            }
            Ok(None) => {
                // Incomplete packet, continue reading
            }
            Err(e) => {
                eprintln!("Error parsing ConnAck: {:?}", e);
                return Err(Status::internal("Failed to parse ConnAck"));
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50515".parse()?;
    let relay = MyRelay::default();

    Server::builder()
        .add_service(MqttRelayServiceServer::new(relay))
        .serve(addr)
        .await?;

    Ok(())
}

async fn mqtt_client_loop(
    mut rx: mpsc::Receiver<mqttv5pb::Publish>,
    mut stream: tokio::net::TcpStream,
) {
    let mut parser = MqttParser::new();

    loop {
        tokio::select! {
            Some(pbdata) = rx.recv() => {
                println!("Received pb data: {:?}", pbdata);

                let publish_packet: mqtt_serde::mqttv5::publish::MqttPublish = pbdata.into();
                let data = publish_packet.to_bytes().unwrap(); //@FIXME unwrap
                if let Err(e) = stream.write_all(&data).await {
                    eprintln!("Failed to write to broker: {}", e);
                    break;
                }
            }
            result = stream.read_buf(parser.buffer_mut()) => {
                match result {
                    Ok(0) => {
                        println!("Connection closed by the broker");
                        break;
                    }
                    Ok(n) => {
                        println!("Read {} bytes from broker", n);
                        while let Ok(Some(packet)) = parser.next_packet() {
                            println!("Received packet from broker: {:?}", packet);
                            // TODO: Process the received data from broker
                        }
                    }
                    Err(e) => {
                        eprintln!("Error reading from stream: {}", e);
                        break;
                    }
                }
            }
            else => {
                break;
            }
        }
    }
}

impl From<mqtt_serde::mqttv5::connect::MqttConnect> for mqttv5pb::Connect {
    fn from(connect: mqtt_serde::mqttv5::connect::MqttConnect) -> Self {
        mqttv5pb::Connect {
            client_id: connect.client_id.clone(),
            protocol_name: "MQTT".into(),
            protocol_version: connect.protocol_version as u32,
            clean_start: connect.is_clean_start(),
            keep_alive: connect.keep_alive as u32,
            username: connect.username.unwrap_or_default(),
            password: connect.password.unwrap_or_default(),
            will: None,         // @TODO
            properties: vec![], // @TODO
        }
    }
}

impl From<mqtt_serde::mqttv5::connack::MqttConnAck> for mqttv5pb::Connack {
    fn from(connack: mqtt_serde::mqttv5::connack::MqttConnAck) -> Self {
        mqttv5pb::Connack {
            session_present: connack.session_present,
            reason_code: connack.reason_code as u32,
            properties: connack
                .properties
                .map(|props| {
                    props
                        .into_iter()
                        .filter_map(|p| p.try_into().ok())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

impl TryFrom<Property> for mqttv5pb::Property {
    type Error = ();

    fn try_from(p: Property) -> Result<Self, Self::Error> {
        let property_type = match p {
            Property::PayloadFormatIndicator(val) => Some(
                mqttv5pb::property::PropertyType::PayloadFormatIndicator(val != 0),
            ),
            Property::MessageExpiryInterval(val) => {
                Some(mqttv5pb::property::PropertyType::MessageExpiryInterval(val))
            }
            Property::ContentType(val) => Some(mqttv5pb::property::PropertyType::ContentType(val)),
            Property::ResponseTopic(val) => {
                Some(mqttv5pb::property::PropertyType::ResponseTopic(val))
            }
            Property::CorrelationData(val) => {
                Some(mqttv5pb::property::PropertyType::CorrelationData(val))
            }
            Property::SubscriptionIdentifier(val) => Some(
                mqttv5pb::property::PropertyType::SubscriptionIdentifier(val),
            ),
            Property::SessionExpiryInterval(val) => {
                Some(mqttv5pb::property::PropertyType::SessionExpiryInterval(val))
            }
            Property::AssignedClientIdentifier(val) => Some(
                mqttv5pb::property::PropertyType::AssignedClientIdentifier(val),
            ),
            Property::ServerKeepAlive(val) => Some(
                mqttv5pb::property::PropertyType::ServerKeepAlive(val as u32),
            ),
            Property::AuthenticationMethod(val) => {
                Some(mqttv5pb::property::PropertyType::AuthenticationMethod(val))
            }
            Property::AuthenticationData(val) => {
                Some(mqttv5pb::property::PropertyType::AuthenticationData(val))
            }
            Property::RequestProblemInformation(val) => Some(
                mqttv5pb::property::PropertyType::RequestProblemInformation(val != 0),
            ),
            Property::WillDelayInterval(val) => {
                Some(mqttv5pb::property::PropertyType::WillDelayInterval(val))
            }
            Property::RequestResponseInformation(val) => {
                Some(mqttv5pb::property::PropertyType::RequestResponseInformation(val != 0))
            }
            Property::ResponseInformation(val) => {
                Some(mqttv5pb::property::PropertyType::ResponseInformation(val))
            }
            Property::ServerReference(val) => {
                Some(mqttv5pb::property::PropertyType::ServerReference(val))
            }
            Property::ReasonString(val) => {
                Some(mqttv5pb::property::PropertyType::ReasonString(val))
            }
            Property::ReceiveMaximum(val) => {
                Some(mqttv5pb::property::PropertyType::ReceiveMaximum(val as u32))
            }
            Property::TopicAliasMaximum(val) => Some(
                mqttv5pb::property::PropertyType::TopicAliasMaximum(val as u32),
            ),
            Property::TopicAlias(val) => {
                Some(mqttv5pb::property::PropertyType::TopicAlias(val as u32))
            }
            Property::MaximumQoS(val) => {
                Some(mqttv5pb::property::PropertyType::MaximumQos(val as u32))
            }
            Property::RetainAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::RetainAvailable(val != 0))
            }
            Property::UserProperty(key, value) => {
                Some(mqttv5pb::property::PropertyType::UserProperty(
                    mqttv5pb::UserProperty { key, value },
                ))
            }
            Property::MaximumPacketSize(val) => {
                Some(mqttv5pb::property::PropertyType::MaximumPacketSize(val))
            }
            Property::WildcardSubscriptionAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::WildcardSubscriptionAvailable(val != 0))
            }
            Property::SubscriptionIdentifierAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::SubscriptionIdentifiersAvailable(val != 0))
            }
            Property::SharedSubscriptionAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::SharedSubscriptionAvailable(val != 0))
            }
        };
        Ok(mqttv5pb::Property { property_type })
    }
}

impl From<mqttv5pb::Publish> for mqtt_serde::mqttv5::publish::MqttPublish {
    fn from(publish: mqttv5pb::Publish) -> Self {
        mqtt_serde::mqttv5::publish::MqttPublish::new(
            publish.qos as u8,
            publish.topic,
            Some(publish.message_id as u16),
            publish.payload,
            publish.retain,
            publish.dup,
        )
    }
}

impl TryFrom<mqttv5pb::Property> for Property {
    type Error = ();

    fn try_from(p: mqttv5pb::Property) -> Result<Self, Self::Error> {
        match p.property_type {
            Some(mqttv5pb::property::PropertyType::PayloadFormatIndicator(val)) => {
                Ok(Property::PayloadFormatIndicator(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::MessageExpiryInterval(val)) => {
                Ok(Property::MessageExpiryInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::ContentType(val)) => {
                Ok(Property::ContentType(val))
            }
            Some(mqttv5pb::property::PropertyType::ResponseTopic(val)) => {
                Ok(Property::ResponseTopic(val))
            }
            Some(mqttv5pb::property::PropertyType::CorrelationData(val)) => {
                Ok(Property::CorrelationData(val))
            }
            Some(mqttv5pb::property::PropertyType::SubscriptionIdentifier(val)) => {
                Ok(Property::SubscriptionIdentifier(val))
            }
            Some(mqttv5pb::property::PropertyType::SessionExpiryInterval(val)) => {
                Ok(Property::SessionExpiryInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::AssignedClientIdentifier(val)) => {
                Ok(Property::AssignedClientIdentifier(val))
            }
            Some(mqttv5pb::property::PropertyType::ServerKeepAlive(val)) => {
                Ok(Property::ServerKeepAlive(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::AuthenticationMethod(val)) => {
                Ok(Property::AuthenticationMethod(val))
            }
            Some(mqttv5pb::property::PropertyType::AuthenticationData(val)) => {
                Ok(Property::AuthenticationData(val))
            }
            Some(mqttv5pb::property::PropertyType::RequestProblemInformation(val)) => {
                Ok(Property::RequestProblemInformation(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::WillDelayInterval(val)) => {
                Ok(Property::WillDelayInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::RequestResponseInformation(val)) => {
                Ok(Property::RequestResponseInformation(if val {
                    1
                } else {
                    0
                }))
            }
            Some(mqttv5pb::property::PropertyType::ResponseInformation(val)) => {
                Ok(Property::ResponseInformation(val))
            }
            Some(mqttv5pb::property::PropertyType::ServerReference(val)) => {
                Ok(Property::ServerReference(val))
            }
            Some(mqttv5pb::property::PropertyType::ReasonString(val)) => {
                Ok(Property::ReasonString(val))
            }
            Some(mqttv5pb::property::PropertyType::ReceiveMaximum(val)) => {
                Ok(Property::ReceiveMaximum(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::TopicAliasMaximum(val)) => {
                Ok(Property::TopicAliasMaximum(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::TopicAlias(val)) => {
                Ok(Property::TopicAlias(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::MaximumQos(val)) => {
                Ok(Property::MaximumQoS(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::RetainAvailable(val)) => {
                Ok(Property::RetainAvailable(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::UserProperty(user_prop)) => {
                Ok(Property::UserProperty(user_prop.key, user_prop.value))
            }
            Some(mqttv5pb::property::PropertyType::MaximumPacketSize(val)) => {
                Ok(Property::MaximumPacketSize(val))
            }
            Some(mqttv5pb::property::PropertyType::WildcardSubscriptionAvailable(val)) => {
                Ok(Property::WildcardSubscriptionAvailable(if val {
                    1
                } else {
                    0
                }))
            }
            Some(mqttv5pb::property::PropertyType::SubscriptionIdentifiersAvailable(val)) => {
                Ok(Property::SubscriptionIdentifierAvailable(if val {
                    1
                } else {
                    0
                }))
            }
            Some(mqttv5pb::property::PropertyType::SharedSubscriptionAvailable(val)) => {
                Ok(Property::SharedSubscriptionAvailable(if val {
                    1
                } else {
                    0
                }))
            }
            None => Err(()),
        }
    }
}

impl From<mqttv5pb::Connect> for mqtt_serde::mqttv5::connect::MqttConnect {
    fn from(connect: mqttv5pb::Connect) -> Self {
        let properties: Vec<Property> = connect
            .properties
            .into_iter()
            .filter_map(|p| p.try_into().ok())
            .collect();

        mqtt_serde::mqttv5::connect::MqttConnect::new(
            connect.client_id,
            Some(connect.username),
            Some(connect.password),
            None,
            0,
            connect.clean_start,
            properties,
        )
    }
}
