use dashmap::DashMap;
use std::sync::Arc;
use std::{env, net::SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{debug, error, info, Level};

// Import shared conversions and protobuf types from the proxy workspace
use mqtt_grpc_proxy::convert_mqtt_to_stream_message;
use mqtt_grpc_proxy::mqtt_unified_pb;
use mqtt_unified_pb::mqtt_relay_service_server::{MqttRelayService, MqttRelayServiceServer};
use mqtt_unified_pb::MqttStreamMessage;

use flowsdk::mqtt_serde::parser::stream::MqttParser;

use tokio::net::TcpSocket;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

// Control message enum for broker communication
#[derive(Debug, Clone)]
pub enum BrokerControlMessage {
    Packet(flowsdk::mqtt_serde::control_packet::MqttPacket),
}

type ConnectionInfo = (
    mpsc::Sender<BrokerControlMessage>, // Control message sender for all MQTT commands
    mpsc::Sender<Result<MqttStreamMessage, Status>>, // Response stream sender
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
    // Streaming implementation for bidirectional communication
    type StreamMqttMessagesStream =
        tokio_stream::wrappers::ReceiverStream<Result<MqttStreamMessage, Status>>;

    async fn stream_mqtt_messages(
        &self,
        request: Request<tonic::Streaming<MqttStreamMessage>>,
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

                        if let Some(payload) = stream_msg.payload {
                            match payload {
                                mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(
                                    packet,
                                ) => {
                                    if let Some(p) = packet.packet {
                                        if let Some(client_id) = find_client_by_session(
                                            &connections,
                                            &stream_msg.session_id,
                                        ) {
                                            if let Some(conn) = connections.get(&client_id) {
                                                let (broker_tx, _, _) = conn.value();
                                                if let Err(e) = broker_tx
                                                    .send(BrokerControlMessage::Packet(p.into()))
                                                    .await
                                                {
                                                    error!(
                                                        "Failed to forward packet to broker for client {}: {}",
                                                        client_id, e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(
                                    packet,
                                ) => {
                                    if let Some(p) = packet.packet {
                                        if let Some(client_id) = find_client_by_session(
                                            &connections,
                                            &stream_msg.session_id,
                                        ) {
                                            if let Some(conn) = connections.get(&client_id) {
                                                let (broker_tx, _, _) = conn.value();
                                                if let Err(e) = broker_tx
                                                    .send(BrokerControlMessage::Packet(p.into()))
                                                    .await
                                                {
                                                    error!(
                                                        "Failed to forward packet to broker for client {}: {}",
                                                        client_id, e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                mqtt_unified_pb::mqtt_stream_message::Payload::SessionControl(
                                    control,
                                ) => {
                                    // Handle session control messages
                                    match control.control_type {
                                        x if x == mqtt_unified_pb::session_control::ControlType::Establish as i32 => {
                                            let client_id = control.client_id.clone();
                                            info!("Received Establish from client: {}", client_id);

                                            // Create broker connection for this client
                                            let connections = connections.clone();
                                            let response_tx_for_spawn = response_tx.clone();
                                            let session_id = stream_msg.session_id.clone();
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

                                            tokio::spawn(async move {
                                                match mqtt_connect_to_broker(broker_addr).await {
                                                    Ok(stream) => {
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
                                                            5, // @TODO: get from connect packet
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
                                        x if x == mqtt_unified_pb::session_control::ControlType::Terminate as i32 => {
                                            if let Some(client_id) =
                                                find_client_by_session(&connections, &stream_msg.session_id)
                                            {
                                                debug!("Client {} disconnecting", client_id);
                                                connections.remove(&client_id);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
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

async fn mqtt_connect_to_broker(broker_addr: SocketAddr) -> Result<tokio::net::TcpStream, Status> {
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    let stream = socket.connect(broker_addr).await?;
    info!("Connected to MQTT broker at {}", broker_addr);
    Ok(stream)
}

async fn mqtt_client_loop_simple(
    token: CancellationToken,
    connections: Arc<DashMap<String, ConnectionInfo>>,
    mut rx: mpsc::Receiver<BrokerControlMessage>,
    mut stream: tokio::net::TcpStream,
    response_tx: mpsc::Sender<Result<MqttStreamMessage, Status>>,
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
                    BrokerControlMessage::Packet(packet) => {

                        packet.to_bytes().unwrap()
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
                            if let Some(stream_msg) = convert_mqtt_to_stream_message(
                                &packet,
                                session_id.clone(),
                                sequence_counter,
                                mqtt_unified_pb::MessageDirection::BrokerToClient,
                            ) {
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
