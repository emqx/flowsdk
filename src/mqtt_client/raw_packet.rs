//! Raw Packet API - FOR TESTING ONLY
//!
//! ⚠️ **DANGER**: This module provides dangerous low-level packet access.
//! Only use for protocol compliance testing. **DO NOT use in production.**
//!
//! This module is only available when the `protocol-testing` feature is enabled.
//!
//! # Safety
//!
//! The APIs in this module bypass all validation and can create malformed packets
//! that violate the MQTT specification. They are designed exclusively for testing
//! server behavior with protocol violations, reserved bit handling, and malformed
//! packet rejection.
//!
//! # Example
//!
//! ```no_run
//! use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
//! use flowsdk::mqtt_serde::control_packet::MqttPacket;
//! use flowsdk::mqtt_serde::mqttv5::connectv5::MqttConnect;
//!
//! # async fn example() -> std::io::Result<()> {
//! // Create malformed CONNECT with reserved bit set
//! let connect = MqttConnect::default();
//! let mut builder = RawPacketBuilder::from_packet(
//!     MqttPacket::Connect5(connect)
//! )?;
//!
//! // Violate MQTT-2.1.3-1 by setting reserved flag bit
//! builder.set_fixed_header_flags(0x01);
//!
//! let malformed_packet = builder.build();
//! # Ok(())
//! # }
//! ```

use crate::mqtt_serde::control_packet::MqttPacket;
use std::io;

#[cfg(feature = "protocol-testing")]
pub mod malformed;

#[cfg(feature = "protocol-testing")]
pub mod test_client;

/// Raw packet builder for crafting arbitrary MQTT packets
///
/// ⚠️ **DANGER**: Can create malformed packets that violate MQTT spec.
/// Only use for protocol compliance testing.
///
/// # Features
///
/// - Start from a valid packet or build from scratch
/// - Modify specific bytes at any position
/// - Corrupt headers, flags, variable lengths
/// - Insert/remove bytes
/// - Full control over packet structure
///
/// # Example
///
/// ```no_run
/// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
/// # use flowsdk::mqtt_serde::control_packet::MqttPacket;
/// # use flowsdk::mqtt_serde::mqttv5::connectv5::MqttConnect;
/// # fn example() -> std::io::Result<()> {
/// let connect = MqttConnect::default();
/// let mut builder = RawPacketBuilder::from_packet(
///     MqttPacket::Connect5(connect)
/// )?;
///
/// // Corrupt the packet
/// builder.set_fixed_header_flags(0x0F); // Set all reserved bits
/// builder.set_byte(5, 0xFF); // Corrupt variable header
///
/// let malformed = builder.build();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RawPacketBuilder {
    bytes: Vec<u8>,
}

