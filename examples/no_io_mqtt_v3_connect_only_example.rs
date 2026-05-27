// SPDX-License-Identifier: MPL-2.0

use clap::{ArgAction, Parser};
use flowsdk::mqtt_client::{MqttClientOptions, NoIoMqttClient};
use std::io::{self, Write};
use std::net::TcpStream;

#[derive(Debug, Parser)]
#[command(name = "no_io_mqtt_v3_connect_only_example")]
#[command(about = "Send only MQTT v3 CONNECT then close transport")]
struct Args {
    #[arg(long)]
    host: String,

    #[arg(long, default_value_t = 1883)]
    port: u16,

    #[arg(long = "clientid")]
    clientid: String,

    #[arg(long = "clean-session", default_value_t = true, action = ArgAction::Set)]
    clean_session: bool,
}

fn run_example() -> io::Result<()> {
    let args = Args::parse();
    let broker = format!("{}:{}", args.host, args.port);

    println!("MQTT v3.1.1 connect-only example");
    println!("Broker: {}", broker);
    println!("Client ID: {}", args.clientid);
    println!("Clean Session: {}", args.clean_session);

    let options = MqttClientOptions::builder()
        .peer(&broker)
        .client_id(args.clientid)
        .mqtt_version(3)
        .clean_start(args.clean_session)
        .keep_alive(60)
        .build();

    let mut client = NoIoMqttClient::new(options);
    let mut transport = TcpStream::connect(&broker)?;
    transport.set_nodelay(true)?;

    client.connect();
    let connect_bytes = client.take_outgoing();

    if connect_bytes.is_empty() {
        return Err(io::Error::other("No CONNECT packet was generated"));
    }

    transport.write_all(&connect_bytes)?;
    transport.flush()?;

    println!("Sent CONNECT packet ({} bytes)", connect_bytes.len());
    println!("Closing transport without sending MQTT DISCONNECT");

    drop(transport);
    Ok(())
}

fn main() -> io::Result<()> {
    run_example()
}
