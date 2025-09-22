//! # MQTT Protocol Buffer Conversion Library
//!
//! This library provides bidirectional conversion between MQTT packets and Protocol Buffer
//! messages for both MQTT v3.1.1 and MQTT v5.0 protocols.
//!
//! ## Features
//!
//! - **Bidirectional Conversions**: Convert between MQTT packets and protobuf messages in both directions
//! - **Multi-Version Support**: Supports both MQTT v3.1.1 and MQTT v5.0 protocols
//! - **Stream Payload Handling**: Unified interface for handling different MQTT versions in streaming contexts
//! - **Version Detection**: Automatic detection of MQTT protocol version from packets
//! - **Byte Serialization**: Direct conversion from protobuf messages to MQTT byte streams
//!
//! ## Examples
//!
//! ### Converting MQTT v3 packets to protobuf:
//! ```rust
//! use mqtt_grpc_duality::*;
//! use flowsdk::mqtt_serde::mqttv3::connect::MqttConnect;
//!
//! let mqtt_connect = MqttConnect::new("client_id".to_string(), 60, true);
//! let pb_connect: mqttv3pb::Connect = mqtt_connect.into();
//! ```
//!
//! ### Converting protobuf back to MQTT v3:
//! ```rust
//! # use mqtt_grpc_duality::*;
//! let pb_connect = mqttv3pb::Connect::default();
//! let mqtt_connect: flowsdk::mqtt_serde::mqttv3::connect::MqttConnect = pb_connect.into();
//! ```

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

// MQTT v3 imports
use flowsdk::mqtt_serde::mqttv3::connack::MqttConnAck as MqttConnAckV3;
use flowsdk::mqtt_serde::mqttv3::connect::MqttConnect as MqttConnectV3;
use flowsdk::mqtt_serde::mqttv3::disconnect::MqttDisconnect as MqttDisconnectV3;
use flowsdk::mqtt_serde::mqttv3::puback::MqttPubAck as MqttPubAckV3;
use flowsdk::mqtt_serde::mqttv3::pubcomp::MqttPubComp as MqttPubCompV3;
use flowsdk::mqtt_serde::mqttv3::publish::MqttPublish as MqttPublishV3;
use flowsdk::mqtt_serde::mqttv3::pubrec::MqttPubRec as MqttPubRecV3;
use flowsdk::mqtt_serde::mqttv3::pubrel::MqttPubRel as MqttPubRelV3;
use flowsdk::mqtt_serde::mqttv3::suback::MqttSubAck as MqttSubAckV3;
use flowsdk::mqtt_serde::mqttv3::subscribe::MqttSubscribe as MqttSubscribeV3;
use flowsdk::mqtt_serde::mqttv3::unsuback::MqttUnsubAck as MqttUnsubAckV3;
use flowsdk::mqtt_serde::mqttv3::unsubscribe::MqttUnsubscribe as MqttUnsubscribeV3;

// Generate protobuf types at compile time
pub mod mqttv5pb {
    tonic::include_proto!("mqttv5");
}

pub mod mqttv3pb {
    tonic::include_proto!("mqttv3");
}

