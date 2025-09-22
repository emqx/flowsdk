use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::parser::{parse_utf8_string, parse_vbi, ParseError, ParseOk};
use bytes::{Buf, BytesMut};

/// A stateful parser for a stream of MQTT data.
/// It internally buffers data from a stream and yields complete packets.
#[derive(Debug)]
pub struct MqttParser {
    buffer: BytesMut,
    // 0 means undefined for new, 4 means MQTT v3.1.1, 5 means MQTT v5.0
    mqtt_version: u8,
}

impl Default for MqttParser {
    fn default() -> Self {
        Self::new(16384, 0) // Default buffer size and MQTT version
    }
}

impl MqttParser {
    /// Creates a new, empty parser.
    pub fn new(buffer_size: usize, mqtt_version: u8) -> Self {
        MqttParser {
            buffer: BytesMut::with_capacity(buffer_size),
            mqtt_version,
        }
    }

    pub fn get_mqtt_vsn(&self) -> u8 {
        self.mqtt_version
    }

    /// Determines the MQTT version from the buffer if undefined
    pub fn set_mqtt_vsn(&mut self, mut offset: usize) -> Result<u8, ParseError> {
        if self.mqtt_version != 0 {
            return Ok(self.mqtt_version);
        }

        // precheck if it's a CONNECT packet
        if self.buffer[offset] != 0x10 {
            return Err(ParseError::ParseError(
                "Expected CONNECT packet to determine MQTT version".to_string(),
            ));
        }
        offset += 1;

        let (_len, consumed) = parse_vbi(&self.buffer[offset..])?;
        offset += consumed;

        let (_protocol_name, consumed) = parse_utf8_string(&self.buffer[offset..])?;

        let vsn_offset = offset + consumed;
        if self.buffer.len() < vsn_offset + 1 {
            return Err(ParseError::More(
                vsn_offset + 1 - self.buffer.len(),
                "MQTT version".to_string(),
            ));
        }

        self.mqtt_version = self.buffer[vsn_offset];
        Ok(self.mqtt_version)
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
        assert!(
            self.mqtt_version != 0,
            "MQTT version must be set before parsing packets"
        );
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
