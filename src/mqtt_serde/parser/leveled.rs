// SPDX-License-Identifier: MPL-2.0

//! Leveled MQTT packet parser.
//!
//! Provides configurable parse depth for MQTT packets:
//! - Level 0 (`Full`): Complete parse into typed structs (default, existing behavior)
//! - Level 1 (`HeadersParsed`): Parse variable headers, keep payload as raw bytes
//! - Level 2 (`RawBody`): Parse fixed header only, keep remaining bytes raw
//! - Level 3 (`TypeOnly`): Parse packet type and flags only, discard body

use bytes::Bytes;

use crate::mqtt_serde::control_packet::{ControlPacketType, MqttPacket};
use crate::mqtt_serde::mqttv5::common::properties::Property;
use crate::mqtt_serde::parser::{parse_remaining_length, ParseError};

/// Controls how deeply the parser interprets incoming MQTT packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParseLevel {
    /// Full parse into typed structs (current behavior).
    #[default]
    Full = 0,
    /// Parse variable headers into typed fields; keep payload as raw bytes.
    HeadersParsed = 1,
    /// Parse only the fixed header; retain remaining bytes as a single raw buffer.
    RawBody = 2,
    /// Parse only the packet type and flags; skip/discard all body bytes.
    TypeOnly = 3,
}

#[allow(clippy::large_enum_variant)]
/// Result of a leveled parse. The variant depends on the configured `ParseLevel`.
#[derive(Debug)]
pub enum ParsedPacket {
    /// Level 0: Fully parsed packet (identical to existing `MqttPacket`).
    Full(MqttPacket),
    /// Level 1: Variable header fields parsed, payload kept as raw bytes.
    HeadersParsed(HeadersParsedPacket),
    /// Level 2: Only fixed header parsed; remaining bytes as a raw slice.
    RawBody(RawBodyPacket),
    /// Level 3: Only the packet type and flags are available.
    TypeOnly(TypeOnlyPacket),
}

impl ParsedPacket {
    /// Unwraps a `ParsedPacket::Full` into the inner `MqttPacket`.
    /// Panics if the variant is not `Full`.
    pub fn into_packet(self) -> MqttPacket {
        match self {
            ParsedPacket::Full(pkt) => pkt,
            other => panic!("Expected ParsedPacket::Full, got {:?}", other),
        }
    }

    /// Returns a reference to the inner `MqttPacket` if this is `Full`, or `None`.
    pub fn as_packet(&self) -> Option<&MqttPacket> {
        match self {
            ParsedPacket::Full(pkt) => Some(pkt),
            _ => None,
        }
    }

    /// Returns the `ControlPacketType` regardless of parse level.
    pub fn packet_type(&self) -> ControlPacketType {
        match self {
            ParsedPacket::Full(pkt) => pkt.packet_type(),
            ParsedPacket::HeadersParsed(p) => p.packet_type,
            ParsedPacket::RawBody(p) => p.packet_type,
            ParsedPacket::TypeOnly(p) => p.packet_type,
        }
    }
}

/// Level 3 result: just the type and flags.
#[derive(Debug, Clone, Copy)]
pub struct TypeOnlyPacket {
    pub packet_type: ControlPacketType,
    pub flags: u8,
}

/// Level 2 result: fixed header parsed, everything else raw.
#[derive(Debug, Clone)]
pub struct RawBodyPacket {
    pub packet_type: ControlPacketType,
    pub flags: u8,
    /// The remaining bytes after the fixed header (variable header + payload).
    pub remaining: Bytes,
}

/// Level 1 result: variable header parsed, payload raw.
#[derive(Debug)]
pub struct HeadersParsedPacket {
    pub packet_type: ControlPacketType,
    pub flags: u8,
    pub mqtt_version: u8,
    /// The parsed variable header.
    pub variable_header: VariableHeader,
    /// Raw payload bytes (not interpreted). For packets with no payload
    /// (PINGREQ, PINGRESP, etc.) this will be empty.
    pub raw_payload: Bytes,
}

