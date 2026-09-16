// SPDX-License-Identifier: MPL-2.0

use super::ffi_types::MqttErrorFFI;
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "uniffi-bindings", derive(uniffi::Enum, serde::Serialize))]
pub enum MqttPropertyFFI {
    PayloadFormatIndicator { value: u8 },
    MessageExpiryInterval { value: u32 },
    ContentType { value: String },
    ResponseTopic { value: String },
    CorrelationData { value: Vec<u8> },
    SubscriptionIdentifier { value: u32 },
    SessionExpiryInterval { value: u32 },
    AssignedClientIdentifier { value: String },
    ServerKeepAlive { value: u16 },
    AuthenticationMethod { value: String },
    AuthenticationData { value: Vec<u8> },
    RequestProblemInformation { value: u8 },
    WillDelayInterval { value: u32 },
    RequestResponseInformation { value: u8 },
    ResponseInformation { value: String },
    ServerReference { value: String },
    ReasonString { value: String },
    ReceiveMaximum { value: u16 },
    TopicAliasMaximum { value: u16 },
    TopicAlias { value: u16 },
    MaximumQoS { value: u8 },
    RetainAvailable { value: u8 },
    UserProperty { key: String, value: String },
    MaximumPacketSize { value: u32 },
    WildcardSubscriptionAvailable { value: u8 },
    SubscriptionIdentifierAvailable { value: u8 },
    SharedSubscriptionAvailable { value: u8 },
}

impl From<MqttPropertyFFI> for Property {
    fn from(property: MqttPropertyFFI) -> Self {
        match property {
            MqttPropertyFFI::PayloadFormatIndicator { value } => {
                Self::PayloadFormatIndicator(value)
            }
            MqttPropertyFFI::MessageExpiryInterval { value } => Self::MessageExpiryInterval(value),
            MqttPropertyFFI::ContentType { value } => Self::ContentType(value),
            MqttPropertyFFI::ResponseTopic { value } => Self::ResponseTopic(value),
            MqttPropertyFFI::CorrelationData { value } => Self::CorrelationData(value),
            MqttPropertyFFI::SubscriptionIdentifier { value } => {
                Self::SubscriptionIdentifier(value)
            }
            MqttPropertyFFI::SessionExpiryInterval { value } => Self::SessionExpiryInterval(value),
            MqttPropertyFFI::AssignedClientIdentifier { value } => {
                Self::AssignedClientIdentifier(value)
            }
            MqttPropertyFFI::ServerKeepAlive { value } => Self::ServerKeepAlive(value),
            MqttPropertyFFI::AuthenticationMethod { value } => Self::AuthenticationMethod(value),
            MqttPropertyFFI::AuthenticationData { value } => Self::AuthenticationData(value),
            MqttPropertyFFI::RequestProblemInformation { value } => {
                Self::RequestProblemInformation(value)
            }
            MqttPropertyFFI::WillDelayInterval { value } => Self::WillDelayInterval(value),
            MqttPropertyFFI::RequestResponseInformation { value } => {
                Self::RequestResponseInformation(value)
            }
            MqttPropertyFFI::ResponseInformation { value } => Self::ResponseInformation(value),
            MqttPropertyFFI::ServerReference { value } => Self::ServerReference(value),
            MqttPropertyFFI::ReasonString { value } => Self::ReasonString(value),
            MqttPropertyFFI::ReceiveMaximum { value } => Self::ReceiveMaximum(value),
            MqttPropertyFFI::TopicAliasMaximum { value } => Self::TopicAliasMaximum(value),
            MqttPropertyFFI::TopicAlias { value } => Self::TopicAlias(value),
            MqttPropertyFFI::MaximumQoS { value } => Self::MaximumQoS(value),
            MqttPropertyFFI::RetainAvailable { value } => Self::RetainAvailable(value),
            MqttPropertyFFI::UserProperty { key, value } => Self::UserProperty(key, value),
            MqttPropertyFFI::MaximumPacketSize { value } => Self::MaximumPacketSize(value),
            MqttPropertyFFI::WildcardSubscriptionAvailable { value } => {
                Self::WildcardSubscriptionAvailable(value)
            }
            MqttPropertyFFI::SubscriptionIdentifierAvailable { value } => {
                Self::SubscriptionIdentifierAvailable(value)
            }
            MqttPropertyFFI::SharedSubscriptionAvailable { value } => {
                Self::SharedSubscriptionAvailable(value)
            }
        }
    }
}

