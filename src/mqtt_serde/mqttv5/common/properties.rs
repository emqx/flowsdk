// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};

use crate::mqtt_serde::parser;
use crate::mqtt_serde::parser::{parse_utf8_string, ParseError};
use crate::mqtt_serde::{decode_binary_data, encode_utf8_string, encode_variable_length};

pub type Properties = Vec<Property>;
// MQTT 5.0: 2.2.2
enum PropertyID {
    PayloadFormatIndicator = 0x01,
    MessageExpiryInterval = 0x02,
    ContentType = 0x03,
    ResponseTopic = 0x08,
    CorrelationData = 0x09,
    SubscriptionIdentifier = 0x0b,
    SessionExpiryInterval = 0x11,
    AssignedClientIdentifier = 0x12,
    ServerKeepAlive = 0x13,
    AuthenticationMethod = 0x15,
    AuthenticationData = 0x16,
    RequestProblemInformation = 0x17,
    WillDelayInterval = 0x18,
    RequestResponseInformation = 0x19,
    ResponseInformation = 0x1a,
    ServerReference = 0x1c,
    ReasonString = 0x1f,
    ReceiveMaximum = 0x21,
    TopicAliasMaximum = 0x22,
    TopicAlias = 0x23,
    MaximumQoS = 0x24,
    RetainAvailable = 0x25,
    UserProperty = 0x26,
    MaximumPacketSize = 0x27,
    WildcardSubscriptionAvailable = 0x28,
    SubscriptionIdentifierAvailable = 0x29,
    SharedSubscriptionAvailable = 0x2a,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum Property {
    PayloadFormatIndicator(u8),
    MessageExpiryInterval(u32),
    ContentType(String),
    ResponseTopic(String),
    CorrelationData(Vec<u8>),
    SubscriptionIdentifier(u32),
    SessionExpiryInterval(u32),
    AssignedClientIdentifier(String),
    ServerKeepAlive(u16),
    AuthenticationMethod(String),
    AuthenticationData(Vec<u8>),
    RequestProblemInformation(u8),
    WillDelayInterval(u32),
    RequestResponseInformation(u8),
    ResponseInformation(String),
    ServerReference(String),
    ReasonString(String),
    ReceiveMaximum(u16),
    TopicAliasMaximum(u16),
    TopicAlias(u16),
    MaximumQoS(u8),
    RetainAvailable(u8),
    UserProperty(String, String),
    MaximumPacketSize(u32),
    WildcardSubscriptionAvailable(u8),
    SubscriptionIdentifierAvailable(u8),
    SharedSubscriptionAvailable(u8),
}

impl Property {
    fn encode(&self, bytes: &mut Vec<u8>) -> Result<(), parser::ParseError> {
        match self {
            Property::PayloadFormatIndicator(val) => {
                bytes.push(property_id(PropertyID::PayloadFormatIndicator));
                bytes.push(*val);
            }
            Property::MessageExpiryInterval(val) => {
                bytes.push(property_id(PropertyID::MessageExpiryInterval));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::ContentType(val) => {
                bytes.push(property_id(PropertyID::ContentType));
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::ResponseTopic(val) => {
                bytes.push(property_id(PropertyID::ResponseTopic));
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::CorrelationData(val) => {
                bytes.push(property_id(PropertyID::CorrelationData));
                bytes.extend(crate::mqtt_serde::encode_binary_data(val)?);
            }
            Property::SubscriptionIdentifier(val) => {
                bytes.push(property_id(PropertyID::SubscriptionIdentifier));
                bytes.extend(crate::mqtt_serde::encode_variable_length(*val as usize));
            }
            Property::SessionExpiryInterval(val) => {
                bytes.push(property_id(PropertyID::SessionExpiryInterval));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::AssignedClientIdentifier(val) => {
                bytes.push(property_id(PropertyID::AssignedClientIdentifier));
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::ServerKeepAlive(val) => {
                bytes.push(property_id(PropertyID::ServerKeepAlive));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::AuthenticationMethod(val) => {
                bytes.push(property_id(PropertyID::AuthenticationMethod));
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::AuthenticationData(val) => {
                bytes.push(property_id(PropertyID::AuthenticationData));
                bytes.extend(crate::mqtt_serde::encode_binary_data(val)?);
            }
            Property::RequestProblemInformation(val) => {
                bytes.push(property_id(PropertyID::RequestProblemInformation));
                bytes.push(*val);
            }
            Property::WillDelayInterval(val) => {
                bytes.push(property_id(PropertyID::WillDelayInterval));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::RequestResponseInformation(val) => {
                bytes.push(property_id(PropertyID::RequestResponseInformation));
                bytes.push(*val);
            }
            Property::ResponseInformation(val) => {
                bytes.push(property_id(PropertyID::ResponseInformation));
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::ServerReference(val) => {
                bytes.push(property_id(PropertyID::ServerReference));
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::ReasonString(val) => {
                bytes.push(property_id(PropertyID::ReasonString));
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::ReceiveMaximum(val) => {
                bytes.push(property_id(PropertyID::ReceiveMaximum));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::TopicAliasMaximum(val) => {
                bytes.push(property_id(PropertyID::TopicAliasMaximum));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::TopicAlias(val) => {
                bytes.push(property_id(PropertyID::TopicAlias));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::MaximumQoS(val) => {
                bytes.push(property_id(PropertyID::MaximumQoS));
                bytes.push(*val);
            }
            Property::RetainAvailable(val) => {
                bytes.push(property_id(PropertyID::RetainAvailable));
                bytes.push(*val);
            }
            Property::UserProperty(key, val) => {
                bytes.push(property_id(PropertyID::UserProperty));
                bytes.extend(encode_utf8_string(key.as_str())?);
                bytes.extend(encode_utf8_string(val.as_str())?);
            }
            Property::MaximumPacketSize(val) => {
                bytes.push(property_id(PropertyID::MaximumPacketSize));
                bytes.extend_from_slice(&val.to_be_bytes());
            }
            Property::WildcardSubscriptionAvailable(val) => {
                bytes.push(property_id(PropertyID::WildcardSubscriptionAvailable));
                bytes.push(*val);
            }
            Property::SubscriptionIdentifierAvailable(val) => {
                bytes.push(property_id(PropertyID::SubscriptionIdentifierAvailable));
                bytes.push(*val);
            }
            Property::SharedSubscriptionAvailable(val) => {
                bytes.push(property_id(PropertyID::SharedSubscriptionAvailable));
                bytes.push(*val);
            }
        }
        Ok(())
    }
}

// function given the property id, return the property data type
fn property_id(property: PropertyID) -> u8 {
    property as u8
}

pub fn encode_properities_hdr(properties: &Vec<Property>) -> Result<Vec<u8>, parser::ParseError> {
    let mut props = Vec::new();
    encode_properities(properties, &mut props)?;
    let mut bytes = Vec::new();
    bytes.extend(encode_variable_length(props.len()));
    bytes.extend(props);
    Ok(bytes)
}

fn encode_properities(
    properties: &Vec<Property>,
    bytes: &mut Vec<u8>,
) -> Result<(), parser::ParseError> {
    for p in properties {
        p.encode(bytes)?;
    }
    Ok(())
}

pub fn parse_properties_hdr(buffer: &[u8]) -> Result<(Vec<Property>, usize), ParseError> {
    let mut offset = 0;

    // get prop len
    let (prop_len, consumed) = parser::parse_vbi(buffer)?;
    offset += consumed;

    if buffer.len() < offset + prop_len {
        return Err(ParseError::IncompleteProperty);
    }

    let (props, consumed) = parse_properties(&buffer[offset..offset + prop_len])?;
    offset += consumed;

    Ok((props, offset))
}

fn parse_properties(buffer: &[u8]) -> Result<(Vec<Property>, usize), ParseError> {
    let mut properties = Vec::new();
    let mut offset = 0;

    while offset < buffer.len() {
        let (prop, consumed) = parse_property(&buffer[offset..])?;
        properties.push(prop);
        offset += consumed;
    }

    Ok((properties, offset))
}

fn parse_property(buffer: &[u8]) -> Result<(Property, usize), ParseError> {
    let mut offset = 0;
    let (property_id, consumed) = parser::parse_vbi(buffer)?;
    offset += consumed;

    let property = PropertyID::try_from(property_id)
        .map_err(|_| ParseError::ParseError("Unknown property ID".to_string()))?;

    match property {
        PropertyID::PayloadFormatIndicator => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::PayloadFormatIndicator(val), offset))
        }
        PropertyID::MessageExpiryInterval => {
            let mut val = [0; 4];
            let slice = buffer
                .get(offset..offset + 4)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 4;
            Ok((
                Property::MessageExpiryInterval(u32::from_be_bytes(val)),
                offset,
            ))
        }
        PropertyID::ContentType => {
            let (val, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::ContentType(val), offset))
        }
        PropertyID::ResponseTopic => {
            let (val, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::ResponseTopic(val), offset))
        }
        PropertyID::CorrelationData => {
            let (val, consumed) = decode_binary_data(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::CorrelationData(val), offset))
        }
        PropertyID::SubscriptionIdentifier => {
            let (val, consumed) = parser::parse_vbi(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::SubscriptionIdentifier(val as u32), offset))
        }
        PropertyID::SessionExpiryInterval => {
            let mut val = [0; 4];
            let slice = buffer
                .get(offset..offset + 4)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 4;
            Ok((
                Property::SessionExpiryInterval(u32::from_be_bytes(val)),
                offset,
            ))
        }
        PropertyID::AssignedClientIdentifier => {
            let (val, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::AssignedClientIdentifier(val), offset))
        }
        PropertyID::ServerKeepAlive => {
            let mut val = [0; 2];
            let slice = buffer
                .get(offset..offset + 2)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 2;
            Ok((Property::ServerKeepAlive(u16::from_be_bytes(val)), offset))
        }
        PropertyID::AuthenticationMethod => {
            let (val, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::AuthenticationMethod(val), offset))
        }
        PropertyID::AuthenticationData => {
            let (val, consumed) = decode_binary_data(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::AuthenticationData(val), offset))
        }
        PropertyID::RequestProblemInformation => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::RequestProblemInformation(val), offset))
        }
        PropertyID::WillDelayInterval => {
            let mut val = [0; 4];
            let slice = buffer
                .get(offset..offset + 4)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 4;
            Ok((Property::WillDelayInterval(u32::from_be_bytes(val)), offset))
        }
        PropertyID::RequestResponseInformation => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::RequestResponseInformation(val), offset))
        }
        PropertyID::ResponseInformation => {
            let (val, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::ResponseInformation(val), offset))
        }
        PropertyID::ServerReference => {
            let (val, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::ServerReference(val), offset))
        }
        PropertyID::ReasonString => {
            let (val, consumed) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed;
            Ok((Property::ReasonString(val), offset))
        }
        PropertyID::ReceiveMaximum => {
            let mut val = [0; 2];
            let slice = buffer
                .get(offset..offset + 2)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 2;
            Ok((Property::ReceiveMaximum(u16::from_be_bytes(val)), offset))
        }
        PropertyID::TopicAliasMaximum => {
            let mut val = [0; 2];
            let slice = buffer
                .get(offset..offset + 2)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 2;
            Ok((Property::TopicAliasMaximum(u16::from_be_bytes(val)), offset))
        }
        PropertyID::TopicAlias => {
            let mut val = [0; 2];
            let slice = buffer
                .get(offset..offset + 2)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 2;
            Ok((Property::TopicAlias(u16::from_be_bytes(val)), offset))
        }
        PropertyID::MaximumQoS => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::MaximumQoS(val), offset))
        }
        PropertyID::RetainAvailable => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::RetainAvailable(val), offset))
        }
        PropertyID::UserProperty => {
            let (key, consumed_key) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed_key;
            let (val, consumed_val) = parse_utf8_string(&buffer[offset..])?;
            offset += consumed_val;
            Ok((Property::UserProperty(key, val), offset))
        }
        PropertyID::MaximumPacketSize => {
            let mut val = [0; 4];
            let slice = buffer
                .get(offset..offset + 4)
                .ok_or(ParseError::IncompleteProperty)?;
            val.copy_from_slice(slice);
            offset += 4;
            Ok((Property::MaximumPacketSize(u32::from_be_bytes(val)), offset))
        }
        PropertyID::WildcardSubscriptionAvailable => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::WildcardSubscriptionAvailable(val), offset))
        }
        PropertyID::SubscriptionIdentifierAvailable => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::SubscriptionIdentifierAvailable(val), offset))
        }
        PropertyID::SharedSubscriptionAvailable => {
            let val = *buffer.get(offset).ok_or(ParseError::IncompleteProperty)?;
            offset += 1;
            Ok((Property::SharedSubscriptionAvailable(val), offset))
        }
    }
}

