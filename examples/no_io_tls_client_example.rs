// SPDX-License-Identifier: MPL-2.0
//! MQTT_PEER=host:8883 MQTT_SERVER_NAME=host cargo run --features rustls-tls --example no_io_tls_client_example
//! Optional MQTT_CA_FILE, MQTT_CERT_FILE/MQTT_KEY_FILE, MQTT_USERNAME/MQTT_PASSWORD.
#[path = "support/no_io_loop.rs"]
mod driver;
use flowsdk::mqtt_client::{
    transport::rustls_tls::RustlsTlsConfig, MqttClientOptions, OperationTimeouts, TlsMqttEngine,
};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let peer = std::env::var("MQTT_PEER")?;
    let name = std::env::var("MQTT_SERVER_NAME")?;
    let mut tls = RustlsTlsConfig::builder().use_system_roots(true);
    if let Ok(path) = std::env::var("MQTT_CA_FILE") {
        tls = tls.add_roots_from_pem_file(path)?;
    }
    match (
        std::env::var("MQTT_CERT_FILE"),
        std::env::var("MQTT_KEY_FILE"),
    ) {
        (Ok(cert), Ok(key)) => tls = tls.client_auth_from_pem_files(cert, key)?,
        (Err(_), Err(_)) => {}
        _ => return Err("Set both MQTT_CERT_FILE and MQTT_KEY_FILE for mutual TLS".into()),
    }
    let mut options = MqttClientOptions::builder()
        .peer(&peer)
        .client_id(
            std::env::var("MQTT_CLIENT_ID").unwrap_or_else(|_| "flowsdk-no-io-tls-example".into()),
        )
        .reconnect(true)
        .operation_timeouts(OperationTimeouts::cloud())
        .max_incoming_packet_size(1024 * 1024)
        .max_incoming_buffer_bytes(1024 * 1024)
        .max_outgoing_buffer_bytes(8 * 1024 * 1024)
        .build();
    if let Ok(user) = std::env::var("MQTT_USERNAME") {
        options = options.username(user);
    }
    if let Ok(password) = std::env::var("MQTT_PASSWORD") {
        options = options.password(password.into_bytes());
    }
    let engine = TlsMqttEngine::new(options, &name, Arc::new(tls.build().to_client_config()?))?;
    driver::run(&peer, engine)
}
