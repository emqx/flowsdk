// Shared gRPC conversion logic for mqtt-grpc-proxy
// This module provides shared conversion implementations between MQTT and protobuf types
// Used by both r-proxy and s-proxy to eliminate code duplication

use flowsdk::mqtt_serde::mqttv5::auth::MqttAuth;
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
use flowsdk::mqtt_serde::mqttv5::connack::MqttConnAck;
use flowsdk::mqtt_serde::mqttv5::connect::MqttConnect;
use flowsdk::mqtt_serde::mqttv5::disconnect::MqttDisconnect;
use flowsdk::mqtt_serde::mqttv5::puback::MqttPubAck;
use flowsdk::mqtt_serde::mqttv5::pubcomp::MqttPubComp;
use flowsdk::mqtt_serde::mqttv5::publish::MqttPublish;
use flowsdk::mqtt_serde::mqttv5::pubrec::MqttPubRec;
use flowsdk::mqtt_serde::mqttv5::pubrel::MqttPubRel;
use flowsdk::mqtt_serde::mqttv5::suback::MqttSubAck;
use flowsdk::mqtt_serde::mqttv5::subscribe::MqttSubscribe;
use flowsdk::mqtt_serde::mqttv5::unsuback::MqttUnsubAck;
use flowsdk::mqtt_serde::mqttv5::unsubscribe::MqttUnsubscribe;

// Generate protobuf types at compile time
pub mod mqttv5pb {
    tonic::include_proto!("mqttv5");
}

// Helper function to convert WillProperties to protobuf properties
fn convert_will_properties_to_pb(
    will_props: &flowsdk::mqtt_serde::mqttv5::will::WillProperties,
) -> Vec<mqttv5pb::Property> {
    let mut properties = Vec::new();

    // Add WillDelayInterval if present
    if let Some(will_delay_interval) = will_props.will_delay_interval {
        if let Ok(prop) = Property::WillDelayInterval(will_delay_interval).try_into() {
            properties.push(prop);
        }
    }

    // Add PayloadFormatIndicator if present
    if let Some(payload_format_indicator) = will_props.payload_format_indicator {
        if let Ok(prop) = Property::PayloadFormatIndicator(payload_format_indicator).try_into() {
            properties.push(prop);
        }
    }

    // Add MessageExpiryInterval if present
    if let Some(message_expiry_interval) = will_props.message_expiry_interval {
        if let Ok(prop) = Property::MessageExpiryInterval(message_expiry_interval).try_into() {
            properties.push(prop);
        }
    }

    // Add ContentType if present
    if let Some(ref content_type) = will_props.content_type {
        if let Ok(prop) = Property::ContentType(content_type.clone()).try_into() {
            properties.push(prop);
        }
    }

    // Add ResponseTopic if present
    if let Some(ref response_topic) = will_props.response_topic {
        if let Ok(prop) = Property::ResponseTopic(response_topic.clone()).try_into() {
            properties.push(prop);
        }
    }

    // Add CorrelationData if present
    if let Some(ref correlation_data) = will_props.correlation_data {
        if let Ok(prop) = Property::CorrelationData(correlation_data.clone()).try_into() {
            properties.push(prop);
        }
    }

    // Add UserProperties
    properties.extend(convert_properties_to_pb(will_props.user_properties.clone()));

    properties
}

// Helper function to convert protobuf properties to WillProperties
#[allow(dead_code)]
fn convert_pb_to_will_properties(
    pb_properties: Vec<mqttv5pb::Property>,
) -> flowsdk::mqtt_serde::mqttv5::will::WillProperties {
    let mut will_props = flowsdk::mqtt_serde::mqttv5::will::WillProperties::default();

    for pb_prop in pb_properties {
        if let Some(property_type) = pb_prop.property_type {
            match property_type {
                mqttv5pb::property::PropertyType::WillDelayInterval(val) => {
                    will_props.will_delay_interval = Some(val);
                }
                mqttv5pb::property::PropertyType::PayloadFormatIndicator(val) => {
                    will_props.payload_format_indicator = Some(if val { 1 } else { 0 });
                }
                mqttv5pb::property::PropertyType::MessageExpiryInterval(val) => {
                    will_props.message_expiry_interval = Some(val);
                }
                mqttv5pb::property::PropertyType::ContentType(val) => {
                    will_props.content_type = Some(val);
                }
                mqttv5pb::property::PropertyType::ResponseTopic(val) => {
                    will_props.response_topic = Some(val);
                }
                mqttv5pb::property::PropertyType::CorrelationData(val) => {
                    will_props.correlation_data = Some(val);
                }
                mqttv5pb::property::PropertyType::UserProperty(user_prop) => {
                    will_props
                        .user_properties
                        .push(Property::UserProperty(user_prop.key, user_prop.value));
                }
                _ => {
                    // Ignore properties that don't belong to Will
                }
            }
        }
    }

    will_props
}

