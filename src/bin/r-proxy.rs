use std::env;
use std::net::SocketAddr;

use mqtt_grpc_duality::mqtt_serde::control_packet::MqttControlPacket;
use mqtt_grpc_duality::mqtt_serde::control_packet::MqttPacket;
use mqtt_grpc_duality::mqtt_serde::mqttv5;
use mqtt_grpc_duality::mqtt_serde::mqttv5::common::properties::Property;
use mqtt_grpc_duality::mqtt_serde::mqttv5::connack::MqttConnAck;
use mqtt_grpc_duality::mqtt_serde::mqttv5::connect::MqttConnect;
use mqtt_grpc_duality::mqtt_serde::mqttv5::suback::MqttSubAck;
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
                    Ok(Some(packet)) => {
                        match packet {
                            MqttPacket::Connect(connect) => {
                                info!("Received Connect packet: {:?}", connect);
                                // Connect should be handled during initial connection setup
                                eprintln!("Unexpected Connect packet in client loop");
                            }

                            MqttPacket::ConnAck(connack) => {
                                info!("Received ConnAck packet: {:?}", connack);
                                // ConnAck is typically sent by server, not expected from client
                                eprintln!("Unexpected ConnAck packet from client");
                            }

                            MqttPacket::Publish(publish) => {
                                info!("Received Publish packet: {:?}", publish);
                                // Forward the publish packet to the gRPC server
                                let publish_msg: mqttv5pb::Publish = publish.into();
                                let mut request = tonic::Request::new(publish_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                // Handle different QoS levels
                                let response = conn.grpc_client.mqtt_publish_qos1(request).await;
                                if let Err(e) = response {
                                    eprintln!("Failed to send Publish packet to gRPC server: {:?}", e);
                                }
                            }

                            MqttPacket::PubAck(puback) => {
                                info!("Received PubAck packet: {:?}", puback);
                                // Convert to gRPC and send via MQTTPuback
                                let puback_msg: mqttv5pb::Puback = puback.into();
                                let mut request = tonic::Request::new(puback_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_puback(request).await;
                                if let Err(e) = response {
                                    eprintln!("Failed to send PubAck packet to gRPC server: {:?}", e);
                                } else {
                                    info!("Successfully sent PubAck to gRPC server");
                                }
                            }

                            MqttPacket::PubRec(pubrec) => {
                                info!("Received PubRec packet: {:?}", pubrec);
                                // Convert to gRPC and send via MQTTPubrec
                                let pubrec_msg: mqttv5pb::Pubrec = pubrec.into();
                                let mut request = tonic::Request::new(pubrec_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_pubrec(request).await;
                                match response {
                                    Ok(pubrel_response) => {
                                        info!("Received PubRel response from gRPC server");
                                        // Convert gRPC PubRel back to MQTT and send to client
                                        let mqtt_pubrel: mqttv5::pubrel::MqttPubRel = pubrel_response.into_inner().into();
                                        if let Ok(pubrel_bytes) = mqtt_pubrel.to_bytes() {
                                            if let Err(e) = conn.write_h.write_all(&pubrel_bytes).await {
                                                eprintln!("Failed to send PubRel to client: {:?}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to send PubRec packet to gRPC server: {:?}", e);
                                    }
                                }
                            }

                            MqttPacket::PubRel(pubrel) => {
                                info!("Received PubRel packet: {:?}", pubrel);
                                // Convert to gRPC and send via MQTTPubrel
                                let pubrel_msg: mqttv5pb::Pubrel = pubrel.into();
                                let mut request = tonic::Request::new(pubrel_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_pubrel(request).await;
                                match response {
                                    Ok(pubcomp_response) => {
                                        info!("Received PubComp response from gRPC server");
                                        // Convert gRPC PubComp back to MQTT and send to client
                                        let mqtt_pubcomp: mqttv5::pubcomp::MqttPubComp = pubcomp_response.into_inner().into();
                                        if let Ok(pubcomp_bytes) = mqtt_pubcomp.to_bytes() {
                                            if let Err(e) = conn.write_h.write_all(&pubcomp_bytes).await {
                                                eprintln!("Failed to send PubComp to client: {:?}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to send PubRel packet to gRPC server: {:?}", e);
                                    }
                                }
                            }

                            MqttPacket::PubComp(pubcomp) => {
                                info!("Received PubComp packet: {:?}", pubcomp);
                                // Convert to gRPC and send via MQTTPubcomp
                                let pubcomp_msg: mqttv5pb::Pubcomp = pubcomp.into();
                                let mut request = tonic::Request::new(pubcomp_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_pubcomp(request).await;
                                if let Err(e) = response {
                                    eprintln!("Failed to send PubComp packet to gRPC server: {:?}", e);
                                } else {
                                    info!("Successfully sent PubComp to gRPC server");
                                }
                            }

                            MqttPacket::Subscribe(subscribe) => {
                                info!("Received Subscribe packet: {:?}", subscribe);
                                // Forward the subscribe packet to the gRPC server
                                let subscribe_msg: mqttv5pb::Subscribe = subscribe.into();
                                let mut request = tonic::Request::new(subscribe_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_subscribe(request).await;
                                match response {
                                    Ok(suback_response) => {
                                        info!("Received SubAck from gRPC server");
                                        // Convert gRPC SubAck back to MQTT and send to client
                                        let mqtt_suback: MqttSubAck = suback_response.into_inner().into();
                                        if let Ok(suback_bytes) = mqtt_suback.to_bytes() {
                                            if let Err(e) = conn.write_h.write_all(&suback_bytes).await {
                                                eprintln!("Failed to send SubAck to client: {:?}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to send Subscribe packet to gRPC server: {:?}", e);
                                    }
                                }
                            }

                            MqttPacket::SubAck(suback) => {
                                info!("Received SubAck packet: {:?}", suback);
                                // SubAck is typically sent by server, not expected from client
                                // But if received, convert to gRPC and send via MQTTSuback
                                eprintln!("Unexpected SubAck packet from client, forwarding to gRPC server");

                                let suback_msg: mqttv5pb::Suback = suback.into();
                                let mut request = tonic::Request::new(suback_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_suback(request).await;
                                if let Err(e) = response {
                                    eprintln!("Failed to send SubAck packet to gRPC server: {:?}", e);
                                } else {
                                    info!("Successfully sent SubAck to gRPC server");
                                }
                            }

                            MqttPacket::Unsubscribe(unsubscribe) => {
                                info!("Received Unsubscribe packet: {:?}", unsubscribe);
                                // Convert to gRPC and send via MQTTUnsubscribe
                                let unsubscribe_msg: mqttv5pb::Unsubscribe = unsubscribe.into();
                                let mut request = tonic::Request::new(unsubscribe_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_unsubscribe(request).await;
                                match response {
                                    Ok(unsuback_response) => {
                                        info!("Received UnsubAck from gRPC server");
                                        // Convert gRPC UnsubAck back to MQTT and send to client
                                        let mqtt_unsuback: mqttv5::unsuback::MqttUnsubAck = unsuback_response.into_inner().into();
                                        if let Ok(unsuback_bytes) = mqtt_unsuback.to_bytes() {
                                            if let Err(e) = conn.write_h.write_all(&unsuback_bytes).await {
                                                eprintln!("Failed to send UnsubAck to client: {:?}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to send Unsubscribe packet to gRPC server: {:?}", e);
                                    }
                                }
                            }

                            MqttPacket::UnsubAck(unsuback) => {
                                info!("Received UnsubAck packet: {:?}", unsuback);
                                // UnsubAck is typically sent by server, not expected from client
                                // But if received, convert to gRPC and send via MQTTUnsuback
                                eprintln!("Unexpected UnsubAck packet from client, forwarding to gRPC server");

                                let unsuback_msg: mqttv5pb::Unsuback = unsuback.into();
                                let mut request = tonic::Request::new(unsuback_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_unsuback(request).await;
                                if let Err(e) = response {
                                    eprintln!("Failed to send UnsubAck packet to gRPC server: {:?}", e);
                                } else {
                                    info!("Successfully sent UnsubAck to gRPC server");
                                }
                            }

                            MqttPacket::PingReq(pingreq) => {
                                info!("Received PingReq packet: {:?}", pingreq);
                                // Convert to gRPC and send via MQTTPingreq
                                let pingreq_msg: mqttv5pb::Pingreq = pingreq.into();
                                let mut request = tonic::Request::new(pingreq_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_pingreq(request).await;
                                match response {
                                    Ok(_pingresp_response) => {
                                        info!("Received PingResp from gRPC server");
                                        // Convert gRPC PingResp back to MQTT and send to client
                                        let mqtt_pingresp = mqttv5::pingresp::MqttPingResp::new();
                                        if let Ok(pingresp_bytes) = mqtt_pingresp.to_bytes() {
                                            if let Err(e) = conn.write_h.write_all(&pingresp_bytes).await {
                                                eprintln!("Failed to send PingResp to client: {:?}", e);
                                            } else {
                                                info!("Sent PingResp to client");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to send PingReq packet to gRPC server: {:?}", e);
                                        // Fallback: respond with PingResp directly
                                        let pingresp = mqttv5::pingresp::MqttPingResp::new();
                                        if let Ok(pingresp_bytes) = pingresp.to_bytes() {
                                            if let Err(e) = conn.write_h.write_all(&pingresp_bytes).await {
                                                eprintln!("Failed to send PingResp to client: {:?}", e);
                                            } else {
                                                info!("Sent PingResp to client (fallback)");
                                            }
                                        }
                                    }
                                }
                            }

                            MqttPacket::PingResp(pingresp) => {
                                info!("Received PingResp packet: {:?}", pingresp);
                                // PingResp is typically sent by server, not expected from client
                                eprintln!("Unexpected PingResp packet from client");
                            }

                            MqttPacket::Disconnect(disconnect) => {
                                info!("Received Disconnect packet: {:?}", disconnect);
                                // Convert to gRPC and send via MQTTDisconnect
                                let disconnect_msg: mqttv5pb::Disconnect = disconnect.into();
                                let mut request = tonic::Request::new(disconnect_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_disconnect(request).await;
                                if let Err(e) = response {
                                    eprintln!("Failed to send Disconnect packet to gRPC server: {:?}", e);
                                } else {
                                    info!("Successfully sent Disconnect to gRPC server");
                                }

                                // Client is disconnecting gracefully
                                info!("Client {} is disconnecting gracefully", conn.client_id);
                                return Ok(()); // Exit the function to close connection
                            }

                            MqttPacket::Auth(auth) => {
                                info!("Received Auth packet: {:?}", auth);
                                // Convert to gRPC and send via MQTTAuth
                                let auth_msg: mqttv5pb::Auth = auth.into();
                                let mut request = tonic::Request::new(auth_msg);
                                request.metadata_mut().insert(
                                    "x-client-id",
                                    tonic::metadata::MetadataValue::try_from(
                                        conn.client_id.clone()
                                    ).unwrap(),
                                );

                                let response = conn.grpc_client.mqtt_auth(request).await;
                                match response {
                                    Ok(auth_response) => {
                                        info!("Received Auth response from gRPC server");
                                        // Convert gRPC Auth response back to MQTT and send to client
                                        let mqtt_auth: mqttv5::auth::MqttAuth = auth_response.into_inner().into();
                                        if let Ok(auth_bytes) = mqtt_auth.to_bytes() {
                                            if let Err(e) = conn.write_h.write_all(&auth_bytes).await {
                                                eprintln!("Failed to send Auth response to client: {:?}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to send Auth packet to gRPC server: {:?}", e);
                                    }
                                }
                            }
                        }
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

impl From<mqttv5::subscribe::MqttSubscribe> for mqttv5pb::Subscribe {
    fn from(subscribe: mqttv5::subscribe::MqttSubscribe) -> Self {
        mqttv5pb::Subscribe {
            message_id: subscribe.packet_id as u32,
            subscriptions: subscribe
                .subscriptions
                .into_iter()
                .map(|sub| mqttv5pb::TopicSubscription {
                    topic_filter: sub.topic_filter,
                    qos: sub.qos as u32,
                    no_local: sub.no_local,
                    retain_as_published: sub.retain_as_published,
                    retain_handling: sub.retain_handling as u32,
                })
                .collect(),
            properties: convert_mqtt_properties_to_grpc(subscribe.properties),
        }
    }
}

impl From<mqttv5pb::Suback> for MqttSubAck {
    fn from(suback_grpc: mqttv5pb::Suback) -> Self {
        MqttSubAck {
            packet_id: suback_grpc.message_id as u16,
            reason_codes: suback_grpc
                .reason_codes
                .into_iter()
                .map(|code| code as u8)
                .collect(),
            properties: convert_grpc_properties_to_mqtt(suback_grpc.properties),
        }
    }
}

// Add conversion implementations for other packet types
impl From<mqttv5::puback::MqttPubAck> for mqttv5pb::Puback {
    fn from(puback: mqttv5::puback::MqttPubAck) -> Self {
        mqttv5pb::Puback {
            message_id: puback.packet_id as u32,
            reason_code: puback.reason_code as u32,
            properties: convert_mqtt_properties_to_grpc(puback.properties),
        }
    }
}

impl From<mqttv5::pubrec::MqttPubRec> for mqttv5pb::Pubrec {
    fn from(pubrec: mqttv5::pubrec::MqttPubRec) -> Self {
        mqttv5pb::Pubrec {
            message_id: pubrec.packet_id as u32,
            reason_code: pubrec.reason_code as u32,
            properties: convert_mqtt_properties_to_grpc(pubrec.properties),
        }
    }
}

impl From<mqttv5::pubrel::MqttPubRel> for mqttv5pb::Pubrel {
    fn from(pubrel: mqttv5::pubrel::MqttPubRel) -> Self {
        mqttv5pb::Pubrel {
            message_id: pubrel.packet_id as u32,
            reason_code: pubrel.reason_code as u32,
            properties: convert_mqtt_properties_to_grpc(pubrel.properties),
        }
    }
}

impl From<mqttv5::pubcomp::MqttPubComp> for mqttv5pb::Pubcomp {
    fn from(pubcomp: mqttv5::pubcomp::MqttPubComp) -> Self {
        mqttv5pb::Pubcomp {
            message_id: pubcomp.packet_id as u32,
            reason_code: pubcomp.reason_code as u32,
            properties: convert_mqtt_properties_to_grpc(pubcomp.properties),
        }
    }
}

impl From<mqttv5::unsubscribe::MqttUnsubscribe> for mqttv5pb::Unsubscribe {
    fn from(unsubscribe: mqttv5::unsubscribe::MqttUnsubscribe) -> Self {
        mqttv5pb::Unsubscribe {
            message_id: unsubscribe.packet_id as u32,
            topic_filters: unsubscribe.topic_filters,
            properties: convert_mqtt_properties_to_grpc(unsubscribe.properties),
        }
    }
}

impl From<mqttv5::pingreq::MqttPingReq> for mqttv5pb::Pingreq {
    fn from(_pingreq: mqttv5::pingreq::MqttPingReq) -> Self {
        mqttv5pb::Pingreq {}
    }
}

impl From<mqttv5::disconnect::MqttDisconnect> for mqttv5pb::Disconnect {
    fn from(disconnect: mqttv5::disconnect::MqttDisconnect) -> Self {
        mqttv5pb::Disconnect {
            reason_code: disconnect.reason_code as u32,
            properties: convert_mqtt_properties_to_grpc(disconnect.properties),
        }
    }
}

impl From<mqttv5::auth::MqttAuth> for mqttv5pb::Auth {
    fn from(auth: mqttv5::auth::MqttAuth) -> Self {
        mqttv5pb::Auth {
            reason_code: auth.reason_code as u32,
            properties: convert_mqtt_properties_to_grpc(auth.properties),
        }
    }
}

// Create MqttPacket from individual packet types for RelayPacket RPC
impl From<mqttv5::publish::MqttPublish> for mqttv5pb::MqttPacket {
    fn from(publish: mqttv5::publish::MqttPublish) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Publish(publish.into())),
        }
    }
}

impl From<mqttv5::puback::MqttPubAck> for mqttv5pb::MqttPacket {
    fn from(puback: mqttv5::puback::MqttPubAck) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Puback(puback.into())),
        }
    }
}

impl From<mqttv5::pubrec::MqttPubRec> for mqttv5pb::MqttPacket {
    fn from(pubrec: mqttv5::pubrec::MqttPubRec) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Pubrec(pubrec.into())),
        }
    }
}

impl From<mqttv5::pubrel::MqttPubRel> for mqttv5pb::MqttPacket {
    fn from(pubrel: mqttv5::pubrel::MqttPubRel) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Pubrel(pubrel.into())),
        }
    }
}

impl From<mqttv5::pubcomp::MqttPubComp> for mqttv5pb::MqttPacket {
    fn from(pubcomp: mqttv5::pubcomp::MqttPubComp) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Pubcomp(pubcomp.into())),
        }
    }
}

impl From<mqttv5::subscribe::MqttSubscribe> for mqttv5pb::MqttPacket {
    fn from(subscribe: mqttv5::subscribe::MqttSubscribe) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Subscribe(subscribe.into())),
        }
    }
}

impl From<mqttv5::unsubscribe::MqttUnsubscribe> for mqttv5pb::MqttPacket {
    fn from(unsubscribe: mqttv5::unsubscribe::MqttUnsubscribe) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Unsubscribe(
                unsubscribe.into(),
            )),
        }
    }
}

impl From<mqttv5::pingreq::MqttPingReq> for mqttv5pb::MqttPacket {
    fn from(pingreq: mqttv5::pingreq::MqttPingReq) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Pingreq(pingreq.into())),
        }
    }
}

