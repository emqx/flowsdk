use std::env;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::mqttv5::connect::MqttConnect;
use flowsdk::mqtt_serde::parser::stream::MqttParser;
use flowsdk::mqtt_serde::parser::ParseError;
// Import shared conversions and protobuf types
use mqtt_grpc_proxy::mqttv5pb;
use mqtt_grpc_proxy::mqttv5pb::mqtt_relay_service_client::MqttRelayServiceClient;
use mqtt_grpc_proxy::{convert_mqtt_v5_to_stream_payload, convert_stream_payload_to_mqtt_bytes};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{Request, Status, Streaming};

use dashmap::DashMap;

// Enhanced connection structure for streaming
pub struct StreamingClientConnection {
    pub session_id: String,
    pub client_id: String,
    pub write_h: OwnedWriteHalf,
    pub read_h: OwnedReadHalf,
    pub grpc_stream_sender: mpsc::Sender<mqttv5pb::MqttStreamMessage>,
}

// State management for r-proxy
pub struct RProxyState {
    pub connections: Arc<DashMap<String, StreamingClientConnection>>,
    pub sequence_counter: AtomicU64,
}

async fn run_proxy(
    listen_port: u16,
    destination: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(("0.0.0.0", listen_port)).await?;
    info!("Listening on 0.0.0.0:{}", listen_port);

    let proxy_state = Arc::new(RProxyState {
        connections: Arc::new(DashMap::new()),
        sequence_counter: AtomicU64::new(1),
    });

    loop {
        info!("Accepting connection...");
        let (incoming_stream, addr) = listener.accept().await?;
        let state = proxy_state.clone();
        info!("Accepted connection from {}", addr);
        spawn(async move {
            match handle_new_incoming_tcp(incoming_stream, destination, state).await {
                Ok(_) => {
                    info!("Client connection handled successfully");
                }
                Err(e) => {
                    error!("Error handling client connection: {}", e);
                }
            }
        });
    }
}

async fn handle_new_incoming_tcp(
    incoming: TcpStream,
    destination: SocketAddr,
    state: Arc<RProxyState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Split stream to read and write halves
    let (mut read_half, write_half) = incoming.into_split();
    let mut parser: MqttParser = MqttParser::new(16384, 0);

    // Wait for MQTT Connect packet
    // @TODO: with some timeout
    let connect = wait_for_mqtt_connect(&mut read_half, &mut parser).await?;

    let client_id: String = connect.client_id.clone();
    let session_id: String = format!(
        "{}-{}",
        client_id,
        state.sequence_counter.fetch_add(1, Ordering::SeqCst)
    );

    info!(
        "New MQTT client connecting: {} (session: {})",
        client_id, session_id
    );

    // Connect to s-proxy with streaming
    let channel = tonic::transport::Channel::from_shared(format!("http://{}", destination))?
        .keep_alive_while_idle(true)
        .keep_alive_timeout(std::time::Duration::from_secs(30))
        .connect()
        .await?;

    let mut grpc_client: MqttRelayServiceClient<tonic::transport::Channel> =
        MqttRelayServiceClient::new(channel);

    // Create bidirectional streaming channels
    let (grpc_stream_sender, grpc_stream_receiver) = mpsc::channel(1000);

    // Convert MQTT Connect to protobuf Connect and send directly
    let connect_msg: mqttv5pb::MqttStreamMessage = mqttv5pb::MqttStreamMessage {
        session_id: session_id.clone(),
        sequence_id: 0,
        direction: mqttv5pb::MessageDirection::ClientToBroker as i32,
        payload: Some(mqttv5pb::mqtt_stream_message::Payload::Connect(
            connect.into(), // Direct conversion from MQTT Connect to protobuf Connect
        )),
    };

    grpc_stream_sender.send(connect_msg).await?;

    // Establish gRPC streaming connection
    let outbound_gstream = ReceiverStream::new(grpc_stream_receiver);
    let request = Request::new(outbound_gstream);
    let inbound_gstream = grpc_client
        .stream_mqtt_messages(request)
        .await?
        .into_inner();

    // Store connection for management
    let streaming_conn = StreamingClientConnection {
        session_id: session_id.clone(),
        client_id: client_id.clone(),
        write_h: write_half,
        read_h: read_half,
        grpc_stream_sender,
    };

    // Start the message handling loops (ConnAck will be handled in the loop)
    start_streaming_client_loop(streaming_conn, inbound_gstream, parser, state).await?;

    Ok(())
}

