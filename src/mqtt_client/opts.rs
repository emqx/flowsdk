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
    ///   Default: None (expires immediately)
    pub session_expiry_interval: Option<u32>,
    /// Maximum Packet Size in bytes (MQTT v5 only)
    /// - None: No maximum packet size (unlimited)
    /// - Some(n): Maximum packet size in bytes (must be > 0)
    ///   Default: None (unlimited)
    pub maximum_packet_size: Option<u32>,
    /// Request Response Information (MQTT v5 only)
    /// - None: Not specified
    /// - Some(true): Request response information from server
    /// - Some(false): Do not request response information
    ///   Default: None
    pub request_response_information: Option<bool>,
    /// Request Problem Information (MQTT v5 only)
    /// - None: Not specified
    /// - Some(true): Request problem information in DISCONNECT/PUBACK
    /// - Some(false): Do not request problem information
    ///   Default: None
    pub request_problem_information: Option<bool>,
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
            maximum_packet_size: None,
            request_response_information: None,
            request_problem_information: None,
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

    /// Set the maximum packet size in bytes (MQTT v5)
    ///
    /// This property tells the server the maximum packet size the client is willing
    /// to accept. The server must not send packets larger than this size.
    ///
    /// # Arguments
    /// * `size` - Maximum packet size in bytes (must be > 0)
    ///
    /// # Panics
    /// Panics if size is 0 (protocol violation)
    ///
    /// # Example
    /// ```
    /// # use flowsdk::mqtt_client::MqttClientOptions;
    /// let options = MqttClientOptions::builder()
    ///     .maximum_packet_size(268_435_455)  // Maximum allowed value
    ///     .build();
    /// ```
    pub fn maximum_packet_size(mut self, size: u32) -> Self {
        if size == 0 {
            panic!("maximum_packet_size must be greater than 0 (MQTT v5 protocol violation)");
        }
        self.maximum_packet_size = Some(size);
        self
    }

    /// Disable maximum packet size limit
    ///
    /// Sets maximum_packet_size to None (default), indicating unlimited packet size.
    pub fn no_maximum_packet_size(mut self) -> Self {
        self.maximum_packet_size = None;
        self
    }

    /// Request response information from the server (MQTT v5)
    ///
    /// When set to true, the server should return response information
    /// in the CONNACK packet, which can be used for request/response patterns.
    ///
    /// # Example
    /// ```
    /// # use flowsdk::mqtt_client::MqttClientOptions;
    /// let options = MqttClientOptions::builder()
    ///     .request_response_information(true)
    ///     .build();
    /// ```
    pub fn request_response_information(mut self, request: bool) -> Self {
        self.request_response_information = Some(request);
        self
    }

    /// Request problem information in responses (MQTT v5)
    ///
    /// When set to true, the server should include problem information
    /// (Reason String and User Properties) in DISCONNECT and PUBACK packets.
    ///
    /// # Example
    /// ```
    /// # use flowsdk::mqtt_client::MqttClientOptions;
    /// let options = MqttClientOptions::builder()
    ///     .request_problem_information(true)
    ///     .build();
    /// ```
    pub fn request_problem_information(mut self, request: bool) -> Self {
        self.request_problem_information = Some(request);
        self
    }

    /// Build the options (consumes self, no additional validation needed)
    pub fn build(self) -> Self {
        self
    }
}

