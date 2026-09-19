// SPDX-License-Identifier: MPL-2.0
use flowsdk::mqtt_client::{
    MqttClientError, MqttEvent, NoIoMqttClient, PublishCommand, SubscribeCommand,
};
use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

pub trait Protocol {
    fn connect(&mut self) -> Result<(), MqttClientError>;
    fn reset(&mut self) -> Result<(), MqttClientError>;
    fn lost(&mut self);
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<MqttEvent>, MqttClientError>;
    fn tick(&mut self, now: Instant) -> Vec<MqttEvent>;
    fn output(&mut self) -> Vec<u8>;
    fn events(&mut self) -> Vec<MqttEvent>;
    fn finished(&self) -> bool;
    fn subscribe(&mut self) -> Result<(), MqttClientError>;
    fn publish(&mut self) -> Result<(), MqttClientError>;
    fn disconnect(&mut self) -> Result<(), MqttClientError>;
}

impl Protocol for NoIoMqttClient {
    fn connect(&mut self) -> Result<(), MqttClientError> {
        self.connect()
    }
    fn reset(&mut self) -> Result<(), MqttClientError> {
        self.reset_for_new_transport();
        Ok(())
    }
    fn lost(&mut self) {
        self.handle_connection_lost();
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<MqttEvent>, MqttClientError> {
        Ok(self.handle_incoming(bytes))
    }
    fn tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        self.handle_tick(now)
    }
    fn output(&mut self) -> Vec<u8> {
        self.take_outgoing()
    }
    fn events(&mut self) -> Vec<MqttEvent> {
        self.take_events()
    }
    fn finished(&self) -> bool {
        !self.has_pending_output()
    }
    fn subscribe(&mut self) -> Result<(), MqttClientError> {
        self.subscribe(SubscribeCommand::single("flowsdk/example/commands", 1))
            .map(|_| ())
    }
    fn publish(&mut self) -> Result<(), MqttClientError> {
        self.publish(PublishCommand::simple(
            "flowsdk/example/data",
            b"hello".to_vec(),
            1,
            false,
        ))
        .map(|_| ())
    }
    fn disconnect(&mut self) -> Result<(), MqttClientError> {
        self.disconnect()
    }
}

#[cfg(feature = "rustls-tls")]
impl Protocol for flowsdk::mqtt_client::TlsMqttEngine {
    fn connect(&mut self) -> Result<(), MqttClientError> {
        self.connect()
    }
    fn reset(&mut self) -> Result<(), MqttClientError> {
        self.reset_for_new_transport()
    }
    fn lost(&mut self) {
        self.handle_connection_lost();
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<MqttEvent>, MqttClientError> {
        self.handle_socket_data(bytes)?;
        Ok(self.handle_tick(Instant::now()))
    }
    fn tick(&mut self, now: Instant) -> Vec<MqttEvent> {
        self.handle_tick(now)
    }
    fn output(&mut self) -> Vec<u8> {
        self.take_socket_data()
    }
    fn events(&mut self) -> Vec<MqttEvent> {
        self.take_events()
    }
    fn finished(&self) -> bool {
        self.disconnect_complete()
    }
    fn subscribe(&mut self) -> Result<(), MqttClientError> {
        self.subscribe(SubscribeCommand::single("flowsdk/example/commands", 1))
            .map(|_| ())
    }
    fn publish(&mut self) -> Result<(), MqttClientError> {
        self.publish(PublishCommand::simple(
            "flowsdk/example/data",
            b"hello".to_vec(),
            1,
            false,
        ))
        .map(|_| ())
    }
    fn disconnect(&mut self) -> Result<(), MqttClientError> {
        self.disconnect()
    }
}

fn open(peer: &str) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "No broker address");
    for address in peer.to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
            Ok(stream) => {
                stream.set_nonblocking(true)?;
                return Ok(stream);
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// Small polling example. A production reactor can wake on socket readiness and
/// the engine's next_tick_at() deadline. The unsent tail belongs to the transport.
pub fn run(peer: &str, mut protocol: impl Protocol) -> Result<(), Box<dyn std::error::Error>> {
    let end = Instant::now() + Duration::from_secs(30);
    let mut socket = None;
    let mut reconnect = true;
    let mut pending = Vec::new();
    let mut written = 0;
    let mut input = [0; 4096];
    let mut deferred_events = Vec::new();
    let mut stopping = false;
    let mut stop_at = None;
    loop {
        if Instant::now() >= end && !stopping {
            stopping = true;
            stop_at = Some(Instant::now() + Duration::from_secs(5));
            protocol.disconnect()?;
        }
        if reconnect && !stopping {
            reconnect = false;
            protocol.reset()?;
            match open(peer) {
                Ok(stream) => {
                    socket = Some(stream);
                    protocol.connect()?;
                }
                Err(error) => {
                    eprintln!("Connect failed: {error}");
                    protocol.lost();
                }
            }
        }
        let mut events = std::mem::take(&mut deferred_events);
        events.extend(protocol.tick(Instant::now()));
        let mut lost = false;
        if let Some(stream) = socket.as_mut() {
            match stream.read(&mut input) {
                Ok(0) => lost = true,
                Ok(n) => match protocol.feed(&input[..n]) {
                    Ok(received) => events.extend(received),
                    Err(error) => {
                        eprintln!("Protocol error: {error}");
                        lost = true;
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    eprintln!("Read failed: {error}");
                    lost = true;
                }
            }
        }
        events.extend(protocol.events());
        for event in events {
            match event {
                MqttEvent::Connected(result) if result.is_success() && !stopping => {
                    protocol.subscribe()?;
                    protocol.publish()?;
                }
                MqttEvent::Connected(_)
                | MqttEvent::Disconnected(_)
                | MqttEvent::DisconnectReceived { .. } => lost = true,
                MqttEvent::ReconnectNeeded if !stopping => reconnect = true,
                MqttEvent::MessageReceived(message) => println!(
                    "{}: {}",
                    message.topic_name,
                    String::from_utf8_lossy(&message.payload)
                ),
                MqttEvent::Error(error) => {
                    eprintln!("MQTT error: {error}");
                    lost = true;
                }
                MqttEvent::OperationFailed {
                    operation, error, ..
                } => {
                    eprintln!("{operation:?}: {error}");
                    if operation == flowsdk::mqtt_client::OperationKind::Connect {
                        lost = true;
                    }
                }
                _ => {}
            }
        }
        if !lost {
            if pending.len() == written {
                // Drive TLS plaintext/output after commands in the event loop.
                deferred_events.extend(protocol.tick(Instant::now()));
                pending = protocol.output();
                written = 0;
            }
            if let Some(stream) = socket.as_mut() {
                if written < pending.len() {
                    match stream.write(&pending[written..]) {
                        Ok(0) => lost = true,
                        Ok(n) => written += n,
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) => {}
                        Err(error) => {
                            eprintln!("Write failed: {error}");
                            lost = true;
                        }
                    }
                }
            }
        }
        if lost {
            socket = None;
            pending.clear();
            written = 0;
            if !stopping {
                protocol.lost();
            }
        }
        if stopping
            && (socket.is_none()
                || (pending.len() == written && protocol.finished())
                || stop_at.is_some_and(|at| Instant::now() >= at))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