/// Parsed variable header data for Level 1 parsing.
/// Each variant holds only the variable header fields (no payload).
#[derive(Debug)]
pub enum VariableHeader {
    // V5 variants
    ConnectV5 {
        protocol_name: String,
        protocol_version: u8,
        clean_start: bool,
        keep_alive: u16,
        connect_flags: u8,
        properties: Vec<Property>,
    },
    ConnAckV5 {
        session_present: bool,
        reason_code: u8,
        properties: Option<Vec<Property>>,
    },
    PublishV5 {
        topic_name: String,
        qos: u8,
        dup: bool,
        retain: bool,
        packet_id: Option<u16>,
        properties: Vec<Property>,
    },
    PubAckV5 {
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    },
    PubRecV5 {
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    },
    PubRelV5 {
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    },
    PubCompV5 {
        packet_id: u16,
        reason_code: u8,
        properties: Vec<Property>,
    },
    SubscribeV5 {
        packet_id: u16,
        properties: Vec<Property>,
    },
    SubAckV5 {
        packet_id: u16,
        properties: Vec<Property>,
    },
    UnsubscribeV5 {
        packet_id: u16,
        properties: Vec<Property>,
    },
    UnsubAckV5 {
        packet_id: u16,
        properties: Vec<Property>,
    },
    DisconnectV5 {
        reason_code: u8,
        properties: Vec<Property>,
    },
    AuthV5 {
        reason_code: u8,
        properties: Vec<Property>,
    },

    // V3 variants
    ConnectV3 {
        protocol_name: String,
        protocol_version: u8,
        clean_session: bool,
        keep_alive: u16,
        connect_flags: u8,
    },
    ConnAckV3 {
        session_present: bool,
        return_code: u8,
    },
    PublishV3 {
        topic_name: String,
        qos: u8,
        dup: bool,
        retain: bool,
        message_id: Option<u16>,
    },
    /// V3 packets that only have a message_id in the variable header.
    /// Covers PUBACK, PUBREC, PUBREL, PUBCOMP, SUBSCRIBE, SUBACK, UNSUBSCRIBE, UNSUBACK.
    PacketIdOnlyV3 { message_id: u16 },

    /// Packets with no meaningful variable header (PINGREQ, PINGRESP, DISCONNECT v3).
    Empty,
}

/// Result type for leveled parsing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum LeveledParseOk {
    /// Not enough data in the buffer for a full packet.
    Continue(usize, usize),
    /// A packet was parsed at the configured level.
    Packet(ParsedPacket, usize),
}

// ── Helpers: frame length ───────────────────────────────────────────────

