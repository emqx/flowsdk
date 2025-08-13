use super::control_packet::MqttPacket;
use crate::mqtt_serde::base_data::{BinaryData, TwoByteInteger, Utf8String, VariableByteInteger};
use std::error::Error;
use std::fmt;
use std::io::Error as IoError;

pub type ParserResult = Result<ParseOk, ParseError>;

// First byte of Fixed header
pub const FIXED_HDR_LEN: usize = 1;

#[derive(Debug)]
pub enum ParseError {
    More(usize, String), // not enough data for processing, hint for how many more bytes are needed
    IoError(IoError),
    ParseError(String),
    IncompleteProperty,
    // following two are for UTF-8 parsing errors
    Utf8Error(std::str::Utf8Error),
    FromUtf8Error(std::string::FromUtf8Error),
    StringTooLong,
    BufferTooShort,
    InvalidLength,
    InvalidPropertyId,
    InvalidPacketType,
    UnSuppProtoVsn,
    InternalError(String),
    InvalidVariableByteInteger,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::More(hint, msg) => write!(f, "More data needed ({} bytes): {}", hint, msg),
            ParseError::IoError(e) => write!(f, "IO Error: {}", e),
            ParseError::ParseError(msg) => write!(f, "Parse Error: {}", msg),
            ParseError::IncompleteProperty => write!(f, "Incomplete Property"),
            ParseError::Utf8Error(e) => write!(f, "UTF-8 Error: {}", e),
            ParseError::FromUtf8Error(e) => write!(f, "From UTF-8 Error: {}", e),
            ParseError::StringTooLong => write!(f, "String Too Long"),
            ParseError::BufferTooShort => write!(f, "Buffer Too Short"),
            ParseError::InvalidLength => write!(f, "Invalid Length"),
            ParseError::InvalidPropertyId => write!(f, "Invalid Property ID"),
            ParseError::InvalidPacketType => write!(f, "Invalid Packet Type"),
            ParseError::UnSuppProtoVsn => write!(f, "Unsupported Protocol Version"),
            ParseError::InternalError(msg) => write!(f, "Internal Error: {}", msg),
            ParseError::InvalidVariableByteInteger => write!(f, "Invalid Variable Byte Integer"),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ParseError::IoError(e) => Some(e),
            ParseError::Utf8Error(e) => Some(e),
            ParseError::FromUtf8Error(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum ParseOk {
    Continue(usize, usize),    // (hint, consumed)
    Packet(MqttPacket, usize), // (packet, consumed)
    TopicName(String, usize),  // (topic_name, consumed)
}

pub fn packet_type(buffer: &[u8]) -> Result<u8, ParseError> {
    if buffer.is_empty() {
        return Err(ParseError::BufferTooShort);
    }
    Ok(buffer[0] >> 4)
}

pub fn parse_remaining_length(buffer: &[u8]) -> Result<(usize, usize), ParseError> {
    vbi(buffer)
}

// 1.5.4 UTF-8 Encoded String
pub fn parse_utf8_string(buffer: &[u8]) -> Result<(String, usize), ParseError> {
    Utf8String::decode(buffer)
}

pub fn parse_topic_name(buffer: &[u8]) -> Result<ParseOk, ParseError> {
    let (res, consumed) = parse_utf8_string(buffer)?;
    Ok(ParseOk::TopicName(res.to_string(), consumed))
}

pub fn parse_packet_id(buffer: &[u8]) -> Result<(u16, usize), ParseError> {
    TwoByteInteger::decode(buffer)
}

pub fn parse_binary_data(buffer: &[u8]) -> Result<(Vec<u8>, usize), ParseError> {
    BinaryData::decode(buffer)
}

pub fn parse_vbi(buffer: &[u8]) -> Result<(usize, usize), ParseError> {
    vbi(buffer)
}

/*                   */
/* Internal helpers  */
/*                   */

// Variable Byte Integer
fn vbi(buffer: &[u8]) -> Result<(usize, usize), ParseError> {
    let (value, consumed) = VariableByteInteger::decode(buffer)?;
    Ok((value as usize, consumed))
}

pub mod stream;
