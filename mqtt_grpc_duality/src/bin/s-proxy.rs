use dashmap::DashMap;
use flowsdk::mqtt_serde::control_packet::MqttControlPacket;
use std::sync::Arc;
use std::{env, net::SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tonic::{transport::Server, Request, Response, Status};

// Import shared conversions and protobuf types from the proxy workspace
use mqtt_grpc_proxy::mqttv5pb;
use mqttv5pb::mqtt_relay_service_server::{MqttRelayService, MqttRelayServiceServer};
use mqttv5pb::{MqttPacket, RelayResponse};

use flowsdk::mqtt_serde;
use flowsdk::mqtt_serde::parser::stream::MqttParser;

use tokio::net::TcpSocket;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

// Control message enum for broker communication
#[derive(Debug, Clone)]
pub enum BrokerControlMessage {
    Publish(mqttv5pb::Publish),
    Subscribe(mqttv5pb::Subscribe),
    Unsubscribe(mqttv5pb::Unsubscribe),
    PubAck(mqttv5pb::Puback),
    PubRec(mqttv5pb::Pubrec),
    PubRel(mqttv5pb::Pubrel),
    PubComp(mqttv5pb::Pubcomp),
    PingReq(mqttv5pb::Pingreq),
    Disconnect(mqttv5pb::Disconnect),
}

type ConnectionInfo = (
    mpsc::Sender<BrokerControlMessage>, // Control message sender for all MQTT commands
    mpsc::Sender<Result<mqttv5pb::MqttStreamMessage, Status>>, // Response stream sender
    String,                             // session_id
);

#[derive(Debug)]
pub struct MyRelay {
    // Shared state for managing connections - store publish sender and response stream sender
    connections: Arc<DashMap<String, ConnectionInfo>>,
    broker_addr: SocketAddr,
}

impl MyRelay {
    pub fn new(broker_addr: SocketAddr) -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            broker_addr,
        }
    }
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
        println!("Got a connect request: {:?}", request);

        let clientid = request.get_ref().client_id.clone();
        let (tx, rx) = mpsc::channel::<BrokerControlMessage>(32);

        let stream = match mqtt_connect_to_broker(request, self.broker_addr).await {
            Ok(s) => s,
            Err(e) => {
                return Err(e);
            }
        };

        // Store connection info without creating a circular gRPC connection
        // The response stream sender will be set when stream_mqtt_messages is called
        let dummy_sender = mpsc::channel(1).0; // Will be replaced by streaming
        let session_id = format!("session-{}", clientid); // Generate session ID

        self.connections.insert(
            clientid.clone(),
            (tx.clone(), dummy_sender, session_id.clone()),
        );

        let connections = self.connections.clone();
        let client_id_for_task = clientid.clone();

        let _join = tokio::task::spawn(async move {
            // Wait for the response stream sender to be set by stream_mqtt_messages
            let mut attempts = 0;
            let response_tx = loop {
                if let Some(conn) = connections.get(&client_id_for_task) {
                    let (_, response_tx, _) = conn.value();
                    // Check if it's not the dummy sender (dummy has capacity 1 and is immediately closed)
                    if response_tx.capacity() > 1 {
                        break response_tx.clone();
                    }
                }

                attempts += 1;
                if attempts > 100 {
                    // 10 seconds timeout
                    eprintln!(
                        "Timeout waiting for stream connection for client {}",
                        client_id_for_task
                    );
                    return;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };

            mqtt_client_loop_simple(rx, stream, response_tx, session_id).await;
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
        println!("Got a publish request: {:?}", request);
        let msg_id = request.get_ref().message_id;

        let req_meta = request.metadata().get("x-client-id");
        if let Some(client_id) = req_meta.and_then(|v| v.to_str().ok()) {
            println!("Client ID: {}", client_id);

            if let Some(connection) = self.connections.get(client_id) {
                let (tx, _, _) = connection.value();
                if tx
                    .send(BrokerControlMessage::Publish(request.into_inner().clone()))
                    .await
                    .is_ok()
                {
                    println!("Publish message sent successfully to broker channel.");
                } else {
                    eprintln!(
                        "Failed to send publish message to broker channel, receiver dropped."
                    );
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
    type StreamMqttMessagesStream =
        tokio_stream::wrappers::ReceiverStream<Result<mqttv5pb::MqttStreamMessage, Status>>;

    async fn stream_mqtt_messages(
        &self,
        request: Request<tonic::Streaming<mqttv5pb::MqttStreamMessage>>,
    ) -> Result<Response<Self::StreamMqttMessagesStream>, Status> {
        println!("Got a streaming request: stream_mqtt_messages");

        let mut incoming_stream = request.into_inner();
        let (response_tx, response_rx) = tokio::sync::mpsc::channel(128);
        let connections = self.connections.clone();
        let broker_addr = self.broker_addr; // Capture broker_addr before move

        // Handle the bidirectional stream
        tokio::spawn(async move {
            while let Some(Ok(stream_msg)) = incoming_stream.next().await {
                println!("Received stream message: {:?}", stream_msg);

                match stream_msg.payload {
                    Some(mqttv5pb::mqtt_stream_message::Payload::SessionControl(
                        session_control,
                    )) => {
                        // Handle session establishment
                        if session_control.control_type
                            == mqttv5pb::session_control::ControlType::Establish as i32
                        {
                            println!(
                                "Establishing session for client: {}",
                                session_control.client_id
                            );

                            // Update or create connection with response stream sender
                            if let Some(mut conn) = connections.get_mut(&session_control.client_id)
                            {
                                conn.1 = response_tx.clone();
                                conn.2 = stream_msg.session_id.clone(); // Update with the actual session_id from r-proxy
                            } else {
                                // Create new connection info for streaming-only clients
                                println!(
                                    "Creating new connection for streaming client: {}",
                                    session_control.client_id
                                );

                                // Create broker TCP connection for this client
                                let connect_pb = mqttv5pb::Connect {
                                    protocol_name: "MQTT".to_string(),
                                    protocol_version: 5,
                                    client_id: session_control.client_id.clone(),
                                    clean_start: true,
                                    keep_alive: 60,
                                    properties: vec![],
                                    will: None,
                                    username: String::new(),
                                    password: vec![],
                                };

                                let connect_request = tonic::Request::new(connect_pb);
                                match mqtt_connect_to_broker(connect_request, broker_addr).await {
                                    Ok(stream) => {
                                        let (broker_tx, broker_rx) =
                                            tokio::sync::mpsc::channel::<BrokerControlMessage>(32);

                                        // Store connection info
                                        connections.insert(
                                            session_control.client_id.clone(),
                                            (
                                                broker_tx,
                                                response_tx.clone(),
                                                stream_msg.session_id.clone(),
                                            ),
                                        );

                                        // Start broker message loop
                                        let response_tx_clone = response_tx.clone();
                                        let session_id_clone = stream_msg.session_id.clone();
                                        tokio::spawn(async move {
                                            mqtt_client_loop_simple(
                                                broker_rx,
                                                stream,
                                                response_tx_clone,
                                                session_id_clone,
                                            )
                                            .await;
                                        });

                                        println!("Broker connection established for streaming client: {}", session_control.client_id);
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "Failed to connect to broker for client {}: {}",
                                            session_control.client_id, e
                                        );
                                        // Create dummy connection to prevent errors
                                        let (dummy_tx, _) =
                                            tokio::sync::mpsc::channel::<BrokerControlMessage>(1);
                                        connections.insert(
                                            session_control.client_id.clone(),
                                            (
                                                dummy_tx,
                                                response_tx.clone(),
                                                stream_msg.session_id.clone(),
                                            ),
                                        );
                                    }
                                }
                            }

                            // ConnAck will be sent by the broker and forwarded via mqtt_client_loop_simple
                            println!(
                                "Session established for client: {}, awaiting broker ConnAck",
                                session_control.client_id
                            );
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Publish(publish)) => {
                        // Forward publish messages to broker via the publish channel
                        println!("Received publish from stream: {:?}", publish);

                        // Extract client ID from session or use a default lookup mechanism
                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            if let Some(conn) = connections.get(&client_id) {
                                let (publish_tx, _, _) = conn.value();
                                if let Err(e) = publish_tx
                                    .send(BrokerControlMessage::Publish(publish))
                                    .await
                                {
                                    eprintln!(
                                        "Failed to forward publish to broker for client {}: {}",
                                        client_id, e
                                    );
                                }
                            }
                        } else {
                            eprintln!("No client found for session: {}", stream_msg.session_id);
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Subscribe(subscribe)) => {
                        // Forward subscribe messages to broker
                        println!("Received subscribe from stream: {:?}", subscribe);

                        // Find the connection and forward Subscribe to broker
                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            if let Some(conn_info) = connections.get(&client_id) {
                                let (broker_tx, _, _) = conn_info.value();

                                // Forward Subscribe message to broker via control channel
                                if let Err(e) = broker_tx
                                    .send(BrokerControlMessage::Subscribe(subscribe.clone()))
                                    .await
                                {
                                    eprintln!("Failed to send Subscribe to broker: {}", e);
                                } else {
                                    println!("Subscribe forwarded to broker for client: {} with topics: {:?}", 
                                        client_id, subscribe.subscriptions);
                                }

                                // Note: SubAck will be generated by broker and forwarded via mqtt_client_loop_simple
                            } else {
                                eprintln!("Connection info not found for client: {}", client_id);
                            }
                        } else {
                            eprintln!("Client not found for session: {}", stream_msg.session_id);
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Unsubscribe(unsubscribe)) => {
                        // Forward unsubscribe messages to broker
                        println!("Received unsubscribe from stream: {:?}", unsubscribe);

                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            println!("Unsubscribe forwarded for client: {}", client_id);

                            // Send UnsubAck response back via stream
                            let unsuback_response = mqttv5pb::MqttStreamMessage {
                                session_id: stream_msg.session_id,
                                sequence_id: stream_msg.sequence_id + 1,
                                direction: mqttv5pb::MessageDirection::BrokerToClient as i32,
                                payload: Some(mqttv5pb::mqtt_stream_message::Payload::Unsuback(
                                    mqttv5pb::Unsuback {
                                        message_id: unsubscribe.message_id,
                                        reason_codes: vec![0; unsubscribe.topic_filters.len()], // Success for all
                                        properties: vec![],
                                    },
                                )),
                            };

                            if let Err(e) = response_tx.send(Ok(unsuback_response)).await {
                                eprintln!("Failed to send UnsubAck response: {}", e);
                            }
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Puback(puback)) => {
                        // Handle PubAck from client (QoS 1 acknowledgment) - forward to broker
                        println!("Received PubAck from stream: {:?}", puback);

                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            if let Some(conn) = connections.get(&client_id) {
                                let (broker_tx, _, _) = conn.value();
                                if let Err(e) =
                                    broker_tx.send(BrokerControlMessage::PubAck(puback)).await
                                {
                                    eprintln!(
                                        "Failed to forward PubAck to broker for client {}: {}",
                                        client_id, e
                                    );
                                } else {
                                    println!(
                                        "PubAck forwarded to broker for client: {}",
                                        client_id
                                    );
                                }
                            }
                        } else {
                            eprintln!("No client found for session: {}", stream_msg.session_id);
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Pubrec(pubrec)) => {
                        // Handle PubRec from client (QoS 2 flow) - forward to broker
                        println!("Received PubRec from stream: {:?}", pubrec);

                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            if let Some(conn) = connections.get(&client_id) {
                                let (broker_tx, _, _) = conn.value();
                                if let Err(e) =
                                    broker_tx.send(BrokerControlMessage::PubRec(pubrec)).await
                                {
                                    eprintln!(
                                        "Failed to forward PubRec to broker for client {}: {}",
                                        client_id, e
                                    );
                                } else {
                                    println!(
                                        "PubRec forwarded to broker for client: {}",
                                        client_id
                                    );
                                }
                            }
                        } else {
                            eprintln!("No client found for session: {}", stream_msg.session_id);
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Pubrel(pubrel)) => {
                        // Handle PubRel from client (QoS 2 flow) - forward to broker
                        println!("Received PubRel from stream: {:?}", pubrel);

                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            if let Some(conn) = connections.get(&client_id) {
                                let (broker_tx, _, _) = conn.value();
                                if let Err(e) =
                                    broker_tx.send(BrokerControlMessage::PubRel(pubrel)).await
                                {
                                    eprintln!(
                                        "Failed to forward PubRel to broker for client {}: {}",
                                        client_id, e
                                    );
                                } else {
                                    println!(
                                        "PubRel forwarded to broker for client: {}",
                                        client_id
                                    );
                                }
                            }
                        } else {
                            eprintln!("No client found for session: {}", stream_msg.session_id);
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Pingreq(_)) => {
                        // Handle ping from client
                        println!("Received PingReq from stream");

                        // Send PingResp response
                        let pingresp_response = mqttv5pb::MqttStreamMessage {
                            session_id: stream_msg.session_id,
                            sequence_id: stream_msg.sequence_id + 1,
                            direction: mqttv5pb::MessageDirection::BrokerToClient as i32,
                            payload: Some(mqttv5pb::mqtt_stream_message::Payload::Pingresp(
                                mqttv5pb::Pingresp {},
                            )),
                        };

                        if let Err(e) = response_tx.send(Ok(pingresp_response)).await {
                            eprintln!("Failed to send PingResp response: {}", e);
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Pubcomp(pubcomp)) => {
                        // Handle PubComp from client (QoS 2 flow) - forward to broker
                        println!("Received PubComp from stream: {:?}", pubcomp);

                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            if let Some(conn) = connections.get(&client_id) {
                                let (broker_tx, _, _) = conn.value();
                                if let Err(e) =
                                    broker_tx.send(BrokerControlMessage::PubComp(pubcomp)).await
                                {
                                    eprintln!(
                                        "Failed to forward PubComp to broker for client {}: {}",
                                        client_id, e
                                    );
                                } else {
                                    println!(
                                        "PubComp forwarded to broker for client: {}",
                                        client_id
                                    );
                                }
                            }
                        } else {
                            eprintln!("No client found for session: {}", stream_msg.session_id);
                        }
                    }
                    Some(mqttv5pb::mqtt_stream_message::Payload::Disconnect(disconnect)) => {
                        // Handle disconnect from client
                        println!("Received Disconnect from stream: {:?}", disconnect);

                        if let Some(client_id) =
                            find_client_by_session(&connections, &stream_msg.session_id)
                        {
                            println!("Client {} disconnecting", client_id);
                            connections.remove(&client_id);
                        }
                    }
                    _ => {
                        println!("Unhandled stream message payload: {:?}", stream_msg.payload);
                    }
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            response_rx,
        )))
    }
}

async fn mqtt_connect_to_broker(
    request: Request<mqttv5pb::Connect>,
    broker_addr: SocketAddr,
) -> Result<tokio::net::TcpStream, Status> {
    let msg: mqtt_serde::mqttv5::connect::MqttConnect = request.into_inner().into();
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    let mut stream = socket.connect(broker_addr).await?;
    let data = msg.to_bytes().unwrap(); //@FIXME unwrap
    println!(
        "Connecting to MQTT broker at {} with data {:?}",
        broker_addr, data
    );
    stream
        .write_all(&data)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;
    println!("CONNECT packet sent to MQTT broker at {}", broker_addr);
    Ok(stream)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!(
            "Usage: {} <grpc_listen_port> <broker_host>:<broker_port>",
            args[0]
        );
        eprintln!("Example: {} 50515 127.0.0.1:1883", args[0]);
        return Err("Invalid number of arguments".into());
    }

    let grpc_listen_port = args[1].parse::<u16>()?;
    let broker_addr: SocketAddr = args[2].parse()?;

    let grpc_addr = format!("127.0.0.1:{}", grpc_listen_port).parse()?;
    let relay = MyRelay::new(broker_addr);

    println!("s-proxy starting:");
    println!("  gRPC server listening on: {}", grpc_addr);
    println!("  MQTT broker address: {}", broker_addr);

    Server::builder()
        .add_service(MqttRelayServiceServer::new(relay))
        .serve(grpc_addr)
        .await?;

    Ok(())
}

async fn mqtt_client_loop_simple(
    mut rx: mpsc::Receiver<BrokerControlMessage>,
    mut stream: tokio::net::TcpStream,
    response_tx: mpsc::Sender<Result<mqttv5pb::MqttStreamMessage, Status>>,
    session_id: String,
) {
    let mut parser = MqttParser::new();
    let mut sequence_counter = 1u64;

    loop {
        tokio::select! {
            Some(control_msg) = rx.recv() => {
                println!("Received control message: {:?}", control_msg);

                // Convert control message to MQTT bytes and send to broker
                let mqtt_bytes = match control_msg {
                    BrokerControlMessage::Publish(pb_publish) => {
                        let mqtt_publish: mqtt_serde::mqttv5::publish::MqttPublish = pb_publish.into();
                        mqtt_publish.to_bytes().unwrap()
                    }
                    BrokerControlMessage::Subscribe(pb_subscribe) => {
                        let mqtt_subscribe: mqtt_serde::mqttv5::subscribe::MqttSubscribe = pb_subscribe.into();
                        mqtt_subscribe.to_bytes().unwrap()
                    }
                    BrokerControlMessage::Unsubscribe(pb_unsubscribe) => {
                        let mqtt_unsubscribe: mqtt_serde::mqttv5::unsubscribe::MqttUnsubscribe = pb_unsubscribe.into();
                        mqtt_unsubscribe.to_bytes().unwrap()
                    }
                    BrokerControlMessage::PubAck(pb_puback) => {
                        let mqtt_puback: mqtt_serde::mqttv5::puback::MqttPubAck = pb_puback.into();
                        mqtt_puback.to_bytes().unwrap()
                    }
                    BrokerControlMessage::PubRec(pb_pubrec) => {
                        let mqtt_pubrec: mqtt_serde::mqttv5::pubrec::MqttPubRec = pb_pubrec.into();
                        mqtt_pubrec.to_bytes().unwrap()
                    }
                    BrokerControlMessage::PubRel(pb_pubrel) => {
                        let mqtt_pubrel: mqtt_serde::mqttv5::pubrel::MqttPubRel = pb_pubrel.into();
                        mqtt_pubrel.to_bytes().unwrap()
                    }
                    BrokerControlMessage::PubComp(pb_pubcomp) => {
                        let mqtt_pubcomp: mqtt_serde::mqttv5::pubcomp::MqttPubComp = pb_pubcomp.into();
                        mqtt_pubcomp.to_bytes().unwrap()
                    }
                    BrokerControlMessage::PingReq(_) => {
                        let mqtt_pingreq = mqtt_serde::mqttv5::pingreq::MqttPingReq::new();
                        mqtt_pingreq.to_bytes().unwrap()
                    }
                    BrokerControlMessage::Disconnect(pb_disconnect) => {
                        let mqtt_disconnect: mqtt_serde::mqttv5::disconnect::MqttDisconnect = pb_disconnect.into();
                        mqtt_disconnect.to_bytes().unwrap()
                    }
                };

                if let Err(e) = stream.write_all(&mqtt_bytes).await {
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

                            // Convert MQTT packet to gRPC stream message and forward to r-proxy
                            if let Some(payload) = convert_mqtt_to_stream_payload(&packet) {
                                let stream_msg = mqttv5pb::MqttStreamMessage {
                                    session_id: session_id.clone(),
                                    sequence_id: sequence_counter,
                                    direction: mqttv5pb::MessageDirection::BrokerToClient as i32,
                                    payload: Some(payload),
                                };

                                sequence_counter += 1;

                                if let Err(e) = response_tx.send(Ok(stream_msg)).await {
                                    eprintln!("Failed to send message to r-proxy via stream: {}", e);
                                    break;
                                }
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

// Helper function to find client by session ID
fn find_client_by_session(
    connections: &Arc<DashMap<String, ConnectionInfo>>,
    session_id: &str,
) -> Option<String> {
    for entry in connections.iter() {
        let (_, _, stored_session_id) = entry.value();
        if stored_session_id == session_id {
            return Some(entry.key().clone());
        }
    }
    None
}

// Helper function to convert MQTT packets to stream payloads
fn convert_mqtt_to_stream_payload(
    packet: &flowsdk::mqtt_serde::control_packet::MqttPacket,
) -> Option<mqttv5pb::mqtt_stream_message::Payload> {
    use flowsdk::mqtt_serde::control_packet::MqttPacket;

    match packet {
        MqttPacket::Publish(publish) => Some(mqttv5pb::mqtt_stream_message::Payload::Publish(
            publish.clone().into(),
        )),
        MqttPacket::PubAck(puback) => Some(mqttv5pb::mqtt_stream_message::Payload::Puback(
            puback.clone().into(),
        )),
        MqttPacket::PubRec(pubrec) => Some(mqttv5pb::mqtt_stream_message::Payload::Pubrec(
            pubrec.clone().into(),
        )),
        MqttPacket::PubRel(pubrel) => Some(mqttv5pb::mqtt_stream_message::Payload::Pubrel(
            pubrel.clone().into(),
        )),
        MqttPacket::PubComp(pubcomp) => Some(mqttv5pb::mqtt_stream_message::Payload::Pubcomp(
            pubcomp.clone().into(),
        )),
        MqttPacket::SubAck(suback) => Some(mqttv5pb::mqtt_stream_message::Payload::Suback(
            suback.clone().into(),
        )),
        MqttPacket::UnsubAck(unsuback) => Some(mqttv5pb::mqtt_stream_message::Payload::Unsuback(
            unsuback.clone().into(),
        )),
        MqttPacket::PingResp(_) => Some(mqttv5pb::mqtt_stream_message::Payload::Pingresp(
            mqttv5pb::Pingresp {},
        )),
        MqttPacket::ConnAck(connack) => Some(mqttv5pb::mqtt_stream_message::Payload::Connack(
            connack.clone().into(),
        )),
        MqttPacket::Disconnect(disconnect) => Some(
            mqttv5pb::mqtt_stream_message::Payload::Disconnect(disconnect.clone().into()),
        ),
        MqttPacket::Auth(auth) => Some(mqttv5pb::mqtt_stream_message::Payload::Auth(
            auth.clone().into(),
        )),
        _ => {
            println!("Unhandled packet type from broker: {:?}", packet);
            None
        }
    }
}
