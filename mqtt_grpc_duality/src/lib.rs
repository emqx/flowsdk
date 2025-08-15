// Shared gRPC conversion logic for mqtt-grpc-proxy
// This module provides shared conversion implementations between MQTT and protobuf types
// Used by both r-proxy and s-proxy to eliminate code duplication

use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
use flowsdk::mqtt_serde::mqttv5::connect::MqttConnect;
use flowsdk::mqtt_serde::mqttv5::connack::MqttConnAck;
use flowsdk::mqtt_serde::mqttv5::publish::MqttPublish;
use flowsdk::mqtt_serde::mqttv5::puback::MqttPubAck;
use flowsdk::mqtt_serde::mqttv5::pubrec::MqttPubRec;
use flowsdk::mqtt_serde::mqttv5::pubrel::MqttPubRel;
use flowsdk::mqtt_serde::mqttv5::pubcomp::MqttPubComp;
use flowsdk::mqtt_serde::mqttv5::subscribe::MqttSubscribe;
use flowsdk::mqtt_serde::mqttv5::suback::MqttSubAck;
use flowsdk::mqtt_serde::mqttv5::unsubscribe::MqttUnsubscribe;
use flowsdk::mqtt_serde::mqttv5::unsuback::MqttUnsubAck;
use flowsdk::mqtt_serde::mqttv5::disconnect::MqttDisconnect;
use flowsdk::mqtt_serde::mqttv5::auth::MqttAuth;

// Generate protobuf types at compile time
pub mod mqttv5pb {
    tonic::include_proto!("mqttv5");
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
            will: None,         // @TODO
            properties: vec![], // @TODO
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
                .map(|props| {
                    props
                        .into_iter()
                        .filter_map(|p| p.try_into().ok())
                        .collect()
                })
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
        MqttPublish::new(
            publish.qos as u8,
            publish.topic,
            Some(publish.message_id as u16),
            publish.payload,
            publish.retain,
            publish.dup,
        )
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
                Ok(Property::RequestResponseInformation(if val { 1 } else { 0 }))
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
                Ok(Property::WildcardSubscriptionAvailable(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::SubscriptionIdentifiersAvailable(val)) => {
                Ok(Property::SubscriptionIdentifierAvailable(if val { 1 } else { 0 }))
            }
            Some(mqttv5pb::property::PropertyType::SharedSubscriptionAvailable(val)) => {
                Ok(Property::SharedSubscriptionAvailable(if val { 1 } else { 0 }))
            }
            None => Err(()),
        }
    }
}

