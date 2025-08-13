use mqttv5pb::mqtt_relay_service_client::MqttRelayServiceClient;
use mqttv5pb::Connect;

pub mod mqttv5pb {
    tonic::include_proto!("mqttv5"); // The string specified here must match the proto package name
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = MqttRelayServiceClient::connect("http://[::1]:50515").await?;
    let clientid = "client_id".to_string();
    let connect_packet = Connect {
        client_id: clientid.clone(),
        protocol_name: "MQTT".into(),
        protocol_version: 5,
        clean_start: true,
        keep_alive: 60,
        username: "user".into(),
        password: "pass".into(),
        will: None,
        properties: vec![],
    };

    let request = tonic::Request::new(connect_packet);

    let response = client.mqtt_connect(request).await?;

    println!("RESPONSE={:?}", response);

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
            tonic::metadata::MetadataValue::try_from(clientid.clone()).unwrap(),
        );
        let response = client.mqtt_publish_qos1(request).await?;
        println!("RESPONSE={:?}", response);
        // Simulate some work
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        pkt_id += 1;
    }
}
