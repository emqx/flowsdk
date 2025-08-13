use std::env;
use std::net::SocketAddr;

use mqtt_grpc_duality::mqtt_serde::control_packet::MqttControlPacket;
use mqtt_grpc_duality::mqtt_serde::control_packet::MqttPacket;
use mqtt_grpc_duality::mqtt_serde::mqttv5;
use mqtt_grpc_duality::mqtt_serde::mqttv5::common::properties::Property;
use mqtt_grpc_duality::mqtt_serde::mqttv5::connack::MqttConnAck;
use mqtt_grpc_duality::mqtt_serde::mqttv5::connect::MqttConnect;
use mqtt_grpc_duality::mqtt_serde::parser::stream::MqttParser;
use mqttv5pb::mqtt_relay_service_client::MqttRelayServiceClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tracing::info;

use tonic::Status;

use dashmap::DashMap;
use std::sync::Arc;

fn convert_grpc_properties_to_mqtt(properties: Vec<mqttv5pb::Property>) -> Vec<Property> {
    properties
        .into_iter()
        .filter_map(|p| match p.property_type {
            Some(mqttv5pb::property::PropertyType::PayloadFormatIndicator(val)) => {
                Some(Property::PayloadFormatIndicator(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::MessageExpiryInterval(val)) => {
                Some(Property::MessageExpiryInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::ContentType(val)) => {
                Some(Property::ContentType(val))
            }
            Some(mqttv5pb::property::PropertyType::ResponseTopic(val)) => {
                Some(Property::ResponseTopic(val))
            }
            Some(mqttv5pb::property::PropertyType::CorrelationData(val)) => {
                Some(Property::CorrelationData(val))
            }
            Some(mqttv5pb::property::PropertyType::SubscriptionIdentifier(val)) => {
                Some(Property::SubscriptionIdentifier(val))
            }
            Some(mqttv5pb::property::PropertyType::SessionExpiryInterval(val)) => {
                Some(Property::SessionExpiryInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::AssignedClientIdentifier(val)) => {
                Some(Property::AssignedClientIdentifier(val))
            }
            Some(mqttv5pb::property::PropertyType::ServerKeepAlive(val)) => {
                Some(Property::ServerKeepAlive(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::AuthenticationMethod(val)) => {
                Some(Property::AuthenticationMethod(val))
            }
            Some(mqttv5pb::property::PropertyType::AuthenticationData(val)) => {
                Some(Property::AuthenticationData(val))
            }
            Some(mqttv5pb::property::PropertyType::RequestProblemInformation(val)) => {
                Some(Property::RequestProblemInformation(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::WillDelayInterval(val)) => {
                Some(Property::WillDelayInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::RequestResponseInformation(val)) => {
                Some(Property::RequestResponseInformation(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::ResponseInformation(val)) => {
                Some(Property::ResponseInformation(val))
            }
            Some(mqttv5pb::property::PropertyType::ServerReference(val)) => {
                Some(Property::ServerReference(val))
            }
            Some(mqttv5pb::property::PropertyType::ReasonString(val)) => {
                Some(Property::ReasonString(val))
            }
            Some(mqttv5pb::property::PropertyType::ReceiveMaximum(val)) => {
                Some(Property::ReceiveMaximum(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::TopicAliasMaximum(val)) => {
                Some(Property::TopicAliasMaximum(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::TopicAlias(val)) => {
                Some(Property::TopicAlias(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::MaximumQos(val)) => {
                Some(Property::MaximumQoS(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::RetainAvailable(val)) => {
                Some(Property::RetainAvailable(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::UserProperty(val)) => {
                Some(Property::UserProperty(val.key, val.value))
            }
            Some(mqttv5pb::property::PropertyType::MaximumPacketSize(val)) => {
                Some(Property::MaximumPacketSize(val))
            }
            Some(mqttv5pb::property::PropertyType::WildcardSubscriptionAvailable(val)) => {
                Some(Property::WildcardSubscriptionAvailable(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::SubscriptionIdentifiersAvailable(val)) => {
                Some(Property::SubscriptionIdentifierAvailable(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::SharedSubscriptionAvailable(val)) => {
                Some(Property::SharedSubscriptionAvailable(val as u8))
            }
            None => None,
        })
        .collect()
}

fn convert_mqtt_properties_to_grpc(properties: Vec<Property>) -> Vec<mqttv5pb::Property> {
    properties
        .into_iter()
        .map(|p| match p {
            Property::PayloadFormatIndicator(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::PayloadFormatIndicator(
                    val != 0,
                )),
            },
            Property::MessageExpiryInterval(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::MessageExpiryInterval(val)),
            },
            Property::ContentType(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::ContentType(val)),
            },
            Property::ResponseTopic(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::ResponseTopic(val)),
            },
            Property::CorrelationData(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::CorrelationData(val)),
            },
            Property::SubscriptionIdentifier(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::SubscriptionIdentifier(
                    val,
                )),
            },
            Property::SessionExpiryInterval(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::SessionExpiryInterval(val)),
            },
            Property::AssignedClientIdentifier(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::AssignedClientIdentifier(
                    val,
                )),
            },
            Property::ServerKeepAlive(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::ServerKeepAlive(
                    val as u32,
                )),
            },
            Property::AuthenticationMethod(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::AuthenticationMethod(val)),
            },
            Property::AuthenticationData(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::AuthenticationData(val)),
            },
            Property::RequestProblemInformation(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::RequestProblemInformation(
                    val != 0,
                )),
            },
            Property::WillDelayInterval(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::WillDelayInterval(val)),
            },
            Property::RequestResponseInformation(val) => mqttv5pb::Property {
                property_type: Some(
                    mqttv5pb::property::PropertyType::RequestResponseInformation(val != 0),
                ),
            },
            Property::ResponseInformation(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::ResponseInformation(val)),
            },
            Property::ServerReference(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::ServerReference(val)),
            },
            Property::ReasonString(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::ReasonString(val)),
            },
            Property::ReceiveMaximum(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::ReceiveMaximum(val as u32)),
            },
            Property::TopicAliasMaximum(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::TopicAliasMaximum(
                    val as u32,
                )),
            },
            Property::TopicAlias(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::TopicAlias(val as u32)),
            },
            Property::MaximumQoS(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::MaximumQos(val as u32)),
            },
            Property::RetainAvailable(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::RetainAvailable(val != 0)),
            },
            Property::UserProperty(key, value) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::UserProperty(
                    mqttv5pb::UserProperty { key, value },
                )),
            },
            Property::MaximumPacketSize(val) => mqttv5pb::Property {
                property_type: Some(mqttv5pb::property::PropertyType::MaximumPacketSize(val)),
            },
            Property::WildcardSubscriptionAvailable(val) => mqttv5pb::Property {
                property_type: Some(
                    mqttv5pb::property::PropertyType::WildcardSubscriptionAvailable(val != 0),
                ),
            },
            Property::SubscriptionIdentifierAvailable(val) => mqttv5pb::Property {
                property_type: Some(
                    mqttv5pb::property::PropertyType::SubscriptionIdentifiersAvailable(val != 0),
                ),
            },
            Property::SharedSubscriptionAvailable(val) => mqttv5pb::Property {
                property_type: Some(
                    mqttv5pb::property::PropertyType::SharedSubscriptionAvailable(val != 0),
                ),
            },
        })
        .collect()
}

pub mod mqttv5pb {
    tonic::include_proto!("mqttv5"); // The string specified here must match the proto package name
}

pub struct ClientConnection {
    pub client_id: String,
    pub write_h: OwnedWriteHalf,
    pub read_h: OwnedReadHalf,
    pub grpc_client: MqttRelayServiceClient<tonic::transport::Channel>,
    pub sender: Sender<mqttv5pb::Publish>,
}

async fn run_proxy(
    listen_port: u16,
    destination: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(("0.0.0.0", listen_port)).await?;
    info!("Listening on 0.0.0.0:{}", listen_port);

    let shared_data: Arc<
        DashMap<
            String,
            tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>>,
        >,
    > = Arc::new(DashMap::new());

    // @TODO: currently single acceptor
    loop {
        info!("A Accepting connection...");
        let (incoming_stream, addr) = listener.accept().await?;
        let conn_store = shared_data.clone();
        info!("Accepted connection from {}", addr);
        spawn(async move {
            match handle_new_conn(incoming_stream, destination).await {
                Ok(client_conn) => {
                    info!(
                        "New client connection established: {:?}",
                        client_conn.client_id
                    );
                    //@TODO: handle old conn kick.
                    let client_id = client_conn.client_id.clone();
                    info!("Client ID: {:?}", client_id);

                    let task = spawn(async move { mqtt_client_loop(client_conn).await });

                    // Store the client connection in the shared map
                    conn_store.insert(
                        client_id, // value is the handle of this spawned task
                        task,
                    );
                }
                Err(e) => {
                    eprintln!("Error handling connection from {}: {}", addr, e);
                }
            }
        });
    }
}

async fn mqtt_client_loop(
    mut conn: ClientConnection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut parser = MqttParser::new();
    loop {
        tokio::select! {
            // Read from the client connection
            _ = conn.read_h.read_buf(parser.buffer_mut()) => {
                match parser.next_packet() {
                    Ok(Some(MqttPacket::Publish(publish))) => {
                        // Handle incoming publish packets
                        info!("Received Publish packet: {:?}", publish);
                        // Forward the publish packet to the gRPC server
                        let publish_msg: mqttv5pb::Publish = publish.into();
                        //@TODO: not necessarily with x-client-id
                        let mut request = tonic::Request::new(publish_msg);
                        request.metadata_mut().insert(
                            "x-client-id",
                            tonic::metadata::MetadataValue::try_from(
                                conn.client_id.clone()
                            ).unwrap(),
                        );
                        // @TODO: here may not be QoS1
                        let response = conn.grpc_client.mqtt_publish_qos1(
                            request
                        ).await;
                        if response.is_err() {
                            eprintln!("Failed to send Publish packet to gRPC server {:?}", response.err().unwrap());
                        }
                    }

                    Ok(Some(packet)) => {
                        eprintln!("Received unexpected packet: {:?}", packet);
                    }
                    Ok(None) => {
                        // Incomplete packet, continue reading
                    }
                    Err(e) => {
                        eprintln!("Error parsing MQTT packet: {:?}", e);
                    }
                }

            }
        }
    }
}

async fn wait_for_mqtt_connect(
    stream: &mut OwnedReadHalf,
    parser: &mut MqttParser,
) -> Result<MqttConnect, Status> {
    loop {
        let n = stream
            .read_buf(parser.buffer_mut())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        if n == 0 {
            return Err(Status::unavailable("Connection closed by the client"));
        }

        match parser.next_packet() {
            Ok(Some(mqtt_grpc_duality::mqtt_serde::control_packet::MqttPacket::Connect(
                connect,
            ))) => {
                eprintln!("Received Connect packet: {:?}", connect);
                return Ok(connect);
            }
            Ok(Some(_)) => {
                // @TODO: return an error for unexpected packets
                eprintln!("Received unexpected packet type");
                return Err(Status::internal("Unexpected packet type received"));
            }
            Ok(None) => {
                // Incomplete packet, continue reading
            }
            Err(mqtt_grpc_duality::mqtt_serde::parser::ParseError::More(_, _)) => {
                // Incomplete packet, continue reading
                eprintln!("Incomplete packet, continue reading");
                continue;
            }
            Err(e) => {
                eprintln!("Error parsing Connect: {:?}", e);
                return Err(Status::internal("Failed to parse Connect"));
            }
        }
    }
}

async fn handle_new_conn(
    incoming: TcpStream,
    destination: SocketAddr,
) -> Result<ClientConnection, Box<dyn std::error::Error + Send + Sync>> {
    // 0. Split stream to read and write halves
    let (mut read_half, mut write_half) = incoming.into_split();
    // 1. wait_for_mqtt_connection @TODO: handle timeout

    let mut parser: MqttParser = MqttParser::new();

    // @TODO: handle timeout
    // 1.1 Wait for MQTT Connect packet
    let connect = wait_for_mqtt_connect(&mut read_half, &mut parser)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

    // 2. Connect to the destination grpc server
    let mut client: MqttRelayServiceClient<tonic::transport::Channel> =
        MqttRelayServiceClient::connect(format!("http://{}", destination)).await?;

    let client_id: String = connect.client_id.clone();
    let connect_pb: mqttv5pb::Connect = connect.into();

    // 1. Addressing gRPC server
    let request = tonic::Request::new(connect_pb);

    let response = client.mqtt_connect(request).await?;

    let connack_grpc = response.into_inner();

    let mqtt_connack: MqttConnAck = connack_grpc.into();

    // 2. Reply mqtt connack
    write_half
        .write_all(mqtt_connack.to_bytes()?.as_slice())
        .await?;

    // 3. update connections map
    return Ok(ClientConnection {
        client_id,
        write_h: write_half,
        read_h: read_half,            // Keep the read handle for further processing
        grpc_client: client,          // Store the gRPC client for further use
        sender: mpsc::channel(100).0, // Create a new channel for this client
    });
}

impl From<mqttv5::publish::MqttPublish> for mqttv5pb::Publish {
    fn from(publish: mqttv5::publish::MqttPublish) -> Self {
        mqttv5pb::Publish {
            topic: publish.topic_name,
            payload: publish.payload,
            qos: publish.qos as i32,
            retain: publish.retain,
            dup: publish.dup,
            message_id: publish.packet_id.unwrap_or(0) as u32,
            properties: convert_mqtt_properties_to_grpc(publish.properties),
        }
    }
}
impl From<MqttConnect> for mqttv5pb::Connect {
    fn from(connect: MqttConnect) -> Self {
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

impl From<mqttv5pb::Connack> for MqttConnAck {
    fn from(connack_grpc: mqttv5pb::Connack) -> Self {
        MqttConnAck {
            session_present: connack_grpc.session_present,
            reason_code: connack_grpc.reason_code as u8,
            properties: Some(convert_grpc_properties_to_mqtt(connack_grpc.properties)),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!(
            "Usage: {} <listen_port> <destination_host>:<destination_port>",
            args[0]
        );
        return Err("Invalid number of arguments".into());
    }

    let listen_port = args[1].parse::<u16>()?;
    let destination: SocketAddr = args[2].parse()?;

    run_proxy(listen_port, destination).await?;
    Ok(())
}
