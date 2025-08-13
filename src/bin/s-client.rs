// This is S-Client.rs, a simple MQTT client using gRPC to connect to an MQTT broker.
// It uses the `mqttv5pb` module generated from the MQTT v5.0 protobuf definitions.
// It shoot messages to S-Proxy which relays them to the MQTT broker without the need for connection management.

use mqttv5pb::mqtt_relay_service_client::MqttRelayServiceClient;

pub mod mqttv5pb {
    tonic::include_proto!("mqttv5"); // The string specified here must match the proto package name
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = MqttRelayServiceClient::connect("http://[::1]:50515").await?;

    let mut pkt_id = 1;
    loop {
        let publish = mqttv5pb::Publish {
            topic: "test/topic".into(),
            payload: vec![1, 2, 3, 4],
            qos: 1,
            retain: false,
            dup: false,
            message_id: pkt_id,
            properties: vec![],
        };

        let mut request: tonic::Request<mqttv5pb::Publish> = tonic::Request::new(publish);
        request.metadata_mut().insert(
            "x-client-id",
            tonic::metadata::MetadataValue::from_static("client_id"),
        );
        let response = client.mqtt_publish_qos1(request).await?;
        println!("RESPONSE={:?}", response);
        // Simulate some work
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        pkt_id += 1;
    }
}
