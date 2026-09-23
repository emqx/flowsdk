// SPDX-License-Identifier: MPL-2.0
//! Self-contained MQTT 5 publish/subscribe with QoS 1 and QoS 2 over local TCP.
//! Run: cargo run --example no_io_pubsub -- localhost:1883 [1|2]
//! With no QoS argument, runs both. The application owns I/O; the engine owns MQTT.
use flowsdk::mqtt_client::{MqttClientOptions, MqttEvent, NoIoMqttClient};
use flowsdk::mqtt_client::{PublishCommand, SubscribeCommand};
use std::{
    error::Error,
    io::{self, Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let peer = args.next().unwrap_or_else(|| "localhost:1883".into());
    let levels = match args.next().as_deref() {
        None => vec![1, 2],
        Some("1") => vec![1],
        Some("2") => vec![2],
        _ => return Err("QoS must be 1 or 2".into()),
    };
    for qos in levels {
        demo(&peer, qos)?;
    }
    Ok(())
}

fn demo(peer: &str, qos: u8) -> Result<(), Box<dyn Error>> {
    let id = format!("flowsdk-no-io-{}-{qos}", std::process::id());
    let topic = format!("flowsdk/example/{id}");
    let payload = format!("hello at QoS {qos}").into_bytes();
    let options = MqttClientOptions::builder()
        .peer(peer)
        .client_id(id)
        .mqtt_version(5)
        .clean_start(true)
        .reconnect(false)
        .auto_ack(true) // Engine generates incoming PUBACK / PUBREC / PUBCOMP.
        .build();
    let mut client = NoIoMqttClient::new(options);
    let mut socket = TcpStream::connect(peer)?;
    // Blocking writes handle partial writes; a short read timeout lets us tick.
    socket.set_read_timeout(Some(Duration::from_millis(50)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    client.connect()?;

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut input = [0; 4096];
    let mut events = Vec::new();
    let mut outgoing_id = None;
    let mut incoming_id = None;
    let mut published = false;
    let mut received_pubrel = qos == 1;
    loop {
        if Instant::now() >= deadline {
            return Err(format!("QoS {qos} demo timed out").into());
        }
        events.extend(client.handle_tick(Instant::now()));
        for event in std::mem::take(&mut events) {
            match event {
                MqttEvent::Connected(result) => {
                    if !result.is_success() {
                        return Err(format!("CONNECT rejected: {result:?}").into());
                    }
                    client.subscribe(SubscribeCommand::single(&topic, qos))?;
                }
                MqttEvent::Subscribed(result) => {
                    // Wait for SUBACK, and require the requested QoS for this demo.
                    if result.reason_codes != [qos] {
                        return Err(format!("Requested QoS {qos}, SUBACK: {result:?}").into());
                    }
                    println!("QoS {qos}: subscription granted");
                    outgoing_id = client.publish(PublishCommand::simple(
                        &topic,
                        payload.clone(),
                        qos,
                        false,
                    ))?;
                }
                MqttEvent::Published(result) if result.packet_id == outgoing_id => {
                    if !result.is_success() {
                        return Err(format!("Publish rejected: {result:?}").into());
                    }
                    // Published means PUBACK (QoS 1) or PUBCOMP (QoS 2), not enqueue.
                    published = true;
                    println!("QoS {qos}: publish acknowledged");
                }
                MqttEvent::MessageReceived(message)
                    if message.topic_name == topic && message.payload == payload =>
                {
                    if message.qos != qos {
                        return Err(format!("Delivery QoS was {}", message.qos).into());
                    }
                    incoming_id = message.packet_id;
                    println!(
                        "QoS {qos}: received {}",
                        String::from_utf8_lossy(&message.payload)
                    );
                }
                MqttEvent::PubRelReceived { packet_id, .. } if Some(packet_id) == incoming_id => {
                    // Incoming QoS 2 is not finished at MessageReceived alone.
                    received_pubrel = true;
                }
                MqttEvent::Error(error) | MqttEvent::OperationFailed { error, .. } => {
                    return Err(error.into());
                }
                event @ (MqttEvent::Disconnected(_) | MqttEvent::DisconnectReceived { .. }) => {
                    return Err(format!("Connection ended: {event:?}").into());
                }
                _ => {}
            }
        }

        // Flush commands and automatic acknowledgments, including the final PUBCOMP.
        // On a write error, exit and drop this connection; do not resend a partial buffer.
        socket.write_all(&client.take_outgoing())?;
        events.extend(client.take_events());
        if !events.is_empty() {
            continue;
        }
        if published && incoming_id.is_some() && received_pubrel {
            client.disconnect()?;
            socket.write_all(&client.take_outgoing())?;
            println!("QoS {qos}: publish/subscribe complete");
            return Ok(());
        }
        match socket.read(&mut input) {
            Ok(0) => return Err("Broker closed the TCP connection".into()),
            Ok(n) => events.extend(client.handle_incoming(&input[..n])),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
}
