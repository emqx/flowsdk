use crate::mqtt_serde::mqttv5::common::properties::{parse_properties_hdr, Property};
use crate::mqtt_serde::parser::{parse_binary_data, parse_utf8_string, ParseError};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct Will {
    pub will_qos: u8,
    pub will_retain: bool,
    pub will_delay_interval: u32,
    pub payload_format_indicator: u8,
    pub message_expiry_interval: u32,
    pub content_type: Option<String>,
    pub response_topic: Option<String>,
    pub correlation_data: Option<Vec<u8>>,
    pub user_properties: Vec<Property>,
    pub will_topic: String,
    pub will_message: Vec<u8>,
}

impl Will {
    pub fn new(will_topic: String, will_message: Vec<u8>, will_qos: u8, will_retain: bool) -> Self {
        Will {
            will_qos,
            will_retain,
            will_delay_interval: 0,
            payload_format_indicator: 0,
            message_expiry_interval: 0,
            content_type: None,
            response_topic: None,
            correlation_data: None,
            user_properties: Vec::new(),
            will_topic,
            will_message,
        }
    }

    pub fn from_bytes(buffer: &[u8], connect_flags: u8) -> Result<(Self, usize), ParseError> {
        let mut offset = 0;

        // MQTT 5.0: 3.1.3.2 Will Properties
        let (properties, consumed) = parse_properties_hdr(buffer)?;
        offset += consumed;

        // MQTT 5.0: 3.1.3.3 Will Topic
        let (will_topic, consumed) = parse_utf8_string(&buffer[offset..])?;
        offset += consumed;

        // MQTT 5.0: 3.1.3.4 Will Payload
        let (will_message, consumed) = parse_binary_data(&buffer[offset..])?;
        offset += consumed;

        let will_qos = (connect_flags >> 3) & 0x03;
        let will_retain = (connect_flags & 0x20) != 0;

        let mut will = Will::new(will_topic, will_message, will_qos, will_retain);
        for property in properties {
            match property {
                // MQTT 5.0: 3.1.3.2.2 Will Delay Interval
                Property::WillDelayInterval(value) => will.will_delay_interval = value,
                // MQTT 5.0: 3.1.3.2.3 Payload format indicator
                Property::PayloadFormatIndicator(value) => will.payload_format_indicator = value,
                // MQTT 5.0: 3.1.3.2.4 Message Expiry Interval
                Property::MessageExpiryInterval(value) => will.message_expiry_interval = value,
                // MQTT 5.0: 3.1.3.2.5 Content Type
                Property::ContentType(value) => will.content_type = Some(value),
                // MQTT 5.0: 3.1.3.2.6 Response Topic
                Property::ResponseTopic(value) => will.response_topic = Some(value),
                // MQTT 5.0: 3.1.3.2.7 Correlation Data
                Property::CorrelationData(value) => will.correlation_data = Some(value),
                // MQTT 5.0: 3.1.3.2.8 UserProperty
                _ => {
                    if let Property::UserProperty(key, value) = property {
                        will.user_properties
                            .push(Property::UserProperty(key, value));
                    }
                }
            }
        }

        Ok((will, offset))
    }
}