/// Returns the total packet frame size if enough data is available, or `None`
/// if more bytes are needed. Only inspects byte 0 + the VBI; does **not**
/// validate packet contents.
pub fn packet_frame_len(buffer: &[u8]) -> Result<Option<usize>, ParseError> {
    if buffer.len() < 2 {
        return Ok(None);
    }
    let (remaining_len, vbi_bytes) = match parse_remaining_length(&buffer[1..]) {
        Ok(v) => v,
        Err(ParseError::More(_, _)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let total = 1 + vbi_bytes + remaining_len;
    if buffer.len() < total {
        Ok(None)
    } else {
        Ok(Some(total))
    }
}

// ── Level 3: TypeOnly ──────────────────────────────────────────────────

/// Parse only the packet type and flags from byte 0, skip the rest.
pub fn parse_type_only(buffer: &[u8]) -> Result<LeveledParseOk, ParseError> {
    if buffer.is_empty() {
        return Err(ParseError::BufferEmpty);
    }

    let (pkt_type, flags) = ControlPacketType::from_first_byte(buffer[0])?;

    // Must parse VBI to know total packet length for buffer advancement
    if buffer.len() < 2 {
        return Ok(LeveledParseOk::Continue(1, 0));
    }
    let (remaining_len, vbi_bytes) = match parse_remaining_length(&buffer[1..]) {
        Ok(v) => v,
        Err(ParseError::More(n, _)) => return Ok(LeveledParseOk::Continue(n, 0)),
        Err(e) => return Err(e),
    };
    let total_len = 1 + vbi_bytes + remaining_len;

    if buffer.len() < total_len {
        return Ok(LeveledParseOk::Continue(total_len - buffer.len(), 0));
    }

    Ok(LeveledParseOk::Packet(
        ParsedPacket::TypeOnly(TypeOnlyPacket {
            packet_type: pkt_type,
            flags,
        }),
        total_len,
    ))
}

// ── Level 2: RawBody ───────────────────────────────────────────────────

/// Parse the fixed header, return remaining bytes as a zero-copy view.
pub fn parse_raw_body(buffer: Bytes) -> Result<LeveledParseOk, ParseError> {
    if buffer.is_empty() {
        return Err(ParseError::BufferEmpty);
    }

    let (pkt_type, flags) = ControlPacketType::from_first_byte(buffer[0])?;

    if buffer.len() < 2 {
        return Ok(LeveledParseOk::Continue(1, 0));
    }
    let (remaining_len, vbi_bytes) = match parse_remaining_length(&buffer[1..]) {
        Ok(v) => v,
        Err(ParseError::More(n, _)) => return Ok(LeveledParseOk::Continue(n, 0)),
        Err(e) => return Err(e),
    };
    let remaining_start = 1 + vbi_bytes;
    let total_len = remaining_start + remaining_len;

    if buffer.len() < total_len {
        return Ok(LeveledParseOk::Continue(total_len - buffer.len(), 0));
    }

    let remaining = buffer.slice(remaining_start..total_len);

    Ok(LeveledParseOk::Packet(
        ParsedPacket::RawBody(RawBodyPacket {
            packet_type: pkt_type,
            flags,
            remaining,
        }),
        total_len,
    ))
}

// ── Level 1: HeadersParsed ─────────────────────────────────────────────

/// Parse variable headers, keep payload as a zero-copy `Bytes` view.
pub fn parse_headers_only(buffer: Bytes, mqtt_version: u8) -> Result<LeveledParseOk, ParseError> {
    match mqtt_version {
        3 | 4 => parse_headers_only_v3(buffer),
        5 => parse_headers_only_v5(buffer),
        _ => Err(ParseError::InvalidPacketType),
    }
}

fn parse_headers_only_v5(buffer: Bytes) -> Result<LeveledParseOk, ParseError> {
    if buffer.is_empty() {
        return Err(ParseError::BufferEmpty);
    }

    let (pkt_type, flags) = ControlPacketType::from_first_byte(buffer[0])?;

    if buffer.len() < 2 {
        return Ok(LeveledParseOk::Continue(1, 0));
    }
    let (remaining_len, vbi_bytes) = match parse_remaining_length(&buffer[1..]) {
        Ok(v) => v,
        Err(ParseError::More(n, _)) => return Ok(LeveledParseOk::Continue(n, 0)),
        Err(e) => return Err(e),
    };
    let fixed_hdr_len = 1 + vbi_bytes;
    let total_len = fixed_hdr_len + remaining_len;

    if buffer.len() < total_len {
        return Ok(LeveledParseOk::Continue(total_len - buffer.len(), 0));
    }

    let packet_body = &buffer[fixed_hdr_len..total_len];

    let (variable_header, vhdr_consumed) = match pkt_type {
        ControlPacketType::PUBLISH => parse_publish_header_v5(flags, packet_body)?,
        ControlPacketType::CONNECT => parse_connect_header_v5(packet_body)?,
        ControlPacketType::CONNACK => parse_connack_header_v5(packet_body)?,
        ControlPacketType::SUBSCRIBE => parse_subscribe_header_v5(packet_body)?,
        ControlPacketType::SUBACK => parse_suback_header_v5(packet_body)?,
        ControlPacketType::UNSUBSCRIBE => parse_unsubscribe_header_v5(packet_body)?,
        ControlPacketType::UNSUBACK => parse_unsuback_header_v5(packet_body)?,
        ControlPacketType::PUBACK => parse_ack_with_props_v5(packet_body, VhdrV5Kind::PubAck)?,
        ControlPacketType::PUBREC => parse_ack_with_props_v5(packet_body, VhdrV5Kind::PubRec)?,
        ControlPacketType::PUBREL => parse_ack_with_props_v5(packet_body, VhdrV5Kind::PubRel)?,
        ControlPacketType::PUBCOMP => parse_ack_with_props_v5(packet_body, VhdrV5Kind::PubComp)?,
        ControlPacketType::DISCONNECT => parse_disconnect_header_v5(packet_body, remaining_len)?,
        ControlPacketType::AUTH => parse_auth_header_v5(packet_body, remaining_len)?,
        ControlPacketType::PINGREQ | ControlPacketType::PINGRESP => (VariableHeader::Empty, 0),
    };

    let raw_payload = buffer.slice((fixed_hdr_len + vhdr_consumed)..total_len);

    Ok(LeveledParseOk::Packet(
        ParsedPacket::HeadersParsed(HeadersParsedPacket {
            packet_type: pkt_type,
            flags,
            mqtt_version: 5,
            variable_header,
            raw_payload,
        }),
        total_len,
    ))
}

fn parse_headers_only_v3(buffer: Bytes) -> Result<LeveledParseOk, ParseError> {
    if buffer.is_empty() {
        return Err(ParseError::BufferEmpty);
    }

    let (pkt_type, flags) = ControlPacketType::from_first_byte(buffer[0])?;

    if buffer.len() < 2 {
        return Ok(LeveledParseOk::Continue(1, 0));
    }
    let (remaining_len, vbi_bytes) = match parse_remaining_length(&buffer[1..]) {
        Ok(v) => v,
        Err(ParseError::More(n, _)) => return Ok(LeveledParseOk::Continue(n, 0)),
        Err(e) => return Err(e),
    };
    let fixed_hdr_len = 1 + vbi_bytes;
    let total_len = fixed_hdr_len + remaining_len;

    if buffer.len() < total_len {
        return Ok(LeveledParseOk::Continue(total_len - buffer.len(), 0));
    }

    let packet_body = &buffer[fixed_hdr_len..total_len];

    let (variable_header, vhdr_consumed) = match pkt_type {
        ControlPacketType::PUBLISH => parse_publish_header_v3(flags, packet_body)?,
        ControlPacketType::CONNECT => parse_connect_header_v3(packet_body)?,
        ControlPacketType::CONNACK => parse_connack_header_v3(packet_body)?,
        ControlPacketType::PUBACK
        | ControlPacketType::PUBREC
        | ControlPacketType::PUBREL
        | ControlPacketType::PUBCOMP
        | ControlPacketType::UNSUBACK => {
            let (id, consumed) = parse_packet_id_raw(packet_body)?;
            (VariableHeader::PacketIdOnlyV3 { message_id: id }, consumed)
        }
        ControlPacketType::SUBSCRIBE
        | ControlPacketType::SUBACK
        | ControlPacketType::UNSUBSCRIBE => {
            // Variable header is just packet_id, payload is the subscriptions
            let (id, consumed) = parse_packet_id_raw(packet_body)?;
            (VariableHeader::PacketIdOnlyV3 { message_id: id }, consumed)
        }
        ControlPacketType::PINGREQ
        | ControlPacketType::PINGRESP
        | ControlPacketType::DISCONNECT => (VariableHeader::Empty, 0),
        ControlPacketType::AUTH => {
            return Err(ParseError::ParseError(
                "AUTH packet is not supported in MQTT v3".to_string(),
            ));
        }
    };

    let raw_payload = buffer.slice((fixed_hdr_len + vhdr_consumed)..total_len);

    Ok(LeveledParseOk::Packet(
        ParsedPacket::HeadersParsed(HeadersParsedPacket {
            packet_type: pkt_type,
            flags,
            mqtt_version: 4,
            variable_header,
            raw_payload,
        }),
        total_len,
    ))
}

// ── V5 per-packet variable header parsers ──────────────────────────────

fn parse_publish_header_v5(flags: u8, body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;

    let dup = (flags & 0x08) != 0;
    let qos = (flags & 0x06) >> 1;
    let retain = (flags & 0x01) != 0;

    // Topic name
    let (topic_name, consumed) = crate::mqtt_serde::parser::parse_utf8_string(
        body.get(offset..).ok_or(ParseError::BufferTooShort)?,
    )?;
    offset += consumed;

    // Packet ID (only for QoS > 0)
    let packet_id = if qos > 0 {
        let (id, consumed) = parse_packet_id_raw(&body[offset..])?;
        offset += consumed;
        Some(id)
    } else {
        None
    };

    // Properties
    let (properties, consumed) =
        crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
            body.get(offset..).ok_or(ParseError::BufferTooShort)?,
        )?;
    offset += consumed;

    Ok((
        VariableHeader::PublishV5 {
            topic_name,
            qos,
            dup,
            retain,
            packet_id,
            properties,
        },
        offset,
    ))
}

fn parse_connect_header_v5(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;

    // Protocol Name
    let (protocol_name, consumed) = crate::mqtt_serde::parser::parse_utf8_string(body)?;
    offset += consumed;

    // Protocol Version
    let protocol_version = *body.get(offset).ok_or(ParseError::BufferTooShort)?;
    offset += 1;

    // Connect Flags
    let connect_flags = *body.get(offset).ok_or(ParseError::BufferTooShort)?;
    let clean_start = (connect_flags & 0x02) != 0;
    offset += 1;

    // Keep Alive
    if body.len() < offset + 2 {
        return Err(ParseError::BufferTooShort);
    }
    let keep_alive = u16::from_be_bytes([body[offset], body[offset + 1]]);
    offset += 2;

    // Properties
    let (properties, consumed) =
        crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
            body.get(offset..).ok_or(ParseError::BufferTooShort)?,
        )?;
    offset += consumed;

    Ok((
        VariableHeader::ConnectV5 {
            protocol_name,
            protocol_version,
            clean_start,
            keep_alive,
            connect_flags,
            properties,
        },
        offset,
    ))
}

