use crate::mqtt_serde::mqttv5::subscribev5;
use crate::mqtt_serde::mqttv5::willv5::Will;

#[cfg(feature = "rustls-tls")]
use crate::mqtt_client::transport::rustls_tls::RustlsTlsConfig;
#[cfg(feature = "tls")]
use crate::mqtt_client::transport::tls::TlsConfig;

#[cfg(any(feature = "tls", feature = "rustls-tls"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsBackend {
    Native,
    Rustls,
}

pub struct MqttClientOptions {
    pub peer: String,
    pub client_id: String,
    pub clean_start: bool,
    pub keep_alive: u16,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    pub will: Option<Will>,
    pub reconnect: bool,
    /// MQTT protocol version to use
    /// - 3 or 4: MQTT v3.1.1 (both values use the same protocol)
    /// - 5: MQTT v5.0
    ///   Default: 5 (for backward compatibility)
    pub mqtt_version: u8,
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

    /// TLS configuration for encrypted connections (requires 'tls' feature)
    /// - None: Use plain TCP connection
    /// - Some(config): Use TLS with specified configuration
    ///   Default: None
    #[cfg(feature = "tls")]
    pub tls_config: Option<TlsConfig>,
    /// Rustls TLS configuration (requires 'rustls-tls' feature)
    #[cfg(feature = "rustls-tls")]
    pub rustls_tls_config: Option<RustlsTlsConfig>,
    /// Selected TLS backend (None = no TLS)
    #[cfg(any(feature = "tls", feature = "rustls-tls"))]
    pub tls_backend: Option<TlsBackend>,

    // --------------------------
    // Reconnection settings
    // --------------------------
    /// Base delay for reconnection backoff in milliseconds
    /// - Actual delay uses exponential backoff: base_delay * 2^attempts
    /// - Default: 1000 (1 second)
    pub reconnect_base_delay_ms: u64,

    /// Maximum delay for reconnection backoff in milliseconds
    /// - Caps the exponential backoff to prevent excessive delays
    /// - Default: 60000 (60 seconds)
    pub reconnect_max_delay_ms: u64,

    /// Maximum number of reconnection attempts
    /// - 0: Unlimited attempts (will keep trying forever)
    /// - n: Stop after n failed attempts
    /// - Default: 0 (unlimited)
    pub max_reconnect_attempts: u32,

    // --------------------------
    // Timeout settings
    // --------------------------
    /// Retransmission timeout for QoS 1/2 messages in milliseconds
    /// - How long to wait before resending unacknowledged messages
    /// - Default: 5000 (5 seconds)
    pub retransmission_timeout_ms: u64,

    /// Ping timeout multiplier
    /// - Connection is considered dead if no packet received for keep_alive * multiplier
    /// - Default: 2 (wait 2x keep_alive before timing out)
    pub ping_timeout_multiplier: u32,
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
            mqtt_version: 5,
            sessionless: false,
            subscription_topics: Vec::new(),
            auto_ack: true,
            session_expiry_interval: None,
            maximum_packet_size: None,
            request_response_information: None,
            request_problem_information: None,
            #[cfg(feature = "tls")]
            tls_config: None,
            #[cfg(feature = "rustls-tls")]
            rustls_tls_config: None,
            #[cfg(any(feature = "tls", feature = "rustls-tls"))]
            tls_backend: None,
            reconnect_base_delay_ms: 1000,
            reconnect_max_delay_ms: 60000,
            max_reconnect_attempts: 0,
            retransmission_timeout_ms: 5000,
            ping_timeout_multiplier: 2,
        }
    }
}

pub type MqttClientOptionsBuilder = MqttClientOptions;

impl MqttClientOptions {
    /// Create a new builder with default values
    pub fn builder() -> Self {
        Self::default()
    }

