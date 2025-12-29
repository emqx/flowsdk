use super::commands::{PublishCommand, SubscribeCommand, UnsubscribeCommand};
use super::engine::{MqttEvent, QuicMqttEngine};
use super::opts::MqttClientOptions;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

type CommandError = Box<dyn std::error::Error + Send + Sync>;
type CommandResult<T> = Result<T, CommandError>;

/// High-level asynchronous MQTT over QUIC client using the Tokio runtime and QuicMqttEngine for portability.
/// Ideally you prefer to use tokio_async_client instead.
pub struct TokioQuicMqttClient {
    command_tx: mpsc::Sender<QuicCommand>,
    event_rx: mpsc::Receiver<MqttEvent>,
}

pub enum QuicCommand {
    Connect {
        server_addr: SocketAddr,
        server_name: String,
        crypto: Box<rustls::ClientConfig>,
        resp: oneshot::Sender<CommandResult<()>>,
    },
    Publish {
        cmd: PublishCommand,
        resp: oneshot::Sender<CommandResult<Option<u16>>>,
    },
    Subscribe {
        cmd: SubscribeCommand,
        resp: oneshot::Sender<CommandResult<u16>>,
    },
    Unsubscribe {
        cmd: UnsubscribeCommand,
        resp: oneshot::Sender<CommandResult<u16>>,
    },
    Disconnect {
        resp: oneshot::Sender<CommandResult<()>>,
    },
}

impl TokioQuicMqttClient {
    /// Create a new TokioQuicMqttClient.
    pub fn new(options: MqttClientOptions) -> Result<Self, Box<dyn std::error::Error>> {
        let (command_tx, command_rx) = mpsc::channel(100);
        let (event_tx, event_rx) = mpsc::channel(100);

        let engine = QuicMqttEngine::new(options)?;

        tokio::spawn(async move {
            if let Err(e) = run_engine_loop(engine, command_rx, event_tx).await {
                eprintln!("TokioQuicMqttClient background loop error: {}", e);
            }
        });

        Ok(Self {
            command_tx,
            event_rx,
        })
    }

    pub async fn connect(
        &self,
        server_addr: SocketAddr,
        server_name: String,
        crypto: rustls::ClientConfig,
    ) -> CommandResult<()> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(QuicCommand::Connect {
                server_addr,
                server_name,
                crypto: Box::new(crypto),
                resp: tx,
            })
            .await
            .map_err(|_| "Failed to send connect command")?;
        rx.await
            .map_err(|_| "Command response channel dropped".into())
            .and_then(|r| r)
    }

    pub async fn publish(&self, cmd: PublishCommand) -> CommandResult<Option<u16>> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(QuicCommand::Publish { cmd, resp: tx })
            .await
            .map_err(|_| "Failed to send publish command")?;
        rx.await
            .map_err(|_| "Command response channel dropped".into())
            .and_then(|r| r)
    }

    pub async fn subscribe(&self, cmd: SubscribeCommand) -> CommandResult<u16> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(QuicCommand::Subscribe { cmd, resp: tx })
            .await
            .map_err(|_| "Failed to send subscribe command")?;
        rx.await
            .map_err(|_| "Command response channel dropped".into())
            .and_then(|r| r)
    }

    pub async fn unsubscribe(&self, cmd: UnsubscribeCommand) -> CommandResult<u16> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(QuicCommand::Unsubscribe { cmd, resp: tx })
            .await
            .map_err(|_| "Failed to send unsubscribe command")?;
        rx.await
            .map_err(|_| "Command response channel dropped".into())
            .and_then(|r| r)
    }

    pub async fn disconnect(&self) -> CommandResult<()> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(QuicCommand::Disconnect { resp: tx })
            .await
            .map_err(|_| "Failed to send disconnect command")?;
        rx.await
            .map_err(|_| "Command response channel dropped".into())
            .and_then(|r| r)
    }

    pub async fn next_event(&mut self) -> Option<MqttEvent> {
        self.event_rx.recv().await
    }
}

async fn run_engine_loop(
    mut engine: QuicMqttEngine,
    mut command_rx: mpsc::Receiver<QuicCommand>,
    event_tx: mpsc::Sender<MqttEvent>,
) -> CommandResult<()> {
    let mut socket: Option<UdpSocket> = None;
    let mut interval = tokio::time::interval(Duration::from_millis(10));
    let mut outgoing_buffer: VecDeque<(SocketAddr, Vec<u8>)> = VecDeque::new();
    let mut buf = [0u8; 2048];

    loop {
        tokio::select! {
            Some(cmd) = command_rx.recv() => {
                match cmd {
                    QuicCommand::Connect { server_addr, server_name, crypto, resp } => {
                        let res = (|| -> CommandResult<()> {
                            let std_socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
                            std_socket.set_nonblocking(true)?;
                            socket = Some(UdpSocket::from_std(std_socket)?);
                            engine.connect(server_addr, &server_name, *crypto, Instant::now())?;
                            Ok(())
                        })();
                        let _ = resp.send(res);
                    }
                    QuicCommand::Publish { cmd, resp } => {
                        let _ = resp.send(engine.publish(cmd).map_err(|e| e.into()));
                    }
                    QuicCommand::Subscribe { cmd, resp } => {
                        let _ = resp.send(engine.subscribe(cmd).map_err(|e| e.into()));
                    }
                    QuicCommand::Unsubscribe { cmd, resp } => {
                        let _ = resp.send(engine.unsubscribe(cmd).map_err(|e| e.into()));
                    }
                    QuicCommand::Disconnect { resp } => {
                        engine.disconnect();
                        let _ = resp.send(Ok(()));
                    }
                }
            }

            res = async {
                if let Some(ref s) = socket {
                    s.recv_from(&mut buf).await
                } else {
                    std::future::pending().await
                }
            } => {
                match res {
                    Ok((len, remote)) => {
                        engine.handle_datagram(buf[..len].to_vec(), remote, Instant::now());
                    }
                    Err(e) => {
                        eprintln!("UDP Recv Error: {}", e);
                    }
                }
            }

            _ = interval.tick() => {
                let events = engine.handle_tick(Instant::now());
                for event in events {
                    if (event_tx.send(event).await).is_err() {
                        return Ok(());
                    }
                }
            }
        }

        outgoing_buffer.extend(engine.take_outgoing_datagrams());
        if let Some(ref s) = socket {
            while let Some((dest, data)) = outgoing_buffer.pop_front() {
                match s.try_send_to(&data, dest) {
                    Ok(_) => {}
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        outgoing_buffer.push_front((dest, data));
                        break;
                    }
                    Err(e) => {
                        eprintln!("UDP Send Error: {}", e);
                    }
                }
            }
        }
    }
}
