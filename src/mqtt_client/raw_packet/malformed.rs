//! Malformed packet generators for protocol compliance testing
//!
//! ⚠️ **DANGER**: All functions create packets that violate the MQTT specification.
//! Only use for testing server rejection of malformed packets.

use crate::mqtt_client::raw_packet::RawPacketBuilder;
use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::mqttv5::connect::MqttConnect;
use crate::mqtt_serde::mqttv5::publish::MqttPublish;
use crate::mqtt_serde::mqttv5::subscribe::MqttSubscribe;
use std::io;

/// Generator for malformed MQTT packets
///
/// Provides methods to create packets that violate specific MQTT-5.0 normative statements.
/// Each method documents which statement it violates.
///
/// # Example
///
/// ```no_run
/// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
/// # fn example() -> std::io::Result<()> {
/// // Create CONNECT with reserved bit set (violates MQTT-2.1.3-1)
/// let malformed = MalformedPacketGenerator::connect_reserved_flag()?;
/// assert_eq!(malformed[0] & 0x0F, 0x01);
/// # Ok(())
/// # }
/// ```
pub struct MalformedPacketGenerator;

impl MalformedPacketGenerator {
    /// Create CONNECT packet with reserved flag bit set
    ///
    /// **Violates**: MQTT-2.1.3-1
    /// > "Bits 3,2,1 and 0 of Byte 1 in the CONNECT packet are Reserved and MUST all be set to 0"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// let packet = MalformedPacketGenerator::connect_reserved_flag()?;
    /// // Packet has bit 0 set in fixed header
    /// # Ok(())
    /// # }
    /// ```
    pub fn connect_reserved_flag() -> io::Result<Vec<u8>> {
        let connect = MqttConnect::new(
            "test_client".to_string(),
            None,
            None,
            None,
            60,
            true,
            vec![],
        );
        let mut builder = RawPacketBuilder::from_packet(MqttPacket::Connect5(connect))?;
        builder.set_fixed_header_flags(0x01); // Set reserved bit
        Ok(builder.build())
    }

