// SPDX-License-Identifier: MPL-2.0

// Integration tests for MQTT parser module
// These tests are run as integration tests from the tests/ directory

use flowsdk::mqtt_serde::control_packet::{MqttControlPacket, MqttPacket};
use flowsdk::mqtt_serde::mqttv3::connect::MqttConnect;
use flowsdk::mqtt_serde::mqttv3::disconnect::MqttDisconnect;
use flowsdk::mqtt_serde::mqttv3::pingreq::MqttPingReq;
use flowsdk::mqtt_serde::parser::{
    packet_type, parse_binary_data, parse_packet_id, parse_remaining_length, parse_utf8_string,
    stream::MqttParser, ParseError,
};

// Test helper functions
fn create_mqttv5_connect_packet() -> Vec<u8> {
    // Based on the working test from connect.rs
    vec![
        0x10, 0x22, 0x00, 0x04, 0x4d, 0x51, 0x54, 0x54, 0x05, 0x02, 0x01, 0x2c, 0x05, 0x11, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x10, 0x71, 0x75, 0x69, 0x63, 0x5f, 0x62, 0x65, 0x6e, 0x63, 0x68,
        0x5f, 0x70, 0x75, 0x62, 0x5f, 0x31,
    ]
}

fn create_mqttv3_connect_packet() -> Vec<u8> {
    // Create a proper MQTT v3.1.1 CONNECT packet using the MqttConnect structure
    let connect = MqttConnect::new("tester".to_string(), 60, true);
    connect.to_bytes().unwrap()
}

fn create_mqttv5_pingreq_packet() -> Vec<u8> {
    vec![0xc0, 0x00]
}

fn create_mqttv3_pingreq_packet() -> Vec<u8> {
    let pingreq = MqttPingReq::new();
    pingreq.to_bytes().unwrap()
}

fn create_mqttv5_disconnect_packet() -> Vec<u8> {
    vec![0xe0, 0x00]
}

fn create_mqttv3_disconnect_packet() -> Vec<u8> {
    let disconnect = MqttDisconnect::new();
    disconnect.to_bytes().unwrap()
}

// Basic parser function tests
#[test]
fn test_packet_type_extraction() {
    assert_eq!(packet_type(&[0x10]).unwrap(), 1); // CONNECT
    assert_eq!(packet_type(&[0x20]).unwrap(), 2); // CONNACK
    assert_eq!(packet_type(&[0xc0]).unwrap(), 12); // PINGREQ
    assert_eq!(packet_type(&[0xe0]).unwrap(), 14); // DISCONNECT

    // Test empty buffer
    assert!(matches!(packet_type(&[]), Err(ParseError::BufferEmpty)));
}

#[test]
fn test_parse_remaining_length() {
    // Test single byte length (0-127)
    assert_eq!(parse_remaining_length(&[0x00]).unwrap(), (0, 1));
    assert_eq!(parse_remaining_length(&[0x7f]).unwrap(), (127, 1));

    // Test two byte length (128-16383)
    assert_eq!(parse_remaining_length(&[0x80, 0x01]).unwrap(), (128, 2));
    assert_eq!(parse_remaining_length(&[0xff, 0x7f]).unwrap(), (16383, 2));

    // Test incomplete buffer
    assert!(parse_remaining_length(&[0x80]).is_err());
}

#[test]
fn test_parse_utf8_string() {
    let data = vec![0x00, 0x05, b'h', b'e', b'l', b'l', b'o'];
    let (result, consumed) = parse_utf8_string(&data).unwrap();
    assert_eq!(result, "hello");
    assert_eq!(consumed, 7);

    // Test empty string
    let data = vec![0x00, 0x00];
    let (result, consumed) = parse_utf8_string(&data).unwrap();
    assert_eq!(result, "");
    assert_eq!(consumed, 2);

    // Test incomplete buffer
    let data = vec![0x00, 0x05, b'h', b'i'];
    assert!(parse_utf8_string(&data).is_err());
}

#[test]
fn test_parse_packet_id() {
    let data = vec![0x12, 0x34];
    let (result, consumed) = parse_packet_id(&data).unwrap();
    assert_eq!(result, 0x1234);
    assert_eq!(consumed, 2);

    // Test incomplete buffer
    let data = vec![0x12];
    assert!(parse_packet_id(&data).is_err());
}

