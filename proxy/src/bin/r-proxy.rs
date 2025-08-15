use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mqtt_grpc_duality::mqtt_serde::control_packet::MqttControlPacket;
use mqtt_grpc_duality::mqtt_serde::control_packet::MqttPacket;
use mqtt_grpc_duality::mqtt_serde::mqttv5;
use mqtt_grpc_duality::mqtt_serde::mqttv5::connack::MqttConnAck;
use mqtt_grpc_duality::mqtt_serde::mqttv5::connect::MqttConnect;
use mqtt_grpc_duality::mqtt_serde::mqttv5::suback::MqttSubAck;
use mqtt_grpc_duality::mqtt_serde::parser::stream::MqttParser;
// Import shared conversions and protobuf types
use mqtt_grpc_proxy::mqttv5pb;
use mqtt_grpc_proxy::mqttv5pb::mqtt_relay_service_client::MqttRelayServiceClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::sync::mpsc;
use tracing::info;

use tonic::{Status, Request, Streaming};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

use dashmap::DashMap;

// Enhanced connection structure for streaming
pub struct StreamingClientConnection {
    pub session_id: String,
    pub client_id: String,
    pub write_h: OwnedWriteHalf,
    pub read_h: OwnedReadHalf,
    pub grpc_stream_sender: mpsc::Sender<mqttv5pb::MqttStreamMessage>,
    pub grpc_stream_receiver: mpsc::Receiver<Result<mqttv5pb::MqttStreamMessage, Status>>,
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
            match handle_new_streaming_conn(incoming_stream, destination, state).await {
                Ok(_) => {
                    info!("Client connection handled successfully");
                }
                Err(e) => {
                    eprintln!("Error handling client connection: {}", e);
                }
            }
        });
    }
}

async fn handle_new_streaming_conn(
    incoming: TcpStream,
    destination: SocketAddr,
    state: Arc<RProxyState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Split stream to read and write halves
    let (mut read_half, mut write_half) = incoming.into_split();
    let mut parser: MqttParser = MqttParser::new();

    // Wait for MQTT Connect packet
    let connect = wait_for_mqtt_connect(&mut read_half, &mut parser).await?;
    
    let client_id = connect.client_id.clone();
    let session_id = format!("{}-{}", client_id, state.sequence_counter.fetch_add(1, Ordering::SeqCst));
    
    info!("New MQTT client connecting: {} (session: {})", client_id, session_id);

    // Connect to s-proxy with streaming
    let mut grpc_client: MqttRelayServiceClient<tonic::transport::Channel> =
        MqttRelayServiceClient::connect(format!("http://{}", destination)).await?;

    // Create bidirectional streaming channels
    let (stream_sender, stream_receiver) = mpsc::channel(1000);
    let (_response_sender, response_receiver) = mpsc::channel(1000);
    
    // Send session establishment message
    let session_control = mqttv5pb::SessionControl {
        control_type: mqttv5pb::session_control::ControlType::Establish as i32,
        client_id: client_id.clone(),
    };
    
    let establish_msg = mqttv5pb::MqttStreamMessage {
        session_id: session_id.clone(),
        sequence_id: 0,
        direction: mqttv5pb::MessageDirection::ClientToBroker as i32,
        payload: Some(mqttv5pb::mqtt_stream_message::Payload::SessionControl(session_control)),
    };
    
    stream_sender.send(establish_msg).await?;

    // Establish gRPC streaming connection
    let outbound_stream = ReceiverStream::new(stream_receiver);
    let request = Request::new(outbound_stream);
    let mut inbound_stream = grpc_client.stream_mqtt_messages(request).await?.into_inner();

    // Wait for ConnAck from s-proxy
    if let Some(Ok(connack_msg)) = inbound_stream.next().await {
        if let Some(mqttv5pb::mqtt_stream_message::Payload::Connack(connack)) = connack_msg.payload {
            // Send ConnAck to MQTT client
            let mqtt_connack: MqttConnAck = connack.into();
            let connack_bytes = mqtt_connack.to_bytes()?;
            write_half.write_all(&connack_bytes).await?;
            
            info!("ConnAck sent to client {}", client_id);
        } else {
            return Err("Expected ConnAck from s-proxy".into());
        }
    } else {
        return Err("Failed to receive ConnAck from s-proxy".into());
    }

    // Store connection for management
    let streaming_conn = StreamingClientConnection {
        session_id: session_id.clone(),
        client_id: client_id.clone(),
        write_h: write_half,
        read_h: read_half,
        grpc_stream_sender: stream_sender,
        grpc_stream_receiver: response_receiver,
    };
    
    // Start the message handling loops
    start_streaming_client_loop(streaming_conn, inbound_stream, state).await?;
    
    Ok(())
}

