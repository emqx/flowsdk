use tonic::{transport::Server, Request, Response, Status, Streaming};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

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

    async fn mqtt_puback(
        &self,
        request: Request<mqttv5pb::Puback>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a PubAck request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_pubrec(
        &self,
        request: Request<mqttv5pb::Pubrec>,
    ) -> Result<Response<mqttv5pb::Pubrel>, Status> {
        println!("Got a PubRec request: {:?}", request);

        let pubrel = mqttv5pb::Pubrel {
            message_id: request.get_ref().message_id,
            reason_code: 0,
            properties: Default::default(),
        };
        Ok(Response::new(pubrel))
    }

    async fn mqtt_pubrel(
        &self,
        request: Request<mqttv5pb::Pubrel>,
    ) -> Result<Response<mqttv5pb::Pubcomp>, Status> {
        println!("Got a PubRel request: {:?}", request);

        let pubcomp = mqttv5pb::Pubcomp {
            message_id: request.get_ref().message_id,
            reason_code: 0,
            properties: Default::default(),
        };
        Ok(Response::new(pubcomp))
    }

    async fn mqtt_pubcomp(
        &self,
        request: Request<mqttv5pb::Pubcomp>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a PubComp request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_unsubscribe(
        &self,
        request: Request<mqttv5pb::Unsubscribe>,
    ) -> Result<Response<mqttv5pb::Unsuback>, Status> {
        println!("Got an Unsubscribe request: {:?}", request);

        let topic_count = request.get_ref().topic_filters.len();
        let unsuback = mqttv5pb::Unsuback {
            message_id: request.get_ref().message_id,
            reason_codes: vec![0; topic_count], // Success for all topics
            properties: Default::default(),
        };
        Ok(Response::new(unsuback))
    }

    async fn mqtt_pingreq(
        &self,
        request: Request<mqttv5pb::Pingreq>,
    ) -> Result<Response<mqttv5pb::Pingresp>, Status> {
        println!("Got a PingReq request: {:?}", request);

        let pingresp = mqttv5pb::Pingresp {};
        Ok(Response::new(pingresp))
    }

    async fn mqtt_disconnect(
        &self,
        request: Request<mqttv5pb::Disconnect>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a Disconnect request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_auth(
        &self,
        request: Request<mqttv5pb::Auth>,
    ) -> Result<Response<mqttv5pb::Auth>, Status> {
        println!("Got an Auth request: {:?}", request);

        let auth_response = mqttv5pb::Auth {
            reason_code: 0, // Success
            properties: Default::default(),
        };
        Ok(Response::new(auth_response))
    }

    async fn mqtt_suback(
        &self,
        request: Request<mqttv5pb::Suback>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got a SubAck request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    async fn mqtt_unsuback(
        &self,
        request: Request<mqttv5pb::Unsuback>,
    ) -> Result<Response<RelayResponse>, Status> {
        println!("Got an UnsubAck request: {:?}", request);

        let reply = RelayResponse {
            status_code: 0,
            error_message: String::new(),
        };
        Ok(Response::new(reply))
    }

    type StreamMqttMessagesStream = ReceiverStream<Result<mqttv5pb::MqttStreamMessage, Status>>;

    async fn stream_mqtt_messages(
        &self,
        request: Request<Streaming<mqttv5pb::MqttStreamMessage>>,
    ) -> Result<Response<Self::StreamMqttMessagesStream>, Status> {
        println!("Got a streaming MQTT request: {:?}", request.metadata());

        // Create a channel for responses
        let (tx, rx) = mpsc::channel(100);
        
        // Simple echo server that just sends back an acknowledgment
        let echo_msg = mqttv5pb::MqttStreamMessage {
            session_id: "server-echo".to_string(),
            sequence_id: 1,
            direction: mqttv5pb::MessageDirection::BrokerToClient as i32,
            payload: Some(mqttv5pb::mqtt_stream_message::Payload::Connack(mqttv5pb::Connack {
                session_present: false,
                reason_code: 0,
                properties: Vec::new(),
            })),
        };
        
        let _ = tx.send(Ok(echo_msg)).await;

        Ok(Response::new(ReceiverStream::new(rx)))
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