    /// Build the options
    pub fn build(self) -> Self {
        self
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

    /// Set the MQTT protocol version
    ///
    /// # Arguments
    /// * `version` - MQTT protocol version (3, 4, or 5)
    ///   - 3 or 4: MQTT v3.1.1 (both use the same protocol)
    ///   - 5: MQTT v5.0
    ///
    /// # Panics
    /// Panics if version is not 3, 4, or 5
    ///
    /// # Example
    /// ```
    /// # use flowsdk::mqtt_client::MqttClientOptions;
    /// let options = MqttClientOptions::builder()
    ///     .mqtt_version(3)  // Use MQTT v3.1.1
    ///     .build();
    /// ```
    pub fn mqtt_version(mut self, version: u8) -> Self {
        if version != 3 && version != 4 && version != 5 {
            panic!("mqtt_version must be 3, 4, or 5 (got {})", version);
        }
        self.mqtt_version = version;
        self
    }

    /// Set the session expiry interval in seconds (MQTT v5 only)
    ///
    /// - 0: Session expires immediately on disconnect
    /// - n: Session persists for n seconds after disconnect
    /// - 0xFFFFFFFF: Session never expires
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

    /// Set TLS configuration for encrypted connections (requires 'tls' feature)
    ///
    /// Use this to enable TLS/SSL connections to the MQTT broker with custom
    /// certificate configuration, client identity, etc.
    ///
    /// # Example
    /// ```no_run
    /// # #[cfg(feature = "tls")]
    /// # {
    /// use flowsdk::mqtt_client::MqttClientOptions;
    /// use flowsdk::mqtt_client::transport::tls::TlsConfig;
    /// use native_tls::Certificate;
    ///
    /// let cert_pem = std::fs::read("ca.crt").unwrap();
    /// let cert = Certificate::from_pem(&cert_pem).unwrap();
    ///
    /// let tls_config = TlsConfig::builder()
    ///     .add_root_certificate(cert)
    ///     .build();
    ///
    /// let options = MqttClientOptions::builder()
    ///     .peer("mqtts://broker.example.com:8883")
    ///     .tls_config(tls_config)
    ///     .build();
    /// # }
    /// ```
    #[cfg(feature = "tls")]
    pub fn tls_config(mut self, config: TlsConfig) -> Self {
        self.tls_config = Some(config);
        #[cfg(any(feature = "tls", feature = "rustls-tls"))]
        {
            self.tls_backend = Some(TlsBackend::Native);
        }
        self
    }

    /// Enable TLS with default system certificates (requires 'tls' feature)
    ///
    /// This is a convenience method that creates a default TLS configuration
    /// using the system's trusted root certificates.
    ///
    /// # Example
    /// ```no_run
    /// # #[cfg(feature = "tls")]
    /// # {
    /// use flowsdk::mqtt_client::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::builder()
    ///     .peer("mqtts://broker.example.com:8883")
    ///     .enable_tls()
    ///     .build();
    /// # }
    /// ```
    #[cfg(feature = "tls")]
    pub fn enable_tls(mut self) -> Self {
        self.tls_config = Some(TlsConfig::default());
        #[cfg(any(feature = "tls", feature = "rustls-tls"))]
        {
            self.tls_backend = Some(TlsBackend::Native);
        }
        self
    }

    /// Disable TLS (use plain TCP connection)
    ///
    /// This explicitly disables TLS even if it was previously enabled.
    ///
    /// # Example
    /// ```no_run
    /// # #[cfg(feature = "tls")]
    /// # {
    /// use flowsdk::mqtt_client::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::builder()
    ///     .peer("mqtt://broker.example.com:1883")
    ///     .disable_tls()
    ///     .build();
    /// # }
    /// ```
    #[cfg(feature = "tls")]
    pub fn disable_tls(mut self) -> Self {
        self.tls_config = None;
        #[cfg(any(feature = "tls", feature = "rustls-tls"))]
        {
            // Only clear backend if it was Native
            if matches!(self.tls_backend, Some(TlsBackend::Native)) {
                self.tls_backend = None;
            }
        }
        self
    }

    /// Enable Rustls TLS with default config (requires 'rustls-tls' feature)
    #[cfg(feature = "rustls-tls")]
    pub fn enable_rustls_tls(mut self) -> Self {
        self.rustls_tls_config = Some(RustlsTlsConfig::default());
        self.tls_backend = Some(TlsBackend::Rustls);
        self
    }

    /// Provide a custom Rustls TLS configuration (requires 'rustls-tls' feature)
    #[cfg(feature = "rustls-tls")]
    pub fn rustls_tls_config(mut self, config: RustlsTlsConfig) -> Self {
        self.rustls_tls_config = Some(config);
        self.tls_backend = Some(TlsBackend::Rustls);
        self
    }

    /// Explicitly select TLS backend (requires at least one TLS feature)
    #[cfg(any(feature = "tls", feature = "rustls-tls"))]
    pub fn tls_backend(mut self, backend: TlsBackend) -> Self {
        self.tls_backend = Some(backend);
        self
    }

    // --------------------------
    // Reconnection configuration
    // --------------------------

    /// Set the base delay for reconnection backoff in milliseconds
    ///
    /// The actual delay uses exponential backoff: `base_delay * 2^attempts`
    ///
    /// # Example
    /// ```
    /// use flowsdk::mqtt_client::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::builder()
    ///     .reconnect_base_delay_ms(2000)  // Start with 2 second delay
    ///     .build();
    /// ```
    pub fn reconnect_base_delay_ms(mut self, delay_ms: u64) -> Self {
        self.reconnect_base_delay_ms = delay_ms;
        self
    }

    /// Set the maximum delay for reconnection backoff in milliseconds
    ///
    /// Caps the exponential backoff to prevent excessive delays.
    ///
    /// # Example
    /// ```
    /// use flowsdk::mqtt_client::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::builder()
    ///     .reconnect_max_delay_ms(120000)  // Cap at 2 minutes
    ///     .build();
    /// ```
    pub fn reconnect_max_delay_ms(mut self, delay_ms: u64) -> Self {
        self.reconnect_max_delay_ms = delay_ms;
        self
    }

    /// Set the maximum number of reconnection attempts
    ///
    /// - `0`: Unlimited attempts (will keep trying forever)
    /// - `n`: Stop after n failed attempts
    ///
    /// # Example
    /// ```
    /// use flowsdk::mqtt_client::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::builder()
    ///     .max_reconnect_attempts(5)  // Try 5 times then give up
    ///     .build();
    /// ```
    pub fn max_reconnect_attempts(mut self, attempts: u32) -> Self {
        self.max_reconnect_attempts = attempts;
        self
    }

    // --------------------------
    // Timeout configuration
    // --------------------------

    /// Set the retransmission timeout for QoS 1/2 messages in milliseconds
    ///
    /// How long to wait before resending unacknowledged messages.
    ///
    /// # Example
    /// ```
    /// use flowsdk::mqtt_client::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::builder()
    ///     .retransmission_timeout_ms(10000)  // 10 seconds
    ///     .build();
    /// ```
    pub fn retransmission_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.retransmission_timeout_ms = timeout_ms;
        self
    }

