use crate::mqtt_serde::mqttv5::subscribev5;
use crate::mqtt_serde::mqttv5::willv5::{Will, WillProperties};

pub struct MqttClientOptions {
    pub client_id: String,
    pub clean_start: bool,
    pub keep_alive: u16,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    pub will: Option<Will>,
    pub reconnect: bool,
    // --------------------------
    // Extended options
    // --------------------------
    // if true, do not maintain session state between connections
    // if false, maintain session state
    // default: false
    pub sessionless: bool,
    // subscription topics to subscribe to on connect if not subscribed.
    pub subscription_topics: Vec<subscribev5::TopicSubscription>,
    pub auto_ack: bool, // if true, automatically acknowledge incoming QoS 1 and QoS 2 messages
}
