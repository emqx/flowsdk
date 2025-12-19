use serde::{Deserialize, Serialize};

use crate::mqtt_serde::mqttv5::{
    common::properties::Property, publishv5::MqttPublish, subscribev5::TopicSubscription,
};

/// Fully customizable publish command for MQTT v5
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishCommand {
    pub topic_name: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    pub dup: bool,
    pub packet_id: Option<u16>,
    pub properties: Vec<Property>,
    /// Message priority (0=lowest, 255=highest, default=128)
    pub priority: u8,
}

impl PublishCommand {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        topic_name: String,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
        dup: bool,
        packet_id: Option<u16>,
        properties: Vec<Property>,
        priority: u8,
    ) -> Self {
        Self {
            topic_name,
            payload,
            qos,
            retain,
            dup,
            packet_id,
            properties,
            priority,
        }
    }

    pub fn simple(topic: impl Into<String>, payload: Vec<u8>, qos: u8, retain: bool) -> Self {
        Self::new(
            topic.into(),
            payload,
            qos,
            retain,
            false,
            None,
            Vec::new(),
            128,
        )
    }

    pub fn with_priority(
        topic: impl Into<String>,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
        priority: u8,
    ) -> Self {
        Self::new(
            topic.into(),
            payload,
            qos,
            retain,
            false,
            None,
            Vec::new(),
            priority,
        )
    }

    /// Create a new builder for constructing a PublishCommand
    ///
    /// # Example
    /// ```no_run
    /// use flowsdk::mqtt_client::commands::PublishCommand;
    ///
    /// let cmd = PublishCommand::builder()
    ///     .topic("sensors/temp")
    ///     .payload(b"23.5")
    ///     .qos(1)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn builder() -> PublishCommandBuilder {
        PublishCommandBuilder::new()
    }

    pub fn to_mqtt_publish(&self) -> MqttPublish {
        MqttPublish::new_with_prop(
            self.qos,
            self.topic_name.clone(),
            self.packet_id,
            self.payload.clone(),
            self.retain,
            self.dup,
            self.properties.clone(),
        )
    }

    pub fn to_mqttv3_publish(&self) -> crate::mqtt_serde::mqttv3::publish::MqttPublish {
        crate::mqtt_serde::mqttv3::publish::MqttPublish::new(
            self.topic_name.clone(),
            self.qos,
            self.payload.clone(),
            self.packet_id,
            self.retain,
            self.dup,
        )
    }
}

/// Builder for creating MQTT v5 publish commands
///
/// This builder provides a fluent API for constructing `PublishCommand` instances
/// with full control over MQTT v5 publish options and properties.
#[derive(Debug, Clone)]
pub struct PublishCommandBuilder {
    topic_name: Option<String>,
    payload: Vec<u8>,
    qos: u8,
    retain: bool,
    dup: bool,
    packet_id: Option<u16>,
    properties: Vec<Property>,
    priority: u8,
}

/// Error type for publish builder validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishBuilderError {
    /// Topic name was not provided
    NoTopic,
}

impl std::fmt::Display for PublishBuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTopic => write!(f, "Topic name not provided. Call topic() to set the topic."),
        }
    }
}

impl std::error::Error for PublishBuilderError {}

impl PublishCommandBuilder {
    pub fn new() -> Self {
        Self {
            topic_name: None,
            payload: Vec::new(),
            qos: 0,
            retain: false,
            dup: false,
            packet_id: None,
            properties: Vec::new(),
            priority: 128,
        }
    }

    pub fn topic(mut self, topic: impl Into<String>) -> Self {
        self.topic_name = Some(topic.into());
        self
    }

    pub fn payload(mut self, payload: impl Into<Vec<u8>>) -> Self {
        self.payload = payload.into();
        self
    }

    pub fn qos(mut self, qos: u8) -> Self {
        self.qos = qos;
        self
    }

    pub fn retain(mut self, retain: bool) -> Self {
        self.retain = retain;
        self
    }

    pub fn dup(mut self, dup: bool) -> Self {
        self.dup = dup;
        self
    }

    pub fn with_packet_id(mut self, id: u16) -> Self {
        self.packet_id = Some(id);
        self
    }