impl From<Property> for MqttPropertyFFI {
    fn from(property: Property) -> Self {
        match property {
            Property::PayloadFormatIndicator(value) => Self::PayloadFormatIndicator { value },
            Property::MessageExpiryInterval(value) => Self::MessageExpiryInterval { value },
            Property::ContentType(value) => Self::ContentType { value },
            Property::ResponseTopic(value) => Self::ResponseTopic { value },
            Property::CorrelationData(value) => Self::CorrelationData { value },
            Property::SubscriptionIdentifier(value) => Self::SubscriptionIdentifier { value },
            Property::SessionExpiryInterval(value) => Self::SessionExpiryInterval { value },
            Property::AssignedClientIdentifier(value) => Self::AssignedClientIdentifier { value },
            Property::ServerKeepAlive(value) => Self::ServerKeepAlive { value },
            Property::AuthenticationMethod(value) => Self::AuthenticationMethod { value },
            Property::AuthenticationData(value) => Self::AuthenticationData { value },
            Property::RequestProblemInformation(value) => Self::RequestProblemInformation { value },
            Property::WillDelayInterval(value) => Self::WillDelayInterval { value },
            Property::RequestResponseInformation(value) => {
                Self::RequestResponseInformation { value }
            }
            Property::ResponseInformation(value) => Self::ResponseInformation { value },
            Property::ServerReference(value) => Self::ServerReference { value },
            Property::ReasonString(value) => Self::ReasonString { value },
            Property::ReceiveMaximum(value) => Self::ReceiveMaximum { value },
            Property::TopicAliasMaximum(value) => Self::TopicAliasMaximum { value },
            Property::TopicAlias(value) => Self::TopicAlias { value },
            Property::MaximumQoS(value) => Self::MaximumQoS { value },
            Property::RetainAvailable(value) => Self::RetainAvailable { value },
            Property::UserProperty(key, value) => Self::UserProperty { key, value },
            Property::MaximumPacketSize(value) => Self::MaximumPacketSize { value },
            Property::WildcardSubscriptionAvailable(value) => {
                Self::WildcardSubscriptionAvailable { value }
            }
            Property::SubscriptionIdentifierAvailable(value) => {
                Self::SubscriptionIdentifierAvailable { value }
            }
            Property::SharedSubscriptionAvailable(value) => {
                Self::SharedSubscriptionAvailable { value }
            }
        }
    }
}

pub(super) fn invalid(message: impl Into<String>) -> MqttErrorFFI {
    MqttErrorFFI::InvalidArgument {
        detail: message.into(),
    }
}

pub(super) fn text(value: &str) -> Result<(), MqttErrorFFI> {
    if value.len() > u16::MAX as usize || value.contains('\0') {
        return Err(invalid(
            "MQTT strings must be at most 65535 bytes and contain no NUL",
        ));
    }
    Ok(())
}

pub(super) fn topic(value: &str, allow_empty: bool) -> Result<(), MqttErrorFFI> {
    text(value)?;
    if (!allow_empty && value.is_empty()) || value.contains(['+', '#']) {
        return Err(invalid(
            "A topic name must be nonempty and contain no wildcards",
        ));
    }
    Ok(())
}