#[cfg(test)]
mod mqtt_client_options_tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let options = MqttClientOptions::default();

        assert_eq!(options.peer, "localhost:1883");
        assert_eq!(options.client_id, "mqtt_client");
        assert!(options.clean_start);
        assert_eq!(options.keep_alive, 60);
        assert_eq!(options.username, None);
        assert_eq!(options.password, None);
        assert_eq!(options.will, None);
        assert!(!options.reconnect);
        assert!(!options.sessionless);
        assert!(options.auto_ack);
        assert_eq!(options.session_expiry_interval, None);
        assert_eq!(options.maximum_packet_size, None);
        assert_eq!(options.request_response_information, None);
        assert_eq!(options.request_problem_information, None);
    }

    // ==================== Maximum Packet Size Tests ====================

    #[test]
    fn test_maximum_packet_size_default() {
        let options = MqttClientOptions::default();
        assert_eq!(options.maximum_packet_size, None);
    }

    #[test]
    fn test_maximum_packet_size_builder() {
        let options = MqttClientOptions::builder()
            .maximum_packet_size(1024)
            .build();

        assert_eq!(options.maximum_packet_size, Some(1024));
    }

    #[test]
    fn test_maximum_packet_size_min_value() {
        let options = MqttClientOptions::builder().maximum_packet_size(1).build();

        assert_eq!(options.maximum_packet_size, Some(1));
    }

    #[test]
    fn test_maximum_packet_size_max_value() {
        // MQTT v5 spec allows up to 268,435,455 bytes (0x0FFFFFFF)
        let options = MqttClientOptions::builder()
            .maximum_packet_size(268_435_455)
            .build();

        assert_eq!(options.maximum_packet_size, Some(268_435_455));
    }

    #[test]
    #[should_panic(expected = "maximum_packet_size must be greater than 0")]
    fn test_maximum_packet_size_zero_panics() {
        MqttClientOptions::builder().maximum_packet_size(0).build();
    }

    #[test]
    fn test_no_maximum_packet_size() {
        let options = MqttClientOptions::builder()
            .maximum_packet_size(1024)
            .no_maximum_packet_size()
            .build();

        assert_eq!(options.maximum_packet_size, None);
    }

    // ==================== Request Response Information Tests ====================

    #[test]
    fn test_request_response_information_default() {
        let options = MqttClientOptions::default();
        assert_eq!(options.request_response_information, None);
    }

    #[test]
    fn test_request_response_information_true() {
        let options = MqttClientOptions::builder()
            .request_response_information(true)
            .build();

        assert_eq!(options.request_response_information, Some(true));
    }

    #[test]
    fn test_request_response_information_false() {
        let options = MqttClientOptions::builder()
            .request_response_information(false)
            .build();

        assert_eq!(options.request_response_information, Some(false));
    }

    // ==================== Request Problem Information Tests ====================

    #[test]
    fn test_request_problem_information_default() {
        let options = MqttClientOptions::default();
        assert_eq!(options.request_problem_information, None);
    }

    #[test]
    fn test_request_problem_information_true() {
        let options = MqttClientOptions::builder()
            .request_problem_information(true)
            .build();

        assert_eq!(options.request_problem_information, Some(true));
    }

    #[test]
    fn test_request_problem_information_false() {
        let options = MqttClientOptions::builder()
            .request_problem_information(false)
            .build();

        assert_eq!(options.request_problem_information, Some(false));
    }

    // ==================== Combined MQTT v5 Properties Tests ====================

    #[test]
    fn test_all_mqtt_v5_properties() {
        let options = MqttClientOptions::builder()
            .session_expiry_interval(3600)
            .maximum_packet_size(65536)
            .request_response_information(true)
            .request_problem_information(true)
            .build();

        assert_eq!(options.session_expiry_interval, Some(3600));
        assert_eq!(options.maximum_packet_size, Some(65536));
        assert_eq!(options.request_response_information, Some(true));
        assert_eq!(options.request_problem_information, Some(true));
    }

    #[test]
    fn test_builder_chaining() {
        let options = MqttClientOptions::builder()
            .peer("mqtt.example.com:8883")
            .client_id("test_client")
            .clean_start(false)
            .keep_alive(120)
            .username("user123")
            .password(b"secret".to_vec())
            .reconnect(true)
            .sessionless(true)
            .auto_ack(false)
            .session_expiry_interval(7200)
            .maximum_packet_size(1_048_576)
            .request_response_information(true)
            .request_problem_information(false)
            .build();

        assert_eq!(options.peer, "mqtt.example.com:8883");
        assert_eq!(options.client_id, "test_client");
        assert!(!options.clean_start);
        assert_eq!(options.keep_alive, 120);
        assert_eq!(options.username, Some("user123".to_string()));
        assert_eq!(options.password, Some(b"secret".to_vec()));
        assert!(options.reconnect);
        assert!(options.sessionless);
        assert!(!options.auto_ack);
        assert_eq!(options.session_expiry_interval, Some(7200));
        assert_eq!(options.maximum_packet_size, Some(1_048_576));
        assert_eq!(options.request_response_information, Some(true));
        assert_eq!(options.request_problem_information, Some(false));
    }

    #[test]
    fn test_mqtt_v5_compliance_values() {
        // Test typical MQTT v5 packet size limits
        let test_sizes = vec![1, 256, 1024, 65536, 1_048_576, 268_435_455];

        for size in test_sizes {
            let options = MqttClientOptions::builder()
                .maximum_packet_size(size)
                .build();

            assert_eq!(options.maximum_packet_size, Some(size));
        }
    }
}