#[test]
fn test_parse_binary_data() {
    let data = vec![0x00, 0x04, 0x01, 0x02, 0x03, 0x04];
    let (result, consumed) = parse_binary_data(&data).unwrap();
    assert_eq!(result, vec![0x01, 0x02, 0x03, 0x04]);
    assert_eq!(consumed, 6);

    // Test empty binary data
    let data = vec![0x00, 0x00];
    let (result, consumed) = parse_binary_data(&data).unwrap();
    assert_eq!(result, Vec::<u8>::new());
    assert_eq!(consumed, 2);
}

// MQTT v5.0 Stream Parser Tests
mod mqttv5_stream_tests {
    use super::*;

    #[test]
    fn test_mqtt5_parser_creation() {
        let parser = MqttParser::new(1024, 5);
        assert_eq!(parser.get_mqtt_vsn(), 5);
    }

    #[test]
    fn test_mqtt5_parser_default() {
        let parser = MqttParser::default();
        assert_eq!(parser.get_mqtt_vsn(), 0); // Undefined initially
    }

    #[test]
    fn test_mqtt5_connect_packet_parsing() {
        let mut parser = MqttParser::new(1024, 5);
        let connect_packet = create_mqttv5_connect_packet();

        // Feed the complete packet
        parser.feed(&connect_packet);

        // Parse the packet
        let result = parser.next_packet().unwrap();
        assert!(
            result.is_some(),
            "Parser should successfully parse the MQTT v5 CONNECT packet"
        );

        if let Some(MqttPacket::Connect5(connect)) = result {
            // This packet has client_id "quic_bench_pub_1" based on our test data
            assert_eq!(connect.client_id, "quic_bench_pub_1");
            assert_eq!(connect.protocol_version, 5);
        } else {
            panic!("Expected Connect5 packet, got: {:?}", result);
        }
    }

    #[test]
    fn test_mqtt5_partial_packet_feeding() {
        let mut parser = MqttParser::new(1024, 5);
        let connect_packet = create_mqttv5_connect_packet();

        // Feed only part of the packet
        parser.feed(&connect_packet[..10]);

        // Should return None (incomplete packet)
        let result = parser.next_packet().unwrap();
        assert!(result.is_none());

        // Feed the rest
        parser.feed(&connect_packet[10..]);

        // Now should parse successfully
        let result = parser.next_packet().unwrap();
        assert!(result.is_some());

        if let Some(MqttPacket::Connect5(_)) = result {
            // Success
        } else {
            panic!("Expected Connect5 packet");
        }
    }

    #[test]
    fn test_mqtt5_multiple_packets() {
        let mut parser = MqttParser::new(1024, 5);
        let connect_packet = create_mqttv5_connect_packet();

        // Feed the connect packet twice to test multiple parsing
        parser.feed(&connect_packet);
        parser.feed(&connect_packet);

        // Parse first packet (CONNECT)
        let result1 = parser.next_packet().unwrap();
        assert!(matches!(result1, Some(MqttPacket::Connect5(_))));

        // Parse second packet (another CONNECT)
        let result2 = parser.next_packet().unwrap();
        assert!(matches!(result2, Some(MqttPacket::Connect5(_))));

        // No more packets - this may return an error if buffer is empty
        match parser.next_packet() {
            Ok(None) => {
                // Expected: no more packets
            }
            Ok(Some(_)) => {
                panic!("Expected no more packets");
            }
            Err(_) => {
                // BufferEmpty error is acceptable when no more packets
            }
        }
    }

    #[test]
    fn test_mqtt5_pingreq_parsing() {
        let mut parser = MqttParser::new(1024, 5);
        let pingreq_packet = create_mqttv5_pingreq_packet();

        parser.feed(&pingreq_packet);
        let result = parser.next_packet().unwrap();

        assert!(matches!(result, Some(MqttPacket::PingReq5(_))));
    }

    #[test]
    fn test_mqtt5_disconnect_parsing() {
        let mut parser = MqttParser::new(1024, 5);
        let disconnect_packet = create_mqttv5_disconnect_packet();

        parser.feed(&disconnect_packet);
        let result = parser.next_packet().unwrap();

        if let Some(MqttPacket::Disconnect5(disconnect)) = result {
            assert_eq!(disconnect.reason_code, 0); // Normal disconnection
        } else {
            panic!("Expected Disconnect5 packet");
        }
    }