async fn start_streaming_client_loop(
    mut conn: StreamingClientConnection,
    mut inbound_stream: Streaming<mqttv5pb::MqttStreamMessage>,
    _state: Arc<RProxyState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut parser = MqttParser::new();
    let sequence_counter = Arc::new(AtomicU64::new(1));
    
    loop {
        tokio::select! {
            // Handle messages from MQTT client
            _ = conn.read_h.read_buf(parser.buffer_mut()) => {
                match parser.next_packet() {
                    Ok(Some(packet)) => {
                        // Convert MQTT packet to gRPC stream message
                        if let Some(payload) = convert_mqtt_to_stream_payload(&packet) {
                            let stream_msg = mqttv5pb::MqttStreamMessage {
                                session_id: conn.session_id.clone(),
                                sequence_id: sequence_counter.fetch_add(1, Ordering::SeqCst),
                                direction: mqttv5pb::MessageDirection::ClientToBroker as i32,
                                payload: Some(payload),
                            };
                            
                            if let Err(e) = conn.grpc_stream_sender.send(stream_msg).await {
                                eprintln!("Error sending to gRPC stream: {}", e);
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        // Incomplete packet, continue
                    }
                    Err(e) => {
                        eprintln!("Error parsing MQTT packet: {}", e);
                        break;
                    }
                }
            }
            
            // Handle messages from s-proxy
            msg = inbound_stream.next() => {
                match msg {
                    Some(Ok(stream_msg)) => {
                        if let Some(payload) = stream_msg.payload {
                            if let Some(mqtt_bytes) = convert_stream_payload_to_mqtt_bytes(&payload) {
                                if let Err(e) = conn.write_h.write_all(&mqtt_bytes).await {
                                    eprintln!("Error writing to MQTT client: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("gRPC stream error: {}", e);
                        break;
                    }
                    None => {
                        info!("gRPC stream closed");
                        break;
                    }
                }
            }
        }
    }
    
    info!("Streaming client loop ended for session: {}", conn.session_id);
    Ok(())
}

// Helper function to convert MQTT packets to stream payloads
fn convert_mqtt_to_stream_payload(packet: &MqttPacket) -> Option<mqttv5pb::mqtt_stream_message::Payload> {
    match packet {
        MqttPacket::Connect(connect) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Connect(connect.clone().into()))
        }
        MqttPacket::Publish(publish) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Publish(publish.clone().into()))
        }
        MqttPacket::Subscribe(subscribe) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Subscribe(subscribe.clone().into()))
        }
        MqttPacket::Unsubscribe(unsubscribe) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Unsubscribe(unsubscribe.clone().into()))
        }
        MqttPacket::PubAck(puback) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Puback(puback.clone().into()))
        }
        MqttPacket::PubRec(pubrec) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Pubrec(pubrec.clone().into()))
        }
        MqttPacket::PubRel(pubrel) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Pubrel(pubrel.clone().into()))
        }
        MqttPacket::PubComp(pubcomp) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Pubcomp(pubcomp.clone().into()))
        }
        MqttPacket::PingReq(_) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Pingreq(mqttv5pb::Pingreq {}))
        }
        MqttPacket::Disconnect(disconnect) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Disconnect(disconnect.clone().into()))
        }
        MqttPacket::Auth(auth) => {
            Some(mqttv5pb::mqtt_stream_message::Payload::Auth(auth.clone().into()))
        }
        _ => None,
    }
}

// Helper function to convert stream payloads to MQTT bytes
fn convert_stream_payload_to_mqtt_bytes(payload: &mqttv5pb::mqtt_stream_message::Payload) -> Option<Vec<u8>> {
    match payload {
        mqttv5pb::mqtt_stream_message::Payload::Connack(connack) => {
            let mqtt_connack: MqttConnAck = connack.clone().into();
            mqtt_connack.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Publish(publish) => {
            let mqtt_publish: mqttv5::publish::MqttPublish = publish.clone().into();
            mqtt_publish.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Suback(suback) => {
            let mqtt_suback: MqttSubAck = suback.clone().into();
            mqtt_suback.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Unsuback(unsuback) => {
            let mqtt_unsuback: mqttv5::unsuback::MqttUnsubAck = unsuback.clone().into();
            mqtt_unsuback.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Puback(puback) => {
            let mqtt_puback: mqttv5::puback::MqttPubAck = puback.clone().into();
            mqtt_puback.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pubrec(pubrec) => {
            let mqtt_pubrec: mqttv5::pubrec::MqttPubRec = pubrec.clone().into();
            mqtt_pubrec.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pubrel(pubrel) => {
            let mqtt_pubrel: mqttv5::pubrel::MqttPubRel = pubrel.clone().into();
            mqtt_pubrel.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pubcomp(pubcomp) => {
            let mqtt_pubcomp: mqttv5::pubcomp::MqttPubComp = pubcomp.clone().into();
            mqtt_pubcomp.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pingresp(_) => {
            let mqtt_pingresp = mqttv5::pingresp::MqttPingResp::new();
            mqtt_pingresp.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Disconnect(disconnect) => {
            let mqtt_disconnect: mqttv5::disconnect::MqttDisconnect = disconnect.clone().into();
            mqtt_disconnect.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Auth(auth) => {
            let mqtt_auth: mqttv5::auth::MqttAuth = auth.clone().into();
            mqtt_auth.to_bytes().ok()
        }
        _ => None,
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
            Ok(Some(MqttPacket::Connect(connect))) => {
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
            Err(e) => {
                eprintln!("Error parsing CONNECT: {:?}", e);
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