impl RawPacketBuilder {
    /// Create a new empty raw packet builder
    ///
    /// Use this to build packets from scratch.
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]); // PINGREQ
    /// let packet = builder.build();
    /// ```
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Create a raw packet builder from a valid packet
    ///
    /// This starts with a correctly encoded packet that you can then corrupt.
    ///
    /// # Arguments
    ///
    /// * `packet` - A valid MQTT packet to use as the starting point
    ///
    /// # Errors
    ///
    /// Returns error if the packet cannot be encoded.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// # use flowsdk::mqtt_serde::control_packet::MqttPacket;
    /// # use flowsdk::mqtt_serde::mqttv5::pingreqv5::MqttPingReq;
    /// # fn example() -> std::io::Result<()> {
    /// let ping = MqttPingReq {};
    /// let builder = RawPacketBuilder::from_packet(
    ///     MqttPacket::PingReq5(ping)
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_packet(packet: MqttPacket) -> io::Result<Self> {
        let bytes = packet
            .to_bytes()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Self { bytes })
    }

    /// Set a specific byte at the given position
    ///
    /// # Arguments
    ///
    /// * `index` - Position of the byte to modify (0-based)
    /// * `value` - New byte value
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]);
    /// builder.set_byte(0, 0xC1); // Modify packet type
    /// ```
    pub fn set_byte(&mut self, index: usize, value: u8) -> &mut Self {
        if index < self.bytes.len() {
            self.bytes[index] = value;
        }
        self
    }

    /// Modify the fixed header flags (lower 4 bits of first byte)
    ///
    /// This is useful for testing reserved bit handling.
    ///
    /// # Arguments
    ///
    /// * `flags` - New flags value (only lower 4 bits are used)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// # use flowsdk::mqtt_serde::control_packet::MqttPacket;
    /// # use flowsdk::mqtt_serde::mqttv5::connectv5::MqttConnect;
    /// # fn example() -> std::io::Result<()> {
    /// let connect = MqttConnect::default();
    /// let mut builder = RawPacketBuilder::from_packet(
    ///     MqttPacket::Connect5(connect)
    /// )?;
    ///
    /// // Set reserved flag bit (MQTT-2.1.3-1 violation)
    /// builder.set_fixed_header_flags(0x01);
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_fixed_header_flags(&mut self, flags: u8) -> &mut Self {
        if !self.bytes.is_empty() {
            self.bytes[0] = (self.bytes[0] & 0xF0) | (flags & 0x0F);
        }
        self
    }

    /// Modify the packet type (upper 4 bits of first byte)
    ///
    /// # Arguments
    ///
    /// * `packet_type` - New packet type value (0-15)
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]);
    /// builder.set_packet_type(0x0D); // Change to PINGRESP
    /// ```
    pub fn set_packet_type(&mut self, packet_type: u8) -> &mut Self {
        if !self.bytes.is_empty() {
            self.bytes[0] = (packet_type << 4) | (self.bytes[0] & 0x0F);
        }
        self
    }

    /// Corrupt variable length encoding by setting MSB on a final byte
    ///
    /// This creates a non-minimal encoding (MQTT-1.5.5-1 violation).
    ///
    /// # Arguments
    ///
    /// * `position` - Position of the variable length byte to corrupt
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]); // PINGREQ
    /// builder.corrupt_variable_length(1); // Make it non-minimal
    /// ```
    pub fn corrupt_variable_length(&mut self, position: usize) -> &mut Self {
        if position < self.bytes.len() {
            // Set MSB to indicate continuation (non-minimal encoding)
            self.bytes[position] |= 0x80;
        }
        self
    }

    /// Insert bytes at a specific position
    ///
    /// # Arguments
    ///
    /// * `position` - Where to insert the bytes
    /// * `bytes` - Bytes to insert
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// # use flowsdk::mqtt_serde::control_packet::MqttPacket;
    /// # use flowsdk::mqtt_serde::mqttv5::publishv5::MqttPublish;
    /// # fn example() -> std::io::Result<()> {
    /// let publish = MqttPublish::default();
    /// let mut builder = RawPacketBuilder::from_packet(
    ///     MqttPacket::Publish5(publish)
    /// )?;
    ///
    /// // Insert packet ID into QoS 0 PUBLISH (MQTT-2.2.1-2 violation)
    /// builder.insert_bytes(10, &[0x00, 0x01]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn insert_bytes(&mut self, position: usize, bytes: &[u8]) -> &mut Self {
        self.bytes.splice(position..position, bytes.iter().copied());
        self
    }

    /// Remove bytes at a specific position
    ///
    /// # Arguments
    ///
    /// * `position` - Where to start removing
    /// * `count` - Number of bytes to remove
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00, 0xFF, 0xFF]);
    /// builder.remove_bytes(2, 2); // Remove extra bytes
    /// ```
    pub fn remove_bytes(&mut self, position: usize, count: usize) -> &mut Self {
        let end = (position + count).min(self.bytes.len());
        self.bytes.drain(position..end);
        self
    }

    /// Truncate packet to a specific length
    ///
    /// Useful for creating incomplete packets.
    ///
    /// # Arguments
    ///
    /// * `len` - New length of the packet
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// # use flowsdk::mqtt_serde::control_packet::MqttPacket;
    /// # use flowsdk::mqtt_serde::mqttv5::connectv5::MqttConnect;
    /// # fn example() -> std::io::Result<()> {
    /// let connect = MqttConnect::default();
    /// let mut builder = RawPacketBuilder::from_packet(
    ///     MqttPacket::Connect5(connect)
    /// )?;
    ///
    /// // Create incomplete packet
    /// builder.truncate(10);
    /// # Ok(())
    /// # }
    /// ```
    pub fn truncate(&mut self, len: usize) -> &mut Self {
        self.bytes.truncate(len);
        self
    }

    /// Append raw bytes to the end of the packet
    ///
    /// # Arguments
    ///
    /// * `bytes` - Bytes to append
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0]); // PINGREQ packet type
    /// builder.append_bytes(&[0x00]); // Remaining length
    /// ```
    pub fn append_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// Get mutable reference to the internal byte buffer
    ///
    /// This provides direct access for advanced manipulation.
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]);
    ///
    /// // Direct manipulation
    /// let bytes = builder.bytes_mut();
    /// bytes[0] = 0xD0; // Change to PINGRESP
    /// ```
    pub fn bytes_mut(&mut self) -> &mut Vec<u8> {
        &mut self.bytes
    }

    /// Get reference to the internal byte buffer
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]);
    ///
    /// assert_eq!(builder.bytes(), &[0xC0, 0x00]);
    /// ```
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Build the final raw packet
    ///
    /// Consumes the builder and returns the packet bytes.
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]);
    /// let packet = builder.build();
    /// assert_eq!(packet, vec![0xC0, 0x00]);
    /// ```
    pub fn build(self) -> Vec<u8> {
        self.bytes
    }

    /// Get the current length of the packet
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let mut builder = RawPacketBuilder::new();
    /// builder.append_bytes(&[0xC0, 0x00]);
    /// assert_eq!(builder.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Check if the packet is empty
    ///
    /// # Example
    ///
    /// ```
    /// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
    /// let builder = RawPacketBuilder::new();
    /// assert!(builder.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl Default for RawPacketBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_serde::mqttv5::pingreqv5::MqttPingReq;

    #[test]
    fn test_new_builder() {
        let builder = RawPacketBuilder::new();
        assert!(builder.is_empty());
        assert_eq!(builder.len(), 0);
    }

    #[test]
    fn test_from_packet() {
        let ping = MqttPingReq {};
        let builder = RawPacketBuilder::from_packet(MqttPacket::PingReq5(ping)).unwrap();

        assert_eq!(builder.len(), 2);
        assert_eq!(builder.bytes(), &[0xC0, 0x00]);
    }

    #[test]
    fn test_append_bytes() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0]);
        builder.append_bytes(&[0x00]);

        assert_eq!(builder.bytes(), &[0xC0, 0x00]);
    }

    #[test]
    fn test_set_byte() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0x00]);
        builder.set_byte(0, 0xD0);

        assert_eq!(builder.bytes()[0], 0xD0);
    }

    #[test]
    fn test_set_fixed_header_flags() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0x00]);
        builder.set_fixed_header_flags(0x0F);

        assert_eq!(builder.bytes()[0], 0xCF);
    }

    #[test]
    fn test_set_packet_type() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0x00]);
        builder.set_packet_type(0x0D);

        assert_eq!(builder.bytes()[0], 0xD0);
    }

    #[test]
    fn test_corrupt_variable_length() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0x00]);
        builder.corrupt_variable_length(1);

        assert_eq!(builder.bytes()[1], 0x80);
    }

    #[test]
    fn test_insert_bytes() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0x00]);
        builder.insert_bytes(1, &[0xFF, 0xFE]);

        assert_eq!(builder.bytes(), &[0xC0, 0xFF, 0xFE, 0x00]);
    }

    #[test]
    fn test_remove_bytes() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0xFF, 0xFE, 0x00]);
        builder.remove_bytes(1, 2);

        assert_eq!(builder.bytes(), &[0xC0, 0x00]);
    }

    #[test]
    fn test_truncate() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0x00, 0xFF, 0xFE]);
        builder.truncate(2);

        assert_eq!(builder.bytes(), &[0xC0, 0x00]);
    }

    #[test]
    fn test_build() {
        let mut builder = RawPacketBuilder::new();
        builder.append_bytes(&[0xC0, 0x00]);
        let packet = builder.build();

        assert_eq!(packet, vec![0xC0, 0x00]);
    }
}
