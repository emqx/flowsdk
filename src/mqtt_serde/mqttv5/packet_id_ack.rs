// SPDX-License-Identifier: MPL-2.0

/// Generates a v5.0 "packet identifier acknowledgement" packet: a struct with
/// `packet_id: u16`, `reason_code: u8`, and `properties: Vec<Property>` fields,
/// no payload, and the compact wire format defined in MQTT 5.0 §3.x.2.
///
/// Used by PUBACK, PUBREC, PUBCOMP, and PUBREL.
///
/// Parameters:
/// - `$struct_name`:    the generated struct name (e.g. `MqttPubAck`)
/// - `$packet_type`:    a `ControlPacketType` variant (e.g. `PUBACK`)
/// - `$flags`:          the fixed-header flags nibble (`0x00` for all except PUBREL which uses `0x02`)
/// - `$packet_name`:    a string literal for error messages (e.g. `"PUBACK"`)
/// - `$packet_variant`: the `MqttPacket` enum variant (e.g. `PubAck5`)
/// - `$strict_flags`:   whether to validate the flags nibble under `strict-protocol-compliance`
///   (only needed for PUBREL; pass `true` or `false`)
macro_rules! v5_packet_id_ack {
    (
        $struct_name:ident,
        $packet_type:ident,
        $flags:expr,
        $packet_name:literal,
        $packet_variant:ident,
        $strict_flags:expr $(,)?
    ) => {
        #[derive(Debug, PartialEq, Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
        pub struct $struct_name {
            pub packet_id: u16,
            pub reason_code: u8,
            pub properties: Vec<$crate::mqtt_serde::mqttv5::common::properties::Property>,
        }

        impl $struct_name {
            pub fn new(
                packet_id: u16,
                reason_code: u8,
                properties: Vec<$crate::mqtt_serde::mqttv5::common::properties::Property>,
            ) -> Self {
                Self {
                    packet_id,
                    reason_code,
                    properties,
                }
            }

            pub fn new_success(packet_id: u16) -> Self {
                Self::new(packet_id, 0x00, Vec::new())
            }

            pub fn new_error(
                packet_id: u16,
                reason_code: u8,
                properties: Vec<$crate::mqtt_serde::mqttv5::common::properties::Property>,
            ) -> Self {
                Self::new(packet_id, reason_code, properties)
            }
        }

        impl $crate::mqtt_serde::control_packet::MqttControlPacket for $struct_name {
            fn control_packet_type(&self) -> u8 {
                $crate::mqtt_serde::control_packet::ControlPacketType::$packet_type as u8
            }

            fn flags(&self) -> u8 {
                $flags
            }

            fn variable_header(&self) -> Result<Vec<u8>, $crate::mqtt_serde::parser::ParseError> {
                use $crate::mqtt_serde::mqttv5::common::properties::encode_properities_hdr;
                let mut bytes = self.packet_id.to_be_bytes().to_vec();
                if self.reason_code == 0x00 && self.properties.is_empty() {
                    return Ok(bytes);
                }
                bytes.push(self.reason_code);
                bytes.extend(encode_properities_hdr(&self.properties)?);
                Ok(bytes)
            }

            fn payload(&self) -> Result<Vec<u8>, $crate::mqtt_serde::parser::ParseError> {
                Ok(Vec::new())
            }

            fn from_bytes(
                buffer: &[u8],
            ) -> Result<$crate::mqtt_serde::parser::ParseOk, $crate::mqtt_serde::parser::ParseError>
            {
                use $crate::mqtt_serde::mqttv5::common::properties::parse_properties_hdr;
                use $crate::mqtt_serde::parser::{
                    packet_type, parse_packet_id, parse_remaining_length,
                };

                let pkt_type = packet_type(buffer)?;
                if pkt_type
                    != $crate::mqtt_serde::control_packet::ControlPacketType::$packet_type as u8
                {
                    return Err($crate::mqtt_serde::parser::ParseError::InvalidPacketType);
                }

                #[cfg(feature = "strict-protocol-compliance")]
                if $strict_flags {
                    let flags = buffer[0] & 0x0F;
                    if flags != $flags {
                        return Err($crate::mqtt_serde::parser::ParseError::ParseError(format!(
                            "Invalid {} flags: expected 0x{:02x}, got 0x{:02x}",
                            $packet_name, $flags, flags
                        )));
                    }
                }

                let (size, vbi_len) = parse_remaining_length(&buffer[1..])?;
                let mut offset: usize = 1 + vbi_len;
                let total_len = offset + size;

                if total_len > buffer.len() {
                    return Ok($crate::mqtt_serde::parser::ParseOk::Continue(
                        total_len - buffer.len(),
                        0,
                    ));
                }

                let (packet_id, consumed) = parse_packet_id(
                    buffer
                        .get(offset..)
                        .ok_or($crate::mqtt_serde::parser::ParseError::BufferTooShort)?,
                )?;
                offset += consumed;

                let reason_code = if size > 2 {
                    let code = *buffer
                        .get(offset)
                        .ok_or($crate::mqtt_serde::parser::ParseError::BufferTooShort)?;
                    offset += 1;
                    code
                } else {
                    0x00
                };

                let (properties, consumed) = if offset < total_len {
                    parse_properties_hdr(
                        buffer
                            .get(offset..total_len)
                            .ok_or($crate::mqtt_serde::parser::ParseError::BufferTooShort)?,
                    )?
                } else {
                    (vec![], 0)
                };
                offset += consumed;

                if offset != total_len {
                    return Err($crate::mqtt_serde::parser::ParseError::InternalError(
                        format!("Inconsistent offset {} != total: {}", offset, total_len),
                    ));
                }

                Ok($crate::mqtt_serde::parser::ParseOk::Packet(
                    $crate::mqtt_serde::control_packet::MqttPacket::$packet_variant($struct_name {
                        packet_id,
                        reason_code,
                        properties,
                    }),
                    offset,
                ))
            }
        }
    };
}

pub(crate) use v5_packet_id_ack;
