use std::env;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use flowsdk::mqtt_serde::control_packet::MqttPacket;

use flowsdk::mqtt_serde::parser::stream::MqttParser;
use flowsdk::mqtt_serde::parser::ParseError;
use mqtt_grpc_proxy::{
    convert_mqtt_to_stream_message, convert_stream_message_to_mqtt_packet, mqtt_unified_pb,
    MqttConnectPacket,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Status, Streaming};
use tracing::{debug, error, info};

use dashmap::DashMap;

use mqtt_grpc_proxy::mqtt_unified_pb::mqtt_relay_service_client::MqttRelayServiceClient;
use mqtt_grpc_proxy::mqtt_unified_pb::MqttStreamMessage;

// Enhanced connection structure for streaming
pub struct StreamingClientConnection {
    pub session_id: String,
    pub client_id: String,
    pub mqtt_version: u8,
    pub write_h: OwnedWriteHalf,
    pub read_h: OwnedReadHalf,
    pub grpc_stream_sender: mpsc::Sender<MqttStreamMessage>, // Can handle both V3 and V5
}

// State management for r-proxy
pub struct RProxyState {
    pub connections: Arc<DashMap<String, StreamingClientConnection>>,
    // Atomic counter for unique session IDs
    // @TODO: persist across restarts, maybe use timestamp
    pub sequence_counter: AtomicU64,
    // Shared gRPC client for all MQTT connections (Phase 1: Channel Reuse)
    pub grpc_client: Arc<MqttRelayServiceClient<tonic::transport::Channel>>,
}