    /// Set the ping timeout multiplier
    ///
    /// Connection is considered dead if no packet received for `keep_alive * multiplier`.
    ///
    /// # Example
    /// ```
    /// use flowsdk::mqtt_client::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::builder()
    ///     .keep_alive(60)
    ///     .ping_timeout_multiplier(3)  // Timeout after 180 seconds
    ///     .build();
    /// ```
    pub fn ping_timeout_multiplier(mut self, multiplier: u32) -> Self {
        self.ping_timeout_multiplier = multiplier;
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
        assert_eq!(options.mqtt_version, 5);
        assert!(!options.sessionless);
        assert!(options.auto_ack);
        assert_eq!(options.session_expiry_interval, None);
        assert_eq!(options.maximum_packet_size, None);
        assert_eq!(options.request_response_information, None);
        assert_eq!(options.request_problem_information, None);
        #[cfg(feature = "tls")]
        assert!(options.tls_config.is_none());
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

    // ==================== TLS Configuration Tests ====================

    #[test]
    #[cfg(feature = "tls")]
    fn test_tls_config_default() {
        let options = MqttClientOptions::default();
        assert!(options.tls_config.is_none());
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_enable_tls() {
        let options = MqttClientOptions::builder().enable_tls().build();

        assert!(options.tls_config.is_some());
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_disable_tls() {
        let options = MqttClientOptions::builder()
            .enable_tls()
            .disable_tls()
            .build();

        assert!(options.tls_config.is_none());
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_tls_config_custom() {
        use crate::mqtt_client::transport::tls::TlsConfig;

        let tls_config = TlsConfig::builder()
            .danger_accept_invalid_certs(true)
            .build();

        let options = MqttClientOptions::builder().tls_config(tls_config).build();

        assert!(options.tls_config.is_some());
        let config = options.tls_config.unwrap();
        assert!(config.accept_invalid_certs);
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_tls_with_mqtts_url() {
        let options = MqttClientOptions::builder()
            .peer("mqtts://broker.example.com:8883")
            .enable_tls()
            .build();

        assert_eq!(options.peer, "mqtts://broker.example.com:8883");
        assert!(options.tls_config.is_some());
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_builder_chaining_with_tls() {
        use crate::mqtt_client::transport::tls::TlsConfig;

        let tls_config = TlsConfig::builder()
            .danger_accept_invalid_hostnames(true)
            .build();

        let options = MqttClientOptions::builder()
            .peer("mqtts://secure.example.com:8883")
            .client_id("tls_client")
            .username("user")
            .password(b"pass".to_vec())
            .tls_config(tls_config)
            .maximum_packet_size(65536)
            .build();

        assert_eq!(options.peer, "mqtts://secure.example.com:8883");
        assert_eq!(options.client_id, "tls_client");
        assert!(options.tls_config.is_some());
        let config = options.tls_config.unwrap();
        assert!(config.accept_invalid_hostnames);
    }

    // ==================== MQTT Version Tests ====================

    #[test]
    fn test_mqtt_version_default() {
        let options = MqttClientOptions::default();
        assert_eq!(options.mqtt_version, 5);
    }

    #[test]
    fn test_mqtt_version_v3() {
        let options = MqttClientOptions::builder().mqtt_version(3).build();
        assert_eq!(options.mqtt_version, 3);
    }

    #[test]
    fn test_mqtt_version_v4() {
        let options = MqttClientOptions::builder().mqtt_version(4).build();
        assert_eq!(options.mqtt_version, 4);
    }

    #[test]
    fn test_mqtt_version_v5() {
        let options = MqttClientOptions::builder().mqtt_version(5).build();
        assert_eq!(options.mqtt_version, 5);
    }

    #[test]
    #[should_panic(expected = "mqtt_version must be 3, 4, or 5")]
    fn test_mqtt_version_invalid() {
        MqttClientOptions::builder().mqtt_version(2).build();
    }

    #[test]
    #[should_panic(expected = "mqtt_version must be 3, 4, or 5")]
    fn test_mqtt_version_invalid_high() {
        MqttClientOptions::builder().mqtt_version(6).build();
    }
}