// Helper functions for property conversion
fn convert_properties_to_pb<T>(properties: Vec<T>) -> Vec<mqttv5pb::Property>
where
    T: TryInto<mqttv5pb::Property>,
{
    properties
        .into_iter()
        .filter_map(|p| p.try_into().ok())
        .collect()
}

fn convert_properties_from_pb<T>(properties: Vec<mqttv5pb::Property>) -> Vec<T>
where
    mqttv5pb::Property: TryInto<T>,
{
    properties
        .into_iter()
        .filter_map(|p| p.try_into().ok())
        .collect()
}

// For r-proxy compatibility - keep the trait implementations
impl From<MqttConnect> for mqttv5pb::Connect {
    fn from(connect: MqttConnect) -> Self {
        mqttv5pb::Connect {
            client_id: connect.client_id.clone(),
            protocol_name: "MQTT".into(),
            protocol_version: connect.protocol_version as u32,
            clean_start: connect.is_clean_start(),
            keep_alive: connect.keep_alive as u32,
            username: connect.username.unwrap_or_default(),
            password: connect.password.unwrap_or_default(),
            will: connect.will.map(|will| mqttv5pb::Will {
                qos: will.will_qos as i32,
                retain: will.will_retain,
                topic: will.will_topic,
                payload: will.will_message,
                properties: convert_will_properties_to_pb(&will.properties),
            }),
            properties: convert_properties_to_pb(connect.properties),
        }
    }
}

impl From<MqttConnAck> for mqttv5pb::Connack {
    fn from(connack: MqttConnAck) -> Self {
        mqttv5pb::Connack {
            session_present: connack.session_present,
            reason_code: connack.reason_code as u32,
            properties: connack
                .properties
                .map(convert_properties_to_pb)
                .unwrap_or_default(),
        }
    }
}

impl TryFrom<Property> for mqttv5pb::Property {
    type Error = ();

