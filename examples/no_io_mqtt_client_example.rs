// SPDX-License-Identifier: MPL-2.0
//! Plain TCP example for local development. Use no_io_tls_client_example for cloud credentials.
#[path = "support/no_io_loop.rs"]
mod driver;
use flowsdk::mqtt_client::{MqttClientOptions, NoIoMqttClient, OperationTimeouts};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let peer = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "localhost:1883".into());
    let options = MqttClientOptions::builder()
        .peer(&peer)
        .client_id("flowsdk-no-io-example")
        .reconnect(true)
        .operation_timeouts(OperationTimeouts::cloud())
        .max_incoming_packet_size(1024 * 1024)
        .max_incoming_buffer_bytes(1024 * 1024)
        .max_outgoing_buffer_bytes(8 * 1024 * 1024)
        .build();
    driver::run(&peer, NoIoMqttClient::new(options))
}
