// SPDX-License-Identifier: MPL-2.0

//! Shared field validation for strict MQTT encoding and decoding.

use super::control_packet::ControlPacketType;
use super::mqttv5::common::properties::Property;
use super::parser::ParseError;

pub(super) fn require(valid: bool, message: &str) -> Result<(), ParseError> {
    if valid {
        Ok(())
    } else {
        Err(ParseError::ParseError(message.to_owned()))
    }
}

pub(super) fn string(value: &str) -> Result<(), ParseError> {
    if value.len() > u16::MAX as usize {
        return Err(ParseError::StringTooLong);
    }
    super::validate_mqtt_utf8_string(value)
}

pub(super) fn topic(value: &str) -> Result<(), ParseError> {
    string(value)?;
    require(!value.is_empty(), "Topic name cannot be empty")?;
    require(
        !value.contains(['+', '#']),
        "Topic name cannot contain wildcards",
    )
}

pub(super) fn packet_id(value: u16) -> Result<(), ParseError> {
    require(value != 0, "Packet identifier cannot be zero")
}

pub(super) fn publish(qos: u8, dup: bool, id: Option<u16>) -> Result<(), ParseError> {
    require(qos <= 2, "PUBLISH QoS cannot exceed 2")?;
    require(qos != 0 || !dup, "PUBLISH DUP must be zero for QoS 0")?;
    if qos > 0 {
        packet_id(id.ok_or_else(|| ParseError::ParseError("Missing packet identifier".into()))?)?;
    }
    Ok(())
}

pub(super) fn connect_flags(flags: u8) -> Result<(), ParseError> {
    require(flags & 1 == 0, "CONNECT reserved flag must be zero")?;
    require((flags >> 3) & 3 != 3, "Will QoS cannot exceed 2")?;
    require(
        flags & 4 != 0 || flags & 0x38 == 0,
        "Will QoS and RETAIN require the Will flag",
    )
}