    pub fn add_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }

    pub fn with_message_expiry_interval(mut self, seconds: u32) -> Self {
        self.properties
            .push(Property::MessageExpiryInterval(seconds));
        self
    }

    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.properties
            .push(Property::ContentType(content_type.into()));
        self
    }

    pub fn with_response_topic(mut self, topic: impl Into<String>) -> Self {
        self.properties.push(Property::ResponseTopic(topic.into()));
        self
    }

    pub fn with_correlation_data(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.properties.push(Property::CorrelationData(data.into()));
        self
    }

    pub fn with_topic_alias(mut self, alias: u16) -> Self {
        self.properties.push(Property::TopicAlias(alias));
        self
    }

    pub fn with_user_property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties
            .push(Property::UserProperty(key.into(), value.into()));
        self
    }

    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn build(self) -> Result<PublishCommand, PublishBuilderError> {
        let topic_name = self.topic_name.ok_or(PublishBuilderError::NoTopic)?;

        Ok(PublishCommand {
            topic_name,
            payload: self.payload,
            qos: self.qos,
            retain: self.retain,
            dup: self.dup,
            packet_id: self.packet_id,
            properties: self.properties,
            priority: self.priority,
        })
    }
}

impl Default for PublishCommandBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Fully customizable subscribe command for MQTT v5
#[derive(Debug, Clone)]
pub struct SubscribeCommand {
    pub packet_id: Option<u16>,
    pub subscriptions: Vec<TopicSubscription>,
    pub properties: Vec<Property>,
}

impl SubscribeCommand {
    pub fn new(
        packet_id: Option<u16>,
        subscriptions: Vec<TopicSubscription>,
        properties: Vec<Property>,
    ) -> Self {
        Self {
            packet_id,
            subscriptions,
            properties,
        }
    }

    pub fn single(topic: impl Into<String>, qos: u8) -> Self {
        let subscription = TopicSubscription::new(topic.into(), qos, false, false, 0);
        Self::new(None, vec![subscription], Vec::new())
    }

    pub fn builder() -> SubscribeCommandBuilder {
        SubscribeCommandBuilder::new()
    }
}

#[derive(Debug, Clone)]
pub struct SubscribeCommandBuilder {
    topics: Vec<TopicSubscription>,
    properties: Vec<Property>,
    packet_id: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscribeBuilderError {
    NoTopics,
}

impl std::fmt::Display for SubscribeBuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTopics => write!(
                f,
                "No topics added to subscription. Call add_topic() at least once."
            ),
        }
    }
}

impl std::error::Error for SubscribeBuilderError {}

impl SubscribeCommandBuilder {
    pub fn new() -> Self {
        Self {
            topics: Vec::new(),
            properties: Vec::new(),
            packet_id: None,
        }
    }

    pub fn add_topic(mut self, topic: impl Into<String>, qos: u8) -> Self {
        let subscription = TopicSubscription::new_simple(topic.into(), qos);
        self.topics.push(subscription);
        self
    }

    pub fn add_topic_with_options(
        mut self,
        topic: impl Into<String>,
        qos: u8,
        no_local: bool,
        retain_as_published: bool,
        retain_handling: u8,
    ) -> Self {
        let subscription = TopicSubscription::new(
            topic.into(),
            qos,
            no_local,
            retain_as_published,
            retain_handling,
        );
        self.topics.push(subscription);
        self
    }

    pub fn with_no_local(mut self, no_local: bool) -> Self {
        if let Some(last) = self.topics.last_mut() {
            last.no_local = no_local;
        } else {
            panic!("Cannot set no_local: no topics added yet. Call add_topic() first.");
        }
        self
    }

    pub fn with_retain_as_published(mut self, rap: bool) -> Self {
        if let Some(last) = self.topics.last_mut() {
            last.retain_as_published = rap;
        } else {
            panic!("Cannot set retain_as_published: no topics added yet. Call add_topic() first.");
        }
        self
    }

    pub fn with_retain_handling(mut self, rh: u8) -> Self {
        if rh > 2 {
            panic!("Invalid retain_handling value: {}. Must be 0, 1, or 2.", rh);
        }
        if let Some(last) = self.topics.last_mut() {
            last.retain_handling = rh;
        } else {
            panic!("Cannot set retain_handling: no topics added yet. Call add_topic() first.");
        }
        self
    }

    pub fn with_subscription_id(mut self, id: u32) -> Self {
        self.properties.push(Property::SubscriptionIdentifier(id));
        self
    }

    pub fn add_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }

    pub fn with_packet_id(mut self, id: u16) -> Self {
        self.packet_id = Some(id);
        self
    }

    pub fn build(self) -> Result<SubscribeCommand, SubscribeBuilderError> {
        if self.topics.is_empty() {
            return Err(SubscribeBuilderError::NoTopics);
        }

        Ok(SubscribeCommand {
            packet_id: self.packet_id,
            subscriptions: self.topics,
            properties: self.properties,
        })
    }
}

