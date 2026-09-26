// SPDX-License-Identifier: MPL-2.0
//! Run again with the same directory to resume the MQTT session.
//! cargo run --features durable-session --example durable_session -- localhost:1883 /tmp/flowsdk-session
//! This local TCP example publishes only; incoming application work would need
//! a durable inbox committed with the session checkpoint before delivery/ACK.

#[path = "support/session_store.rs"]
mod session_store;

use flowsdk::mqtt_client::{
    ClientSessionStore, MqttClientOptions, MqttEvent, NoIoMqttClient, PublishCommand,
};
use session_store::FileSessionStore;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let peer = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "localhost:1883".into());
    let directory = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/flowsdk-session".into());
    let mut store = FileSessionStore::new(directory)?;
    let key = "durable-example";
    let mut client = NoIoMqttClient::new(
        MqttClientOptions::builder()
            .peer(&peer)
            .client_id("flowsdk-durable-example")
            .clean_start(false)
            .session_expiry_interval(3600)
            .reconnect(false)
            .build(),
    );
    match store.resume(key)? {
        Some(state) => client.restore_session_state(state)?,
        None => store.create(key, &client.snapshot_session()?)?,
    }
    let mut socket = TcpStream::connect(peer)?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    client.connect()?;
    let end = Instant::now() + Duration::from_secs(5);
    let mut buffer = [0; 8192];
    while Instant::now() < end {
        // Draining can advance queued work, so checkpoint after the drain and
        // before making the bytes visible to the broker.
        let output = client.take_outgoing();
        store.update(key, &client.snapshot_session()?)?;
        socket.write_all(&output)?;
        let events = match socket.read(&mut buffer) {
            Ok(0) => {
                client.handle_connection_lost();
                store.update(key, &client.snapshot_session()?)?;
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            Ok(n) => client.handle_incoming(&buffer[..n]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                client.handle_tick(Instant::now())
            }
            Err(error) => return Err(error.into()),
        };
        store.update(key, &client.snapshot_session()?)?;
        for event in events.into_iter().chain(client.take_events()) {
            match event {
                MqttEvent::Connected(result) => {
                    println!("Connected; session present: {}", result.session_present);
                    client.publish(PublishCommand::simple(
                        "durable/example",
                        b"hello".to_vec(),
                        1,
                        false,
                    ))?;
                    store.update(key, &client.snapshot_session()?)?;
                }
                MqttEvent::Error(error) => return Err(error.into()),
                event => println!("{event:?}"),
            }
        }
    }
    client.disconnect()?;
    let output = client.take_outgoing();
    store.update(key, &client.snapshot_session()?)?;
    socket.write_all(&output)?;
    Ok(())
}