impl TryFrom<usize> for PropertyID {
    type Error = parser::ParseError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(PropertyID::PayloadFormatIndicator),
            0x02 => Ok(PropertyID::MessageExpiryInterval),
            0x03 => Ok(PropertyID::ContentType),
            0x08 => Ok(PropertyID::ResponseTopic),
            0x09 => Ok(PropertyID::CorrelationData),
            0x0B => Ok(PropertyID::SubscriptionIdentifier),
            0x11 => Ok(PropertyID::SessionExpiryInterval),
            0x12 => Ok(PropertyID::AssignedClientIdentifier),
            0x13 => Ok(PropertyID::ServerKeepAlive),
            0x15 => Ok(PropertyID::AuthenticationMethod),
            0x16 => Ok(PropertyID::AuthenticationData),
            0x17 => Ok(PropertyID::RequestProblemInformation),
            0x18 => Ok(PropertyID::WillDelayInterval),
            0x19 => Ok(PropertyID::RequestResponseInformation),
            0x1A => Ok(PropertyID::ResponseInformation),
            0x1C => Ok(PropertyID::ServerReference),
            0x1F => Ok(PropertyID::ReasonString),
            0x21 => Ok(PropertyID::ReceiveMaximum),
            0x22 => Ok(PropertyID::TopicAliasMaximum),
            0x23 => Ok(PropertyID::TopicAlias),
            0x24 => Ok(PropertyID::MaximumQoS),
            0x25 => Ok(PropertyID::RetainAvailable),
            0x26 => Ok(PropertyID::UserProperty),
            0x27 => Ok(PropertyID::MaximumPacketSize),
            0x28 => Ok(PropertyID::WildcardSubscriptionAvailable),
            0x29 => Ok(PropertyID::SubscriptionIdentifierAvailable),
            0x2A => Ok(PropertyID::SharedSubscriptionAvailable),
            _ => Err(ParseError::InvalidPropertyId),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_property() {
        let prop = Property::PayloadFormatIndicator(1);
        let mut bytes = Vec::new();
        prop.encode(&mut bytes).unwrap();
        assert_eq!(bytes, vec![0x01, 0x01]);
    }