    #[test]
    fn test_mqtt5_version_detection() {
        let mut parser = MqttParser::new(1024, 0); // Undefined version
        let connect_packet = create_mqttv5_connect_packet();

        // Feed enough data to detect version
        parser.feed(&connect_packet[..15]); // Include version byte

        // Try to set version
        let version = parser.set_mqtt_vsn(0).unwrap(); // Skip fixed header byte
        assert_eq!(version, 5);
        assert_eq!(parser.get_mqtt_vsn(), 5);
    }

    #[test]
    fn test_mqtt5_buffer_management() {
        let mut parser = MqttParser::new(1024, 5);
        let connect_packet = create_mqttv5_connect_packet();

        parser.feed(&connect_packet);
        assert_eq!(parser.buffer_mut().len(), connect_packet.len());

        // Parse the packet (should consume from buffer)
        let result = parser.next_packet().unwrap();
        if result.is_some() {
            assert_eq!(parser.buffer_mut().len(), 0); // Buffer should be empty after successful parse
        } else {
            // If parsing failed, buffer may still contain data
            println!("Parsing failed, buffer len: {}", parser.buffer_mut().len());
        }
    }
}

// MQTT v3.1.1 Stream Parser Tests
mod mqttv3_stream_tests {
    use super::*;

    #[test]
    fn test_mqtt3_parser_creation() {
        let parser = MqttParser::new(1024, 4);
        assert_eq!(parser.get_mqtt_vsn(), 4);
    }

    #[test]
    fn test_mqtt3_connect_packet_parsing() {
        let mut parser = MqttParser::new(1024, 4);
        let connect_packet = create_mqttv3_connect_packet();

        // Feed the complete packet
        parser.feed(&connect_packet);

        // Parse the packet
        let result = parser.next_packet().unwrap();
        assert!(
            result.is_some(),
            "Parser should successfully parse the MQTT v3 CONNECT packet"
        );

        if let Some(MqttPacket::Connect3(connect)) = result {
            // This packet has client_id "tester" based on our test data
            assert_eq!(connect.client_id, "tester");
            assert_eq!(connect.keep_alive, 60);
            assert!(connect.clean_session);
        } else {
            panic!("Expected Connect3 packet, got: {:?}", result);
        }
    }

    #[test]
    fn test_mqtt3_partial_packet_feeding() {
        let mut parser = MqttParser::new(1024, 4);
        let connect_packet = create_mqttv3_connect_packet();

        // Feed only part of the packet
        parser.feed(&connect_packet[..10]);

        // Should return None (incomplete packet)
        let result = parser.next_packet().unwrap();
        assert!(result.is_none());

        // Feed the rest
        parser.feed(&connect_packet[10..]);

        // Now should parse successfully
        let result = parser.next_packet().unwrap();
        assert!(result.is_some());

        if let Some(MqttPacket::Connect3(_)) = result {
            // Success
        } else {
            panic!("Expected Connect3 packet");
        }
    }

    #[test]
    fn test_mqtt3_multiple_packets() {
        let mut parser = MqttParser::new(1024, 4);
        let connect_packet = create_mqttv3_connect_packet();
        let pingreq_packet = create_mqttv3_pingreq_packet();

        // Feed both packets
        parser.feed(&connect_packet);
        parser.feed(&pingreq_packet);

        // Parse first packet (CONNECT)
        let result1 = parser.next_packet().unwrap();
        assert!(matches!(result1, Some(MqttPacket::Connect3(_))));

        // Parse second packet (PINGREQ)
        let result2 = parser.next_packet().unwrap();
        assert!(matches!(result2, Some(MqttPacket::PingReq3(_))));

        // No more packets - handle potential BufferEmpty error
        match parser.next_packet() {
            Ok(None) => {
                // Expected: no more packets
            }
            Ok(Some(_)) => {
                panic!("Expected no more packets");
            }
            Err(_) => {
                // BufferEmpty error is acceptable when no more packets
            }
        }
    }

