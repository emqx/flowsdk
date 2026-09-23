// SPDX-License-Identifier: MPL-2.0
//! MQTT 5 publish/subscribe over TCP, with acknowledged QoS 1 and QoS 2 exchanges.
//! Run: cargo run --example async_pubsub -- localhost:1883 [1|2]
//! With no QoS argument, runs both. Start a local MQTT broker first.
use flowsdk::mqtt_client::{
    MqttClientError, MqttClientOptions, MqttMessage, TokioAsyncClientConfig, TokioAsyncMqttClient,
    TokioMqttEventHandler,
};
use std::{error::Error, time::Duration};
use tokio::{sync::mpsc, time::timeout};

enum Event {
    Message(MqttMessage),
    Pubrel(u16),
    Error(MqttClientError),
    Disconnected,
}

struct Handler(mpsc::UnboundedSender<Event>);

#[async_trait::async_trait]
impl TokioMqttEventHandler for Handler {
    // Callbacks run on the worker: forward events and await client APIs elsewhere.
    async fn on_message_received(&mut self, message: &MqttMessage) {
        let _ = self.0.send(Event::Message(message.clone()));
    }
    async fn on_pubrel_received(&mut self, packet_id: u16) {
        let _ = self.0.send(Event::Pubrel(packet_id));
    }
    async fn on_error(&mut self, error: &MqttClientError) {
        let _ = self.0.send(Event::Error(error.clone()));
    }
    async fn on_disconnected(&mut self, _: Option<u8>) {
        let _ = self.0.send(Event::Disconnected);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let peer = args.next().unwrap_or_else(|| "localhost:1883".into());
    let levels = match args.next().as_deref() {
        None => vec![1, 2],
        Some("1") => vec![1],
        Some("2") => vec![2],
        _ => return Err("QoS must be 1 or 2".into()),
    };
    if args.next().is_some() {
        return Err("Usage: async_pubsub [host:port] [1|2]".into());
    }
    for qos in levels {
        demo(&peer, qos).await?;
    }
    Ok(())
}

async fn demo(peer: &str, qos: u8) -> Result<(), Box<dyn Error>> {
    let id = format!("flowsdk-async-{}-{qos}", std::process::id());
    let topic = format!("flowsdk/example/{id}");
    let payload = format!("hello at QoS {qos}").into_bytes();
    let options = MqttClientOptions::builder()
        .peer(peer)
        .client_id(id)
        .mqtt_version(5)
        .clean_start(true)
        .auto_ack(true) // Worker sends incoming PUBACK / PUBREC / PUBCOMP.
        .build();
    let config = TokioAsyncClientConfig::builder()
        .auto_reconnect(false)
        .buffer_messages(false)
        .local_network_timeouts()
        .build();
    let (tx, mut events) = mpsc::unbounded_channel();
    let client = TokioAsyncMqttClient::new(options, Box::new(Handler(tx)), config).await?;

    let result: Result<(), Box<dyn Error>> = async {
        let connected = client.connect_sync().await?;
        if !connected.is_success() {
            return Err(format!("CONNECT rejected: {connected:?}").into());
        }
        let subscribed = client.subscribe_sync(&topic, qos).await?;
        if subscribed.reason_codes != [qos] {
            return Err(format!("Requested QoS {qos}, SUBACK: {subscribed:?}").into());
        }
        println!("QoS {qos}: subscription granted");

        let published = client.publish_sync(&topic, &payload, qos, false).await?;
        if !published.is_success() {
            return Err(format!("Publish rejected: {published:?}").into());
        }
        println!("QoS {qos}: publish acknowledged");

        // Publishing and receiving are independent. Wait for the full incoming flow.
        timeout(Duration::from_secs(5), async {
            let mut incoming_id = None;
            while let Some(event) = events.recv().await {
                match event {
                    Event::Message(message)
                        if message.topic_name == topic && message.payload == payload =>
                    {
                        if message.qos != qos {
                            return Err(format!("Delivery QoS was {}", message.qos).into());
                        }
                        println!("QoS {qos}: received {}", String::from_utf8_lossy(&payload));
                        if qos == 1 {
                            return Ok(());
                        }
                        incoming_id = message.packet_id;
                    }
                    Event::Pubrel(id) if incoming_id == Some(id) => return Ok(()),
                    Event::Error(error) => return Err(error.into()),
                    Event::Disconnected => return Err("Connection ended before delivery".into()),
                    _ => {}
                }
            }
            Err::<(), Box<dyn Error>>("Event channel closed before delivery".into())
        })
        .await??;
        // The worker flushes automatic ACKs before handling this disconnect command.
        client.disconnect_sync().await?;
        println!("QoS {qos}: publish/subscribe complete");
        Ok(())
    }
    .await;
    // Join the worker on success and on error; preserve the original operation error.
    let shutdown = client.shutdown().await;
    result?;
    shutdown?;
    Ok(())
}