    /// Create packet with non-minimal variable length encoding
    ///
    /// **Violates**: MQTT-1.5.5-1
    /// > "The encoded value MUST use the minimum number of bytes necessary to represent the value"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::non_minimal_variable_length()?;
    /// // Remaining length uses more bytes than needed
    /// # Ok(())
    /// # }
    /// ```
    pub fn non_minimal_variable_length() -> io::Result<Vec<u8>> {
        // Create PINGREQ with non-minimal encoding
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0]); // PINGREQ packet type
        builder.append_bytes(&[0x80, 0x00]); // Remaining length 0 with continuation bit
        Ok(builder.build())
    }

    /// Create QoS 0 PUBLISH with packet identifier
    ///
    /// **Violates**: MQTT-2.2.1-2
    /// > "A PUBLISH packet MUST NOT contain a Packet Identifier if its QoS value is set to 0"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = gen.qos0_with_packet_id()?;
    /// // QoS 0 PUBLISH incorrectly includes packet ID
    /// # Ok(())
    /// # }
    /// ```
    pub fn qos0_with_packet_id() -> io::Result<Vec<u8>> {
        let mut publish = MqttPublish::new(0, "test".to_string(), None, vec![], false, false);
        publish.topic_name = "test".to_string();
        publish.qos = 0;
        publish.payload = vec![];

        let mut builder = RawPacketBuilder::from_packet(MqttPacket::Publish5(publish))?;

        // Insert packet ID (0x0001) after topic name
        // Topic name is 2 bytes length + 4 bytes "test" + 1 byte properties length = 7 bytes
        // Fixed header is 1 byte type + 1 byte remaining length = 2 bytes
        builder.insert_bytes(9, &[0x00, 0x01]); // Insert packet ID
        Ok(builder.build())
    }

    /// Create packet with invalid QoS (both bits set to 1)
    ///
    /// **Violates**: MQTT-3.3.1-4
    /// > "If the QoS bits are set to 1 and 1, this is a Malformed Packet"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::invalid_qos_both_bits()?;
    /// // QoS bits are 11 (invalid)
    /// # Ok(())
    /// # }
    /// ```
    pub fn invalid_qos_both_bits() -> io::Result<Vec<u8>> {
        let mut publish = MqttPublish::new(1, "test".to_string(), Some(1), vec![], false, false);
        publish.topic_name = "test".to_string();
        publish.qos = 1;
        publish.packet_id = Some(1);
        publish.payload = vec![];

        let mut builder = RawPacketBuilder::from_packet(MqttPacket::Publish5(publish))?;

        // Set QoS bits to 11 (0x06 in fixed header flags)
        let current_flags = builder.bytes()[0] & 0x0F;
        let new_flags = (current_flags & !0x06) | 0x06;
        builder.set_fixed_header_flags(new_flags);
        Ok(builder.build())
    }

    /// Create PUBLISH with topic alias set to zero
    ///
    /// **Violates**: MQTT-3.3.2.3.4-1
    /// > "A Topic Alias value of 0 is not permitted"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::topic_alias_zero()?;
    /// // Topic Alias property set to 0
    /// # Ok(())
    /// # }
    /// ```
    pub fn topic_alias_zero() -> io::Result<Vec<u8>> {
        let mut builder = RawPacketBuilder::new();

        // Build PUBLISH manually with Topic Alias = 0
        builder.append_bytes(&[0x30]); // PUBLISH, QoS 0
        builder.append_bytes(&[0x0C]); // Remaining length (will adjust)
        builder.append_bytes(&[0x00, 0x04]); // Topic name length
        builder.append_bytes(b"test"); // Topic name
        builder.append_bytes(&[0x03]); // Properties length
        builder.append_bytes(&[0x23]); // Topic Alias property ID
        builder.append_bytes(&[0x00, 0x00]); // Topic Alias = 0 (INVALID)

        Ok(builder.build())
    }

    /// Create CONNECT with Will Flag set but no Will Payload
    ///
    /// **Violates**: MQTT-3.1.2.8-1
    /// > "If the Will Flag is set to 1, the Will Properties, Will Topic and Will Payload fields MUST be present"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::will_flag_without_payload()?;
    /// // Will Flag set but payload missing
    /// # Ok(())
    /// # }
    /// ```
    pub fn will_flag_without_payload() -> io::Result<Vec<u8>> {
        use crate::mqtt_serde::mqttv5::will::Will;

        // Create connect with will
        let will = Will::new("will/topic".to_string(), vec![1, 2, 3], 0, false);

        let connect = MqttConnect::new(
            "test_client".to_string(),
            None,
            None,
            Some(will),
            60,
            true,
            vec![],
        );

        let mut builder = RawPacketBuilder::from_packet(MqttPacket::Connect5(connect))?;

        // Find and remove will payload
        // This is a simplified version - we truncate to remove the will message bytes
        let bytes = builder.bytes_mut();
        // Remove the last 5 bytes (2 bytes length + 3 bytes payload)
        let new_len = bytes.len().saturating_sub(5);
        builder.truncate(new_len);

        Ok(builder.build())
    }

    /// Create CONNECT with incorrect protocol name
    ///
    /// **Violates**: MQTT-3.1.2.1-1
    /// > "The Server MUST validate that the Protocol Name is valid"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::incorrect_protocol_name()?;
    /// // Protocol name is "WRONG" instead of "MQTT"
    /// # Ok(())
    /// # }
    /// ```
    pub fn incorrect_protocol_name() -> io::Result<Vec<u8>> {
        let connect = MqttConnect::new(
            "test_client".to_string(),
            None,
            None,
            None,
            60,
            true,
            vec![],
        );
        let mut builder = RawPacketBuilder::from_packet(MqttPacket::Connect5(connect))?;

        // Replace "MQTT" with "WRONG" in protocol name
        // Protocol name starts at byte 4 (after fixed header and remaining length)
        builder.set_byte(4, 0x00); // Length MSB
        builder.set_byte(5, 0x05); // Length LSB (5 bytes)
        builder.set_byte(6, b'W');
        builder.set_byte(7, b'R');
        builder.set_byte(8, b'O');
        builder.set_byte(9, b'N');
        builder.set_byte(10, b'G');

        Ok(builder.build())
    }

    /// Create second CONNECT packet on same connection
    ///
    /// **Violates**: MQTT-3.1.0-2
    /// > "A Client can only send the CONNECT packet once over a Network Connection"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let first = MalformedPacketGenerator::valid_connect()?;
    /// let second = MalformedPacketGenerator::second_connect()?;
    /// // Send first, then second on same connection
    /// # Ok(())
    /// # }
    /// ```
    pub fn second_connect() -> io::Result<Vec<u8>> {
        // Just return a normal CONNECT - the violation is sending it twice
        let connect = MqttConnect::new(
            "test_client".to_string(),
            None,
            None,
            None,
            60,
            true,
            vec![],
        );
        let builder = RawPacketBuilder::from_packet(MqttPacket::Connect5(connect))?;
        Ok(builder.build())
    }

    /// Create SUBSCRIBE with reserved bits set in Subscription Options
    ///
    /// **Violates**: MQTT-3.8.3.1-2
    /// > "Bits 6 and 7 of the Subscription Options byte are reserved for future use. The Server MUST treat a SUBSCRIBE packet as malformed if any of Reserved bits are non-zero"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::subscribe_reserved_bits()?;
    /// // Subscription options has reserved bits set
    /// # Ok(())
    /// # }
    /// ```
    pub fn subscribe_reserved_bits() -> io::Result<Vec<u8>> {
        use crate::mqtt_serde::mqttv5::subscribe::TopicSubscription;

        let subscription = TopicSubscription::new_simple("test/topic".to_string(), 0);

        let subscribe = MqttSubscribe::new(1, vec![subscription], vec![]);

        let mut builder = RawPacketBuilder::from_packet(MqttPacket::Subscribe5(subscribe))?;

        // Find subscription options byte and set reserved bits
        let bytes = builder.bytes_mut();
        let len = bytes.len();
        if len > 0 {
            bytes[len - 1] |= 0xC0; // Set bits 6 and 7
        }

        Ok(builder.build())
    }

    /// Create PUBLISH with Subscription Identifier = 0
    ///
    /// **Violates**: MQTT-3.3.4-6
    /// > "The value of the Subscription Identifier MUST be greater than 0"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::subscription_identifier_zero()?;
    /// // Subscription Identifier property set to 0
    /// # Ok(())
    /// # }
    /// ```
    pub fn subscription_identifier_zero() -> io::Result<Vec<u8>> {
        let mut builder = RawPacketBuilder::new();

        // Build PUBLISH with Subscription Identifier = 0
        builder.append_bytes(&[0x30]); // PUBLISH, QoS 0
        builder.append_bytes(&[0x0B]); // Remaining length
        builder.append_bytes(&[0x00, 0x04]); // Topic name length
        builder.append_bytes(b"test"); // Topic name
        builder.append_bytes(&[0x02]); // Properties length
        builder.append_bytes(&[0x0B]); // Subscription Identifier property ID
        builder.append_bytes(&[0x00]); // Subscription Identifier = 0 (INVALID)

        Ok(builder.build())
    }

    /// Create CONNACK with invalid protocol version
    ///
    /// **Violates**: MQTT-3.2.2.2-2
    /// > "If the Server does not support the version of the MQTT protocol requested by the Client, the Server MUST respond with a CONNACK packet containing the Reason Code of 0x84 (Unsupported Protocol Version)"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::invalid_protocol_version()?;
    /// // Protocol version set to invalid value
    /// # Ok(())
    /// # }
    /// ```
    pub fn invalid_protocol_version() -> io::Result<Vec<u8>> {
        let connect = MqttConnect::new(
            "test_client".to_string(),
            None,
            None,
            None,
            60,
            true,
            vec![],
        );
        let mut builder = RawPacketBuilder::from_packet(MqttPacket::Connect5(connect))?;

        // Change protocol version from 5 to invalid value (e.g., 99)
        // Protocol version is at byte 11 (after protocol name length and name)
        builder.set_byte(11, 99);

        Ok(builder.build())
    }

    /// Create packet with remaining length exceeding maximum
    ///
    /// **Violates**: MQTT-2.1.4-1
    /// > "The maximum number of bytes in the Remaining Length field is four"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::remaining_length_too_large()?;
    /// // Remaining length field has 5 bytes (invalid)
    /// # Ok(())
    /// # }
    /// ```
    pub fn remaining_length_too_large() -> io::Result<Vec<u8>> {
        let mut builder = RawPacketBuilder::new();

        // Create packet with 5-byte remaining length (invalid)
        builder.append_bytes(&[0xC0]); // PINGREQ
        builder.append_bytes(&[0x80, 0x80, 0x80, 0x80, 0x01]); // 5 bytes with continuation bits

        Ok(builder.build())
    }

    /// Create PUBLISH with duplicate Topic Alias properties
    ///
    /// **Violates**: MQTT-3.3.2.3-1
    /// > "If the Topic Alias or Subscription Identifier properties are included more than once, the behavior is undefined"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::duplicate_topic_alias()?;
    /// // Topic Alias property appears twice
    /// # Ok(())
    /// # }
    /// ```
    pub fn duplicate_topic_alias() -> io::Result<Vec<u8>> {
        let mut builder = RawPacketBuilder::new();

        // Build PUBLISH with duplicate Topic Alias properties
        builder.append_bytes(&[0x30]); // PUBLISH, QoS 0
        builder.append_bytes(&[0x10]); // Remaining length
        builder.append_bytes(&[0x00, 0x04]); // Topic name length
        builder.append_bytes(b"test"); // Topic name
        builder.append_bytes(&[0x07]); // Properties length
        builder.append_bytes(&[0x23]); // Topic Alias property ID
        builder.append_bytes(&[0x00, 0x01]); // Topic Alias = 1
        builder.append_bytes(&[0x23]); // Topic Alias property ID (DUPLICATE)
        builder.append_bytes(&[0x00, 0x02]); // Topic Alias = 2

        Ok(builder.build())
    }

    /// Create CONNECT with Client ID longer than allowed
    ///
    /// **Violates**: Server-specific limits on Client ID length
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::client_id_too_long()?;
    /// // Client ID is 65536 characters long
    /// # Ok(())
    /// # }
    /// ```
    pub fn client_id_too_long() -> io::Result<Vec<u8>> {
        let mut connect = MqttConnect::new(
            "test_client".to_string(),
            None,
            None,
            None,
            60,
            true,
            vec![],
        );
        connect.client_id = "A".repeat(65536); // Extremely long client ID

        let builder = RawPacketBuilder::from_packet(MqttPacket::Connect5(connect))?;
        Ok(builder.build())
    }

    /// Create PUBLISH with empty topic name (when topic alias not used)
    ///
    /// **Violates**: MQTT-3.3.2.1-1
    /// > "The Topic Name in the PUBLISH packet MUST NOT contain wildcard characters"
    /// > "The Topic Name MUST be a UTF-8 Encoded String"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::empty_topic_name()?;
    /// // Topic name is empty and no Topic Alias
    /// # Ok(())
    /// # }
    /// ```
    pub fn empty_topic_name() -> io::Result<Vec<u8>> {
        let mut builder = RawPacketBuilder::new();

        // Build PUBLISH with empty topic name
        builder.append_bytes(&[0x30]); // PUBLISH, QoS 0
        builder.append_bytes(&[0x03]); // Remaining length
        builder.append_bytes(&[0x00, 0x00]); // Topic name length = 0 (INVALID)
        builder.append_bytes(&[0x00]); // Properties length = 0

        Ok(builder.build())
    }

    /// Create PUBLISH with wildcard characters in topic name
    ///
    /// **Violates**: MQTT-3.3.2.1-1
    /// > "The Topic Name in the PUBLISH packet MUST NOT contain wildcard characters"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::topic_with_wildcards()?;
    /// // Topic name contains '#' or '+'
    /// # Ok(())
    /// # }
    /// ```
    pub fn topic_with_wildcards() -> io::Result<Vec<u8>> {
        let mut publish = MqttPublish::new(0, "test".to_string(), None, vec![], false, false);
        publish.topic_name = "test/+/topic/#".to_string(); // Invalid wildcards
        publish.qos = 0;
        publish.payload = vec![];

        let builder = RawPacketBuilder::from_packet(MqttPacket::Publish5(publish))?;
        Ok(builder.build())
    }

    /// Create packet with reserved packet type
    ///
    /// **Violates**: MQTT-2.1.2-1
    /// > "If invalid flags are received it is a Malformed Packet. The Server MUST close the Network Connection"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let packet = MalformedPacketGenerator::reserved_packet_type()?;
    /// // Packet type is 0 or 15 (reserved)
    /// # Ok(())
    /// # }
    /// ```
    pub fn reserved_packet_type() -> io::Result<Vec<u8>> {
        let mut builder = RawPacketBuilder::new();

        // Create packet with reserved type 0
        builder.append_bytes(&[0x00, 0x00]); // Type 0, Remaining length 0

        Ok(builder.build())
    }

    /// Create valid CONNECT for testing (not malformed)
    ///
    /// This is useful as a baseline for comparison.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let valid = MalformedPacketGenerator::valid_connect()?;
    /// let malformed = MalformedPacketGenerator::connect_reserved_flag()?;
    /// // Compare valid vs malformed
    /// # Ok(())
    /// # }
    /// ```
    pub fn valid_connect() -> io::Result<Vec<u8>> {
        let connect = MqttConnect::new(
            "test_client".to_string(),
            None,
            None,
            None,
            60,
            true,
            vec![],
        );
        let builder = RawPacketBuilder::from_packet(MqttPacket::Connect5(connect))?;
        Ok(builder.build())
    }

    /// Create valid PINGREQ for testing (not malformed)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::malformed::MalformedPacketGenerator;
    /// # fn example() -> std::io::Result<()> {
    /// // Static methods - no instance needed
    /// let ping = MalformedPacketGenerator::valid_pingreq()?;
    /// assert_eq!(ping, vec![0xC0, 0x00]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn valid_pingreq() -> io::Result<Vec<u8>> {
        Ok(vec![0xC0, 0x00])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_reserved_flag() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::connect_reserved_flag().unwrap();

        // First byte should have packet type 1 (CONNECT) and reserved bit set
        assert_eq!(packet[0] & 0xF0, 0x10); // CONNECT type
        assert_eq!(packet[0] & 0x0F, 0x01); // Reserved bit set
    }

    #[test]
    fn test_non_minimal_variable_length() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::non_minimal_variable_length().unwrap();

        // Should be PINGREQ with non-minimal remaining length
        assert_eq!(packet[0], 0xC0); // PINGREQ
        assert_eq!(packet[1], 0x80); // Continuation bit set
        assert_eq!(packet[2], 0x00); // Value 0
    }

    #[test]
    fn test_invalid_qos_both_bits() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::invalid_qos_both_bits().unwrap();

        // QoS bits should be 11 (0x06 in flags)
        let qos_bits = (packet[0] >> 1) & 0x03;
        assert_eq!(qos_bits, 0x03);
    }

    #[test]
    fn test_topic_alias_zero() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::topic_alias_zero().unwrap();

        // Packet should contain Topic Alias property (0x23) with value 0
        assert!(packet.len() > 10);
        // Find property ID 0x23
        let mut found = false;
        for i in 0..packet.len() - 2 {
            if packet[i] == 0x23 && packet[i + 1] == 0x00 && packet[i + 2] == 0x00 {
                found = true;
                break;
            }
        }
        assert!(found, "Topic Alias property with value 0 not found");
    }

    #[test]
    fn test_incorrect_protocol_name() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::incorrect_protocol_name().unwrap();

        // Protocol name should be "WRONG" not "MQTT"
        assert_eq!(packet[5], 0x05); // Length 5
        assert_eq!(&packet[6..11], b"WRONG");
    }

    #[test]
    fn test_second_connect() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::second_connect().unwrap();

        // Should be a valid CONNECT packet
        assert_eq!(packet[0] & 0xF0, 0x10);
    }

    #[test]
    fn test_remaining_length_too_large() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::remaining_length_too_large().unwrap();

        // Should have 5 bytes for remaining length
        assert_eq!(packet[0], 0xC0); // PINGREQ
        assert_eq!(packet.len(), 6); // 1 + 5 bytes
    }

    #[test]
    fn test_duplicate_topic_alias() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::duplicate_topic_alias().unwrap();

        // Count occurrences of property ID 0x23 (Topic Alias)
        let count = packet.windows(1).filter(|w| w[0] == 0x23).count();
        assert_eq!(count, 2, "Should have 2 Topic Alias properties");
    }

    #[test]
    fn test_empty_topic_name() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::empty_topic_name().unwrap();

        // Topic name length should be 0
        assert_eq!(packet[2], 0x00);
        assert_eq!(packet[3], 0x00);
    }

    #[test]
    fn test_topic_with_wildcards() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::topic_with_wildcards().unwrap();

        // Packet should contain '+' or '#'
        let has_wildcard = packet.iter().any(|&b| b == b'+' || b == b'#');
        assert!(has_wildcard);
    }

    #[test]
    fn test_reserved_packet_type() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::reserved_packet_type().unwrap();

        // Packet type should be 0
        assert_eq!(packet[0] & 0xF0, 0x00);
    }

    #[test]
    fn test_valid_connect() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::valid_connect().unwrap();

        // Should be valid CONNECT
        assert_eq!(packet[0], 0x10); // CONNECT with no reserved bits
    }

    #[test]
    fn test_valid_pingreq() {
        // Static methods - no instance needed
        let packet = MalformedPacketGenerator::valid_pingreq().unwrap();

        assert_eq!(packet, vec![0xC0, 0x00]);
    }
}
