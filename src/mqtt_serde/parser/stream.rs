// SPDX-License-Identifier: MPL-2.0

use crate::mqtt_serde::control_packet::MqttPacket;
use crate::mqtt_serde::parser::leveled::{
    packet_frame_len, parse_headers_only, parse_raw_body, parse_type_only, LeveledParseOk,
    ParseLevel, ParsedPacket,
};
use crate::mqtt_serde::parser::{parse_utf8_string, parse_vbi, ParseError, ParseOk};
use bytes::{Buf, BytesMut};

/// A stateful parser for a stream of MQTT data.
/// It internally buffers data from a stream and yields complete packets.
#[derive(Debug)]
pub struct MqttParser {
    buffer: BytesMut,
    // 0 means undefined for new, 4 means MQTT v3.1.1, 5 means MQTT v5.0
    mqtt_version: u8,
    parse_level: ParseLevel,
}

impl Default for MqttParser {
    fn default() -> Self {
        Self::new(16384, 0) // Default buffer size and MQTT version
    }
}

impl From<u8> for ParseLevel {
    fn from(value: u8) -> Self {
        match value {
            1 => ParseLevel::HeadersParsed,
            2 => ParseLevel::RawBody,
            3 => ParseLevel::TypeOnly,
            _ => ParseLevel::Full,
        }
    }
}

impl MqttParser {
    /// Creates a new, empty parser.
    pub fn new(buffer_size: usize, mqtt_version: u8) -> Self {
        MqttParser {
            buffer: BytesMut::with_capacity(buffer_size),
            mqtt_version,
            parse_level: ParseLevel::Full,
        }
    }

    /// Creates a new parser with the specified parse level.
    pub fn with_level(buffer_size: usize, mqtt_version: u8, parse_level: ParseLevel) -> Self {
        MqttParser {
            buffer: BytesMut::with_capacity(buffer_size),
            mqtt_version,
            parse_level,
        }
    }

    pub fn set_parse_level(&mut self, level: ParseLevel) {
        self.parse_level = level;
    }