async fn run_proxy(
    listen_port: u16,
    destination: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(("0.0.0.0", listen_port)).await?;
    info!("Listening on 0.0.0.0:{}", listen_port);

    // Create shared gRPC channel (Phase 1: Channel Reuse)
    // This channel will be reused by all MQTT client connections
    info!(
        "Establishing shared gRPC connection to s-proxy at {}",
        destination
    );
    let channel = tonic::transport::Channel::from_shared(format!("http://{}", destination))?
        .keep_alive_while_idle(true)
        .keep_alive_timeout(std::time::Duration::from_secs(30))
        .connect_lazy(); // Use connect_lazy for efficient connection pooling

    let grpc_client = Arc::new(MqttRelayServiceClient::new(channel));
    info!("Shared gRPC channel established and ready for client connections");

    let proxy_state = Arc::new(RProxyState {
        connections: Arc::new(DashMap::new()),
        sequence_counter: AtomicU64::new(1),
        grpc_client, // Share this client across all MQTT connections
    });

    loop {
        info!("Accepting connection...");
        let (incoming_stream, addr) = listener.accept().await?;
        let state = proxy_state.clone();
        info!("Accepted connection from {}", addr);
        spawn(async move {
            match handle_new_incoming_tcp(incoming_stream, state).await {
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

/// Setup a new gRPC bidirectional stream using the shared channel
///
/// Phase 1: This function reuses the shared gRPC channel instead of creating
/// a new connection for each MQTT client. Each client gets its own isolated
/// bidirectional stream over the shared channel.
async fn setup_grpc_streaming(
    grpc_client: Arc<MqttRelayServiceClient<tonic::transport::Channel>>,
    session_id: String,
    client_id: String,
) -> Result<
    (
        mpsc::Sender<MqttStreamMessage>,
        Streaming<MqttStreamMessage>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    // Clone the shared client (cheap Arc clone, shares underlying channel)
    let mut client = (*grpc_client).clone();

    // Create bidirectional streaming channels for THIS MQTT client
    let (grpc_stream_sender, grpc_stream_receiver) = mpsc::channel(1000);

    // Establish NEW streaming RPC (but reuses underlying channel)
    let outbound_gstream = ReceiverStream::new(grpc_stream_receiver);
    let mut request = Request::new(outbound_gstream);

    // Add session_id and client_id as gRPC metadata (HTTP/2 headers)
    let metadata = request.metadata_mut();
    metadata.insert("session-id", session_id.parse().unwrap());
    metadata.insert("client-id", client_id.parse().unwrap());

    let inbound_gstream = client.stream_mqtt_messages(request).await?.into_inner();

    Ok((grpc_stream_sender, inbound_gstream))
}

async fn handle_new_incoming_tcp(
    incoming: TcpStream,
    state: Arc<RProxyState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Split stream to read and write halves
    let (mut in_read_half, out_write_half) = incoming.into_split();

    // Wait for MQTT Connect packet
    // @TODO: with some timeout
    let mut parser: MqttParser = MqttParser::new(16384, 0);
    let connect = wait_for_mqtt_connect(&mut in_read_half, &mut parser).await?;

    let client_id: String = connect.client_id().to_string();
    let mqtt_version = connect.version();
    let session_id: String = format!(
        "{}-{}",
        client_id,
        state.sequence_counter.fetch_add(1, Ordering::SeqCst)
    );

    info!(
        "New MQTT client connecting: {} (session: {})",
        client_id, session_id
    );

    // Setup gRPC streaming using shared channel (Phase 1: Channel Reuse)
    // Session metadata will be sent via gRPC metadata (HTTP/2 headers)
    debug!(
        "Setting up gRPC stream for client {} using shared channel",
        client_id
    );
    let (gstream_sender, inbound_gstream) = setup_grpc_streaming(
        state.grpc_client.clone(),
        session_id.clone(),
        client_id.clone(),
    )
    .await?;
    debug!(
        "gRPC stream established for client {} (reused shared channel) with session metadata",
        client_id
    );

    let control_type = match mqtt_version {
        5 => mqtt_unified_pb::session_control::ControlType::Establishv5,
        3 => mqtt_unified_pb::session_control::ControlType::Establishv3,
        _ => {
            error!("Unsupported MQTT version: {}", mqtt_version);
            return Err("Unsupported MQTT version".into());
        }
    };

    // Send initial session control message to establish the session
    let session_control: mqtt_unified_pb::SessionControl = mqtt_unified_pb::SessionControl {
        control_type: control_type as i32,
        client_id: client_id.clone(),
    };
    let session_control_msg = MqttStreamMessage {
        sequence_id: 0,
        direction: mqtt_unified_pb::MessageDirection::ClientToBroker as i32,
        payload: Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::SessionControl(session_control),
        ),
    };
    gstream_sender.send(session_control_msg).await?;

    // Send initial Connect message based on the version
    let connect_packet = match connect {
        MqttConnectPacket::V5(v5_connect) => MqttPacket::Connect5(v5_connect),
        MqttConnectPacket::V3(v3_connect) => MqttPacket::Connect3(v3_connect),
    };

    let msg = convert_mqtt_to_stream_message(
        &connect_packet,
        0,
        mqtt_unified_pb::MessageDirection::ClientToBroker,
    )
    .ok_or("Failed to convert connect packet to stream message")?;

    gstream_sender.send(msg).await?;

    // Store connection for management
    let streaming_conn = StreamingClientConnection {
        session_id: session_id.clone(),
        client_id: client_id.clone(),
        mqtt_version,
        write_h: out_write_half,
        read_h: in_read_half,
        grpc_stream_sender: gstream_sender, // Unified sender for both V3 and V5
    };

    // Start the message handling loops (ConnAck will be handled in the loop)
    start_streaming_loop(streaming_conn, inbound_gstream, parser, state).await?;

    Ok(())
}

async fn start_streaming_loop(
    mut conn: StreamingClientConnection,
    mut inbound_stream: Streaming<MqttStreamMessage>,
    mut parser: MqttParser,
    _state: Arc<RProxyState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let sequence_counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(1));

    'outer: loop {
        tokio::select! {
            // Handle MQTT messages, client facing.
            n = conn.read_h.read_buf(parser.buffer_mut()) => {
                match n {
                    Ok(0) => {
                        info!("Client {} disconnected (EOF)", conn.client_id);
                        break 'outer;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!("Error reading from client {}: {}", conn.client_id, e);
                        break 'outer;
                    }
                }
                // Parse all available packets from the buffer
                loop {
                    match parser.next_packet() {
                        Ok(Some(packet)) => {
                            debug!("Parsed MQTT packet from client {}: {:?}", conn.client_id, packet);
                            // Convert MQTT packet to gRPC stream message using unified approach
                            // Note: session_id is sent via gRPC metadata, not in the message
                            let stream_msg = convert_mqtt_to_stream_message(
                                &packet,
                                sequence_counter.fetch_add(1, Ordering::SeqCst),
                                mqtt_unified_pb::MessageDirection::ClientToBroker,
                            );


                            if let Some(msg) = stream_msg {
                                if let Err(e) = conn.grpc_stream_sender.send(msg).await {
                                    error!("Failed to send MQTT packet to gRPC stream for client {}: {}", conn.client_id, e);
                                    break;
                                } else {
                                    debug!("MQTT packet forwarded to s-proxy for client {} (session: {}): {:?}",
                                           conn.client_id, conn.session_id, packet);
                                }
                            } else {
                                error!("Failed to convert MQTT packet to stream message for client {}: {:?}", conn.client_id, packet);
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
                        if let Some(packet) = convert_stream_message_to_mqtt_packet(stream_msg) {
                            if let Ok(mqtt_bytes) = packet.to_bytes() {
                                if let Err(e) = conn.write_h.write_all(&mqtt_bytes).await {
                                    error!("Failed to write MQTT response to client {}: {}, grpc payload: {:?}", conn.client_id, e, packet);
                                    break;
                                }
                                debug!("Forwarded gRPC message to MQTT client {}: {:?}", conn.client_id, packet);
                            } else {
                                error!("Failed to convert payload to MQTT bytes for client {}: {:?}", conn.client_id, packet);
                            }
                        } else {
                            error!("Failed to convert stream message to MQTT packet");
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
) -> Result<MqttConnectPacket, Status> {
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
                return Ok(MqttConnectPacket::V5(connect));
            }
            Ok(Some(MqttPacket::Connect3(connect))) => {
                return Ok(MqttConnectPacket::V3(connect));
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