pub(super) fn validate(
    properties: Vec<MqttPropertyFFI>,
    version: u8,
    allowed: impl Fn(&MqttPropertyFFI) -> bool,
) -> Result<Vec<Property>, MqttErrorFFI> {
    if version != 5 && !properties.is_empty() {
        return Err(invalid("Properties require MQTT 5"));
    }
    let mut seen = std::collections::HashSet::new();
    for property in &properties {
        if !allowed(property) {
            return Err(invalid(format!(
                "Property {property:?} is not allowed in this packet"
            )));
        }
        if !matches!(property, MqttPropertyFFI::UserProperty { .. })
            && !seen.insert(std::mem::discriminant(property))
        {
            return Err(invalid(
                "Only user properties may be repeated in outgoing client packets",
            ));
        }
        use MqttPropertyFFI::*;
        match property {
            PayloadFormatIndicator { value }
            | RequestProblemInformation { value }
            | RequestResponseInformation { value }
            | MaximumQoS { value }
            | RetainAvailable { value }
            | WildcardSubscriptionAvailable { value }
            | SubscriptionIdentifierAvailable { value }
            | SharedSubscriptionAvailable { value }
                if *value > 1 =>
            {
                return Err(invalid("Boolean property must be 0 or 1"))
            }
            SubscriptionIdentifier { value } if *value == 0 || *value > 268_435_455 => {
                return Err(invalid("Subscription identifier must be in 1..268435455"))
            }
            ReceiveMaximum { value: 0 }
            | TopicAlias { value: 0 }
            | MaximumPacketSize { value: 0 } => {
                return Err(invalid("Property value must be nonzero"))
            }
            ContentType { value }
            | AssignedClientIdentifier { value }
            | AuthenticationMethod { value }
            | ResponseInformation { value }
            | ServerReference { value }
            | ReasonString { value } => text(value)?,
            ResponseTopic { value } => topic(value, false)?,
            UserProperty { key, value } => {
                text(key)?;
                text(value)?;
            }
            CorrelationData { value } | AuthenticationData { value } if value.len() > 65535 => {
                return Err(invalid("Binary property exceeds 65535 bytes"))
            }
            _ => {}
        }
    }
    Ok(properties.into_iter().map(Into::into).collect())
}