impl Default for SubscribeCommandBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Fully customizable unsubscribe command for MQTT v5
#[derive(Debug, Clone)]
pub struct UnsubscribeCommand {
    pub packet_id: Option<u16>,
    pub topics: Vec<String>,
    pub properties: Vec<Property>,
}

impl UnsubscribeCommand {
    pub fn new(packet_id: Option<u16>, topics: Vec<String>, properties: Vec<Property>) -> Self {
        Self {
            packet_id,
            topics,
            properties,
        }
    }

    pub fn from_topics(topics: Vec<String>) -> Self {
        Self::new(None, topics, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // PublishCommand Builder Tests
    // ============================================================================

    #[test]
    fn test_simple_publish() {
        let cmd = PublishCommand::builder()
            .topic("sensors/temp")
            .payload(b"23.5")
            .qos(1)
            .build()
            .unwrap();

        assert_eq!(cmd.topic_name, "sensors/temp");
        assert_eq!(cmd.payload, b"23.5");
        assert_eq!(cmd.qos, 1);
        assert!(!cmd.retain);
        assert!(!cmd.dup);
        assert!(cmd.packet_id.is_none());
        assert!(cmd.properties.is_empty());
    }

    #[test]
    fn test_publish_with_retain() {
        let cmd = PublishCommand::builder()
            .topic("status/online")
            .payload(b"true")
            .retain(true)
            .build()
            .unwrap();

        assert!(cmd.retain);
        assert_eq!(cmd.qos, 0); // Default QoS
    }

    #[test]
    fn test_publish_with_qos_levels() {
        for qos in 0..=2 {
            let cmd = PublishCommand::builder()
                .topic("test/topic")
                .qos(qos)
                .build()
                .unwrap();

            assert_eq!(cmd.qos, qos);
        }
    }

    #[test]
    fn test_publish_with_packet_id() {
        let cmd = PublishCommand::builder()
            .topic("test/topic")
            .with_packet_id(123)
            .build()
            .unwrap();

        assert_eq!(cmd.packet_id, Some(123));
    }

    #[test]
    fn test_publish_with_dup() {
        let cmd = PublishCommand::builder()
            .topic("test/topic")
            .dup(true)
            .build()
            .unwrap();

        assert!(cmd.dup);
    }

    #[test]
    fn test_publish_with_message_expiry() {
        let cmd = PublishCommand::builder()
            .topic("alerts/temp")
            .payload(b"warning")
            .with_message_expiry_interval(300)
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            &cmd.properties[0],
            Property::MessageExpiryInterval(300)
        ));
    }