    pub fn parse_level(&self) -> ParseLevel {
        self.parse_level
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
    /// - If the buffer is empty or does not contain a full packet, it returns `Ok(None)`,
    ///   indicating that more data is needed.
    /// - If the data in the buffer is malformed, it returns `Err(ParseError)`.
    pub fn next_packet(&mut self) -> Result<Option<MqttPacket>, ParseError> {
        assert!(
            self.mqtt_version != 0,
            "MQTT version must be set before parsing packets"
        );

        if self.buffer.is_empty() {
            return Ok(None);
        }

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

    /// Returns the next parsed result at the configured parse level.
    ///
    /// For Level 0 (`Full`), this is equivalent to wrapping `next_packet()` in `ParsedPacket::Full`.
    /// Higher levels skip progressively more parsing for better throughput.
    pub fn next_parsed(&mut self) -> Result<Option<ParsedPacket>, ParseError> {
        if self.buffer.is_empty() {
            return Ok(None);
        }

        match self.parse_level {
            ParseLevel::Full => {
                assert!(
                    self.mqtt_version != 0,
                    "MQTT version must be set before parsing packets"
                );
                match MqttPacket::from_bytes_with_version(&self.buffer, self.mqtt_version) {
                    Ok(ParseOk::Packet(packet, consumed)) => {
                        self.buffer.advance(consumed);
                        Ok(Some(ParsedPacket::Full(packet)))
                    }
                    Ok(ParseOk::Continue(_, _)) => Ok(None),
                    Err(e) => Err(e),
                    Ok(ParseOk::TopicName(_, _)) => Err(ParseError::ParseError(
                        "Unexpected ParseOk variant".to_string(),
                    )),
                }
            }
            ParseLevel::TypeOnly => match parse_type_only(&self.buffer) {
                Ok(LeveledParseOk::Packet(pkt, consumed)) => {
                    self.buffer.advance(consumed);
                    Ok(Some(pkt))
                }
                Ok(LeveledParseOk::Continue(_, _)) => Ok(None),
                Err(e) => Err(e),
            },
            ParseLevel::RawBody => {
                let total_len = match packet_frame_len(&self.buffer)? {
                    Some(n) => n,
                    None => return Ok(None),
                };
                let packet_bytes = self.buffer.split_to(total_len).freeze();
                match parse_raw_body(packet_bytes) {
                    Ok(LeveledParseOk::Packet(pkt, _)) => Ok(Some(pkt)),
                    Err(e) => Err(e),
                    _ => unreachable!("frame completeness already verified"),
                }
            }
            ParseLevel::HeadersParsed => {
                assert!(
                    self.mqtt_version != 0,
                    "MQTT version must be set before parsing packets"
                );
                let total_len = match packet_frame_len(&self.buffer)? {
                    Some(n) => n,
                    None => return Ok(None),
                };
                let packet_bytes = self.buffer.split_to(total_len).freeze();
                match parse_headers_only(packet_bytes, self.mqtt_version) {
                    Ok(LeveledParseOk::Packet(pkt, _)) => Ok(Some(pkt)),
                    Err(e) => Err(e),
                    _ => unreachable!("frame completeness already verified"),
                }
            }
        }
    }

    pub fn buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.buffer
    }
}

pub struct MqttStream<T> {
    parser: MqttParser,
    stream: T,
}

impl<T> MqttStream<T>
where
    T: std::io::Read,
{
    pub fn new(stream: T, buffer_size: usize, mqtt_version: u8) -> Self {
        MqttStream {
            parser: MqttParser::new(buffer_size, mqtt_version),
            stream,
        }
    }

    pub fn mut_stream(&mut self) -> &mut T {
        &mut self.stream
    }

    pub fn write(&mut self, data: &[u8]) -> std::io::Result<usize>
    where
        T: std::io::Write,
    {
        self.stream.write(data)
    }
}

impl<T> Iterator for MqttStream<T>
where
    T: std::io::Read,
{
    type Item = Result<MqttPacket, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.parser.next_packet() {
            Ok(Some(packet)) => Some(Ok(packet)),
            Ok(None) => {
                // Reserve space in the parser's buffer for reading
                let buffer = self.parser.buffer_mut();
                let initial_len = buffer.len();
                buffer.resize(initial_len + 1024, 0);

                match self.stream.read(&mut buffer[initial_len..]) {
                    Ok(0) => {
                        // End of stream - remove the unused space we reserved
                        buffer.truncate(initial_len);
                        None
                    }
                    Ok(n) => {
                        // Adjust buffer to actual bytes read
                        buffer.truncate(initial_len + n);
                        self.next() // Try to parse again
                    }
                    Err(e) => {
                        // Remove the unused space we reserved
                        buffer.truncate(initial_len);
                        Some(Err(ParseError::IoError(e)))
                    }
                }
            }
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_serde::control_packet::{ControlPacketType, MqttControlPacket};
    use crate::mqtt_serde::mqttv5::publishv5;
    use crate::mqtt_serde::parser::leveled::{ParseLevel, ParsedPacket, VariableHeader};

    // ── MqttParser construction ────────────────────────────────────────

    #[test]
    fn test_parser_default() {
        let parser = MqttParser::default();
        assert_eq!(parser.get_mqtt_vsn(), 0);
        assert_eq!(parser.parse_level(), ParseLevel::Full);
    }

    #[test]
    fn test_parser_with_level() {
        let parser = MqttParser::with_level(4096, 5, ParseLevel::TypeOnly);
        assert_eq!(parser.get_mqtt_vsn(), 5);
        assert_eq!(parser.parse_level(), ParseLevel::TypeOnly);
    }

    #[test]
    fn test_set_parse_level() {
        let mut parser = MqttParser::new(4096, 5);
        assert_eq!(parser.parse_level(), ParseLevel::Full);
        parser.set_parse_level(ParseLevel::RawBody);
        assert_eq!(parser.parse_level(), ParseLevel::RawBody);
    }

    #[test]
    fn test_parse_level_from_u8() {
        assert_eq!(ParseLevel::from(0), ParseLevel::Full);
        assert_eq!(ParseLevel::from(1), ParseLevel::HeadersParsed);
        assert_eq!(ParseLevel::from(2), ParseLevel::RawBody);
        assert_eq!(ParseLevel::from(3), ParseLevel::TypeOnly);
        assert_eq!(ParseLevel::from(99), ParseLevel::Full); // default
    }

    // ── next_parsed: empty buffer ──────────────────────────────────────

    #[test]
    fn test_next_parsed_empty_buffer() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::TypeOnly);
        assert!(parser.next_parsed().unwrap().is_none());
    }

    // ── next_parsed: Level 0 (Full) ────────────────────────────────────