pub(super) fn fixed_header(byte: u8) -> Result<(), ParseError> {
    let kind = byte >> 4;
    let flags = byte & 0x0f;
    let valid = match kind {
        3 => {
            let qos = (flags >> 1) & 3;
            qos <= 2 && (qos != 0 || flags & 8 == 0)
        }
        6 | 8 | 10 => flags == 2,
        1..=15 => flags == 0,
        _ => false,
    };
    require(valid, "Invalid MQTT fixed header flags")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PropertyContext {
    Connect,
    ConnAck,
    Publish,
    Will,
    Subscribe,
    Unsubscribe,
    Ack,
    Disconnect,
    Auth,
}

pub(super) fn property_value(property: &Property) -> Result<(), ParseError> {
    use Property::*;
    match property {
        PayloadFormatIndicator(v)
        | RequestProblemInformation(v)
        | RequestResponseInformation(v)
        | MaximumQoS(v)
        | RetainAvailable(v)
        | WildcardSubscriptionAvailable(v)
        | SubscriptionIdentifierAvailable(v)
        | SharedSubscriptionAvailable(v) => require(*v <= 1, "Property value must be 0 or 1"),
        ReceiveMaximum(v) | TopicAlias(v) => require(*v != 0, "Property value cannot be zero"),
        MaximumPacketSize(v) => require(*v != 0, "Maximum Packet Size cannot be zero"),
        SubscriptionIdentifier(v) => require(
            (1..=268_435_455).contains(v),
            "Invalid Subscription Identifier",
        ),
        ResponseTopic(v) => topic(v),
        AssignedClientIdentifier(v) => {
            string(v)?;
            require(!v.is_empty(), "Assigned Client Identifier cannot be empty")
        }
        ContentType(v)
        | AuthenticationMethod(v)
        | ResponseInformation(v)
        | ServerReference(v)
        | ReasonString(v) => string(v),
        UserProperty(k, v) => {
            string(k)?;
            string(v)
        }
        CorrelationData(v) | AuthenticationData(v) => {
            if v.len() > u16::MAX as usize {
                Err(ParseError::StringTooLong)
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

pub(super) fn properties(
    properties: &[Property],
    context: PropertyContext,
) -> Result<(), ParseError> {
    use Property::*;
    use PropertyContext as C;
    // A bit per property identifier avoids allocations and quadratic duplicate checks.
    let mut seen = 0u64;
    for property in properties {
        let (id, allowed) = match property {
            PayloadFormatIndicator(_) => (0x01, matches!(context, C::Publish | C::Will)),
            MessageExpiryInterval(_) => (0x02, matches!(context, C::Publish | C::Will)),
            ContentType(_) => (0x03, matches!(context, C::Publish | C::Will)),
            ResponseTopic(_) => (0x08, matches!(context, C::Publish | C::Will)),
            CorrelationData(_) => (0x09, matches!(context, C::Publish | C::Will)),
            SubscriptionIdentifier(_) => (0x0b, matches!(context, C::Publish | C::Subscribe)),
            SessionExpiryInterval(_) => (
                0x11,
                matches!(context, C::Connect | C::ConnAck | C::Disconnect),
            ),
            AssignedClientIdentifier(_) => (0x12, context == C::ConnAck),
            ServerKeepAlive(_) => (0x13, context == C::ConnAck),
            AuthenticationMethod(_) => (0x15, matches!(context, C::Connect | C::ConnAck | C::Auth)),
            AuthenticationData(_) => (0x16, matches!(context, C::Connect | C::ConnAck | C::Auth)),
            RequestProblemInformation(_) => (0x17, context == C::Connect),
            WillDelayInterval(_) => (0x18, context == C::Will),
            RequestResponseInformation(_) => (0x19, context == C::Connect),
            ResponseInformation(_) => (0x1a, context == C::ConnAck),
            ServerReference(_) => (0x1c, matches!(context, C::ConnAck | C::Disconnect)),
            ReasonString(_) => (
                0x1f,
                matches!(context, C::ConnAck | C::Ack | C::Disconnect | C::Auth),
            ),
            ReceiveMaximum(_) => (0x21, matches!(context, C::Connect | C::ConnAck)),
            TopicAliasMaximum(_) => (0x22, matches!(context, C::Connect | C::ConnAck)),
            TopicAlias(_) => (0x23, context == C::Publish),
            MaximumQoS(_) => (0x24, context == C::ConnAck),
            RetainAvailable(_) => (0x25, context == C::ConnAck),
            UserProperty(_, _) => (0x26, true),
            MaximumPacketSize(_) => (0x27, matches!(context, C::Connect | C::ConnAck)),
            WildcardSubscriptionAvailable(_) => (0x28, context == C::ConnAck),
            SubscriptionIdentifierAvailable(_) => (0x29, context == C::ConnAck),
            SharedSubscriptionAvailable(_) => (0x2a, context == C::ConnAck),
        };
        require(allowed, "Property is not allowed in this packet")?;
        let repeatable = id == 0x26 || (id == 0x0b && context == C::Publish);
        require(
            repeatable || seen & (1 << id) == 0,
            "Duplicate singleton property",
        )?;
        seen |= 1 << id;
        property_value(property)?;
    }
    if context == C::Connect {
        require(
            seen & (1 << 0x16) == 0 || seen & (1 << 0x15) != 0,
            "CONNECT Authentication Data requires Authentication Method",
        )?;
    }
    Ok(())
}

pub(super) fn reason(kind: ControlPacketType, code: u8) -> Result<(), ParseError> {
    use ControlPacketType::*;
    let allowed: &[u8] = match kind {
        CONNACK => &[
            0, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8c, 0x90, 0x95,
            0x97, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9f,
        ],
        PUBACK | PUBREC => &[0, 0x10, 0x80, 0x83, 0x87, 0x90, 0x91, 0x97, 0x99],
        PUBREL | PUBCOMP => &[0, 0x92],
        SUBACK => &[
            0, 1, 2, 0x80, 0x83, 0x87, 0x8f, 0x91, 0x97, 0x9e, 0xa1, 0xa2,
        ],
        UNSUBACK => &[0, 0x11, 0x80, 0x83, 0x87, 0x8f, 0x91],
        DISCONNECT => &[
            0, 4, 0x80, 0x81, 0x82, 0x83, 0x87, 0x89, 0x8b, 0x8d, 0x8e, 0x8f, 0x90, 0x93, 0x94,
            0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f, 0xa0, 0xa1, 0xa2,
        ],
        AUTH => &[0, 0x18, 0x19],
        _ => &[],
    };
    require(
        allowed.contains(&code),
        "Invalid reason code for packet type",
    )
}

pub(super) fn subscription(
    subscription: &super::mqttv5::subscribev5::TopicSubscription,
) -> Result<(), ParseError> {
    super::validate_topic_filter(&subscription.topic_filter)?;
    super::validate_shared_subscription(&subscription.topic_filter)?;
    require(subscription.qos <= 2, "Subscription QoS cannot exceed 2")?;
    require(
        subscription.retain_handling <= 2,
        "Retain Handling cannot exceed 2",
    )?;
    require(
        !subscription.no_local || !subscription.topic_filter.starts_with("$share/"),
        "No Local is not allowed on a shared subscription",
    )
}