impl From<mqttv5::disconnect::MqttDisconnect> for mqttv5pb::MqttPacket {
    fn from(disconnect: mqttv5::disconnect::MqttDisconnect) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Disconnect(disconnect.into())),
        }
    }
}

impl From<mqttv5::auth::MqttAuth> for mqttv5pb::MqttPacket {
    fn from(auth: mqttv5::auth::MqttAuth) -> Self {
        mqttv5pb::MqttPacket {
            packet: Some(mqttv5pb::mqtt_packet::Packet::Auth(auth.into())),
        }
    }
}

// Add response conversions for gRPC to MQTT
impl From<mqttv5pb::Pubrel> for mqttv5::pubrel::MqttPubRel {
    fn from(pubrel_grpc: mqttv5pb::Pubrel) -> Self {
        mqttv5::pubrel::MqttPubRel {
            packet_id: pubrel_grpc.message_id as u16,
            reason_code: pubrel_grpc.reason_code as u8,
            properties: convert_grpc_properties_to_mqtt(pubrel_grpc.properties),
        }
    }
}

impl From<mqttv5pb::Pubcomp> for mqttv5::pubcomp::MqttPubComp {
    fn from(pubcomp_grpc: mqttv5pb::Pubcomp) -> Self {
        mqttv5::pubcomp::MqttPubComp {
            packet_id: pubcomp_grpc.message_id as u16,
            reason_code: pubcomp_grpc.reason_code as u8,
            properties: convert_grpc_properties_to_mqtt(pubcomp_grpc.properties),
        }
    }
}

