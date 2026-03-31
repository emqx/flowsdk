// SPDX-License-Identifier: MPL-2.0

/// Generates a v3.1.1 "simple acknowledgement" packet: a struct with a single
/// `message_id: u16` field, no payload, and a fixed-header-only wire format.
///
/// Used by PUBACK, PUBREC, PUBCOMP, PUBREL, and UNSUBACK.
///
/// Parameters:
/// - `$struct_name`: the generated struct name (e.g. `MqttPubAck`)
/// - `$packet_type`: a `ControlPacketType` variant name (e.g. `PUBACK`)
/// - `$flags`: the expected fixed-header flags nibble (e.g. `0x00` or `0x02` for PUBREL)
/// - `$packet_name`: a string literal used in error messages (e.g. `"PUBACK"`)
/// - `$packet_variant`: the `MqttPacket` enum variant to wrap the decoded packet (e.g. `PubAck3`)
macro_rules! v3_packet_id_ack {
    (
        $struct_name:ident,
        $packet_type:ident,
        $flags:expr,
        $packet_name:literal,
        $packet_variant:ident $(,)?
    ) => {
        #[derive(Debug, PartialEq, Eq, Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
        pub struct $struct_name {
            pub message_id: u16,
        }

        impl $struct_name {
            pub fn new(message_id: u16) -> Self {
                Self { message_id }
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
                Ok(self.message_id.to_be_bytes().to_vec())
            }

            fn payload(&self) -> Result<Vec<u8>, $crate::mqtt_serde::parser::ParseError> {
                Ok(Vec::new())
            }

            fn from_bytes(
                buffer: &[u8],
            ) -> Result<$crate::mqtt_serde::parser::ParseOk, $crate::mqtt_serde::parser::ParseError>
            {
                let pkt_type = $crate::mqtt_serde::parser::packet_type(buffer)?;
                if pkt_type
                    != $crate::mqtt_serde::control_packet::ControlPacketType::$packet_type as u8
                {
                    return Err($crate::mqtt_serde::parser::ParseError::InvalidPacketType);
                }

                let flags = buffer[0] & 0x0F;
                if flags != $flags {
                    return Err($crate::mqtt_serde::parser::ParseError::ParseError(
                        concat!($packet_name, " packet has invalid fixed header flags").to_string(),
                    ));
                }

                let (size, vbi_len) =
                    $crate::mqtt_serde::parser::parse_remaining_length(&buffer[1..])?;
                let total_len = 1 + vbi_len + size;

                if total_len > buffer.len() {
                    return Ok($crate::mqtt_serde::parser::ParseOk::Continue(
                        total_len - buffer.len(),
                        0,
                    ));
                }

                if size != 2 {
                    return Err($crate::mqtt_serde::parser::ParseError::ParseError(
                        concat!($packet_name, " packet must have remaining length of 2")
                            .to_string(),
                    ));
                }

                let message_id = u16::from_be_bytes([buffer[1 + vbi_len], buffer[1 + vbi_len + 1]]);

                Ok($crate::mqtt_serde::parser::ParseOk::Packet(
                    $crate::mqtt_serde::control_packet::MqttPacket::$packet_variant(
                        $struct_name::new(message_id),
                    ),
                    total_len,
                ))
            }
        }
    };
}

pub(crate) use v3_packet_id_ack;
