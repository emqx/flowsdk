use flowsdk::mqtt_client::commands::{PublishCommand, SubscribeCommand};
use flowsdk::mqtt_client::engine::MqttEvent;
use flowsdk::mqtt_client::opts::MqttClientOptions;
use flowsdk::mqtt_client::tokio_quic_client::TokioQuicMqttClient;
use std::net::ToSocketAddrs;
use std::time::Duration;

/// Demonstrates how to use the TokioQuicMqttClient which is a wrapper around the QuicMqttEngine.
/// This is a more low-level example that shows how to use the QuicMqttEngine directly.
/// User should use TokioQuicMqttClient in most cases.
async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mqtt_opts = MqttClientOptions::builder()
        .client_id("quic-async-wrapper-client")
        .peer("broker.emqx.io:14567")
        .build();

    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs()? {
        root_store.add(cert).ok();
    }
    let crypto = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let server_addr = ("broker.emqx.io", 14567u16)
        .to_socket_addrs()?
        .next()
        .ok_or("DNS Failure")?;
    let mut client = TokioQuicMqttClient::new(mqtt_opts)?;

    client
        .connect(server_addr, "broker.emqx.io".to_string(), crypto)
        .await
        .map_err(|e| e.to_string())?;

    let mut subscribed = false;
    let mut published = false;

    loop {
        tokio::select! {
            event = client.next_event() => {
                let Some(event) = event else { break; };
                match event {
                    MqttEvent::Connected(_) => {
                        if !subscribed {
                            let sub_cmd = SubscribeCommand::builder().add_topic("test/topic/wrapper", 1).build()?;
                            client.subscribe(sub_cmd).await.map_err(|e| e.to_string())?;
                            subscribed = true;
                        }
                    }
                    MqttEvent::Subscribed(res) => {
                        println!("Subscribed: ID={:?}", res.packet_id);
                        if !published {
                            let pub_cmd = PublishCommand::builder().topic("test/topic/wrapper").payload("Hello!".to_string()).qos(1).build()?;
                            client.publish(pub_cmd).await.map_err(|e| e.to_string())?;
                            published = true;
                        }
                    }
                    MqttEvent::Published(res) => {
                        println!("Published: ID={:?}", res.packet_id);
                        return Ok(());
                    }
                    MqttEvent::Disconnected(_) => return Ok(()),
                    _ => {}
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(30)) => return Ok(()),
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_example().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_example() {
        run_example().await.unwrap();
    }
}