pub mod mqtt_unified_pb {
    tonic::include_proto!("mqtt");
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

// MQTT v3 From trait implementations
impl From<MqttConnectV3> for mqttv3pb::Connect {
    fn from(connect: MqttConnectV3) -> Self {
        mqttv3pb::Connect {
            client_id: connect.client_id.clone(),
            protocol_name: "MQTT".to_string(), // Default for v3.1.1
            protocol_version: 4,               // MQTT v3.1.1 uses protocol version 4
            clean_session: connect.clean_session,
            keep_alive: connect.keep_alive as u32,
            username: connect.username.unwrap_or_default(),
            password: connect.password.unwrap_or_default(),
            will: connect.will.map(|will| mqttv3pb::Will {
                qos: will.qos as i32,
                retain: will.retain,
                topic: will.topic,
                payload: will.message,
            }),
        }
    }
}

impl From<MqttConnAckV3> for mqttv3pb::Connack {
    fn from(connack: MqttConnAckV3) -> Self {
        mqttv3pb::Connack {
            session_present: connack.session_present,
            return_code: connack.return_code as u32,
        }
    }
}

impl From<MqttPublishV3> for mqttv3pb::Publish {
    fn from(publish: MqttPublishV3) -> Self {
        mqttv3pb::Publish {
            topic: publish.topic_name,
            payload: publish.payload,
            qos: publish.qos as i32,
            retain: publish.retain,
            dup: publish.dup,
            message_id: publish.message_id.unwrap_or(0) as u32,
        }
    }
}

impl From<MqttSubscribeV3> for mqttv3pb::Subscribe {
    fn from(subscribe: MqttSubscribeV3) -> Self {
        mqttv3pb::Subscribe {
            message_id: subscribe.message_id as u32,
            subscriptions: subscribe
                .subscriptions
                .into_iter()
                .map(|sub| mqttv3pb::TopicSubscription {
                    topic_filter: sub.topic_filter,
                    qos: sub.qos as u32,
                })
                .collect(),
        }
    }
}

impl From<MqttSubAckV3> for mqttv3pb::Suback {
    fn from(suback: MqttSubAckV3) -> Self {
        mqttv3pb::Suback {
            message_id: suback.message_id as u32,
            return_codes: suback.return_codes.into_iter().map(|c| c as u32).collect(),
        }
    }
}

impl From<MqttUnsubscribeV3> for mqttv3pb::Unsubscribe {
    fn from(unsubscribe: MqttUnsubscribeV3) -> Self {
        mqttv3pb::Unsubscribe {
            message_id: unsubscribe.message_id as u32,
            topic_filters: unsubscribe.topic_filters,
        }
    }
}

impl From<MqttUnsubAckV3> for mqttv3pb::Unsuback {
    fn from(unsuback: MqttUnsubAckV3) -> Self {
        mqttv3pb::Unsuback {
            message_id: unsuback.message_id as u32,
        }
    }
}

impl From<MqttPubAckV3> for mqttv3pb::Puback {
    fn from(puback: MqttPubAckV3) -> Self {
        mqttv3pb::Puback {
            message_id: puback.message_id as u32,
        }
    }
}

impl From<MqttPubRecV3> for mqttv3pb::Pubrec {
    fn from(pubrec: MqttPubRecV3) -> Self {
        mqttv3pb::Pubrec {
            message_id: pubrec.message_id as u32,
        }
    }
}

impl From<MqttPubRelV3> for mqttv3pb::Pubrel {
    fn from(pubrel: MqttPubRelV3) -> Self {
        mqttv3pb::Pubrel {
            message_id: pubrel.message_id as u32,
        }
    }
}

impl From<MqttPubCompV3> for mqttv3pb::Pubcomp {
    fn from(pubcomp: MqttPubCompV3) -> Self {
        mqttv3pb::Pubcomp {
            message_id: pubcomp.message_id as u32,
        }
    }
}

impl From<MqttDisconnectV3> for mqttv3pb::Disconnect {
    fn from(_disconnect: MqttDisconnectV3) -> Self {
        // MQTT v3 DISCONNECT is an empty packet
        mqttv3pb::Disconnect {}
    }
}

// MQTT v3 Reverse conversions: protobuf → MQTT v3
impl From<mqttv3pb::Connect> for MqttConnectV3 {
    fn from(connect: mqttv3pb::Connect) -> Self {
        use flowsdk::mqtt_serde::mqttv3::connect::Will;

        let will = connect.will.map(|w| Will {
            qos: w.qos as u8,
            retain: w.retain,
            topic: w.topic,
            message: w.payload,
        });

        MqttConnectV3 {
            client_id: connect.client_id,
            clean_session: connect.clean_session,
            keep_alive: connect.keep_alive as u16,
            username: if connect.username.is_empty() {
                None
            } else {
                Some(connect.username)
            },
            password: if connect.password.is_empty() {
                None
            } else {
                Some(connect.password)
            },
            will,
        }
    }
}

impl From<mqttv3pb::Connack> for MqttConnAckV3 {
    fn from(connack: mqttv3pb::Connack) -> Self {
        MqttConnAckV3 {
            session_present: connack.session_present,
            return_code: connack.return_code as u8,
        }
    }
}

impl From<mqttv3pb::Publish> for MqttPublishV3 {
    fn from(publish: mqttv3pb::Publish) -> Self {
        MqttPublishV3 {
            topic_name: publish.topic,
            payload: publish.payload,
            qos: publish.qos as u8,
            retain: publish.retain,
            dup: publish.dup,
            message_id: if publish.message_id == 0 {
                None
            } else {
                Some(publish.message_id as u16)
            },
        }
    }
}

impl From<mqttv3pb::Subscribe> for MqttSubscribeV3 {
    fn from(subscribe: mqttv3pb::Subscribe) -> Self {
        use flowsdk::mqtt_serde::mqttv3::subscribe::SubscriptionTopic;

        let subscriptions = subscribe
            .subscriptions
            .into_iter()
            .map(|sub| SubscriptionTopic {
                topic_filter: sub.topic_filter,
                qos: sub.qos as u8,
            })
            .collect();

        MqttSubscribeV3 {
            message_id: subscribe.message_id as u16,
            subscriptions,
        }
    }
}

impl From<mqttv3pb::Suback> for MqttSubAckV3 {
    fn from(suback: mqttv3pb::Suback) -> Self {
        MqttSubAckV3 {
            message_id: suback.message_id as u16,
            return_codes: suback.return_codes.into_iter().map(|c| c as u8).collect(),
        }
    }
}

impl From<mqttv3pb::Unsubscribe> for MqttUnsubscribeV3 {
    fn from(unsubscribe: mqttv3pb::Unsubscribe) -> Self {
        MqttUnsubscribeV3 {
            message_id: unsubscribe.message_id as u16,
            topic_filters: unsubscribe.topic_filters,
        }
    }
}

impl From<mqttv3pb::Unsuback> for MqttUnsubAckV3 {
    fn from(unsuback: mqttv3pb::Unsuback) -> Self {
        MqttUnsubAckV3 {
            message_id: unsuback.message_id as u16,
        }
    }
}

impl From<mqttv3pb::Puback> for MqttPubAckV3 {
    fn from(puback: mqttv3pb::Puback) -> Self {
        MqttPubAckV3 {
            message_id: puback.message_id as u16,
        }
    }
}

impl From<mqttv3pb::Pubrec> for MqttPubRecV3 {
    fn from(pubrec: mqttv3pb::Pubrec) -> Self {
        MqttPubRecV3 {
            message_id: pubrec.message_id as u16,
        }
    }
}

impl From<mqttv3pb::Pubrel> for MqttPubRelV3 {
    fn from(pubrel: mqttv3pb::Pubrel) -> Self {
        MqttPubRelV3 {
            message_id: pubrel.message_id as u16,
        }
    }
}

impl From<mqttv3pb::Pubcomp> for MqttPubCompV3 {
    fn from(pubcomp: mqttv3pb::Pubcomp) -> Self {
        MqttPubCompV3 {
            message_id: pubcomp.message_id as u16,
        }
    }
}

impl From<mqttv3pb::Disconnect> for MqttDisconnectV3 {
    fn from(_disconnect: mqttv3pb::Disconnect) -> Self {
        // MQTT v3 DISCONNECT is an empty packet
        MqttDisconnectV3::new()
    }
}

// New unified stream message logic

pub fn convert_mqtt_to_stream_message(
    packet: &MqttPacket,
    session_id: String,
    sequence_id: u64,
    direction: mqtt_unified_pb::MessageDirection,
) -> Option<mqtt_unified_pb::MqttStreamMessage> {
    let payload = match packet {
        MqttPacket::Connect3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Connect(p.clone().into())),
            }),
        ),
        MqttPacket::ConnAck3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Connack(p.clone().into())),
            }),
        ),
        MqttPacket::Publish3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Publish(p.clone().into())),
            }),
        ),
        MqttPacket::PubAck3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Puback(p.clone().into())),
            }),
        ),
        MqttPacket::PubRec3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Pubrec(p.clone().into())),
            }),
        ),
        MqttPacket::PubRel3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Pubrel(p.clone().into())),
            }),
        ),
        MqttPacket::PubComp3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Pubcomp(p.clone().into())),
            }),
        ),
        MqttPacket::Subscribe3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Subscribe(p.clone().into())),
            }),
        ),
        MqttPacket::SubAck3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Suback(p.clone().into())),
            }),
        ),
        MqttPacket::Unsubscribe3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Unsubscribe(p.clone().into())),
            }),
        ),
        MqttPacket::UnsubAck3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Unsuback(p.clone().into())),
            }),
        ),
        MqttPacket::PingReq3(_) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Pingreq(mqttv3pb::Pingreq {})),
            }),
        ),
        MqttPacket::PingResp3(_) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Pingresp(
                    mqttv3pb::Pingresp {},
                )),
            }),
        ),
        MqttPacket::Disconnect3(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(mqttv3pb::MqttPacket {
                packet: Some(mqttv3pb::mqtt_packet::Packet::Disconnect(p.clone().into())),
            }),
        ),

        MqttPacket::Connect5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Connect(p.clone().into())),
            }),
        ),
        MqttPacket::ConnAck5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Connack(p.clone().into())),
            }),
        ),
        MqttPacket::Publish5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Publish(p.clone().into())),
            }),
        ),
        MqttPacket::PubAck5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Puback(p.clone().into())),
            }),
        ),
        MqttPacket::PubRec5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Pubrec(p.clone().into())),
            }),
        ),
        MqttPacket::PubRel5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Pubrel(p.clone().into())),
            }),
        ),
        MqttPacket::PubComp5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Pubcomp(p.clone().into())),
            }),
        ),
        MqttPacket::Subscribe5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Subscribe(p.clone().into())),
            }),
        ),
        MqttPacket::SubAck5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Suback(p.clone().into())),
            }),
        ),
        MqttPacket::Unsubscribe5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Unsubscribe(p.clone().into())),
            }),
        ),
        MqttPacket::UnsubAck5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Unsuback(p.clone().into())),
            }),
        ),
        MqttPacket::PingReq5(_) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Pingreq(mqttv5pb::Pingreq {})),
            }),
        ),
        MqttPacket::PingResp5(_) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Pingresp(
                    mqttv5pb::Pingresp {},
                )),
            }),
        ),
        MqttPacket::Disconnect5(p) => Some(
            mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Disconnect(p.clone().into())),
            }),
        ),
        MqttPacket::Auth(p) => Some(mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(
            mqttv5pb::MqttPacket {
                packet: Some(mqttv5pb::mqtt_packet::Packet::Auth(p.clone().into())),
            },
        )),
    };

    payload.map(|p| mqtt_unified_pb::MqttStreamMessage {
        session_id,
        sequence_id,
        direction: direction as i32,
        payload: Some(p),
    })
}

