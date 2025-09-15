use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::parser::{ParseError, ParseOk};
use bytes::{Buf, BytesMut};

/// A stateful parser for a stream of MQTT data.
/// It internally buffers data from a stream and yields complete packets.
#[derive(Debug)]
pub struct MqttParser {
    buffer: BytesMut,
    mqtt_version: u8,
}

impl Default for MqttParser {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttParser {
    /// Creates a new, empty parser.
    pub fn new() -> Self {
        MqttParser {
            buffer: BytesMut::with_capacity(4096), // Start with a reasonable capacity
            mqtt_version: 5,                       // Default to MQTT v5
        }
    }

    /// Appends new data from the stream to the internal buffer.
    pub fn feed(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Attempts to parse a single MQTT packet from the internal buffer.
    ///
    /// - If a full packet is available, it returns `Ok(Some(MqttPacket))`,
    ///   and the corresponding bytes are removed from the buffer.
    /// - If the buffer does not contain a full packet, it returns `Ok(None)`.
    /// - If the data in the buffer is malformed, it returns `Err(ParseError)`.
    pub fn next_packet(&mut self) -> Result<Option<MqttPacket>, ParseError> {
        match MqttPacket::from_bytes_with_version(&self.buffer, self.mqtt_version) {
            Ok(ParseOk::Packet(packet, consumed)) => {
                // A full packet was parsed, advance the buffer
                self.buffer.advance(consumed);
                Ok(Some(packet))
            }
            Ok(ParseOk::Continue(_, _)) => {
                // Not enough data in the buffer for a full packet
                Ok(None)
            }
            Err(e) => {
                // An unrecoverable parsing error occurred
                Err(e)
            }
            // This case should not be returned by a top-level parser
            Ok(ParseOk::TopicName(_, _)) => Err(ParseError::ParseError(
                "Unexpected ParseOk variant".to_string(),
            )),
        }
    }

    pub fn buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.buffer
    }
}