    #[test]
    fn test_encode_zero_len_properties_hdrs() {
        assert_eq!(vec![0x00_u8], encode_properities_hdr(&Vec::new()).unwrap());
    }

    #[test]
    fn test_encode_properties_hdr() {
        let properties: Vec<Property> = vec![
            Property::SharedSubscriptionAvailable(0x01),
            Property::SubscriptionIdentifierAvailable(0x01),
            Property::WildcardSubscriptionAvailable(0x01),
            Property::MaximumPacketSize(1048576),
            Property::RetainAvailable(0x01),
            Property::TopicAliasMaximum(65535),
            Property::ReceiveMaximum(32),
        ];
        let bytes = encode_properities_hdr(&properties).unwrap();
        let expected = hex::decode("132a01290128012700100000250122ffff210020").unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn test_parse_property() {
        let buffer = [0x2a, 0x01];
        let (prop, consumed) = parse_property(&buffer).unwrap();
        assert_eq!(consumed, 2);
        match prop {
            Property::SharedSubscriptionAvailable(val) => {
                assert_eq!(val, 0x01);
            }
            _ => panic!("Invalid property"),
        }
    }

    #[test]
    fn test_parse_properties() {
        let buffer = hex::decode("2a01290128012700100000250122ffff210020").unwrap();
        let (props, consumed) = parse_properties(&buffer).unwrap();
        assert_eq!(consumed, buffer.len());
        assert_eq!(props.len(), 7);
        assert_eq!(props[0], Property::SharedSubscriptionAvailable(0x01));
        assert_eq!(props[1], Property::SubscriptionIdentifierAvailable(0x01));
        assert_eq!(props[2], Property::WildcardSubscriptionAvailable(0x01));
        assert_eq!(props[3], Property::MaximumPacketSize(1048576));
        assert_eq!(props[4], Property::RetainAvailable(0x01));
        assert_eq!(props[5], Property::TopicAliasMaximum(65535));
        assert_eq!(props[6], Property::ReceiveMaximum(32));
    }

    #[test]
    fn test_parse_properties_hdr() {
        let buffer = hex::decode("132a01290128012700100000250122ffff210020").unwrap();
        let (props, consumed) = parse_properties_hdr(&buffer).unwrap();
        assert_eq!(consumed, buffer.len());
        assert_eq!(props.len(), 7);
        assert_eq!(props[0], Property::SharedSubscriptionAvailable(0x01));
        assert_eq!(props[1], Property::SubscriptionIdentifierAvailable(0x01));
        assert_eq!(props[2], Property::WildcardSubscriptionAvailable(0x01));
        assert_eq!(props[3], Property::MaximumPacketSize(1048576));
        assert_eq!(props[4], Property::RetainAvailable(0x01));
        assert_eq!(props[5], Property::TopicAliasMaximum(65535));
        assert_eq!(props[6], Property::ReceiveMaximum(32));
    }

    #[test]
    fn test_parse_empty_properties_hdr() {
        let buffer = hex::decode("00").unwrap();
        let (props, consumed) = parse_properties_hdr(&buffer).unwrap();
        assert_eq!(consumed, buffer.len());
        assert_eq!(props.len(), 0);
        assert_eq!(props, []);
    }
}