    fn try_from(p: Property) -> Result<Self, Self::Error> {
        let property_type = match p {
            Property::PayloadFormatIndicator(val) => Some(
                mqttv5pb::property::PropertyType::PayloadFormatIndicator(val != 0),
            ),
            Property::MessageExpiryInterval(val) => {
                Some(mqttv5pb::property::PropertyType::MessageExpiryInterval(val))
            }
            Property::ContentType(val) => Some(mqttv5pb::property::PropertyType::ContentType(val)),
            Property::ResponseTopic(val) => {
                Some(mqttv5pb::property::PropertyType::ResponseTopic(val))
            }
            Property::CorrelationData(val) => {
                Some(mqttv5pb::property::PropertyType::CorrelationData(val))
            }
            Property::SubscriptionIdentifier(val) => Some(
                mqttv5pb::property::PropertyType::SubscriptionIdentifier(val),
            ),
            Property::SessionExpiryInterval(val) => {
                Some(mqttv5pb::property::PropertyType::SessionExpiryInterval(val))
            }
            Property::AssignedClientIdentifier(val) => Some(
                mqttv5pb::property::PropertyType::AssignedClientIdentifier(val),
            ),
            Property::ServerKeepAlive(val) => Some(
                mqttv5pb::property::PropertyType::ServerKeepAlive(val as u32),
            ),
            Property::AuthenticationMethod(val) => {
                Some(mqttv5pb::property::PropertyType::AuthenticationMethod(val))
            }
            Property::AuthenticationData(val) => {
                Some(mqttv5pb::property::PropertyType::AuthenticationData(val))
            }
            Property::RequestProblemInformation(val) => Some(
                mqttv5pb::property::PropertyType::RequestProblemInformation(val != 0),
            ),
            Property::WillDelayInterval(val) => {
                Some(mqttv5pb::property::PropertyType::WillDelayInterval(val))
            }
            Property::RequestResponseInformation(val) => {
                Some(mqttv5pb::property::PropertyType::RequestResponseInformation(val != 0))
            }
            Property::ResponseInformation(val) => {
                Some(mqttv5pb::property::PropertyType::ResponseInformation(val))
            }
            Property::ServerReference(val) => {
                Some(mqttv5pb::property::PropertyType::ServerReference(val))
            }
            Property::ReasonString(val) => {
                Some(mqttv5pb::property::PropertyType::ReasonString(val))
            }
            Property::ReceiveMaximum(val) => {
                Some(mqttv5pb::property::PropertyType::ReceiveMaximum(val as u32))
            }
            Property::TopicAliasMaximum(val) => Some(
                mqttv5pb::property::PropertyType::TopicAliasMaximum(val as u32),
            ),
            Property::TopicAlias(val) => {
                Some(mqttv5pb::property::PropertyType::TopicAlias(val as u32))
            }
            Property::MaximumQoS(val) => {
                Some(mqttv5pb::property::PropertyType::MaximumQos(val as u32))
            }
            Property::RetainAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::RetainAvailable(val != 0))
            }
            Property::UserProperty(key, value) => {
                Some(mqttv5pb::property::PropertyType::UserProperty(
                    mqttv5pb::UserProperty { key, value },
                ))
            }
            Property::MaximumPacketSize(val) => {
                Some(mqttv5pb::property::PropertyType::MaximumPacketSize(val))
            }
            Property::WildcardSubscriptionAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::WildcardSubscriptionAvailable(val != 0))
            }
            Property::SubscriptionIdentifierAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::SubscriptionIdentifiersAvailable(val != 0))
            }
            Property::SharedSubscriptionAvailable(val) => {
                Some(mqttv5pb::property::PropertyType::SharedSubscriptionAvailable(val != 0))
            }
        };
        Ok(mqttv5pb::Property { property_type })
    }
}

impl From<mqttv5pb::Publish> for MqttPublish {
    fn from(publish: mqttv5pb::Publish) -> Self {
        let mut mqtt_publish = MqttPublish::new(
            publish.qos as u8,
            publish.topic,
            Some(publish.message_id as u16),
            publish.payload,
            publish.retain,
            publish.dup,
        );

        // Convert properties if present
        mqtt_publish.properties = convert_properties_from_pb(publish.properties);

        mqtt_publish
    }
}

impl TryFrom<mqttv5pb::Property> for Property {
    type Error = ();