fn parse_connack_header_v5(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    if body.len() < 2 {
        return Err(ParseError::BufferTooShort);
    }
    let session_present = (body[0] & 0x01) != 0;
    let reason_code = body[1];
    let mut offset = 2;

    let properties = if body.len() > 2 {
        let (props, consumed) =
            crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(&body[offset..])?;
        offset += consumed;
        Some(props)
    } else {
        None
    };

    Ok((
        VariableHeader::ConnAckV5 {
            session_present,
            reason_code,
            properties,
        },
        offset,
    ))
}

fn parse_subscribe_header_v5(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;
    let (packet_id, consumed) = parse_packet_id_raw(body)?;
    offset += consumed;

    let (properties, consumed) =
        crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
            body.get(offset..).ok_or(ParseError::BufferTooShort)?,
        )?;
    offset += consumed;

    Ok((
        VariableHeader::SubscribeV5 {
            packet_id,
            properties,
        },
        offset,
    ))
}

fn parse_suback_header_v5(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;
    let (packet_id, consumed) = parse_packet_id_raw(body)?;
    offset += consumed;

    let (properties, consumed) =
        crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
            body.get(offset..).ok_or(ParseError::BufferTooShort)?,
        )?;
    offset += consumed;

    Ok((
        VariableHeader::SubAckV5 {
            packet_id,
            properties,
        },
        offset,
    ))
}