    #[test]
    fn test_next_parsed_full_v5() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::Full);
        let publish =
            publishv5::MqttPublish::new(0, "t/1".to_string(), None, vec![0x61; 5], false, false);
        let bytes = publish.to_bytes().unwrap();
        parser.feed(&bytes);

        let result = parser.next_parsed().unwrap().unwrap();
        assert_eq!(result.packet_type(), ControlPacketType::PUBLISH);
        assert!(result.as_packet().is_some());
        // Buffer should be drained
        assert!(parser.next_parsed().unwrap().is_none());
    }

    // ── next_parsed: Level 3 (TypeOnly) ────────────────────────────────

    #[test]
    fn test_next_parsed_type_only() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::TypeOnly);
        let publish =
            publishv5::MqttPublish::new(0, "t".to_string(), None, vec![1, 2, 3], false, false);
        parser.feed(&publish.to_bytes().unwrap());

        let result = parser.next_parsed().unwrap().unwrap();
        assert_eq!(result.packet_type(), ControlPacketType::PUBLISH);
        assert!(result.as_packet().is_none());
        assert!(parser.next_parsed().unwrap().is_none());
    }

    // ── next_parsed: Level 2 (RawBody) ─────────────────────────────────

    #[test]
    fn test_next_parsed_raw_body() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::RawBody);
        let publish =
            publishv5::MqttPublish::new(0, "t".to_string(), None, vec![1, 2, 3], false, false);
        parser.feed(&publish.to_bytes().unwrap());

        let result = parser.next_parsed().unwrap().unwrap();
        match result {
            ParsedPacket::RawBody(ref pkt) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PUBLISH);
                assert!(!pkt.remaining.is_empty());
            }
            other => panic!("Expected RawBody, got {:?}", other),
        }
        assert!(parser.next_parsed().unwrap().is_none());
    }

    // ── next_parsed: Level 1 (HeadersParsed) ───────────────────────────

    #[test]
    fn test_next_parsed_headers_parsed() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::HeadersParsed);
        let publish = publishv5::MqttPublish::new(
            1,
            "a/b".to_string(),
            Some(42),
            vec![0xAA; 10],
            false,
            false,
        );
        parser.feed(&publish.to_bytes().unwrap());

        let result = parser.next_parsed().unwrap().unwrap();
        match result {
            ParsedPacket::HeadersParsed(ref pkt) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PUBLISH);
                match &pkt.variable_header {
                    VariableHeader::PublishV5 {
                        topic_name,
                        qos,
                        packet_id,
                        ..
                    } => {
                        assert_eq!(topic_name, "a/b");
                        assert_eq!(*qos, 1);
                        assert_eq!(*packet_id, Some(42));
                    }
                    other => panic!("Expected PublishV5, got {:?}", other),
                }
                assert_eq!(pkt.raw_payload.len(), 10);
            }
            other => panic!("Expected HeadersParsed, got {:?}", other),
        }
        assert!(parser.next_parsed().unwrap().is_none());
    }

    // ── Partial feed / incremental parsing ─────────────────────────────

    #[test]
    fn test_next_parsed_partial_feed() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::TypeOnly);
        let publish =
            publishv5::MqttPublish::new(0, "topic".to_string(), None, vec![0x61; 50], false, false);
        let bytes = publish.to_bytes().unwrap();

        // Feed only half
        let mid = bytes.len() / 2;
        parser.feed(&bytes[..mid]);
        assert!(parser.next_parsed().unwrap().is_none());

        // Feed the rest
        parser.feed(&bytes[mid..]);
        let result = parser.next_parsed().unwrap().unwrap();
        assert_eq!(result.packet_type(), ControlPacketType::PUBLISH);
    }

    #[test]
    fn test_next_parsed_partial_feed_raw_body() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::RawBody);
        let publish =
            publishv5::MqttPublish::new(0, "t".to_string(), None, vec![0xBB; 20], false, false);
        let bytes = publish.to_bytes().unwrap();

        // Feed byte by byte until we get a result
        let mut got_packet = false;
        for &b in &bytes {
            parser.feed(&[b]);
            if let Some(pkt) = parser.next_parsed().unwrap() {
                assert_eq!(pkt.packet_type(), ControlPacketType::PUBLISH);
                got_packet = true;
                break;
            }
        }
        assert!(got_packet);
    }

    #[test]
    fn test_next_parsed_partial_feed_headers_parsed() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::HeadersParsed);
        let publish =
            publishv5::MqttPublish::new(0, "t".to_string(), None, vec![0xCC; 10], false, false);
        let bytes = publish.to_bytes().unwrap();

        // Feed first 3 bytes only
        parser.feed(&bytes[..3]);
        assert!(parser.next_parsed().unwrap().is_none());

        // Feed the rest
        parser.feed(&bytes[3..]);
        let result = parser.next_parsed().unwrap().unwrap();
        assert_eq!(result.packet_type(), ControlPacketType::PUBLISH);
    }

    // ── Multi-packet streaming ─────────────────────────────────────────

    #[test]
    fn test_next_parsed_multiple_packets() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::TypeOnly);

        let p1 = publishv5::MqttPublish::new(0, "a".to_string(), None, vec![1], false, false);
        let p2 = publishv5::MqttPublish::new(0, "b".to_string(), None, vec![2], false, false);
        // PINGREQ
        let ping = [0xC0u8, 0x00];

        let mut all_bytes = p1.to_bytes().unwrap();
        all_bytes.extend_from_slice(&p2.to_bytes().unwrap());
        all_bytes.extend_from_slice(&ping);
        parser.feed(&all_bytes);

        // Should yield 3 packets
        let r1 = parser.next_parsed().unwrap().unwrap();
        assert_eq!(r1.packet_type(), ControlPacketType::PUBLISH);

        let r2 = parser.next_parsed().unwrap().unwrap();
        assert_eq!(r2.packet_type(), ControlPacketType::PUBLISH);

        let r3 = parser.next_parsed().unwrap().unwrap();
        assert_eq!(r3.packet_type(), ControlPacketType::PINGREQ);

        assert!(parser.next_parsed().unwrap().is_none());
    }

    #[test]
    fn test_next_parsed_multiple_packets_headers_parsed() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::HeadersParsed);

        let p1 = publishv5::MqttPublish::new(0, "x".to_string(), None, vec![10], false, false);
        let p2 =
            publishv5::MqttPublish::new(1, "y".to_string(), Some(7), vec![20, 30], true, false);

        let mut all_bytes = p1.to_bytes().unwrap();
        all_bytes.extend_from_slice(&p2.to_bytes().unwrap());
        parser.feed(&all_bytes);

        // First packet
        let r1 = parser.next_parsed().unwrap().unwrap();
        match r1 {
            ParsedPacket::HeadersParsed(ref pkt) => {
                match &pkt.variable_header {
                    VariableHeader::PublishV5 {
                        topic_name, qos, ..
                    } => {
                        assert_eq!(topic_name, "x");
                        assert_eq!(*qos, 0);
                    }
                    other => panic!("Expected PublishV5, got {:?}", other),
                }
                assert_eq!(pkt.raw_payload.len(), 1);
            }
            other => panic!("Expected HeadersParsed, got {:?}", other),
        }

        // Second packet
        let r2 = parser.next_parsed().unwrap().unwrap();
        match r2 {
            ParsedPacket::HeadersParsed(ref pkt) => match &pkt.variable_header {
                VariableHeader::PublishV5 {
                    topic_name,
                    qos,
                    retain,
                    packet_id,
                    ..
                } => {
                    assert_eq!(topic_name, "y");
                    assert_eq!(*qos, 1);
                    assert!(*retain);
                    assert_eq!(*packet_id, Some(7));
                }
                other => panic!("Expected PublishV5, got {:?}", other),
            },
            other => panic!("Expected HeadersParsed, got {:?}", other),
        }

        assert!(parser.next_parsed().unwrap().is_none());
    }

    // ── Level switching mid-stream ─────────────────────────────────────

    #[test]
    fn test_switch_parse_level_between_packets() {
        let mut parser = MqttParser::with_level(4096, 5, ParseLevel::TypeOnly);

        let p1 = publishv5::MqttPublish::new(0, "t".to_string(), None, vec![1], false, false);
        let p2 = publishv5::MqttPublish::new(0, "t".to_string(), None, vec![2], false, false);

        let mut all_bytes = p1.to_bytes().unwrap();
        all_bytes.extend_from_slice(&p2.to_bytes().unwrap());
        parser.feed(&all_bytes);

        // Parse first as TypeOnly
        let r1 = parser.next_parsed().unwrap().unwrap();
        assert!(matches!(r1, ParsedPacket::TypeOnly(_)));

        // Switch to RawBody for the second
        parser.set_parse_level(ParseLevel::RawBody);
        let r2 = parser.next_parsed().unwrap().unwrap();
        assert!(matches!(r2, ParsedPacket::RawBody(_)));

        assert!(parser.next_parsed().unwrap().is_none());
    }

    // ── next_packet (Level 0 only) ─────────────────────────────────────

    #[test]
    fn test_next_packet_v5() {
        let mut parser = MqttParser::new(4096, 5);
        let publish = publishv5::MqttPublish::new(0, "t".to_string(), None, vec![1], false, false);
        parser.feed(&publish.to_bytes().unwrap());

        let pkt = parser.next_packet().unwrap().unwrap();
        assert_eq!(pkt.packet_type(), ControlPacketType::PUBLISH);
        assert!(parser.next_packet().unwrap().is_none());
    }

    #[test]
    fn test_next_packet_empty() {
        let mut parser = MqttParser::new(4096, 5);
        assert!(parser.next_packet().unwrap().is_none());
    }

    #[test]
    fn test_next_packet_partial() {
        let mut parser = MqttParser::new(4096, 5);
        let publish =
            publishv5::MqttPublish::new(0, "t".to_string(), None, vec![0; 20], false, false);
        let bytes = publish.to_bytes().unwrap();
        parser.feed(&bytes[..3]);
        assert!(parser.next_packet().unwrap().is_none());
        parser.feed(&bytes[3..]);
        assert!(parser.next_packet().unwrap().is_some());
    }

    // ── set_mqtt_vsn ───────────────────────────────────────────────────

    #[test]
    fn test_set_mqtt_vsn_from_connect_v5() {
        use crate::mqtt_serde::mqttv5::connect::MqttConnect;
        let connect = MqttConnect::new("c".to_string(), None, None, None, 60, true, Vec::new());
        let mut parser = MqttParser::new(4096, 0);
        parser.feed(&connect.to_bytes().unwrap());
        assert_eq!(parser.set_mqtt_vsn(0).unwrap(), 5);
        assert_eq!(parser.get_mqtt_vsn(), 5);
    }

    #[test]
    fn test_set_mqtt_vsn_from_connect_v3() {
        use crate::mqtt_serde::mqttv3::connect::MqttConnect;
        let connect = MqttConnect::new("c".to_string(), 60, true);
        let mut parser = MqttParser::new(4096, 0);
        parser.feed(&connect.to_bytes().unwrap());
        assert_eq!(parser.set_mqtt_vsn(0).unwrap(), 4);
    }

    #[test]
    fn test_set_mqtt_vsn_already_set() {
        let mut parser = MqttParser::new(4096, 5);
        assert_eq!(parser.set_mqtt_vsn(0).unwrap(), 5);
    }

    #[test]
    fn test_set_mqtt_vsn_not_connect() {
        let mut parser = MqttParser::new(4096, 0);
        // Feed a PINGREQ (not CONNECT)
        parser.feed(&[0xC0, 0x00]);
        assert!(parser.set_mqtt_vsn(0).is_err());
    }

    // ── V3 next_parsed via MqttParser ──────────────────────────────────

    #[test]
    fn test_next_parsed_v3_full() {
        use crate::mqtt_serde::mqttv3::publishv3;
        let mut parser = MqttParser::with_level(4096, 4, ParseLevel::Full);
        let publish = publishv3::MqttPublish::new("t".to_string(), 0, vec![1], None, false, false);
        parser.feed(&publish.to_bytes().unwrap());
        let result = parser.next_parsed().unwrap().unwrap();
        assert_eq!(result.packet_type(), ControlPacketType::PUBLISH);
        assert!(result.as_packet().is_some());
    }

    #[test]
    fn test_next_parsed_v3_headers_parsed() {
        use crate::mqtt_serde::mqttv3::publishv3;
        let mut parser = MqttParser::with_level(4096, 4, ParseLevel::HeadersParsed);
        let publish =
            publishv3::MqttPublish::new("t".to_string(), 1, vec![1, 2], Some(99), false, false);
        parser.feed(&publish.to_bytes().unwrap());
        let result = parser.next_parsed().unwrap().unwrap();
        match result {
            ParsedPacket::HeadersParsed(pkt) => {
                assert_eq!(pkt.mqtt_version, 4);
                match &pkt.variable_header {
                    VariableHeader::PublishV3 {
                        topic_name,
                        message_id,
                        ..
                    } => {
                        assert_eq!(topic_name, "t");
                        assert_eq!(*message_id, Some(99));
                    }
                    other => panic!("Expected PublishV3, got {:?}", other),
                }
            }
            other => panic!("Expected HeadersParsed, got {:?}", other),
        }
    }
}
