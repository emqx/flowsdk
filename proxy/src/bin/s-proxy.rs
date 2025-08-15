use dashmap::DashMap;
use mqtt_grpc_duality::mqtt_serde::control_packet::{
    MqttControlPacket, MqttPacket as InternalMqttPacket,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};

// Import shared conversions and protobuf types from the proxy workspace
use mqtt_grpc_proxy::mqttv5pb;
use mqttv5pb::mqtt_relay_service_client::MqttRelayServiceClient;
use mqttv5pb::mqtt_relay_service_server::{MqttRelayService, MqttRelayServiceServer};
use mqttv5pb::{MqttPacket, RelayResponse};

use mqtt_grpc_duality::mqtt_serde;
use mqtt_grpc_duality::mqtt_serde::parser::stream::MqttParser;

use crate::mpsc::Sender;
use tokio::net::TcpSocket;
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct MyRelay {
    // Shared state for managing connections - store both publish sender and gRPC client
    connections: Arc<DashMap<String, (Sender<mqttv5pb::Publish>, MqttRelayServiceClient<Channel>)>>,
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

        let stream = match mqtt_connect_to_broker(request).await {
            Ok(s) => s,
            Err(e) => {
                return Err(e);
            }
        };

        // Create gRPC client connection to r-proxy
        let channel = match Channel::from_static("http://[::1]:50516").connect().await {
            Ok(channel) => channel,
            Err(e) => {
                eprintln!("Failed to connect to r-proxy: {}", e);
                return Err(Status::internal("Failed to connect to r-proxy"));
            }
        };
        let grpc_client = MqttRelayServiceClient::new(channel);

        self.connections
            .insert(clientid.clone(), (tx.clone(), grpc_client.clone()));

        let _join = tokio::task::spawn(async move {
            mqtt_client_loop(rx, stream, grpc_client).await;
        });

        // Return a default successful CONNACK - the real CONNACK will be forwarded by mqtt_client_loop
        let m = mqttv5pb::Connack {
            session_present: false,
            reason_code: 0, // Success
            properties: vec![],
        };

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

            if let Some(connection) = self.connections.get(client_id) {
                let (tx, _) = connection.value();
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

    async fn mqtt_puback(
        &self,
        request: Request<mqttv5pb::Puback>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a PubAck request: {:?}", request);

        // In s-proxy, PubAck typically doesn't need to be forwarded to broker
        // as it's an acknowledgment from client to server
        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_pubrec(
        &self,
        request: Request<mqttv5pb::Pubrec>,
    ) -> Result<Response<mqttv5pb::Pubrel>, Status> {
        println!("Got a PubRec request: {:?}", request);

        // For QoS 2 flow, respond with PubRel
        let pubrel = mqttv5pb::Pubrel {
            message_id: request.get_ref().message_id,
            reason_code: 0,
            properties: Default::default(),
        };
        Ok(Response::new(pubrel))
    }

    async fn mqtt_pubrel(
        &self,
        request: Request<mqttv5pb::Pubrel>,
    ) -> Result<Response<mqttv5pb::Pubcomp>, Status> {
        println!("Got a PubRel request: {:?}", request);

        // Complete QoS 2 flow with PubComp
        let pubcomp = mqttv5pb::Pubcomp {
            message_id: request.get_ref().message_id,
            reason_code: 0,
            properties: Default::default(),
        };
        Ok(Response::new(pubcomp))
    }

    async fn mqtt_pubcomp(
        &self,
        request: Request<mqttv5pb::Pubcomp>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a PubComp request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_unsubscribe(
        &self,
        request: Request<mqttv5pb::Unsubscribe>,
    ) -> Result<Response<mqttv5pb::Unsuback>, Status> {
        println!("Got an Unsubscribe request: {:?}", request);

        // TODO: Forward unsubscribe to broker if needed
        let topic_count = request.get_ref().topic_filters.len();
        let unsuback = mqttv5pb::Unsuback {
            message_id: request.get_ref().message_id,
            reason_codes: vec![0; topic_count], // Success for all topics
            properties: Default::default(),
        };
        Ok(Response::new(unsuback))
    }

    async fn mqtt_pingreq(
        &self,
        request: Request<mqttv5pb::Pingreq>,
    ) -> Result<Response<mqttv5pb::Pingresp>, Status> {
        println!("Got a PingReq request: {:?}", request);

        // Respond immediately with PingResp
        let pingresp = mqttv5pb::Pingresp {};
        Ok(Response::new(pingresp))
    }

    async fn mqtt_disconnect(
        &self,
        request: Request<mqttv5pb::Disconnect>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a Disconnect request: {:?}", request);

        // TODO: Clean up client connection
        let req_meta = request.metadata().get("x-client-id");
        if let Some(client_id) = req_meta.and_then(|v| v.to_str().ok()) {
            println!("Disconnecting client: {}", client_id);
            self.connections.remove(client_id);
        }

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_auth(
        &self,
        request: Request<mqttv5pb::Auth>,
    ) -> Result<Response<mqttv5pb::Auth>, Status> {
        println!("Got an Auth request: {:?}", request);

        // TODO: Handle authentication logic
        let auth_response = mqttv5pb::Auth {
            reason_code: 0, // Success
            properties: Default::default(),
        };
        Ok(Response::new(auth_response))
    }

    async fn mqtt_suback(
        &self,
        request: Request<mqttv5pb::Suback>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a SubAck request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_unsuback(
        &self,
        request: Request<mqttv5pb::Unsuback>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got an UnsubAck request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    // Streaming implementation for bidirectional communication
    type StreamMqttMessagesStream = tokio_stream::wrappers::ReceiverStream<Result<mqttv5pb::MqttStreamMessage, Status>>;

    async fn stream_mqtt_messages(
        &self,
        _request: Request<tonic::Streaming<mqttv5pb::MqttStreamMessage>>,
    ) -> Result<Response<Self::StreamMqttMessagesStream>, Status> {
        println!("Got a streaming request: stream_mqtt_messages");
        
        // For now, create a simple response stream
        // This would need to be enhanced based on your actual streaming requirements
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        
        // Close the channel immediately for now - this can be enhanced later
        drop(tx);
        
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

async fn mqtt_connect_to_broker(
    request: Request<mqttv5pb::Connect>,
) -> Result<tokio::net::TcpStream, Status> {
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
    println!("CONNECT packet sent to MQTT broker at {}", addr);
    Ok(stream)
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
    mut grpc_client: MqttRelayServiceClient<Channel>,
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

                            // Convert MQTT packet to gRPC and forward to r-proxy
                            if let Err(e) = forward_mqtt_packet_to_grpc(&packet, &mut grpc_client).await {
                                eprintln!("Failed to forward packet to r-proxy: {}", e);
                            }
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

async fn forward_mqtt_packet_to_grpc(
    packet: &InternalMqttPacket,
    grpc_client: &mut MqttRelayServiceClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    match packet {
        InternalMqttPacket::Publish(publish) => {
            let req = Request::new(publish.clone().into());
            grpc_client.mqtt_publish_qos1(req).await?;
        }
        InternalMqttPacket::PubAck(puback) => {
            let req = Request::new(puback.clone().into());
            grpc_client.mqtt_puback(req).await?;
        }
        InternalMqttPacket::PubRec(pubrec) => {
            let req = Request::new(pubrec.clone().into());
            grpc_client.mqtt_pubrec(req).await?;
        }
        InternalMqttPacket::PubRel(pubrel) => {
            let req = Request::new(pubrel.clone().into());
            grpc_client.mqtt_pubrel(req).await?;
        }
        InternalMqttPacket::PubComp(pubcomp) => {
            let req = Request::new(pubcomp.clone().into());
            grpc_client.mqtt_pubcomp(req).await?;
        }
        InternalMqttPacket::SubAck(suback) => {
            let req = Request::new(suback.clone().into());
            grpc_client.mqtt_suback(req).await?;
        }
        InternalMqttPacket::UnsubAck(unsuback) => {
            let req = Request::new(unsuback.clone().into());
            grpc_client.mqtt_unsuback(req).await?;
        }
        InternalMqttPacket::PingResp(_pingresp) => {
            // Convert PingResp to PingReq for gRPC call since broker sends PingResp
            // but client expects PingReq pattern in reverse direction
            let pingreq = mqttv5pb::Pingreq {};
            let req = Request::new(pingreq);
            grpc_client.mqtt_pingreq(req).await?;
        }
        InternalMqttPacket::ConnAck(connack) => {
            // CONNACK from broker indicates successful connection
            println!("Received CONNACK from broker: {:?}", connack);
        }
        InternalMqttPacket::Disconnect(disconnect) => {
            let req = Request::new(disconnect.clone().into());
            grpc_client.mqtt_disconnect(req).await?;
        }
        InternalMqttPacket::Auth(auth) => {
            let req = Request::new(auth.clone().into());
            grpc_client.mqtt_auth(req).await?;
        }
        _ => {
            println!("Unhandled packet type from broker: {:?}", packet);
        }
    }

    Ok(())
}