    #[test]
    fn test_publish_with_content_type() {
        let cmd = PublishCommand::builder()
            .topic("data/json")
            .payload(br#"{"temp":23.5}"#)
            .with_content_type("application/json")
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            &cmd.properties[0],
            Property::ContentType(ct) if ct == "application/json"
        ));
    }

    #[test]
    fn test_publish_with_response_topic() {
        let cmd = PublishCommand::builder()
            .topic("requests/temp")
            .with_response_topic("responses/temp")
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            &cmd.properties[0],
            Property::ResponseTopic(rt) if rt == "responses/temp"
        ));
    }

    #[test]
    fn test_publish_with_correlation_data() {
        let cmd = PublishCommand::builder()
            .topic("requests/data")
            .with_correlation_data(b"request-123")
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            &cmd.properties[0],
            Property::CorrelationData(data) if data == b"request-123"
        ));
    }

    #[test]
    fn test_publish_with_topic_alias() {
        let cmd = PublishCommand::builder()
            .topic("sensors/temperature/room1")
            .with_topic_alias(42)
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(&cmd.properties[0], Property::TopicAlias(42)));
    }

    #[test]
    fn test_publish_with_user_properties() {
        let cmd = PublishCommand::builder()
            .topic("sensors/temp")
            .with_user_property("sensor_id", "42")
            .with_user_property("location", "room1")
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 2);
        assert!(matches!(
            &cmd.properties[0],
            Property::UserProperty(k, v) if k == "sensor_id" && v == "42"
        ));
        assert!(matches!(
            &cmd.properties[1],
            Property::UserProperty(k, v) if k == "location" && v == "room1"
        ));
    }

    #[test]
    fn test_publish_with_custom_property() {
        let cmd = PublishCommand::builder()
            .topic("test/topic")
            .add_property(Property::UserProperty("key".into(), "value".into()))
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            &cmd.properties[0],
            Property::UserProperty(k, v) if k == "key" && v == "value"
        ));
    }

    #[test]
    fn test_publish_with_multiple_properties() {
        let cmd = PublishCommand::builder()
            .topic("data/sensor")
            .payload(br#"{"temp":23.5}"#)
            .with_content_type("application/json")
            .with_message_expiry_interval(3600)
            .with_user_property("sensor_id", "42")
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 3);
    }

    #[test]
    fn test_publish_complex_command() {
        let cmd = PublishCommand::builder()
            .topic("sensors/temperature/room1")
            .payload(b"23.5")
            .qos(2)
            .retain(true)
            .with_packet_id(456)
            .with_topic_alias(10)
            .with_content_type("text/plain")
            .with_message_expiry_interval(7200)
            .with_user_property("location", "building-A")
            .build()
            .unwrap();

        assert_eq!(cmd.topic_name, "sensors/temperature/room1");
        assert_eq!(cmd.payload, b"23.5");
        assert_eq!(cmd.qos, 2);
        assert!(cmd.retain);
        assert_eq!(cmd.packet_id, Some(456));
        assert_eq!(cmd.properties.len(), 4);
    }

    #[test]
    fn test_publish_no_topic_error() {
        let result = PublishCommand::builder().payload(b"test").build();

        assert!(result.is_err());
        assert!(matches!(result, Err(PublishBuilderError::NoTopic)));

        // Test error message
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Topic name not provided"));
    }

    #[test]
    fn test_publish_empty_payload() {
        let cmd = PublishCommand::builder()
            .topic("test/topic")
            .build()
            .unwrap();

        assert!(cmd.payload.is_empty());
    }

    #[test]
    fn test_publish_payload_into_conversion() {
        // Test with &[u8]
        let cmd1 = PublishCommand::builder()
            .topic("test/topic")
            .payload(b"test" as &[u8])
            .build()
            .unwrap();
        assert_eq!(cmd1.payload, b"test");

        // Test with Vec<u8>
        let cmd2 = PublishCommand::builder()
            .topic("test/topic")
            .payload(vec![1, 2, 3, 4])
            .build()
            .unwrap();
        assert_eq!(cmd2.payload, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_publish_topic_into_conversion() {
        // Test with &str
        let cmd1 = PublishCommand::builder()
            .topic("test/topic")
            .build()
            .unwrap();
        assert_eq!(cmd1.topic_name, "test/topic");

        // Test with String
        let cmd2 = PublishCommand::builder()
            .topic(String::from("test/topic2"))
            .build()
            .unwrap();
        assert_eq!(cmd2.topic_name, "test/topic2");
    }

    #[test]
    fn test_builder_default() {
        let builder1 = PublishCommandBuilder::default();
        let builder2 = PublishCommandBuilder::new();

        // Both should have the same initial state
        assert_eq!(builder1.topic_name, builder2.topic_name);
        assert_eq!(builder1.qos, builder2.qos);
        assert_eq!(builder1.retain, builder2.retain);
        assert_eq!(builder1.dup, builder2.dup);
    }

    #[test]
    fn test_builder_is_clone() {
        let builder = PublishCommand::builder()
            .topic("test/topic")
            .payload(b"test")
            .qos(1);

        let builder_clone = builder.clone();
        let cmd1 = builder.build().unwrap();
        let cmd2 = builder_clone.build().unwrap();

        assert_eq!(cmd1.topic_name, cmd2.topic_name);
        assert_eq!(cmd1.payload, cmd2.payload);
        assert_eq!(cmd1.qos, cmd2.qos);
    }

    #[test]
    fn test_request_response_pattern() {
        let cmd = PublishCommand::builder()
            .topic("requests/get_temperature")
            .payload(b"room1")
            .qos(1)
            .with_response_topic("responses/temperature")
            .with_correlation_data(b"req-12345")
            .with_user_property("request_id", "12345")
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 3);
        // Verify all properties are present
        assert!(cmd
            .properties
            .iter()
            .any(|p| matches!(p, Property::ResponseTopic(_))));
        assert!(cmd
            .properties
            .iter()
            .any(|p| matches!(p, Property::CorrelationData(_))));
        assert!(cmd
            .properties
            .iter()
            .any(|p| matches!(p, Property::UserProperty(_, _))));
    }

    // ============================================================================
    // SubscribeCommand Builder Tests
    // ============================================================================

    #[test]
    fn test_simple_subscription() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temp", 1)
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions.len(), 1);
        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temp");
        assert_eq!(cmd.subscriptions[0].qos, 1);
        assert!(!cmd.subscriptions[0].no_local);
        assert!(!cmd.subscriptions[0].retain_as_published);
        assert_eq!(cmd.subscriptions[0].retain_handling, 0);
        assert!(cmd.packet_id.is_none());
        assert!(cmd.properties.is_empty());
    }

    #[test]
    fn test_subscription_with_no_local() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/+/temp", 1)
            .with_no_local(true)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/+/temp");
        assert_eq!(sub.qos, 1);
        assert!(sub.no_local);
        assert!(!sub.retain_as_published);
        assert_eq!(sub.retain_handling, 0);
    }

    #[test]
    fn test_subscription_with_retain_as_published() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/#", 2)
            .with_retain_as_published(true)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/#");
        assert_eq!(sub.qos, 2);
        assert!(!sub.no_local);
        assert!(sub.retain_as_published);
        assert_eq!(sub.retain_handling, 0);
    }

    #[test]
    fn test_subscription_with_retain_handling() {
        // Test all valid retain_handling values
        for rh in 0..=2 {
            let cmd = SubscribeCommand::builder()
                .add_topic("test/topic", 1)
                .with_retain_handling(rh)
                .build()
                .unwrap();

            assert_eq!(cmd.subscriptions[0].retain_handling, rh);
        }
    }

    #[test]
    fn test_subscription_with_all_options() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/+/temp", 2)
            .with_no_local(true)
            .with_retain_as_published(true)
            .with_retain_handling(1)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/+/temp");
        assert_eq!(sub.qos, 2);
        assert!(sub.no_local);
        assert!(sub.retain_as_published);
        assert_eq!(sub.retain_handling, 1);
    }

    #[test]
    fn test_multiple_topics() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temp", 1)
            .with_no_local(true)
            .add_topic("sensors/humidity", 2)
            .with_retain_handling(1)
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions.len(), 2);

        // First subscription
        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temp");
        assert_eq!(cmd.subscriptions[0].qos, 1);
        assert!(cmd.subscriptions[0].no_local);
        assert!(!cmd.subscriptions[0].retain_as_published);
        assert_eq!(cmd.subscriptions[0].retain_handling, 0);

        // Second subscription
        assert_eq!(cmd.subscriptions[1].topic_filter, "sensors/humidity");
        assert_eq!(cmd.subscriptions[1].qos, 2);
        assert!(!cmd.subscriptions[1].no_local);
        assert!(!cmd.subscriptions[1].retain_as_published);
        assert_eq!(cmd.subscriptions[1].retain_handling, 1);
    }

    #[test]
    fn test_add_topic_with_options() {
        let cmd = SubscribeCommand::builder()
            .add_topic_with_options("sensors/temp", 2, true, true, 1)
            .build()
            .unwrap();

        let sub = &cmd.subscriptions[0];
        assert_eq!(sub.topic_filter, "sensors/temp");
        assert_eq!(sub.qos, 2);
        assert!(sub.no_local);
        assert!(sub.retain_as_published);
        assert_eq!(sub.retain_handling, 1);
    }

    #[test]
    fn test_with_subscription_id() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/#", 1)
            .with_subscription_id(42)
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            cmd.properties[0],
            Property::SubscriptionIdentifier(42)
        ));
    }

    #[test]
    fn test_add_property() {
        let cmd = SubscribeCommand::builder()
            .add_topic("test/topic", 1)
            .add_property(Property::UserProperty("key".into(), "value".into()))
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 1);
        assert!(matches!(
            &cmd.properties[0],
            Property::UserProperty(k, v) if k == "key" && v == "value"
        ));
    }

    #[test]
    fn test_multiple_properties() {
        let cmd = SubscribeCommand::builder()
            .add_topic("test/topic", 1)
            .with_subscription_id(100)
            .add_property(Property::UserProperty("key1".into(), "value1".into()))
            .add_property(Property::UserProperty("key2".into(), "value2".into()))
            .build()
            .unwrap();

        assert_eq!(cmd.properties.len(), 3);
        assert!(matches!(
            cmd.properties[0],
            Property::SubscriptionIdentifier(100)
        ));
    }

    #[test]
    fn test_with_packet_id() {
        let cmd = SubscribeCommand::builder()
            .add_topic("test/topic", 1)
            .with_packet_id(123)
            .build()
            .unwrap();

        assert_eq!(cmd.packet_id, Some(123));
    }

    #[test]
    fn test_no_topics_error() {
        let result = SubscribeCommand::builder().build();

        assert!(result.is_err());
        assert!(matches!(result, Err(SubscribeBuilderError::NoTopics)));

        // Test error message
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No topics added"));
    }

    #[test]
    #[should_panic(expected = "no topics added yet")]
    fn test_with_no_local_before_topic_panics() {
        SubscribeCommand::builder().with_no_local(true);
    }

    #[test]
    #[should_panic(expected = "no topics added yet")]
    fn test_with_retain_as_published_before_topic_panics() {
        SubscribeCommand::builder().with_retain_as_published(true);
    }

    #[test]
    #[should_panic(expected = "no topics added yet")]
    fn test_with_retain_handling_before_topic_panics() {
        SubscribeCommand::builder().with_retain_handling(1);
    }

    #[test]
    #[should_panic(expected = "Invalid retain_handling value")]
    fn test_invalid_retain_handling_value() {
        SubscribeCommand::builder()
            .add_topic("test", 1)
            .with_retain_handling(3); // Invalid: max is 2
    }

    #[test]
    fn test_subscribe_builder_default() {
        let builder1 = SubscribeCommandBuilder::default();
        let builder2 = SubscribeCommandBuilder::new();

        // Both should have the same initial state
        assert_eq!(builder1.topics.len(), builder2.topics.len());
        assert_eq!(builder1.properties.len(), builder2.properties.len());
        assert_eq!(builder1.packet_id, builder2.packet_id);
    }

    #[test]
    fn test_string_ownership() {
        let topic = String::from("sensors/temp");
        let cmd = SubscribeCommand::builder()
            .add_topic(topic.clone(), 1) // Clone to test Into<String>
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions[0].topic_filter, topic);
    }

    #[test]
    fn test_str_slice() {
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temp", 1) // &str
            .build()
            .unwrap();

        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temp");
    }

    #[test]
    fn test_complex_subscription() {
        // Test a realistic complex subscription scenario
        let cmd = SubscribeCommand::builder()
            .add_topic("sensors/temperature/#", 1)
            .with_no_local(false)
            .with_retain_handling(0)
            .add_topic("sensors/humidity/+/data", 2)
            .with_no_local(true)
            .with_retain_as_published(true)
            .with_retain_handling(2)
            .add_topic("alerts/#", 1)
            .with_retain_handling(1)
            .with_subscription_id(999)
            .add_property(Property::UserProperty("client".into(), "test".into()))
            .with_packet_id(42)
            .build()
            .unwrap();

        // Verify structure
        assert_eq!(cmd.subscriptions.len(), 3);
        assert_eq!(cmd.packet_id, Some(42));
        assert_eq!(cmd.properties.len(), 2);

        // Verify first topic
        assert_eq!(cmd.subscriptions[0].topic_filter, "sensors/temperature/#");
        assert_eq!(cmd.subscriptions[0].qos, 1);
        assert!(!cmd.subscriptions[0].no_local);
        assert_eq!(cmd.subscriptions[0].retain_handling, 0);

        // Verify second topic
        assert_eq!(cmd.subscriptions[1].topic_filter, "sensors/humidity/+/data");
        assert_eq!(cmd.subscriptions[1].qos, 2);
        assert!(cmd.subscriptions[1].no_local);
        assert!(cmd.subscriptions[1].retain_as_published);
        assert_eq!(cmd.subscriptions[1].retain_handling, 2);

        // Verify third topic
        assert_eq!(cmd.subscriptions[2].topic_filter, "alerts/#");
        assert_eq!(cmd.subscriptions[2].qos, 1);
        assert_eq!(cmd.subscriptions[2].retain_handling, 1);

        // Verify properties
        assert!(matches!(
            cmd.properties[0],
            Property::SubscriptionIdentifier(999)
        ));
        assert!(matches!(
            &cmd.properties[1],
            Property::UserProperty(k, v) if k == "client" && v == "test"
        ));
    }

    #[test]
    fn test_subscribe_builder_is_clone() {
        let builder = SubscribeCommand::builder()
            .add_topic("test", 1)
            .with_subscription_id(42);

        let builder_clone = builder.clone();

        let cmd1 = builder.build().unwrap();
        let cmd2 = builder_clone.build().unwrap();

        assert_eq!(cmd1.subscriptions.len(), cmd2.subscriptions.len());
        assert_eq!(cmd1.properties.len(), cmd2.properties.len());
    }
}