fn parse_unsubscribe_header_v5(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;
    let (packet_id, consumed) = parse_packet_id_raw(body)?;
    offset += consumed;

    let (properties, consumed) =
        crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
            body.get(offset..).ok_or(ParseError::BufferTooShort)?,
        )?;
    offset += consumed;

    Ok((
        VariableHeader::UnsubscribeV5 {
            packet_id,
            properties,
        },
        offset,
    ))
}

fn parse_unsuback_header_v5(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;
    let (packet_id, consumed) = parse_packet_id_raw(body)?;
    offset += consumed;

    let (properties, consumed) =
        crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
            body.get(offset..).ok_or(ParseError::BufferTooShort)?,
        )?;
    offset += consumed;

    Ok((
        VariableHeader::UnsubAckV5 {
            packet_id,
            properties,
        },
        offset,
    ))
}

/// Shared parser for PUBACK, PUBREC, PUBREL, PUBCOMP (v5).
/// These all have: packet_id (2 bytes) + optional reason_code (1 byte) + optional properties.
#[allow(clippy::enum_variant_names)]
enum VhdrV5Kind {
    PubAck,
    PubRec,
    PubRel,
    PubComp,
}

fn parse_ack_with_props_v5(
    body: &[u8],
    kind: VhdrV5Kind,
) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;
    let (packet_id, consumed) = parse_packet_id_raw(body)?;
    offset += consumed;

    // Reason code and properties are optional per MQTT 5.0 spec
    let (reason_code, properties) = if body.len() > offset {
        let rc = body[offset];
        offset += 1;
        if body.len() > offset {
            let (props, consumed) =
                crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(
                    &body[offset..],
                )?;
            offset += consumed;
            (rc, props)
        } else {
            (rc, Vec::new())
        }
    } else {
        (0x00, Vec::new()) // Default success
    };

    let vhdr = match kind {
        VhdrV5Kind::PubAck => VariableHeader::PubAckV5 {
            packet_id,
            reason_code,
            properties,
        },
        VhdrV5Kind::PubRec => VariableHeader::PubRecV5 {
            packet_id,
            reason_code,
            properties,
        },
        VhdrV5Kind::PubRel => VariableHeader::PubRelV5 {
            packet_id,
            reason_code,
            properties,
        },
        VhdrV5Kind::PubComp => VariableHeader::PubCompV5 {
            packet_id,
            reason_code,
            properties,
        },
    };

    // These packets have no payload, so consumed == entire body
    Ok((vhdr, offset))
}