    #[test]
    fn test_mqtt3_pingreq_parsing() {
        let mut parser = MqttParser::new(1024, 4);
        let pingreq_packet = create_mqttv3_pingreq_packet();

        parser.feed(&pingreq_packet);
        let result = parser.next_packet().unwrap();

        assert!(matches!(result, Some(MqttPacket::PingReq3(_))));
    }

    #[test]
    fn test_mqtt3_disconnect_parsing() {
        let mut parser = MqttParser::new(1024, 4);
        let disconnect_packet = create_mqttv3_disconnect_packet();

        parser.feed(&disconnect_packet);
        let result = parser.next_packet().unwrap();

        assert!(matches!(result, Some(MqttPacket::Disconnect3(_))));
    }

    #[test]
    fn test_mqtt3_version_detection() {
        let mut parser = MqttParser::new(1024, 0); // Undefined version
        let connect_packet = create_mqttv3_connect_packet();

        // Feed enough data to detect version
        parser.feed(&connect_packet[..15]); // Include version byte

        // Try to set version
        let version = parser.set_mqtt_vsn(0).unwrap(); // Skip fixed header byte
        assert_eq!(version, 4);
        assert_eq!(parser.get_mqtt_vsn(), 4);
    }
}

// Error handling and edge cases
#[test]
fn test_parser_with_malformed_data() {
    let mut parser = MqttParser::new(1024, 5);

    // Feed malformed packet (invalid packet type)
    let malformed_data = vec![0xff, 0x02, 0x00, 0x00];
    parser.feed(&malformed_data);

    // Should return error
    assert!(parser.next_packet().is_err());
}

#[test]
fn test_parser_v5_with_empty_feed() {
    let mut parser = MqttParser::new(1024, 5);

    // Feed empty data
    parser.feed(&[]);

    // Should return error for empty buffer when trying to parse
    match parser.next_packet() {
        Err(ParseError::BufferEmpty) => {
            // Expected behavior
        }
        Ok(None) => {
            // Also acceptable - no packet available
        }
        other => panic!("Unexpected result: {:?}", other),
    }
}

#[test]
fn test_parser_v3_with_empty_feed() {
    let mut parser = MqttParser::new(1024, 3);

    // Feed empty data
    parser.feed(&[]);

    // Should return error for empty buffer when trying to parse
    match parser.next_packet() {
        Err(ParseError::BufferEmpty) => {
            // Expected behavior
        }
        Ok(None) => {
            // Also acceptable - no packet available
        }
        other => panic!("Unexpected result: {:?}", other),
    }
}

#[test]
fn test_version_detection_insufficient_data() {
    let mut parser = MqttParser::new(1024, 0);

    // Feed insufficient data for version detection
    parser.feed(&[0x10, 0x0a]); // Just fixed header

    // Should return more for insufficient data
    if let Err(ParseError::BufferTooShort) = parser.set_mqtt_vsn(0) {
        // Expected error
        // @TODO improve error type to More with hint
    } else {
        panic!("Expected BufferTooShort error due to insufficient data");
    }
}

#[test]
fn test_mixed_protocol_versions() {
    // Test that we can handle different protocol versions correctly
    let parser_v3 = MqttParser::new(1024, 4);
    let parser_v5 = MqttParser::new(1024, 5);

    // Just verify parser creation with different versions
    assert_eq!(parser_v3.get_mqtt_vsn(), 4);
    assert_eq!(parser_v5.get_mqtt_vsn(), 5);
}

#[test]
fn test_stream_buffer_capacity() {
    let buffer_size = 128;
    let mut parser = MqttParser::new(buffer_size, 5);

    // Buffer should have the correct initial capacity
    assert!(parser.buffer_mut().capacity() >= buffer_size);
}