async fn start_streaming_client_loop(
    mut conn: StreamingClientConnection,
    mut inbound_stream: Streaming<mqttv5pb::MqttStreamMessage>,
    mut parser: MqttParser,
    _state: Arc<RProxyState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let sequence_counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(1));

    'outer: loop {
        tokio::select! {
            // Handle MQTT messages, client facing.
            _ = conn.read_h.read_buf(parser.buffer_mut()) => {
                // Parse all available packets from the buffer
                loop {
                    match parser.next_packet() {
                        Ok(Some(packet)) => {
                            debug!("Parsed MQTT packet from client {}: {:?}", conn.client_id, packet);
                            // Convert MQTT packet to gRPC stream message
                            if let Some(payload) = convert_mqtt_v5_to_stream_payload(&packet) {
                                let stream_msg = mqttv5pb::MqttStreamMessage {
                                    session_id: conn.session_id.clone(),
                                    sequence_id: sequence_counter.fetch_add(1, Ordering::SeqCst),
                                    direction: mqttv5pb::MessageDirection::ClientToBroker as i32,
                                    payload: Some(payload),
                                };

                                if let Err(e) = conn.grpc_stream_sender.send(stream_msg).await {
                                    error!("Failed to send MQTT packet to gRPC stream for client {}: {}", conn.client_id, e);
                                    break;
                                } else {
                                    debug!("MQTT packet forwarded to s-proxy for client {}: {:?}", conn.client_id, packet);
                                }
                            }
                            // Continue parsing more packets from the same buffer
                            if parser.buffer_mut().is_empty() {
                                break; // No more data to parse, exit inner loop
                            }
                        }
                        Ok(None) => {
                            // No more complete packets available, exit parsing loop
                            break;
                        }
                        Err(ParseError::BufferEmpty) => {
                            // No data to read, exit parsing loop
                            debug!("No data to read for client {}, ending...", conn.client_id);
                            break 'outer;
                        }
                        Err(e) => {
                            error!("Error parsing MQTT packet from client {}: {}", conn.client_id, e);
                            break 'outer;
                        }
                    }
                }
            }

            // Handle gRPC messages, s-proxy facing
            msg = inbound_stream.next() => {
                match msg {
                    Some(Ok(stream_msg)) => {
                        if let Some(payload) = &stream_msg.payload {
                            // Convert all payloads (including ConnAck) to MQTT bytes and forward
                            if let Some(mqtt_bytes) = convert_stream_payload_to_mqtt_bytes(payload) {
                                if let Err(e) = conn.write_h.write_all(&mqtt_bytes).await {
                                    error!("Failed to write MQTT response to client {}: {}, grpc payload: {:?}", conn.client_id, e, payload);
                                    break;
                                }
                                debug!("Forwarded gRPC message to MQTT client {}: {:?}", conn.client_id, payload);
                            } else {
                                error!("Failed to convert payload to MQTT bytes for client {}: {:?}", conn.client_id, payload);
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!("gRPC stream error for client {}: {}", conn.client_id, e);
                        break;
                    }
                    None => {
                        info!("gRPC stream closed for client {}", conn.client_id);
                        break;
                    }
                }
            }
        }
    }

    info!(
        "Streaming client loop ended for session: {} (client: {})",
        conn.session_id, conn.client_id
    );
    Ok(())
}

async fn wait_for_mqtt_connect(
    stream: &mut OwnedReadHalf,
    parser: &mut MqttParser,
) -> Result<MqttConnect, Status> {
    // init read hint for locate the vsn
    let mut read_hint = 7;

    loop {
        let n = stream
            .read_buf(parser.buffer_mut())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        if n == 0 {
            return Err(Status::unavailable("Connection closed by the client"));
        }

        if n < read_hint {
            read_hint -= n;
            continue;
        } else {
            read_hint = 0;
        }

        match parser.set_mqtt_vsn(0) {
            Ok(_) => {}
            Err(ParseError::More(hint, _)) => {
                read_hint = hint;
                continue;
            }
            // @TODO: handle other errors properly, like empty buffer
            Err(e) => {
                error!("Failed to set MQTT version: {}", e);
                return Err(Status::internal("Failed to set MQTT version"));
            }
        }

        match parser.next_packet() {
            Ok(Some(MqttPacket::Connect5(connect))) => {
                return Ok(connect);
            }
            Ok(Some(_)) => {
                return Err(Status::invalid_argument(
                    "Expected CONNECT packet, got something else",
                ));
            }
            Ok(None) => {
                // Incomplete packet, continue reading
            }
            Err(ParseError::More(hint, _)) => {
                // Need more data, continue reading
                read_hint = hint;
            }
            Err(e) => {
                error!("Error parsing CONNECT packet: {:?}", e);
                return Err(Status::invalid_argument("Failed to parse CONNECT packet"));
            }
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