pub fn convert_stream_message_to_mqtt_packet(
    msg: mqtt_unified_pb::MqttStreamMessage,
) -> Option<MqttPacket> {
    match msg.payload {
        Some(mqtt_unified_pb::mqtt_stream_message::Payload::MqttV3Packet(v3_packet)) => {
            v3_packet.packet.map(|p| p.into())
        }
        Some(mqtt_unified_pb::mqtt_stream_message::Payload::MqttV5Packet(v5_packet)) => {
            v5_packet.packet.map(|p| p.into())
        }
        _ => None,
    }
}

impl From<mqttv3pb::mqtt_packet::Packet> for MqttPacket {
    fn from(packet: mqttv3pb::mqtt_packet::Packet) -> Self {
        match packet {
            mqttv3pb::mqtt_packet::Packet::Connect(p) => MqttPacket::Connect3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Connack(p) => MqttPacket::ConnAck3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Publish(p) => MqttPacket::Publish3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Puback(p) => MqttPacket::PubAck3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Pubrec(p) => MqttPacket::PubRec3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Pubrel(p) => MqttPacket::PubRel3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Pubcomp(p) => MqttPacket::PubComp3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Subscribe(p) => MqttPacket::Subscribe3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Suback(p) => MqttPacket::SubAck3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Unsubscribe(p) => MqttPacket::Unsubscribe3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Unsuback(p) => MqttPacket::UnsubAck3(p.into()),
            mqttv3pb::mqtt_packet::Packet::Pingreq(_) => {
                MqttPacket::PingReq3(flowsdk::mqtt_serde::mqttv3::pingreq::MqttPingReq {})
            }
            mqttv3pb::mqtt_packet::Packet::Pingresp(_) => {
                MqttPacket::PingResp3(flowsdk::mqtt_serde::mqttv3::pingresp::MqttPingResp {})
            }
            mqttv3pb::mqtt_packet::Packet::Disconnect(p) => MqttPacket::Disconnect3(p.into()),
        }
    }
}