impl super::ffi_types::MqttPublishOptionsFFI {
    pub(super) fn command(
        self,
        topic_name: String,
        payload: Vec<u8>,
        version: u8,
        priority_supported: bool,
    ) -> Result<flowsdk::mqtt_client::commands::PublishCommand, MqttErrorFFI> {
        if self.qos > 2 {
            return Err(invalid("QoS must be 0, 1, or 2"));
        }
        if !priority_supported && self.priority.is_some() {
            return Err(MqttErrorFFI::Unsupported {
                detail: "Publish priority is not supported on QUIC".into(),
            });
        }
        let properties = validate(self.properties, version, |p| {
            matches!(
                p,
                MqttPropertyFFI::PayloadFormatIndicator { .. }
                    | MqttPropertyFFI::MessageExpiryInterval { .. }
                    | MqttPropertyFFI::ContentType { .. }
                    | MqttPropertyFFI::ResponseTopic { .. }
                    | MqttPropertyFFI::CorrelationData { .. }
                    | MqttPropertyFFI::TopicAlias { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        let has_alias = properties
            .iter()
            .any(|p| matches!(p, Property::TopicAlias(_)));
        topic(&topic_name, has_alias)?;
        Ok(flowsdk::mqtt_client::commands::PublishCommand::new(
            topic_name,
            payload,
            self.qos,
            self.retain,
            false,
            None,
            properties,
            self.priority.unwrap_or(128),
        ))
    }
}

pub(super) fn filter(value: &str) -> Result<(), MqttErrorFFI> {
    text(value)?;
    flowsdk::mqtt_serde::validate_topic_filter(value).map_err(|e| invalid(e.to_string()))?;
    flowsdk::mqtt_serde::validate_shared_subscription(value).map_err(|e| invalid(e.to_string()))?;
    if let Some(rest) = value.strip_prefix("$share/") {
        if rest
            .split('/')
            .next()
            .is_some_and(|name| name.contains(['+', '#']))
        {
            return Err(invalid(
                "Shared subscription group must not contain wildcards",
            ));
        }
    }
    Ok(())
}

impl super::ffi_types::MqttSubscribeOptionsFFI {
    pub(super) fn command(
        self,
        version: u8,
    ) -> Result<flowsdk::mqtt_client::commands::SubscribeCommand, MqttErrorFFI> {
        if self.subscriptions.is_empty() {
            return Err(invalid("At least one subscription is required"));
        }
        let properties = validate(self.properties, version, |p| {
            matches!(
                p,
                MqttPropertyFFI::SubscriptionIdentifier { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        let mut subscriptions = Vec::with_capacity(self.subscriptions.len());
        for sub in self.subscriptions {
            filter(&sub.topic_filter)?;
            if sub.qos > 2 || sub.retain_handling > 2 {
                return Err(invalid("QoS and retain_handling must be 0, 1, or 2"));
            }
            if version != 5 && (sub.no_local || sub.retain_as_published || sub.retain_handling != 0)
            {
                return Err(invalid("Subscription options require MQTT 5"));
            }
            if sub.no_local && sub.topic_filter.starts_with("$share/") {
                return Err(invalid("No Local cannot be used with shared subscriptions"));
            }
            subscriptions.push(
                flowsdk::mqtt_serde::mqttv5::subscribev5::TopicSubscription::new(
                    sub.topic_filter,
                    sub.qos,
                    sub.no_local,
                    sub.retain_as_published,
                    sub.retain_handling,
                ),
            );
        }
        Ok(flowsdk::mqtt_client::commands::SubscribeCommand::new(
            None,
            subscriptions,
            properties,
        ))
    }
}

impl super::ffi_types::MqttUnsubscribeOptionsFFI {
    pub(super) fn command(
        self,
        version: u8,
    ) -> Result<flowsdk::mqtt_client::commands::UnsubscribeCommand, MqttErrorFFI> {
        if self.topics.is_empty() {
            return Err(invalid("At least one topic is required"));
        }
        for topic in &self.topics {
            filter(topic)?;
        }
        let properties = validate(self.properties, version, |p| {
            matches!(p, MqttPropertyFFI::UserProperty { .. })
        })?;
        Ok(flowsdk::mqtt_client::commands::UnsubscribeCommand::new(
            None,
            self.topics,
            properties,
        ))
    }
}

impl super::ffi_types::MqttConnectOptionsFFI {
    pub(super) fn into_core(
        self,
    ) -> Result<flowsdk::mqtt_client::opts::MqttClientOptions, MqttErrorFFI> {
        let version = self.options.mqtt_version;
        if !matches!(version, 3..=5) {
            return Err(invalid("MQTT version must be 3, 4, or 5"));
        }
        text(&self.options.client_id)?;
        if let Some(username) = &self.options.username {
            text(username)?;
        }
        if self.binary_password.is_some() && self.options.password.is_some() {
            return Err(invalid("Provide either a string or binary password"));
        }
        let password = self
            .binary_password
            .or_else(|| self.options.password.clone().map(String::into_bytes));
        if password.as_ref().is_some_and(|p| p.len() > 65535) {
            return Err(invalid("Password exceeds 65535 bytes"));
        }
        let properties = validate(self.properties, version, |p| {
            matches!(
                p,
                MqttPropertyFFI::SessionExpiryInterval { .. }
                    | MqttPropertyFFI::ReceiveMaximum { .. }
                    | MqttPropertyFFI::MaximumPacketSize { .. }
                    | MqttPropertyFFI::TopicAliasMaximum { .. }
                    | MqttPropertyFFI::RequestResponseInformation { .. }
                    | MqttPropertyFFI::RequestProblemInformation { .. }
                    | MqttPropertyFFI::UserProperty { .. }
                    | MqttPropertyFFI::AuthenticationMethod { .. }
                    | MqttPropertyFFI::AuthenticationData { .. }
            )
        })?;
        if properties
            .iter()
            .any(|p| matches!(p, Property::AuthenticationData(_)))
            && !properties
                .iter()
                .any(|p| matches!(p, Property::AuthenticationMethod(_)))
        {
            return Err(invalid(
                "CONNECT authentication data requires an authentication method",
            ));
        }
        let mut options: flowsdk::mqtt_client::opts::MqttClientOptions = self.options.into();
        options.password = password;
        for p in &properties {
            match p {
                Property::SessionExpiryInterval(value) => {
                    options.session_expiry_interval = Some(*value)
                }
                Property::MaximumPacketSize(value) => options.maximum_packet_size = Some(*value),
                Property::RequestResponseInformation(value) => {
                    options.request_response_information = Some(*value != 0)
                }
                Property::RequestProblemInformation(value) => {
                    options.request_problem_information = Some(*value != 0)
                }
                _ => {}
            }
        }
        options.connect_properties = properties;
        if let Some(will) = self.will {
            options.will = Some(will.into_core(version)?);
        }
        if let Some(tuning) = self.engine_options {
            tuning.apply(&mut options)?;
        }
        Ok(options)
    }
}

impl super::ffi_types::MqttWillFFI {
    fn into_core(
        self,
        version: u8,
    ) -> Result<flowsdk::mqtt_serde::mqttv5::willv5::Will, MqttErrorFFI> {
        topic(&self.topic, false)?;
        if self.qos > 2 || self.payload.len() > 65535 {
            return Err(invalid("Invalid Will QoS or payload length"));
        }
        let properties = validate(self.properties, version, |p| {
            matches!(
                p,
                MqttPropertyFFI::WillDelayInterval { .. }
                    | MqttPropertyFFI::PayloadFormatIndicator { .. }
                    | MqttPropertyFFI::MessageExpiryInterval { .. }
                    | MqttPropertyFFI::ContentType { .. }
                    | MqttPropertyFFI::ResponseTopic { .. }
                    | MqttPropertyFFI::CorrelationData { .. }
                    | MqttPropertyFFI::UserProperty { .. }
            )
        })?;
        let mut will = flowsdk::mqtt_serde::mqttv5::willv5::Will::new(
            self.topic,
            self.payload,
            self.qos,
            self.retain,
        );
        for p in properties {
            match p {
                Property::WillDelayInterval(v) => will.properties.will_delay_interval = Some(v),
                Property::PayloadFormatIndicator(v) => {
                    will.properties.payload_format_indicator = Some(v)
                }
                Property::MessageExpiryInterval(v) => {
                    will.properties.message_expiry_interval = Some(v)
                }
                Property::ContentType(v) => will.properties.content_type = Some(v),
                Property::ResponseTopic(v) => will.properties.response_topic = Some(v),
                Property::CorrelationData(v) => will.properties.correlation_data = Some(v),
                p @ Property::UserProperty(_, _) => will.properties.user_properties.push(p),
                _ => unreachable!("validated Will property"),
            }
        }
        Ok(will)
    }
}

impl super::ffi_types::MqttEngineOptionsFFI {
    fn apply(
        self,
        options: &mut flowsdk::mqtt_client::opts::MqttClientOptions,
    ) -> Result<(), MqttErrorFFI> {
        macro_rules! nonzero {
            ($source:ident, $target:ident) => {
                if let Some(value) = self.$source {
                    if value == 0 {
                        return Err(invalid(concat!(
                            stringify!($source),
                            " must be greater than zero"
                        )));
                    }
                    options.$target = value as _;
                }
            };
        }
        nonzero!(retransmission_timeout_ms, retransmission_timeout_ms);
        nonzero!(ping_timeout_multiplier, ping_timeout_multiplier);
        nonzero!(max_outgoing_packet_count, max_outgoing_packet_count);
        nonzero!(max_event_count, max_event_count);
        nonzero!(parser_buffer_size, parser_buffer_size);
        nonzero!(max_inflight, receive_maximum);
        if let Some(value) = self.auto_keepalive {
            options.auto_keepalive = value;
        }
        if let Some(value) = self.auto_ack {
            options.auto_ack = value;
        }
        if let Some(value) = self.sessionless {
            options.sessionless = value;
        }
        if options.sessionless
            && (!options.clean_start || options.session_expiry_interval.unwrap_or(0) != 0)
        {
            return Err(invalid(
                "Sessionless mode requires clean_start=True and zero session expiry",
            ));
        }
        if !self.subscriptions.is_empty() {
            options.subscription_topics = super::ffi_types::MqttSubscribeOptionsFFI {
                subscriptions: self.subscriptions,
                properties: vec![],
            }
            .command(options.mqtt_version)?
            .subscriptions;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::ffi_types::MqttPublishOptionsFFI;
    use super::*;

    #[test]
    fn invalid_publish_options_are_rejected_before_queueing() {
        use MqttPropertyFFI::*;
        for properties in [
            vec![PayloadFormatIndicator { value: 2 }],
            vec![TopicAlias { value: 0 }],
            vec![
                ContentType { value: "a".into() },
                ContentType { value: "b".into() },
            ],
            vec![CorrelationData {
                value: vec![0; 65536],
            }],
            vec![ResponseTopic {
                value: "response/#".into(),
            }],
            vec![UserProperty {
                key: "bad\0key".into(),
                value: "value".into(),
            }],
            vec![SessionExpiryInterval { value: 10 }],
            vec![SubscriptionIdentifier { value: 1 }],
        ] {
            assert!(MqttPublishOptionsFFI {
                properties,
                ..Default::default()
            }
            .command("test/topic".into(), vec![], 5, true)
            .is_err());
        }
        assert!(MqttPublishOptionsFFI {
            qos: 3,
            ..Default::default()
        }
        .command("test/topic".into(), vec![], 5, true)
        .is_err());
        assert!(MqttPublishOptionsFFI::default()
            .command("bad/+".into(), vec![], 5, true)
            .is_err());
        assert!(MqttPublishOptionsFFI {
            properties: vec![ContentType {
                value: "text/plain".into()
            }],
            ..Default::default()
        }
        .command("test/topic".into(), vec![], 3, true)
        .is_err());
        assert!(MqttPublishOptionsFFI {
            priority: Some(42),
            ..Default::default()
        }
        .command("test/topic".into(), vec![], 5, false)
        .is_err());
    }

    #[test]
    fn invalid_subscription_options_are_rejected() {
        use super::super::ffi_types::{
            MqttSubscribeOptionsFFI, MqttSubscriptionFFI, MqttUnsubscribeOptionsFFI,
        };
        for filter in ["", "a/#/b", "a/+b", "$share//a", "$share/+/a"] {
            assert!(MqttUnsubscribeOptionsFFI {
                topics: vec![filter.into()],
                ..Default::default()
            }
            .command(5)
            .is_err());
        }
        assert!(MqttSubscribeOptionsFFI::default().command(5).is_err());
        for (version, topic_filter, qos, no_local, retain_handling) in [
            (5, "topic", 3, false, 0),
            (5, "topic", 0, false, 3),
            (3, "topic", 0, true, 0),
            (5, "$share/group/topic", 0, true, 0),
        ] {
            assert!(MqttSubscribeOptionsFFI {
                subscriptions: vec![MqttSubscriptionFFI {
                    topic_filter: topic_filter.into(),
                    qos,
                    no_local,
                    retain_handling,
                    ..Default::default()
                }],
                ..Default::default()
            }
            .command(version)
            .is_err());
        }
    }
}
