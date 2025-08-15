use crate::mqtt_serde::parser::ParseError;

pub struct TwoByteInteger;

impl TwoByteInteger {
    pub fn encode(val: u16) -> [u8; 2] {
        val.to_be_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<(u16, usize), ParseError> {
        if bytes.len() < 2 {
            return Err(ParseError::BufferTooShort);
        }
        let mut array = [0u8; 2];
        array.copy_from_slice(&bytes[0..2]);
        Ok((u16::from_be_bytes(array), 2))
    }
}

pub struct FourByteInteger;

impl FourByteInteger {
    pub fn encode(val: u32) -> [u8; 4] {
        val.to_be_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<(u32, usize), ParseError> {
        if bytes.len() < 4 {
            return Err(ParseError::BufferTooShort);
        }
        let mut array = [0u8; 4];
        array.copy_from_slice(&bytes[0..4]);
        Ok((u32::from_be_bytes(array), 4))
    }
}

pub struct VariableByteInteger;

impl VariableByteInteger {
    pub fn encode(val: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut num = val;
        loop {
            let mut byte = (num % 128) as u8;
            num /= 128;
            if num > 0 {
                byte |= 128;
            }
            bytes.push(byte);
            if num == 0 {
                break;
            }
        }
        bytes
    }

    pub fn decode(buffer: &[u8]) -> Result<(usize, usize), ParseError> {
        let mut multiplier = 1;
        let mut value = 0;
        let mut i: usize = 0;

        if buffer.is_empty() {
            return Err(ParseError::BufferTooShort);
        }

        loop {
            let byte = *buffer.get(i).ok_or(ParseError::More(
                1,
                "vbi: not enough bytes for remaining length".to_string(),
            ))?;

            if byte > 127 && i == 3 {
                // most significant bit is 1 and we have 4 bytes, so the remaining length is invalid
                return Err(ParseError::ParseError(
                    "invalid remaining length, MSB is 1".to_string(),
                ));
            }

            value += (byte & 127) as usize * multiplier;
            assert!(
                value <= 268_435_455,
                "Remaining length must not exceed 268435455"
            );

            multiplier *= 128;

            i += 1;
            if byte & 128 == 0 {
                // most significant bit is 0, no more bytes following
                // so stop here.
                break;
            }
        }

        // MQTT 5.0 Protocol Compliance: Check for non-minimal VBI encoding
        #[cfg(feature = "strict-protocol-compliance")]
        {
            // A VBI encoding is non-minimal if it could be represented with fewer bytes
            // This happens when there are unnecessary continuation bytes
            let minimal_encoded = VariableByteInteger::encode(value as u32);
            if minimal_encoded.len() != i {
                return Err(ParseError::ParseError(
                    "Variable Byte Integer encoding is not minimal".to_string(),
                ));
            }
        }

        Ok((value, i))
    }
}

pub struct BinaryData;

impl BinaryData {
    pub fn encode(data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(2 + data.len());
        bytes.extend_from_slice(&(data.len() as u16).to_be_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<(Vec<u8>, usize), ParseError> {
        let (len, _) = TwoByteInteger::decode(bytes)?;
        let len = len as usize;
        let start = 2;
        let end = start + len;
        if bytes.len() < end {
            return Err(ParseError::BufferTooShort);
        }
        Ok((bytes[start..end].to_vec(), end))
    }
}

pub struct Utf8String;

impl Utf8String {
    pub fn encode(s: &str) -> Vec<u8> {
        BinaryData::encode(s.as_bytes())
    }

    pub fn decode(bytes: &[u8]) -> Result<(String, usize), ParseError> {
        let (data, len) = BinaryData::decode(bytes)?;
        let s = String::from_utf8(data).map_err(|e| ParseError::Utf8Error(e.utf8_error()))?;
        Ok((s, len))
    }
}

pub struct Utf8StringPair;

impl Utf8StringPair {
    pub fn encode(key: &str, value: &str) -> Vec<u8> {
        let mut bytes = Utf8String::encode(key);
        bytes.extend_from_slice(&Utf8String::encode(value));
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<((String, String), usize), ParseError> {
        let (key, key_len) = Utf8String::decode(bytes)?;
        let (value, value_len) = Utf8String::decode(&bytes[key_len..])?;
        Ok(((key, value), key_len + value_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_two_byte_integer() {
        let val = 12345u16;
        let encoded = TwoByteInteger::encode(val);
        let (decoded, len) = TwoByteInteger::decode(&encoded).unwrap();
        assert_eq!(val, decoded);
        assert_eq!(2, len);
    }

    #[test]
    fn test_four_byte_integer() {
        let val = 1234567890u32;
        let encoded = FourByteInteger::encode(val);
        let (decoded, len) = FourByteInteger::decode(&encoded).unwrap();
        assert_eq!(val, decoded);
        assert_eq!(4, len);
    }

    #[test]
    fn test_variable_byte_integer() {
        let values = [0, 127, 128, 16383, 16384, 2097151, 2097152, 268435455];
        for &val in &values {
            let encoded = VariableByteInteger::encode(val);
            let (decoded, len) = VariableByteInteger::decode(&encoded).unwrap();
            assert_eq!(val, decoded as u32);
            assert_eq!(encoded.len(), len);
        }

        assert_eq!((0, 1), VariableByteInteger::decode(&[0x00]).unwrap());
        assert_eq!((0, 1), VariableByteInteger::decode(&[0x00, 0x00]).unwrap());
        assert_eq!(
            (128, 2),
            VariableByteInteger::decode(&[0x80, 0x01]).unwrap()
        );
        assert_eq!(
            (129, 2),
            VariableByteInteger::decode(&[0x81, 0x01]).unwrap()
        );
        assert_eq!(
            (16383, 2),
            VariableByteInteger::decode(&[0xff, 0x7f]).unwrap()
        );
        assert_eq!(
            (16384, 3),
            VariableByteInteger::decode(&[0x80, 0x80, 0x01]).unwrap()
        );
        assert_eq!(
            (2097151, 3),
            VariableByteInteger::decode(&[0xff, 0xff, 0x7f]).unwrap()
        );
        assert_eq!(
            (2097152, 4),
            VariableByteInteger::decode(&[0x80, 0x80, 0x80, 0x01]).unwrap()
        );
        assert_eq!(
            (268435455, 4),
            VariableByteInteger::decode(&[0xff, 0xff, 0xff, 0x7f]).unwrap()
        );

        // test invalid
        assert!(matches!(
            VariableByteInteger::decode(&[0xff, 0xff, 0xff, 0x81]),
            Err(ParseError::ParseError(_))
        ));
        assert!(matches!(
            VariableByteInteger::decode(&[0x80, 0x80, 0x80, 0x80]),
            Err(ParseError::ParseError(_))
        ));
        assert!(matches!(
            VariableByteInteger::decode(&[0xff, 0xff, 0xff, 0xff]),
            Err(ParseError::ParseError(_))
        ));

        assert!(matches!(
            VariableByteInteger::decode(&[0xff]),
            Err(ParseError::More(1, _))
        ));
        assert!(matches!(
            VariableByteInteger::decode(&[0xff, 0xff]),
            Err(ParseError::More(1, _))
        ));
        assert!(matches!(
            VariableByteInteger::decode(&[0xff, 0x80, 0x80]),
            Err(ParseError::More(1, _))
        ));
    }

    #[test]
    fn test_binary_data() {
        let data = b"hello world";
        let encoded = BinaryData::encode(data);
        let (decoded, len) = BinaryData::decode(&encoded).unwrap();
        assert_eq!(data.to_vec(), decoded);
        assert_eq!(encoded.len(), len);
    }

    #[test]
    fn test_utf8_string() {
        let s = "hello world";
        let encoded = Utf8String::encode(s);
        let (decoded, len) = Utf8String::decode(&encoded).unwrap();
        assert_eq!(s, decoded);
        assert_eq!(encoded.len(), len);
    }

    #[test]
    fn test_utf8_string_pair() {
        let key = "key";
        let value = "value";
        let encoded = Utf8StringPair::encode(key, value);
        let ((decoded_key, decoded_value), len) = Utf8StringPair::decode(&encoded).unwrap();
        assert_eq!(key, decoded_key);
        assert_eq!(value, decoded_value);
        assert_eq!(encoded.len(), len);
    }
}