#[test]
fn test_large_packet_handling() {
    let mut parser = MqttParser::new(4096, 5);

    // Create a larger packet (CONNECT with long client ID)
    let long_client_id = "a".repeat(1000);
    let mut large_packet = vec![
        0x10, 0x00, // Will update length
        0x00, 0x04, 0x4d, 0x51, 0x54, 0x54, // Protocol name "MQTT"
        0x05, // Protocol version 5
        0x02, // Connect flags
        0x00, 0x3c, // Keep alive
        0x00, // Properties length 0
    ];

    // Add length-prefixed client ID
    let client_id_len = long_client_id.len() as u16;
    large_packet.extend(client_id_len.to_be_bytes());
    large_packet.extend(long_client_id.bytes());

    // Update remaining length (total - 2 for fixed header)
    let remaining_len = large_packet.len() - 2;
    if remaining_len <= 127 {
        large_packet[1] = remaining_len as u8;
    } else {
        // Use two-byte encoding for larger lengths
        large_packet[1] = ((remaining_len & 0x7f) | 0x80) as u8;
        large_packet.insert(2, (remaining_len >> 7) as u8);
    }

    parser.feed(&large_packet);
    let result = parser.next_packet().unwrap();

    if let Some(MqttPacket::Connect5(connect)) = result {
        assert_eq!(connect.client_id.len(), 1000);
    } else {
        panic!("Expected Connect5 packet with large client ID");
    }
}

use std::io::Read;
use std::vec;

#[test]
fn test_stream_v5() {
    // read data from file: tests/fixtures/mqttv5_stream.bin
    let mut file = std::fs::File::open("tests/fixtures/mqttv5_stream.bin").unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let mut parser = MqttParser::new(4096, 0);
    println!("Buffer length: {}", buffer.len());
    parser.feed(&buffer);
    assert_eq!(5, parser.set_mqtt_vsn(0).unwrap()); // Detect version
    let result = parser.next_packet().unwrap();

    // first packet should be Connect5 with client_id "dev_bench_pub_1528648950_1"
    if let Some(MqttPacket::Connect5(connect)) = result {
        assert_eq!(connect.client_id, "dev_bench_pub_1528648950_1");
    } else {
        panic!("Expected Connect5 packet with client ID");
    }

    // next one should be connack
    /* */
    let result = parser.next_packet().unwrap();
    if let Some(MqttPacket::ConnAck5(connack)) = result {
        assert!(!connack.session_present);
        assert_eq!(connack.reason_code, 0);
    } else {
        panic!("Expected ConnAck5 packet, got {:?}", result);
    }

    // following 8 packets should be publish5
    for _ in 0..8 {
        let result = parser.next_packet().unwrap();
        if let Some(MqttPacket::Publish5(publish)) = result {
            assert_eq!(publish.topic_name, "bbv5");
            assert_eq!(publish.payload, vec![97u8; 256]);
        } else {
            panic!("Expected Publish5 packet");
        }
    }

    // next is None
    if let Ok(None) = parser.next_packet() {
        // Expected
    } else {
        panic!("Expected Ok(None) (Buffer empty)");
    }
}

#[test]
fn test_stream_v3() {
    // read data from file: tests/fixtures/mqttv3_stream.bin
    let mut file = std::fs::File::open("tests/fixtures/mqttv3_stream.bin").unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let mut parser = MqttParser::new(4096, 0);
    println!("Buffer length: {}", buffer.len());
    parser.feed(&buffer);
    assert_eq!(3, parser.set_mqtt_vsn(0).unwrap()); // Detect version
    let result = parser.next_packet().unwrap();

    // first packet should be Connect5 with client_id "dev_bench_pub_1528648950_1"
    if let Some(MqttPacket::Connect3(connect)) = result {
        assert_eq!(connect.client_id, "dev_bench_pub_2458008952_88");
    } else {
        panic!("Expected Connect3 packet with client ID");
    }

    // next one should be connack
    /* */
    let result = parser.next_packet().unwrap();
    if let Some(MqttPacket::ConnAck3(connack)) = result {
        assert!(!connack.session_present);
        assert_eq!(connack.return_code, 0);
    } else {
        panic!("Expected ConnAck3 packet, got {:?}", result);
    }
    // following 9 packets should be publish3
    for n in 0..9 {
        let result = parser.next_packet().unwrap();
        if let Some(MqttPacket::Publish3(publish)) = result {
            assert_eq!(publish.topic_name, "aa");
            assert_eq!(publish.payload, vec![97u8; 256]);
        } else {
            panic!("Expected Publish3 packet {}", n);
        }
    }

    // next is None
    if let Ok(None) = parser.next_packet() {
        // Expected Ok(None)
    } else {
        panic!("Expected Ok(None) (Buffer empty)");
    }
}