fn parse_disconnect_header_v5(
    body: &[u8],
    remaining_len: usize,
) -> Result<(VariableHeader, usize), ParseError> {
    // DISCONNECT v5 has optional reason code and properties, no payload
    if remaining_len == 0 {
        return Ok((
            VariableHeader::DisconnectV5 {
                reason_code: 0x00,
                properties: Vec::new(),
            },
            0,
        ));
    }

    let reason_code = body[0];
    let mut offset = 1;

    let properties = if body.len() > offset {
        let (props, consumed) =
            crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(&body[offset..])?;
        offset += consumed;
        props
    } else {
        Vec::new()
    };

    Ok((
        VariableHeader::DisconnectV5 {
            reason_code,
            properties,
        },
        offset,
    ))
}

fn parse_auth_header_v5(
    body: &[u8],
    remaining_len: usize,
) -> Result<(VariableHeader, usize), ParseError> {
    // AUTH v5: reason code + properties, no payload
    if remaining_len == 0 {
        return Ok((
            VariableHeader::AuthV5 {
                reason_code: 0x00,
                properties: Vec::new(),
            },
            0,
        ));
    }

    let reason_code = body[0];
    let mut offset = 1;

    let properties = if body.len() > offset {
        let (props, consumed) =
            crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr(&body[offset..])?;
        offset += consumed;
        props
    } else {
        Vec::new()
    };

    Ok((
        VariableHeader::AuthV5 {
            reason_code,
            properties,
        },
        offset,
    ))
}

// ── V3 per-packet variable header parsers ──────────────────────────────

fn parse_publish_header_v3(flags: u8, body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;

    let dup = (flags & 0x08) != 0;
    let qos = (flags & 0x06) >> 1;
    let retain = (flags & 0x01) != 0;

    // Topic name
    let (topic_name, consumed) = crate::mqtt_serde::parser::parse_utf8_string(body)?;
    offset += consumed;

    // Message ID (only for QoS > 0)
    let message_id = if qos > 0 {
        let (id, consumed) = parse_packet_id_raw(&body[offset..])?;
        offset += consumed;
        Some(id)
    } else {
        None
    };

    Ok((
        VariableHeader::PublishV3 {
            topic_name,
            qos,
            dup,
            retain,
            message_id,
        },
        offset,
    ))
}

fn parse_connect_header_v3(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    let mut offset = 0;

    // Protocol Name
    let (protocol_name, consumed) = crate::mqtt_serde::parser::parse_utf8_string(body)?;
    offset += consumed;

    // Protocol Version
    let protocol_version = *body.get(offset).ok_or(ParseError::BufferTooShort)?;
    offset += 1;

    // Connect Flags
    let connect_flags = *body.get(offset).ok_or(ParseError::BufferTooShort)?;
    let clean_session = (connect_flags & 0x02) != 0;
    offset += 1;

    // Keep Alive
    if body.len() < offset + 2 {
        return Err(ParseError::BufferTooShort);
    }
    let keep_alive = u16::from_be_bytes([body[offset], body[offset + 1]]);
    offset += 2;

    Ok((
        VariableHeader::ConnectV3 {
            protocol_name,
            protocol_version,
            clean_session,
            keep_alive,
            connect_flags,
        },
        offset,
    ))
}

fn parse_connack_header_v3(body: &[u8]) -> Result<(VariableHeader, usize), ParseError> {
    if body.len() < 2 {
        return Err(ParseError::BufferTooShort);
    }
    let session_present = (body[0] & 0x01) != 0;
    let return_code = body[1];

    Ok((
        VariableHeader::ConnAckV3 {
            session_present,
            return_code,
        },
        2,
    ))
}

// ── Helpers ────────────────────────────────────────────────────────────

