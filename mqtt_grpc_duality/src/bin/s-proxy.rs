use dashmap::DashMap;
use flowsdk::mqtt_serde::control_packet::MqttControlPacket;
use std::sync::Arc;
use std::{env, net::SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{debug, error, info};

// Import shared conversions and protobuf types from the proxy workspace
use mqtt_grpc_proxy::convert_mqtt_v5_to_stream_payload;
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
        debug!("Received relay packet request: {:?}", request);

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
        let client_id = &request.get_ref().client_id;
        info!("MQTT Connect request from client: {}", client_id);
        debug!("Connect request details: {:?}", request);

        let clientid = request.get_ref().client_id.clone();
        let (tx, rx) = mpsc::channel::<BrokerControlMessage>(32);

        let token = CancellationToken::new();

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
                    error!(
                        "Timeout waiting for stream connection for client: {}",
                        client_id_for_task
                    );
                    return;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            };

            mqtt_client_loop_simple(
                token.clone(),
                connections,
                rx,
                stream,
                response_tx,
                session_id,
                5,
            )
            .await;
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
        let msg_id = request.get_ref().message_id;
        debug!("MQTT Publish QoS1 request, message_id: {}", msg_id);

        let req_meta = request.metadata().get("x-client-id");
        if let Some(client_id) = req_meta.and_then(|v| v.to_str().ok()) {
            debug!("Processing publish for client: {}", client_id);

            if let Some(connection) = self.connections.get(client_id) {
                let (tx, _, _) = connection.value();
                let publish_msg = request.get_ref().clone();
                if tx
                    .send(BrokerControlMessage::Publish(publish_msg))
                    .await
                    .is_ok()
                {
                    debug!(
                        "Publish message forwarded to broker for client: {}",
                        client_id
                    );
                } else {
                    error!(
                        "Failed to forward publish message for client: {}, broker channel closed",
                        client_id
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
        let message_id = request.get_ref().message_id;
        debug!("MQTT Subscribe request, message_id: {}", message_id);

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
        let message_id = request.get_ref().message_id;
        debug!("MQTT PubAck request, message_id: {}", message_id);

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
        let message_id = request.get_ref().message_id;
        debug!(
            "MQTT PubRec request, message_id: {} - responding with PubRel",
            message_id
        );

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
        let message_id = request.get_ref().message_id;
        debug!(
            "MQTT PubRel request, message_id: {} - completing QoS 2 flow with PubComp",
            message_id
        );

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
        let message_id = request.get_ref().message_id;
        debug!("MQTT PubComp request, message_id: {}", message_id);

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
        let message_id = request.get_ref().message_id;
        let topic_count = request.get_ref().topic_filters.len();
        debug!(
            "MQTT Unsubscribe request, message_id: {}, topics: {}",
            message_id, topic_count
        );

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
        _request: Request<mqttv5pb::Pingreq>,
    ) -> Result<Response<mqttv5pb::Pingresp>, Status> {
        debug!("MQTT PingReq request - responding with PingResp");

        // Respond immediately with PingResp
        let pingresp = mqttv5pb::Pingresp {};
        Ok(Response::new(pingresp))
    }

    async fn mqtt_disconnect(
        &self,
        request: Request<mqttv5pb::Disconnect>,
    ) -> Result<Response<RelayResponse>, Status> {
        debug!("MQTT Disconnect request");

        // TODO: Clean up client connection
        let req_meta = request.metadata().get("x-client-id");
        if let Some(client_id) = req_meta.and_then(|v| v.to_str().ok()) {
            info!("Disconnecting client: {}", client_id);
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
        let reason_code = request.get_ref().reason_code;
        debug!("MQTT Auth request, reason_code: {}", reason_code);

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
        let message_id = request.get_ref().message_id;
        debug!("MQTT SubAck request, message_id: {}", message_id);

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
        let message_id = request.get_ref().message_id;
        debug!("MQTT UnsubAck request, message_id: {}", message_id);

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
        info!("Initiating bidirectional MQTT stream");

        let mut incoming_stream = request.into_inner();
        let (response_tx, response_rx) = tokio::sync::mpsc::channel(128);
        let connections = self.connections.clone();
        let broker_addr = self.broker_addr; // Capture broker_addr before move

        let token = CancellationToken::new();

        // @FIXME: This is a one green thread per request, which is not ideal for scalability
        // Handle the bidirectional stream
        tokio::spawn(async move {
            loop {
                let cloned_token: CancellationToken = token.clone();

                match incoming_stream.next().await {
                    None => {
                        debug!("Stream closed by client, exiting grpc stream handler");
                        token.cancel();
                        break;
                    }
                    Some(Err(e)) => {
                        error!("Error in incoming grpc stream: {}", e);
                        break;
                    }

                    Some(Ok(stream_msg)) => {
                        debug!(
                            "Received stream message for session: {}",
                            stream_msg.session_id
                        );

                        match stream_msg.payload {
                            Some(mqttv5pb::mqtt_stream_message::Payload::Connect(connect)) => {
                                // Handle MQTT Connect message directly
                                let client_id = connect.client_id.clone();
                                info!("Received Connect from client: {}", client_id);

                                // Create broker connection for this client
                                let connections = connections.clone();
                                let response_tx_for_spawn = response_tx.clone();
                                let session_id = stream_msg.session_id.clone();

                                tokio::spawn(async move {
                                    let connect_request = tonic::Request::new(connect);
                                    match mqtt_connect_to_broker(connect_request, broker_addr).await
                                    {
                                        Ok(stream) => {
                                            let (broker_tx, broker_rx) =
                                                tokio::sync::mpsc::channel::<BrokerControlMessage>(
                                                    32,
                                                );

                                            // Store connection info
                                            connections.insert(
                                                client_id.clone(),
                                                (
                                                    broker_tx,
                                                    response_tx_for_spawn.clone(),
                                                    session_id.clone(),
                                                ),
                                            );

                                            info!(
                                                "Broker connection established for client: {}",
                                                client_id
                                            );

                                            // Start the MQTT client loop to handle broker communication
                                            mqtt_client_loop_simple(
                                                cloned_token,
                                                connections.clone(),
                                                broker_rx,
                                                stream,
                                                response_tx_for_spawn,
                                                session_id,
                                                5,
                                            )
                                            .await;
                                        }
                                        Err(e) => {
                                            error!(
                                                "Failed to connect to broker for client {}: {}",
                                                client_id, e
                                            );
                                        }
                                    }
                                });
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Publish(publish)) => {
                                // Forward publish messages to broker via the publish channel
                                debug!("Received publish from stream: {:?}", publish);

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
                                            error!(
                                        "Failed to forward publish to broker for client {}: {}",
                                        client_id, e
                                    );
                                        }
                                    }
                                } else {
                                    error!(
                                        "No client found for session: {}",
                                        stream_msg.session_id
                                    );
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Subscribe(subscribe)) => {
                                // Forward subscribe messages to broker
                                debug!("Received subscribe from stream: {:?}", subscribe);

                                // Find the connection and forward Subscribe to broker
                                if let Some(client_id) =
                                    find_client_by_session(&connections, &stream_msg.session_id)
                                {
                                    if let Some(conn_info) = connections.get(&client_id) {
                                        let (broker_tx, _, _) = conn_info.value();

                                        // Forward Subscribe message to broker via control channel
                                        if let Err(e) = broker_tx
                                            .send(BrokerControlMessage::Subscribe(
                                                subscribe.clone(),
                                            ))
                                            .await
                                        {
                                            error!("Failed to send Subscribe to broker: {}", e);
                                        } else {
                                            debug!("Subscribe forwarded to broker for client: {} with topics: {:?}",
                                        client_id, subscribe.subscriptions);
                                        }

                                        // Note: SubAck will be generated by broker and forwarded via mqtt_client_loop_simple
                                    } else {
                                        error!(
                                            "Connection info not found for client: {}",
                                            client_id
                                        );
                                    }
                                } else {
                                    error!(
                                        "Client not found for session: {}",
                                        stream_msg.session_id
                                    );
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Unsubscribe(
                                unsubscribe,
                            )) => {
                                // Forward unsubscribe messages to broker
                                debug!("Received unsubscribe from stream: {:?}", unsubscribe);

                                if let Some(client_id) =
                                    find_client_by_session(&connections, &stream_msg.session_id)
                                {
                                    if let Some(conn) = connections.get(&client_id) {
                                        let (broker_tx, _, _) = conn.value();
                                        if let Err(e) = broker_tx
                                            .send(BrokerControlMessage::Unsubscribe(unsubscribe))
                                            .await
                                        {
                                            error!(
                                        "Failed to forward Unsubscribe to broker for client {}: {}",
                                        client_id, e
                                    );
                                        } else {
                                            debug!(
                                                "Unsubscribe forwarded to broker for client: {}",
                                                client_id
                                            );
                                        }
                                    }
                                } else {
                                    error!(
                                        "No client found for session: {}",
                                        stream_msg.session_id
                                    );
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Puback(puback)) => {
                                // Handle PubAck from client (QoS 1 acknowledgment) - forward to broker
                                debug!("Received PubAck from stream: {:?}", puback);

                                if let Some(client_id) =
                                    find_client_by_session(&connections, &stream_msg.session_id)
                                {
                                    if let Some(conn) = connections.get(&client_id) {
                                        let (broker_tx, _, _) = conn.value();
                                        if let Err(e) = broker_tx
                                            .send(BrokerControlMessage::PubAck(puback))
                                            .await
                                        {
                                            error!(
                                        "Failed to forward PubAck to broker for client {}: {}",
                                        client_id, e
                                    );
                                        } else {
                                            debug!(
                                                "PubAck forwarded to broker for client: {}",
                                                client_id
                                            );
                                        }
                                    }
                                } else {
                                    error!(
                                        "No client found for session: {}",
                                        stream_msg.session_id
                                    );
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Pubrec(pubrec)) => {
                                // Handle PubRec from client (QoS 2 flow) - forward to broker
                                debug!("Received PubRec from stream: {:?}", pubrec);

                                if let Some(client_id) =
                                    find_client_by_session(&connections, &stream_msg.session_id)
                                {
                                    if let Some(conn) = connections.get(&client_id) {
                                        let (broker_tx, _, _) = conn.value();
                                        if let Err(e) = broker_tx
                                            .send(BrokerControlMessage::PubRec(pubrec))
                                            .await
                                        {
                                            error!(
                                        "Failed to forward PubRec to broker for client {}: {}",
                                        client_id, e
                                    );
                                        } else {
                                            debug!(
                                                "PubRec forwarded to broker for client: {}",
                                                client_id
                                            );
                                        }
                                    }
                                } else {
                                    error!(
                                        "No client found for session: {}",
                                        stream_msg.session_id
                                    );
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Pubrel(pubrel)) => {
                                // Handle PubRel from client (QoS 2 flow) - forward to broker
                                debug!("Received PubRel from stream: {:?}", pubrel);

                                if let Some(client_id) =
                                    find_client_by_session(&connections, &stream_msg.session_id)
                                {
                                    if let Some(conn) = connections.get(&client_id) {
                                        let (broker_tx, _, _) = conn.value();
                                        if let Err(e) = broker_tx
                                            .send(BrokerControlMessage::PubRel(pubrel))
                                            .await
                                        {
                                            error!(
                                        "Failed to forward PubRel to broker for client {}: {}",
                                        client_id, e
                                    );
                                        } else {
                                            debug!(
                                                "PubRel forwarded to broker for client: {}",
                                                client_id
                                            );
                                        }
                                    }
                                } else {
                                    error!(
                                        "No client found for session: {}",
                                        stream_msg.session_id
                                    );
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Pingreq(_)) => {
                                // Handle ping from client
                                debug!("Received PingReq from stream");

                                // Send PingResp response
                                let pingresp_response = mqttv5pb::MqttStreamMessage {
                                    session_id: stream_msg.session_id,
                                    sequence_id: stream_msg.sequence_id + 1,
                                    direction: mqttv5pb::MessageDirection::BrokerToClient as i32,
                                    payload: Some(
                                        mqttv5pb::mqtt_stream_message::Payload::Pingresp(
                                            mqttv5pb::Pingresp {},
                                        ),
                                    ),
                                };

                                if let Err(e) = response_tx.send(Ok(pingresp_response)).await {
                                    error!("Failed to send PingResp response: {}", e);
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Pubcomp(pubcomp)) => {
                                // Handle PubComp from client (QoS 2 flow) - forward to broker
                                debug!("Received PubComp from stream: {:?}", pubcomp);

                                if let Some(client_id) =
                                    find_client_by_session(&connections, &stream_msg.session_id)
                                {
                                    if let Some(conn) = connections.get(&client_id) {
                                        let (broker_tx, _, _) = conn.value();
                                        if let Err(e) = broker_tx
                                            .send(BrokerControlMessage::PubComp(pubcomp))
                                            .await
                                        {
                                            error!(
                                        "Failed to forward PubComp to broker for client {}: {}",
                                        client_id, e
                                    );
                                        } else {
                                            debug!(
                                                "PubComp forwarded to broker for client: {}",
                                                client_id
                                            );
                                        }
                                    }
                                } else {
                                    error!(
                                        "No client found for session: {}",
                                        stream_msg.session_id
                                    );
                                }
                            }
                            Some(mqttv5pb::mqtt_stream_message::Payload::Disconnect(
                                disconnect,
                            )) => {
                                // Handle disconnect from client - forward to broker
                                debug!("Received Disconnect from stream: {:?}", disconnect);

                                if let Some(client_id) =
                                    find_client_by_session(&connections, &stream_msg.session_id)
                                {
                                    debug!("Client {} disconnecting", client_id);

                                    // Forward Disconnect to broker first
                                    if let Some(conn) = connections.get(&client_id) {
                                        let (broker_tx, _, _) = conn.value();
                                        if let Err(e) = broker_tx
                                            .send(BrokerControlMessage::Disconnect(disconnect))
                                            .await
                                        {
                                            error!(
                                        "Failed to forward Disconnect to broker for client {}: {}",
                                        client_id, e
                                    );
                                        } else {
                                            debug!(
                                                "Disconnect forwarded to broker for client: {}",
                                                client_id
                                            );
                                        }
                                    }

                                    // Remove connection after forwarding to broker
                                    connections.remove(&client_id);
                                }
                            }
                            _ => {
                                error!(
                                    "Unhandled stream message payload: {:?}",
                                    stream_msg.payload
                                );
                            }
                        }
                    }
                }
            }
        });

        // @FIXME: This is a temporary solution to return a stream
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
    debug!(
        "Connecting to MQTT broker at {} with {} bytes",
        broker_addr,
        data.len()
    );
    stream
        .write_all(&data)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;
    info!("CONNECT packet sent to MQTT broker at {}", broker_addr);
    Ok(stream)
}

async fn mqtt_client_loop_simple(
    token: CancellationToken,
    connections: Arc<DashMap<String, ConnectionInfo>>,
    mut rx: mpsc::Receiver<BrokerControlMessage>,
    mut stream: tokio::net::TcpStream,
    response_tx: mpsc::Sender<Result<mqttv5pb::MqttStreamMessage, Status>>,
    session_id: String,
    mqtt_version: u8,
) {
    let mut parser = MqttParser::new(16384, mqtt_version); // 16KB buffer
    let mut sequence_counter = 1u64;

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                // The token was cancelled
                info!("Cancellation token triggered, exiting MQTT client loop for session: {}", session_id);
                connections.remove(&session_id);
                break;
            }

            // Handle control messages from r-proxy
            Some(control_msg) = rx.recv() => {
                debug!("Processing control message for broker");

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
                    error!("Failed to write to broker: {}", e);
                    break;
                }
            }
            // Handle incoming data from the broker
            result = stream.read_buf(parser.buffer_mut()) => {
                match result {
                    Ok(0) => {
                        info!("Connection closed by the broker");
                        connections.remove(&session_id);
                        break;
                    }
                    Ok(n) => {
                        debug!("Read {} bytes from broker", n);
                        while let Ok(Some(packet)) = parser.next_packet() {
                            debug!("Received packet from broker: {:?}", packet);

                            // Convert MQTT packet to gRPC stream message and forward to r-proxy
                            if let Some(payload) = convert_mqtt_v5_to_stream_payload(&packet) {
                                let stream_msg = mqttv5pb::MqttStreamMessage {
                                    session_id: session_id.clone(),
                                    sequence_id: sequence_counter,
                                    direction: mqttv5pb::MessageDirection::BrokerToClient as i32,
                                    payload: Some(payload),
                                };

                                sequence_counter += 1;

                                if let Err(e) = response_tx.send(Ok(stream_msg)).await {
                                    error!("Failed to send message to r-proxy via stream: {}", e);
                                    break;
                                }
                            }

                            debug!("Processed packet from broker, continuing to next packet");
                        }
                    }
                    Err(e) => {
                        error!("Error reading from stream: {}", e);
                        break;
                    }
                }
            }
            else => {
                info!("No more data to read from broker, exiting MQTT client loop for session: {}", session_id);
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        error!(
            "Usage: {} <grpc_listen_port> <broker_host>:<broker_port>",
            args[0]
        );
        error!("Example: {} 50515 127.0.0.1:1883", args[0]);
        return Err("Invalid number of arguments".into());
    }

    let grpc_listen_port = args[1].parse::<u16>()?;
    let broker_addr: SocketAddr = args[2].parse()?;

    let grpc_addr = format!("127.0.0.1:{}", grpc_listen_port).parse()?;
    let relay = MyRelay::new(broker_addr);

    info!("s-proxy starting:");
    info!("  gRPC server listening on: {}", grpc_addr);
    info!("  MQTT broker address: {}", broker_addr);

    Server::builder()
        .add_service(MqttRelayServiceServer::new(relay))
        .serve(grpc_addr)
        .await?;

    Ok(())
}