impl From<mqttv5pb::Connect> for MqttConnect {
    fn from(connect: mqttv5pb::Connect) -> Self {
        let properties: Vec<Property> = connect
            .properties
            .into_iter()
            .filter_map(|p| p.try_into().ok())
            .collect();

        MqttConnect::new(
            connect.client_id,
            Some(connect.username),
            Some(connect.password),
            None,
            0,
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
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttPubAck> for mqttv5pb::Puback {
    fn from(puback: MqttPubAck) -> Self {
        mqttv5pb::Puback {
            message_id: puback.packet_id as u32,
            reason_code: puback.reason_code as u32,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttPubRec> for mqttv5pb::Pubrec {
    fn from(pubrec: MqttPubRec) -> Self {
        mqttv5pb::Pubrec {
            message_id: pubrec.packet_id as u32,
            reason_code: pubrec.reason_code as u32,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttPubRel> for mqttv5pb::Pubrel {
    fn from(pubrel: MqttPubRel) -> Self {
        mqttv5pb::Pubrel {
            message_id: pubrel.packet_id as u32,
            reason_code: pubrel.reason_code as u32,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttPubComp> for mqttv5pb::Pubcomp {
    fn from(pubcomp: MqttPubComp) -> Self {
        mqttv5pb::Pubcomp {
            message_id: pubcomp.packet_id as u32,
            reason_code: pubcomp.reason_code as u32,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttSubAck> for mqttv5pb::Suback {
    fn from(suback: MqttSubAck) -> Self {
        mqttv5pb::Suback {
            message_id: suback.packet_id as u32,
            reason_codes: suback.reason_codes.into_iter().map(|c| c as u32).collect(),
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttUnsubAck> for mqttv5pb::Unsuback {
    fn from(unsuback: MqttUnsubAck) -> Self {
        mqttv5pb::Unsuback {
            message_id: unsuback.packet_id as u32,
            reason_codes: unsuback.reason_codes.into_iter().map(|c| c as u32).collect(),
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttDisconnect> for mqttv5pb::Disconnect {
    fn from(disconnect: MqttDisconnect) -> Self {
        mqttv5pb::Disconnect {
            reason_code: disconnect.reason_code as u32,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttAuth> for mqttv5pb::Auth {
    fn from(auth: MqttAuth) -> Self {
        mqttv5pb::Auth {
            reason_code: auth.reason_code as u32,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

// Reverse conversions (protobuf to MQTT) - needed for two-way communication

impl From<mqttv5pb::Subscribe> for MqttSubscribe {
    fn from(subscribe: mqttv5pb::Subscribe) -> Self {
        MqttSubscribe {
            packet_id: subscribe.message_id as u16,
            properties: Vec::new(), // TODO: Convert properties
            subscriptions: subscribe.subscriptions.into_iter().map(|s| flowsdk::mqtt_serde::mqttv5::subscribe::TopicSubscription {
                topic_filter: s.topic_filter,
                qos: s.qos as u8,
                no_local: false,
                retain_as_published: false,
                retain_handling: 0,
            }).collect(),
        }
    }
}

impl From<mqttv5pb::Unsubscribe> for MqttUnsubscribe {
    fn from(unsubscribe: mqttv5pb::Unsubscribe) -> Self {
        MqttUnsubscribe {
            packet_id: unsubscribe.message_id as u16,
            properties: Vec::new(), // TODO: Convert properties
            topic_filters: unsubscribe.topic_filters,
        }
    }
}

impl From<mqttv5pb::Puback> for MqttPubAck {
    fn from(puback: mqttv5pb::Puback) -> Self {
        MqttPubAck {
            packet_id: puback.message_id as u16,
            reason_code: puback.reason_code as u8,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<mqttv5pb::Pubrec> for MqttPubRec {
    fn from(pubrec: mqttv5pb::Pubrec) -> Self {
        MqttPubRec {
            packet_id: pubrec.message_id as u16,
            reason_code: pubrec.reason_code as u8,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<mqttv5pb::Pubrel> for MqttPubRel {
    fn from(pubrel: mqttv5pb::Pubrel) -> Self {
        MqttPubRel {
            packet_id: pubrel.message_id as u16,
            reason_code: pubrel.reason_code as u8,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<mqttv5pb::Pubcomp> for MqttPubComp {
    fn from(pubcomp: mqttv5pb::Pubcomp) -> Self {
        MqttPubComp {
            packet_id: pubcomp.message_id as u16,
            reason_code: pubcomp.reason_code as u8,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<mqttv5pb::Disconnect> for MqttDisconnect {
    fn from(disconnect: mqttv5pb::Disconnect) -> Self {
        MqttDisconnect {
            reason_code: disconnect.reason_code as u8,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<mqttv5pb::Auth> for MqttAuth {
    fn from(auth: mqttv5pb::Auth) -> Self {
        MqttAuth {
            reason_code: auth.reason_code as u8,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

// Missing reverse implementations for r-proxy compatibility
impl From<mqttv5pb::Connack> for MqttConnAck {
    fn from(connack: mqttv5pb::Connack) -> Self {
        let properties: Vec<Property> = connack.properties.into_iter().filter_map(|p| p.try_into().ok()).collect();
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
            subscriptions: subscribe.subscriptions.into_iter().map(|sub| mqttv5pb::TopicSubscription {
                topic_filter: sub.topic_filter,
                qos: sub.qos as u32,
                no_local: sub.no_local,
                retain_as_published: sub.retain_as_published,
                retain_handling: sub.retain_handling as u32,
            }).collect(),
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<MqttUnsubscribe> for mqttv5pb::Unsubscribe {
    fn from(unsubscribe: MqttUnsubscribe) -> Self {
        mqttv5pb::Unsubscribe {
            message_id: unsubscribe.packet_id as u32,
            topic_filters: unsubscribe.topic_filters,
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<mqttv5pb::Suback> for MqttSubAck {
    fn from(suback: mqttv5pb::Suback) -> Self {
        MqttSubAck {
            packet_id: suback.message_id as u16,
            reason_codes: suback.reason_codes.into_iter().map(|c| c as u8).collect(),
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}

impl From<mqttv5pb::Unsuback> for MqttUnsubAck {
    fn from(unsuback: mqttv5pb::Unsuback) -> Self {
        MqttUnsubAck {
            packet_id: unsuback.message_id as u16,
            reason_codes: unsuback.reason_codes.into_iter().map(|c| c as u8).collect(),
            properties: Vec::new(), // TODO: Convert properties
        }
    }
}