impl From<mqttv5pb::Unsuback> for mqttv5::unsuback::MqttUnsubAck {
    fn from(unsuback_grpc: mqttv5pb::Unsuback) -> Self {
        mqttv5::unsuback::MqttUnsubAck {
            packet_id: unsuback_grpc.message_id as u16,
            reason_codes: unsuback_grpc
                .reason_codes
                .into_iter()
                .map(|code| code as u8)
                .collect(),
            properties: convert_grpc_properties_to_mqtt(unsuback_grpc.properties),
        }
    }
}

// Add missing MQTT to gRPC conversions
impl From<mqttv5::suback::MqttSubAck> for mqttv5pb::Suback {
    fn from(suback: mqttv5::suback::MqttSubAck) -> Self {
        mqttv5pb::Suback {
            message_id: suback.packet_id as u32,
            reason_codes: suback
                .reason_codes
                .into_iter()
                .map(|code| code as u32)
                .collect(),
            properties: convert_mqtt_properties_to_grpc(suback.properties),
        }
    }
}

impl From<mqttv5::unsuback::MqttUnsubAck> for mqttv5pb::Unsuback {
    fn from(unsuback: mqttv5::unsuback::MqttUnsubAck) -> Self {
        mqttv5pb::Unsuback {
            message_id: unsuback.packet_id as u32,
            reason_codes: unsuback
                .reason_codes
                .into_iter()
                .map(|code| code as u32)
                .collect(),
            properties: convert_mqtt_properties_to_grpc(unsuback.properties),
        }
    }
}

impl From<mqttv5pb::Auth> for mqttv5::auth::MqttAuth {
    fn from(auth_grpc: mqttv5pb::Auth) -> Self {
        mqttv5::auth::MqttAuth {
            reason_code: auth_grpc.reason_code as u8,
            properties: convert_grpc_properties_to_mqtt(auth_grpc.properties),
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
