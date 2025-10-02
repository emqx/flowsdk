use crate::mqtt_serde::mqttv5::subscribev5;
use crate::mqtt_serde::mqttv5::willv5::Will;

pub struct MqttClientOptions {
    pub peer: String,
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
    /// Session Expiry Interval in seconds (MQTT v5 only)
    /// - None or 0: Session expires immediately on disconnect
    /// - Some(n): Session persists for n seconds after disconnect
    /// - Some(0xFFFFFFFF): Session never expires (maximum value)
    /// Default: None (expires immediately)
    pub session_expiry_interval: Option<u32>,
}

impl Default for MqttClientOptions {
    fn default() -> Self {
        Self {
            peer: "localhost:1883".to_string(),
            client_id: "mqtt_client".to_string(),
            clean_start: true,
            keep_alive: 60,
            username: None,
            password: None,
            will: None,
            reconnect: false,
            sessionless: false,
            subscription_topics: Vec::new(),
            auto_ack: true,
            session_expiry_interval: None,
        }
    }
}

impl MqttClientOptions {
    /// Create a new builder with default values
    pub fn builder() -> Self {
        Self::default()
    }

    /// Set the broker peer address (host:port)
    pub fn peer(mut self, peer: impl Into<String>) -> Self {
        self.peer = peer.into();
        self
    }

    /// Set the MQTT client ID
    pub fn client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = client_id.into();
        self
    }

    /// Set the clean start flag
    pub fn clean_start(mut self, clean_start: bool) -> Self {
        self.clean_start = clean_start;
        self
    }

    /// Set the keep alive interval in seconds
    pub fn keep_alive(mut self, keep_alive: u16) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    /// Set the username for authentication
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Set the password for authentication
    pub fn password(mut self, password: impl Into<Vec<u8>>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set the Last Will and Testament
    pub fn will(mut self, will: Will) -> Self {
        self.will = Some(will);
        self
    }

    /// Set whether to automatically reconnect on connection loss
    pub fn reconnect(mut self, reconnect: bool) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// Set whether to use sessionless mode (no session state between connections)
    pub fn sessionless(mut self, sessionless: bool) -> Self {
        self.sessionless = sessionless;
        self
    }

    /// Set topics to automatically subscribe to on connect
    pub fn subscription_topics(mut self, topics: Vec<subscribev5::TopicSubscription>) -> Self {
        self.subscription_topics = topics;
        self
    }

    /// Add a single subscription topic
    pub fn add_subscription_topic(mut self, topic: subscribev5::TopicSubscription) -> Self {
        self.subscription_topics.push(topic);
        self
    }

    /// Set whether to automatically acknowledge QoS 1 and QoS 2 messages
    pub fn auto_ack(mut self, auto_ack: bool) -> Self {
        self.auto_ack = auto_ack;
        self
    }

    /// Set the session expiry interval in seconds (MQTT v5)
    /// - None or 0: Session expires immediately on disconnect
    /// - Some(n): Session persists for n seconds after disconnect
    /// - Some(0xFFFFFFFF): Session never expires
    pub fn session_expiry_interval(mut self, interval: u32) -> Self {
        self.session_expiry_interval = Some(interval);
        self
    }

    /// Build the options (consumes self, no additional validation needed)
    pub fn build(self) -> Self {
        self
    }
}