fn parse_packet_id_raw(buffer: &[u8]) -> Result<(u16, usize), ParseError> {
    if buffer.len() < 2 {
        return Err(ParseError::BufferTooShort);
    }
    Ok((u16::from_be_bytes([buffer[0], buffer[1]]), 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_serde::control_packet::MqttControlPacket;
    use crate::mqtt_serde::mqttv3::publishv3;
    use crate::mqtt_serde::mqttv5::publishv5;

    // ── Level 3 tests ──────────────────────────────────────────────────

    #[test]
    fn test_type_only_publish_v5() {
        let publish = publishv5::MqttPublish::new(
            0,
            "test/topic".to_string(),
            None,
            vec![0x61; 100],
            false,
            false,
        );
        let bytes = publish.to_bytes().unwrap();
        match parse_type_only(&bytes).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::TypeOnly(pkt), consumed) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PUBLISH);
                assert_eq!(pkt.flags, 0x00);
                assert_eq!(consumed, bytes.len());
            }
            other => panic!("Expected TypeOnly packet, got {:?}", other),
        }
    }

    #[test]
    fn test_type_only_publish_v3() {
        let publish =
            publishv3::MqttPublish::new("a/b".to_string(), 1, vec![1, 2, 3], Some(42), true, true);
        let bytes = publish.to_bytes().unwrap();
        match parse_type_only(&bytes).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::TypeOnly(pkt), consumed) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PUBLISH);
                // DUP=1, QoS=1, RETAIN=1 => flags = 0x0B
                assert_eq!(pkt.flags, 0x0B);
                assert_eq!(consumed, bytes.len());
            }
            other => panic!("Expected TypeOnly packet, got {:?}", other),
        }
    }

    #[test]
    fn test_type_only_incomplete() {
        // Only 1 byte, no VBI
        let bytes = [0x30];
        match parse_type_only(&bytes).unwrap() {
            LeveledParseOk::Continue(_, _) => {}
            other => panic!("Expected Continue, got {:?}", other),
        }
    }

    #[test]
    fn test_type_only_partial_packet() {
        let publish =
            publishv5::MqttPublish::new(0, "test".to_string(), None, vec![0x61; 50], false, false);
        let bytes = publish.to_bytes().unwrap();
        // Feed only half the bytes
        let half = &bytes[..bytes.len() / 2];
        match parse_type_only(half).unwrap() {
            LeveledParseOk::Continue(needed, _) => {
                assert!(needed > 0);
            }
            other => panic!("Expected Continue, got {:?}", other),
        }
    }

    // ── Level 2 tests ──────────────────────────────────────────────────

    #[test]
    fn test_raw_body_publish_v5() {
        let publish = publishv5::MqttPublish::new(
            0,
            "test/topic".to_string(),
            None,
            vec![0x61; 100],
            false,
            false,
        );
        let bytes = Bytes::from(publish.to_bytes().unwrap());
        let vbi_len = {
            let (_, vbi) = parse_remaining_length(&bytes[1..]).unwrap();
            vbi
        };
        let expected_remaining_len = bytes.len() - 1 - vbi_len;

        match parse_raw_body(bytes.clone()).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::RawBody(pkt), consumed) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PUBLISH);
                assert_eq!(pkt.remaining.len(), expected_remaining_len);
                assert_eq!(consumed, bytes.len());
            }
            other => panic!("Expected RawBody packet, got {:?}", other),
        }
    }

    #[test]
    fn test_raw_body_pingreq() {
        // PINGREQ is 0xC0, 0x00 (no remaining data)
        let bytes = Bytes::from_static(&[0xC0, 0x00]);
        match parse_raw_body(bytes).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::RawBody(pkt), consumed) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PINGREQ);
                assert!(pkt.remaining.is_empty());
                assert_eq!(consumed, 2);
            }
            other => panic!("Expected RawBody packet, got {:?}", other),
        }
    }

    // ── Level 1 tests ──────────────────────────────────────────────────

    #[test]
    fn test_headers_parsed_publish_v5_qos0() {
        let publish = publishv5::MqttPublish::new(
            0,
            "test/topic".to_string(),
            None,
            vec![0x61; 50],
            false,
            false,
        );
        let bytes = Bytes::from(publish.to_bytes().unwrap());
        match parse_headers_only(bytes.clone(), 5).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::HeadersParsed(pkt), consumed) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PUBLISH);
                assert_eq!(consumed, bytes.len());
                match &pkt.variable_header {
                    VariableHeader::PublishV5 {
                        topic_name,
                        qos,
                        packet_id,
                        ..
                    } => {
                        assert_eq!(topic_name, "test/topic");
                        assert_eq!(*qos, 0);
                        assert_eq!(*packet_id, None);
                    }
                    other => panic!("Expected PublishV5, got {:?}", other),
                }
                assert_eq!(pkt.raw_payload.len(), 50);
                assert!(pkt.raw_payload.iter().all(|&b| b == 0x61));
            }
            other => panic!("Expected HeadersParsed packet, got {:?}", other),
        }
    }

    #[test]
    fn test_headers_parsed_publish_v5_qos1() {
        let publish = publishv5::MqttPublish::new(
            1,
            "a/b".to_string(),
            Some(42),
            vec![0xBB; 10],
            true,
            false,
        );
        let bytes = Bytes::from(publish.to_bytes().unwrap());
        match parse_headers_only(bytes.clone(), 5).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::HeadersParsed(pkt), consumed) => {
                assert_eq!(consumed, bytes.len());
                match &pkt.variable_header {
                    VariableHeader::PublishV5 {
                        topic_name,
                        qos,
                        retain,
                        packet_id,
                        ..
                    } => {
                        assert_eq!(topic_name, "a/b");
                        assert_eq!(*qos, 1);
                        assert!(*retain);
                        assert_eq!(*packet_id, Some(42));
                    }
                    other => panic!("Expected PublishV5, got {:?}", other),
                }
                assert_eq!(pkt.raw_payload.len(), 10);
            }
            other => panic!("Expected HeadersParsed packet, got {:?}", other),
        }
    }

    #[test]
    fn test_headers_parsed_publish_v3() {
        let publish = publishv3::MqttPublish::new(
            "topic".to_string(),
            1,
            vec![1, 2, 3],
            Some(7),
            false,
            false,
        );
        let bytes = Bytes::from(publish.to_bytes().unwrap());
        match parse_headers_only(bytes.clone(), 4).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::HeadersParsed(pkt), consumed) => {
                assert_eq!(consumed, bytes.len());
                match &pkt.variable_header {
                    VariableHeader::PublishV3 {
                        topic_name,
                        qos,
                        message_id,
                        ..
                    } => {
                        assert_eq!(topic_name, "topic");
                        assert_eq!(*qos, 1);
                        assert_eq!(*message_id, Some(7));
                    }
                    other => panic!("Expected PublishV3, got {:?}", other),
                }
                assert_eq!(pkt.raw_payload.len(), 3);
            }
            other => panic!("Expected HeadersParsed packet, got {:?}", other),
        }
    }

    #[test]
    fn test_headers_parsed_pingreq_v5() {
        let bytes = Bytes::from_static(&[0xC0, 0x00]);
        match parse_headers_only(bytes, 5).unwrap() {
            LeveledParseOk::Packet(ParsedPacket::HeadersParsed(pkt), consumed) => {
                assert_eq!(pkt.packet_type, ControlPacketType::PINGREQ);
                assert_eq!(consumed, 2);
                assert!(matches!(pkt.variable_header, VariableHeader::Empty));
                assert!(pkt.raw_payload.is_empty());
            }
            other => panic!("Expected HeadersParsed packet, got {:?}", other),
        }
    }

    // ── Cross-level consistency tests ──────────────────────────────────

    #[test]
    fn test_consumed_consistency_publish_v5() {
        let publish = publishv5::MqttPublish::new(
            2,
            "test/topic".to_string(),
            Some(999),
            vec![0xAA; 200],
            true,
            true,
        );
        let bytes = Bytes::from(publish.to_bytes().unwrap());

        let consumed_l0 = match MqttPacket::from_bytes_v5(&bytes).unwrap() {
            crate::mqtt_serde::parser::ParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };
        let consumed_l1 = match parse_headers_only(bytes.clone(), 5).unwrap() {
            LeveledParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };
        let consumed_l2 = match parse_raw_body(bytes.clone()).unwrap() {
            LeveledParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };
        let consumed_l3 = match parse_type_only(&bytes).unwrap() {
            LeveledParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };

        assert_eq!(consumed_l0, consumed_l1);
        assert_eq!(consumed_l1, consumed_l2);
        assert_eq!(consumed_l2, consumed_l3);
        assert_eq!(consumed_l3, bytes.len());
    }

    #[test]
    fn test_consumed_consistency_publish_v3() {
        let publish = publishv3::MqttPublish::new(
            "topic/v3".to_string(),
            2,
            vec![0xBB; 64],
            Some(123),
            false,
            true,
        );
        let bytes = Bytes::from(publish.to_bytes().unwrap());

        let consumed_l0 = match MqttPacket::from_bytes_v3(&bytes).unwrap() {
            crate::mqtt_serde::parser::ParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };
        let consumed_l1 = match parse_headers_only(bytes.clone(), 4).unwrap() {
            LeveledParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };
        let consumed_l2 = match parse_raw_body(bytes.clone()).unwrap() {
            LeveledParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };
        let consumed_l3 = match parse_type_only(&bytes).unwrap() {
            LeveledParseOk::Packet(_, c) => c,
            _ => panic!("Expected Packet"),
        };

        assert_eq!(consumed_l0, consumed_l1);
        assert_eq!(consumed_l1, consumed_l2);
        assert_eq!(consumed_l2, consumed_l3);
    }
}