impl From<mqttv5pb::mqtt_packet::Packet> for MqttPacket {
    fn from(packet: mqttv5pb::mqtt_packet::Packet) -> Self {
        match packet {
            mqttv5pb::mqtt_packet::Packet::Connect(p) => MqttPacket::Connect5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Connack(p) => MqttPacket::ConnAck5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Publish(p) => MqttPacket::Publish5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Puback(p) => MqttPacket::PubAck5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Pubrec(p) => MqttPacket::PubRec5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Pubrel(p) => MqttPacket::PubRel5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Pubcomp(p) => MqttPacket::PubComp5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Subscribe(p) => MqttPacket::Subscribe5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Suback(p) => MqttPacket::SubAck5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Unsubscribe(p) => MqttPacket::Unsubscribe5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Unsuback(p) => MqttPacket::UnsubAck5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Pingreq(_) => {
                MqttPacket::PingReq5(flowsdk::mqtt_serde::mqttv5::pingreq::MqttPingReq::new())
            }
            mqttv5pb::mqtt_packet::Packet::Pingresp(_) => {
                MqttPacket::PingResp5(flowsdk::mqtt_serde::mqttv5::pingresp::MqttPingResp::new())
            }
            mqttv5pb::mqtt_packet::Packet::Disconnect(p) => MqttPacket::Disconnect5(p.into()),
            mqttv5pb::mqtt_packet::Packet::Auth(p) => MqttPacket::Auth(p.into()),
        }
    }
}

#[derive(Debug)]
pub enum MqttConnectPacket {
    V3(MqttConnectV3),
    V5(MqttConnect),
}

impl MqttConnectPacket {
    pub fn client_id(&self) -> &str {
        match self {
            MqttConnectPacket::V3(p) => &p.client_id,
            MqttConnectPacket::V5(p) => &p.client_id,
        }
    }

    pub fn version(&self) -> u8 {
        match self {
            MqttConnectPacket::V3(_) => 3,
            MqttConnectPacket::V5(_) => 5,
        }
    }
}