    fn try_from(p: mqttv5pb::Property) -> Result<Self, Self::Error> {
        match p.property_type {
            Some(mqttv5pb::property::PropertyType::PayloadFormatIndicator(val)) => {
                Ok(Property::PayloadFormatIndicator(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::MessageExpiryInterval(val)) => {
                Ok(Property::MessageExpiryInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::ContentType(val)) => {
                Ok(Property::ContentType(val))
            }
            Some(mqttv5pb::property::PropertyType::ResponseTopic(val)) => {
                Ok(Property::ResponseTopic(val))
            }
            Some(mqttv5pb::property::PropertyType::CorrelationData(val)) => {
                Ok(Property::CorrelationData(val))
            }
            Some(mqttv5pb::property::PropertyType::SubscriptionIdentifier(val)) => {
                Ok(Property::SubscriptionIdentifier(val))
            }
            Some(mqttv5pb::property::PropertyType::SessionExpiryInterval(val)) => {
                Ok(Property::SessionExpiryInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::AssignedClientIdentifier(val)) => {
                Ok(Property::AssignedClientIdentifier(val))
            }
            Some(mqttv5pb::property::PropertyType::ServerKeepAlive(val)) => {
                Ok(Property::ServerKeepAlive(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::AuthenticationMethod(val)) => {
                Ok(Property::AuthenticationMethod(val))
            }
            Some(mqttv5pb::property::PropertyType::AuthenticationData(val)) => {
                Ok(Property::AuthenticationData(val))
            }
            Some(mqttv5pb::property::PropertyType::RequestProblemInformation(val)) => {
                Ok(Property::RequestProblemInformation(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::WillDelayInterval(val)) => {
                Ok(Property::WillDelayInterval(val))
            }
            Some(mqttv5pb::property::PropertyType::RequestResponseInformation(val)) => {
                Ok(Property::RequestResponseInformation(if val {
                    1
                } else {
                    0
                }))
            }
            Some(mqttv5pb::property::PropertyType::ResponseInformation(val)) => {
                Ok(Property::ResponseInformation(val))
            }
            Some(mqttv5pb::property::PropertyType::ServerReference(val)) => {
                Ok(Property::ServerReference(val))
            }
            Some(mqttv5pb::property::PropertyType::ReasonString(val)) => {
                Ok(Property::ReasonString(val))
            }
            Some(mqttv5pb::property::PropertyType::ReceiveMaximum(val)) => {
                Ok(Property::ReceiveMaximum(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::TopicAliasMaximum(val)) => {
                Ok(Property::TopicAliasMaximum(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::TopicAlias(val)) => {
                Ok(Property::TopicAlias(val as u16))
            }
            Some(mqttv5pb::property::PropertyType::MaximumQos(val)) => {
                Ok(Property::MaximumQoS(val as u8))
            }
            Some(mqttv5pb::property::PropertyType::RetainAvailable(val)) => {
                Ok(Property::RetainAvailable(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::UserProperty(user_prop)) => {
                Ok(Property::UserProperty(user_prop.key, user_prop.value))
            }
            Some(mqttv5pb::property::PropertyType::MaximumPacketSize(val)) => {
                Ok(Property::MaximumPacketSize(val))
            }
            Some(mqttv5pb::property::PropertyType::WildcardSubscriptionAvailable(val)) => {
                Ok(Property::WildcardSubscriptionAvailable(if val {
                    1
                } else {
                    0
                }))
            }
            Some(mqttv5pb::property::PropertyType::SubscriptionIdentifiersAvailable(val)) => {
                Ok(Property::SubscriptionIdentifierAvailable(if val {
                    1
                } else {
                    0
                }))
            }
            Some(mqttv5pb::property::PropertyType::SharedSubscriptionAvailable(val)) => {
                Ok(Property::SharedSubscriptionAvailable(if val {
                    1
                } else {
                    0
                }))
            }
            None => Err(()),
        }
    }
}

impl From<mqttv5pb::Connect> for MqttConnect {
    fn from(connect: mqttv5pb::Connect) -> Self {
        let properties: Vec<Property> = convert_properties_from_pb(connect.properties);

        MqttConnect::new(
            connect.client_id,
            if connect.username.is_empty() {
                None
            } else {
                Some(connect.username)
            },
            if connect.password.is_empty() {
                None
            } else {
                Some(connect.password)
            },
            connect
                .will
                .map(|will| flowsdk::mqtt_serde::mqttv5::will::Will {
                    will_qos: will.qos as u8,
                    will_retain: will.retain,
                    will_topic: will.topic,
                    will_message: will.payload,
                    properties: convert_pb_to_will_properties(will.properties),
                }),
            connect.keep_alive as u16,
            connect.clean_start,
            properties,
        )
    }
}

impl From<MqttPublish> for mqttv5pb::Publish {
    fn from(publish: MqttPublish) -> Self {
        mqttv5pb::Publish {
            topic: publish.topic_name,
            payload: publish.payload,
            qos: publish.qos as i32,
            retain: publish.retain,
            dup: publish.dup,
            message_id: publish.packet_id.unwrap_or(0) as u32,
            properties: convert_properties_to_pb(publish.properties),
        }
    }
}

impl From<MqttPubAck> for mqttv5pb::Puback {
    fn from(puback: MqttPubAck) -> Self {
        mqttv5pb::Puback {
            message_id: puback.packet_id as u32,
            reason_code: puback.reason_code as u32,
            properties: convert_properties_to_pb(puback.properties),
        }
    }
}

impl From<MqttPubRec> for mqttv5pb::Pubrec {
    fn from(pubrec: MqttPubRec) -> Self {
        mqttv5pb::Pubrec {
            message_id: pubrec.packet_id as u32,
            reason_code: pubrec.reason_code as u32,
            properties: convert_properties_to_pb(pubrec.properties),
        }
    }
}

impl From<MqttPubRel> for mqttv5pb::Pubrel {
    fn from(pubrel: MqttPubRel) -> Self {
        mqttv5pb::Pubrel {
            message_id: pubrel.packet_id as u32,
            reason_code: pubrel.reason_code as u32,
            properties: convert_properties_to_pb(pubrel.properties),
        }
    }
}

impl From<MqttPubComp> for mqttv5pb::Pubcomp {
    fn from(pubcomp: MqttPubComp) -> Self {
        mqttv5pb::Pubcomp {
            message_id: pubcomp.packet_id as u32,
            reason_code: pubcomp.reason_code as u32,
            properties: convert_properties_to_pb(pubcomp.properties),
        }
    }
}

impl From<MqttSubAck> for mqttv5pb::Suback {
    fn from(suback: MqttSubAck) -> Self {
        mqttv5pb::Suback {
            message_id: suback.packet_id as u32,
            reason_codes: suback.reason_codes.into_iter().map(|c| c as u32).collect(),
            properties: convert_properties_to_pb(suback.properties),
        }
    }
}

impl From<MqttUnsubAck> for mqttv5pb::Unsuback {
    fn from(unsuback: MqttUnsubAck) -> Self {
        mqttv5pb::Unsuback {
            message_id: unsuback.packet_id as u32,
            reason_codes: unsuback
                .reason_codes
                .into_iter()
                .map(|c| c as u32)
                .collect(),
            properties: convert_properties_to_pb(unsuback.properties),
        }
    }
}

impl From<MqttDisconnect> for mqttv5pb::Disconnect {
    fn from(disconnect: MqttDisconnect) -> Self {
        mqttv5pb::Disconnect {
            reason_code: disconnect.reason_code as u32,
            properties: convert_properties_to_pb(disconnect.properties),
        }
    }
}

impl From<MqttAuth> for mqttv5pb::Auth {
    fn from(auth: MqttAuth) -> Self {
        mqttv5pb::Auth {
            reason_code: auth.reason_code as u32,
            properties: convert_properties_to_pb(auth.properties),
        }
    }
}

// Reverse conversions (protobuf to MQTT) - needed for two-way communication

impl From<mqttv5pb::Subscribe> for MqttSubscribe {
    fn from(subscribe: mqttv5pb::Subscribe) -> Self {
        MqttSubscribe {
            packet_id: subscribe.message_id as u16,
            properties: convert_properties_from_pb(subscribe.properties),
            subscriptions: subscribe
                .subscriptions
                .into_iter()
                .map(
                    |s| flowsdk::mqtt_serde::mqttv5::subscribe::TopicSubscription {
                        topic_filter: s.topic_filter,
                        qos: s.qos as u8,
                        no_local: s.no_local,
                        retain_as_published: s.retain_as_published,
                        retain_handling: s.retain_handling as u8,
                    },
                )
                .collect(),
        }
    }
}

impl From<mqttv5pb::Unsubscribe> for MqttUnsubscribe {
    fn from(unsubscribe: mqttv5pb::Unsubscribe) -> Self {
        MqttUnsubscribe {
            packet_id: unsubscribe.message_id as u16,
            properties: convert_properties_from_pb(unsubscribe.properties),
            topic_filters: unsubscribe.topic_filters,
        }
    }
}

impl From<mqttv5pb::Puback> for MqttPubAck {
    fn from(puback: mqttv5pb::Puback) -> Self {
        MqttPubAck {
            packet_id: puback.message_id as u16,
            reason_code: puback.reason_code as u8,
            properties: convert_properties_from_pb(puback.properties),
        }
    }
}

impl From<mqttv5pb::Pubrec> for MqttPubRec {
    fn from(pubrec: mqttv5pb::Pubrec) -> Self {
        MqttPubRec {
            packet_id: pubrec.message_id as u16,
            reason_code: pubrec.reason_code as u8,
            properties: convert_properties_from_pb(pubrec.properties),
        }
    }
}

impl From<mqttv5pb::Pubrel> for MqttPubRel {
    fn from(pubrel: mqttv5pb::Pubrel) -> Self {
        MqttPubRel {
            packet_id: pubrel.message_id as u16,
            reason_code: pubrel.reason_code as u8,
            properties: convert_properties_from_pb(pubrel.properties),
        }
    }
}

impl From<mqttv5pb::Pubcomp> for MqttPubComp {
    fn from(pubcomp: mqttv5pb::Pubcomp) -> Self {
        MqttPubComp {
            packet_id: pubcomp.message_id as u16,
            reason_code: pubcomp.reason_code as u8,
            properties: convert_properties_from_pb(pubcomp.properties),
        }
    }
}

impl From<mqttv5pb::Disconnect> for MqttDisconnect {
    fn from(disconnect: mqttv5pb::Disconnect) -> Self {
        MqttDisconnect {
            reason_code: disconnect.reason_code as u8,
            properties: convert_properties_from_pb(disconnect.properties),
        }
    }
}

impl From<mqttv5pb::Auth> for MqttAuth {
    fn from(auth: mqttv5pb::Auth) -> Self {
        MqttAuth {
            reason_code: auth.reason_code as u8,
            properties: convert_properties_from_pb(auth.properties),
        }
    }
}

// Missing reverse implementations for r-proxy compatibility
impl From<mqttv5pb::Connack> for MqttConnAck {
    fn from(connack: mqttv5pb::Connack) -> Self {
        let properties: Vec<Property> = convert_properties_from_pb(connack.properties);
        MqttConnAck {
            session_present: connack.session_present,
            reason_code: connack.reason_code as u8,
            properties: Some(properties),
        }
    }
}

impl From<MqttSubscribe> for mqttv5pb::Subscribe {
    fn from(subscribe: MqttSubscribe) -> Self {
        mqttv5pb::Subscribe {
            message_id: subscribe.packet_id as u32,
            subscriptions: subscribe
                .subscriptions
                .into_iter()
                .map(|sub| mqttv5pb::TopicSubscription {
                    topic_filter: sub.topic_filter,
                    qos: sub.qos as u32,
                    no_local: sub.no_local,
                    retain_as_published: sub.retain_as_published,
                    retain_handling: sub.retain_handling as u32,
                })
                .collect(),
            properties: convert_properties_to_pb(subscribe.properties),
        }
    }
}

impl From<MqttUnsubscribe> for mqttv5pb::Unsubscribe {
    fn from(unsubscribe: MqttUnsubscribe) -> Self {
        mqttv5pb::Unsubscribe {
            message_id: unsubscribe.packet_id as u32,
            topic_filters: unsubscribe.topic_filters,
            properties: convert_properties_to_pb(unsubscribe.properties),
        }
    }
}

impl From<mqttv5pb::Suback> for MqttSubAck {
    fn from(suback: mqttv5pb::Suback) -> Self {
        MqttSubAck {
            packet_id: suback.message_id as u16,
            reason_codes: suback.reason_codes.into_iter().map(|c| c as u8).collect(),
            properties: convert_properties_from_pb(suback.properties),
        }
    }
}

impl From<mqttv5pb::Unsuback> for MqttUnsubAck {
    fn from(unsuback: mqttv5pb::Unsuback) -> Self {
        MqttUnsubAck {
            packet_id: unsuback.message_id as u16,
            reason_codes: unsuback.reason_codes.into_iter().map(|c| c as u8).collect(),
            properties: convert_properties_from_pb(unsuback.properties),
        }
    }
}

// Additional imports needed for the helper functions
use flowsdk::mqtt_serde::control_packet::MqttPacket;

// Helper function to convert MQTT packets to stream payloads
pub fn convert_mqtt_to_stream_payload(
    packet: &MqttPacket,
) -> Option<mqttv5pb::mqtt_stream_message::Payload> {
    match packet {
        MqttPacket::Connect(connect) => Some(mqttv5pb::mqtt_stream_message::Payload::Connect(
            connect.clone().into(),
        )),
        MqttPacket::ConnAck(connack) => Some(mqttv5pb::mqtt_stream_message::Payload::Connack(
            connack.clone().into(),
        )),
        MqttPacket::Publish(publish) => Some(mqttv5pb::mqtt_stream_message::Payload::Publish(
            publish.clone().into(),
        )),
        MqttPacket::Subscribe(subscribe) => Some(
            mqttv5pb::mqtt_stream_message::Payload::Subscribe(subscribe.clone().into()),
        ),
        MqttPacket::SubAck(suback) => Some(mqttv5pb::mqtt_stream_message::Payload::Suback(
            suback.clone().into(),
        )),
        MqttPacket::Unsubscribe(unsubscribe) => Some(
            mqttv5pb::mqtt_stream_message::Payload::Unsubscribe(unsubscribe.clone().into()),
        ),
        MqttPacket::UnsubAck(unsuback) => Some(mqttv5pb::mqtt_stream_message::Payload::Unsuback(
            unsuback.clone().into(),
        )),
        MqttPacket::PubAck(puback) => Some(mqttv5pb::mqtt_stream_message::Payload::Puback(
            puback.clone().into(),
        )),
        MqttPacket::PubRec(pubrec) => Some(mqttv5pb::mqtt_stream_message::Payload::Pubrec(
            pubrec.clone().into(),
        )),
        MqttPacket::PubRel(pubrel) => Some(mqttv5pb::mqtt_stream_message::Payload::Pubrel(
            pubrel.clone().into(),
        )),
        MqttPacket::PubComp(pubcomp) => Some(mqttv5pb::mqtt_stream_message::Payload::Pubcomp(
            pubcomp.clone().into(),
        )),
        MqttPacket::PingReq(_) => Some(mqttv5pb::mqtt_stream_message::Payload::Pingreq(
            mqttv5pb::Pingreq {},
        )),
        MqttPacket::PingResp(_) => Some(mqttv5pb::mqtt_stream_message::Payload::Pingresp(
            mqttv5pb::Pingresp {},
        )),
        MqttPacket::Disconnect(disconnect) => Some(
            mqttv5pb::mqtt_stream_message::Payload::Disconnect(disconnect.clone().into()),
        ),
        MqttPacket::Auth(auth) => Some(mqttv5pb::mqtt_stream_message::Payload::Auth(
            auth.clone().into(),
        )),
    }
}

// Helper function to convert stream payloads to MQTT bytes
pub fn convert_stream_payload_to_mqtt_bytes(
    payload: &mqttv5pb::mqtt_stream_message::Payload,
) -> Option<Vec<u8>> {
    use flowsdk::mqtt_serde::control_packet::MqttControlPacket;
    use flowsdk::mqtt_serde::mqttv5;

    match payload {
        mqttv5pb::mqtt_stream_message::Payload::Connect(connect) => {
            let mqtt_connect: MqttConnect = connect.clone().into();
            mqtt_connect.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Connack(connack) => {
            let mqtt_connack: MqttConnAck = connack.clone().into();
            mqtt_connack.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Publish(publish) => {
            let mqtt_publish: mqttv5::publish::MqttPublish = publish.clone().into();
            mqtt_publish.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Subscribe(subscribe) => {
            let mqtt_subscribe: MqttSubscribe = subscribe.clone().into();
            mqtt_subscribe.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Suback(suback) => {
            let mqtt_suback: MqttSubAck = suback.clone().into();
            mqtt_suback.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Unsubscribe(unsubscribe) => {
            let mqtt_unsubscribe: MqttUnsubscribe = unsubscribe.clone().into();
            mqtt_unsubscribe.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Unsuback(unsuback) => {
            let mqtt_unsuback: mqttv5::unsuback::MqttUnsubAck = unsuback.clone().into();
            mqtt_unsuback.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Puback(puback) => {
            let mqtt_puback: mqttv5::puback::MqttPubAck = puback.clone().into();
            mqtt_puback.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pubrec(pubrec) => {
            let mqtt_pubrec: mqttv5::pubrec::MqttPubRec = pubrec.clone().into();
            mqtt_pubrec.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pubrel(pubrel) => {
            let mqtt_pubrel: mqttv5::pubrel::MqttPubRel = pubrel.clone().into();
            mqtt_pubrel.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pubcomp(pubcomp) => {
            let mqtt_pubcomp: mqttv5::pubcomp::MqttPubComp = pubcomp.clone().into();
            mqtt_pubcomp.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pingreq(_) => {
            let mqtt_pingreq = mqttv5::pingreq::MqttPingReq::new();
            mqtt_pingreq.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Pingresp(_) => {
            let mqtt_pingresp = mqttv5::pingresp::MqttPingResp::new();
            mqtt_pingresp.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Disconnect(disconnect) => {
            let mqtt_disconnect: mqttv5::disconnect::MqttDisconnect = disconnect.clone().into();
            mqtt_disconnect.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::Auth(auth) => {
            let mqtt_auth: mqttv5::auth::MqttAuth = auth.clone().into();
            mqtt_auth.to_bytes().ok()
        }
        mqttv5pb::mqtt_stream_message::Payload::SessionControl(_) => {
            // SessionControl is a gRPC-specific message that doesn't have an MQTT equivalent
            // This is used for connection management, not actual MQTT protocol messages
            None
        }
    }
}
