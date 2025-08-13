use tonic::{transport::Server, Request, Response, Status};

use mqttv5pb::mqtt_relay_service_server::{MqttRelayService, MqttRelayServiceServer};
use mqttv5pb::{MqttPacket, RelayResponse};

pub mod mqttv5pb {
    tonic::include_proto!("mqttv5"); // The string specified here must match the proto package name
}

#[derive(Debug, Default)]
pub struct MyRelay {}

#[tonic::async_trait]
impl MqttRelayService for MyRelay {
    async fn relay_packet(
        &self,
        request: Request<MqttPacket>, // Accept request of type MqttPacket
    ) -> Result<Response<RelayResponse>, Status> {
        // Return an instance of type RelayResponse
        println!("Got a request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };

        Ok(Response::new(reply)) // Send back our formatted greeting
    }

    async fn mqtt_connect(
        &self,
        request: Request<mqttv5pb::Connect>,
    ) -> Result<Response<mqttv5pb::Connack>, Status> {
        // Handle MQTT connect logic here

        println!("Got a request: {:?}", request);

        let m = mqttv5pb::Connack {
            session_present: true,
            reason_code: 0,
            properties: Default::default(),
        };
        Ok(Response::new(m))
    }

    async fn mqtt_publish_qos1(
        &self,
        request: Request<mqttv5pb::Publish>,
    ) -> Result<Response<mqttv5pb::Puback>, Status> {
        // Handle MQTT publish logic here

        println!("Got a request: {:?}", request);

        let m = mqttv5pb::Puback {
            message_id: request.get_ref().message_id,
            reason_code: 0,
            properties: Default::default(),
        };
        Ok(Response::new(m))
    }

    async fn mqtt_subscribe(
        &self,
        request: Request<mqttv5pb::Subscribe>,
    ) -> Result<Response<mqttv5pb::Suback>, Status> {
        // Handle MQTT subscribe logic here

        println!("Got a request: {:?}", request);

        let m = mqttv5pb::Suback {
            message_id: request.get_ref().message_id,
            reason_codes: vec![0],
            properties: Default::default(),
        };
        Ok(Response::new(m))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50515".parse()?;
    let relay = MyRelay::default();

    Server::builder()
        .add_service(MqttRelayServiceServer::new(relay))
        .serve(addr)
        .await?;

    Ok(())
}
