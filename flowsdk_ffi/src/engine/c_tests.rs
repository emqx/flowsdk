use super::*;
use std::ptr::{null, null_mut};

unsafe fn take_bytes(ptr: *mut u8, len: usize) -> Vec<u8> {
    if ptr.is_null() {
        assert_eq!(len, 0);
        return vec![];
    }
    let bytes = std::slice::from_raw_parts(ptr, len).to_vec();
    mqtt_engine_free_bytes(ptr, len);
    bytes
}

unsafe fn take_string(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null());
    let value = CStr::from_ptr(ptr).to_str().unwrap().to_owned();
    mqtt_engine_free_string(ptr);
    value
}

#[test]
fn c_tcp_client_round_trips_packets_and_owns_returned_buffers() {
    let client_id = CString::new("c-api-test").unwrap();
    let topic = CString::new("c/test").unwrap();
    let username = CString::new("user").unwrap();
    let password = CString::new("password").unwrap();
    for version in [3, 5] {
        // Input strings outlive every call; each returned allocation is freed once.
        unsafe {
            let opts = MqttOptionsC {
                client_id: client_id.as_ptr(),
                mqtt_version: version,
                clean_start: true,
                keep_alive: 30,
                username: username.as_ptr(),
                password: password.as_ptr(),
                reconnect_base_delay_ms: 10,
                reconnect_max_delay_ms: 100,
                max_reconnect_attempts: 2,
            };
            let engine = mqtt_engine_new_with_opts(&opts);
            assert!(!engine.is_null());
            assert_eq!(mqtt_engine_get_version(engine), version);
            assert_eq!(mqtt_engine_is_connected(engine), 0);
            mqtt_engine_connect(engine);
            let mut len = 0;
            let data = mqtt_engine_take_outgoing(engine, &mut len);
            let connect = take_bytes(data, len);
            assert_eq!(connect[0], 0x10);
            assert!(connect.windows(8).any(|v| v == b"password"));
            let connack = if version == 5 {
                vec![0x20, 3, 0, 0, 0]
            } else {
                vec![0x20, 2, 0, 0]
            };
            mqtt_engine_handle_incoming(engine, connack.as_ptr(), connack.len());
            assert_eq!(mqtt_engine_is_connected(engine), 1);
            let events = mqtt_engine_take_events_list(engine);
            assert_eq!(mqtt_event_list_len(events), 1);
            assert_eq!(mqtt_event_list_get_tag(events, 0), 1);
            assert_eq!(mqtt_event_list_get_connected_rc(events, 0), 0);
            mqtt_event_list_free(events);
            for (id, header, ack_header) in [
                (
                    mqtt_engine_publish(engine, topic.as_ptr(), b"payload".as_ptr(), 7, 1),
                    0x32,
                    0x40,
                ),
                (mqtt_engine_subscribe(engine, topic.as_ptr(), 1), 0x82, 0x90),
                (mqtt_engine_unsubscribe(engine, topic.as_ptr()), 0xa2, 0xb0),
            ] {
                assert!(id > 0);
                // All three requests are queued above; packet IDs still identify their ACKs.
                let mut ack = vec![ack_header, 2, (id >> 8) as u8, id as u8];
                if ack_header == 0x90 {
                    if version == 5 {
                        ack.push(0);
                    }
                    ack.push(1);
                } else if ack_header == 0xb0 && version == 5 {
                    ack.extend([0, 0]);
                }
                ack[1] = (ack.len() - 2) as u8;
                mqtt_engine_handle_incoming(engine, ack.as_ptr(), ack.len());
                let events = mqtt_engine_take_events_list(engine);
                let expected = match header {
                    0x32 => 4,
                    0x82 => 5,
                    _ => 6,
                };
                assert_eq!(mqtt_event_list_get_tag(events, 0), expected);
                if expected == 4 {
                    assert_eq!(mqtt_event_list_get_published_pid(events, 0), id);
                }
                if expected == 5 {
                    assert_eq!(mqtt_event_list_get_subscribed_pid(events, 0), id);
                }
                mqtt_event_list_free(events);
            }
            let data = mqtt_engine_take_outgoing(engine, &mut len);
            assert!(!take_bytes(data, len).is_empty());
            mqtt_engine_handle_tick(engine, 1);
            assert!(mqtt_engine_next_tick_ms(engine) >= 0);
            mqtt_engine_auth(engine, 0xff);
            #[cfg(feature = "json")]
            assert!(take_string(mqtt_engine_take_events(engine)).contains("Error"));
            mqtt_engine_disconnect(engine);
            let data = mqtt_engine_take_outgoing(engine, &mut len);
            assert_eq!(take_bytes(data, len)[0], 0xe0);
            assert_eq!(mqtt_engine_is_connected(engine), 0);
            mqtt_engine_handle_connection_lost(engine);
            mqtt_engine_free(engine);
        }
    }
}

#[test]
fn c_null_arguments_and_invalid_options_return_sentinels() {
    unsafe {
        assert!(mqtt_engine_new(null(), 0).is_null());
        assert!(mqtt_engine_new_with_opts(null()).is_null());
        let engine = mqtt_engine_new(null(), 5);
        assert!(!engine.is_null());
        assert_eq!(mqtt_engine_publish(engine, null(), null(), 0, 0), -1);
        assert_eq!(mqtt_engine_subscribe(engine, null(), 0), -1);
        assert_eq!(mqtt_engine_unsubscribe(engine, null()), -1);
        let mut len = 99;
        assert!(mqtt_engine_take_outgoing(engine, &mut len).is_null());
        assert_eq!(len, 0);
        mqtt_engine_free(engine);
        mqtt_engine_connect(null_mut());
        mqtt_engine_handle_incoming(null_mut(), null(), 0);
        mqtt_engine_handle_tick(null_mut(), 0);
        mqtt_engine_disconnect(null_mut());
        mqtt_engine_auth(null_mut(), 0);
        mqtt_engine_handle_connection_lost(null_mut());
        assert_eq!(mqtt_engine_is_connected(null_mut()), 0);
        assert_eq!(mqtt_engine_get_version(null_mut()), 0);
        assert_eq!(mqtt_engine_next_tick_ms(null_mut()), -1);
        assert!(mqtt_engine_take_outgoing(null_mut(), null_mut()).is_null());
        assert!(mqtt_engine_take_events_list(null_mut()).is_null());
        mqtt_engine_free(null_mut());
        mqtt_engine_free_bytes(null_mut(), 0);
        mqtt_engine_free_string(null_mut());
        mqtt_event_list_free(null_mut());
    }
}

#[test]
fn c_event_inspection_preserves_metadata_and_returns_owned_copies() {
    let list = Box::into_raw(Box::new(MqttEventListFFI {
        events: vec![
            MqttEventFFI::MessageReceived(MqttMessageFFI {
                stream_id: Some(4),
                topic: "topic".into(),
                payload: vec![0, 255],
                qos: 1,
                retain: true,
                dup: true,
                packet_id: Some(42),
                properties: vec![],
            }),
            MqttEventFFI::Error {
                message: "failure".into(),
            },
            MqttEventFFI::StreamClosed {
                stream_id: 4,
                reason: "finished".into(),
                by_peer: true,
            },
            MqttEventFFI::StreamReset {
                stream_id: 8,
                error_code: 42,
            },
            MqttEventFFI::StreamStopped {
                stream_id: 12,
                error_code: 43,
            },
            MqttEventFFI::Disconnected {
                reason_code: Some(0x87),
                properties: vec![],
            },
            MqttEventFFI::PingResponse { success: true },
            MqttEventFFI::ReconnectNeeded,
            MqttEventFFI::ReconnectScheduled {
                attempt: 2,
                delay_ms: 100,
            },
            MqttEventFFI::AuthReceived(AuthResultFFI {
                reason_code: 0x18,
                properties: vec![],
            }),
            MqttEventFFI::PublishReceived {
                packet_id: Some(42),
                stream_id: Some(4),
            },
            MqttEventFFI::PubRelReceived {
                packet_id: 42,
                stream_id: Some(4),
            },
            MqttEventFFI::TransportClosed {
                reason: "closed".into(),
                by_peer: false,
                error_code: Some(0),
            },
            MqttEventFFI::ZeroRttStatusChanged {
                status: QuicZeroRttStatusFFI::Rejected,
            },
        ],
    }));
    unsafe {
        assert_eq!((*list).len(), 14);
        assert!(!(*list).is_empty());
        assert!((*list).get(0).is_some());
        assert!((*list).get(100).is_none());
        for (index, tag) in [3, 8, 11, 12, 13, 2, 7, 9, 10, 14, 15, 16, 17, 18]
            .into_iter()
            .enumerate()
        {
            assert_eq!(mqtt_event_list_get_tag(list, index), tag);
        }
        assert_eq!(
            take_string(mqtt_event_list_get_message_topic(list, 0)),
            "topic"
        );
        let mut len = 0;
        let data = mqtt_event_list_get_message_payload(list, 0, &mut len);
        assert_eq!(take_bytes(data, len), [0, 255]);
        assert_eq!(
            take_string(mqtt_event_list_get_error_message(list, 1)),
            "failure"
        );
        assert_eq!(
            take_string(mqtt_event_list_get_stream_close_reason(list, 2)),
            "finished"
        );
        assert_eq!(mqtt_event_list_get_stream_closed_by_peer(list, 2), 1);
        for (index, stream, code) in [(2, 4, 0), (3, 8, 42), (4, 12, 43)] {
            assert_eq!(mqtt_event_list_get_stream_id(list, index), stream);
            assert_eq!(mqtt_event_list_get_stream_error_code(list, index), code);
        }
        for (ptr, index) in [(list as *const _, 100), (null(), 0)] {
            assert_eq!(mqtt_event_list_get_tag(ptr, index), 0);
            assert_eq!(mqtt_event_list_get_connected_rc(ptr, index), 0);
            assert_eq!(mqtt_event_list_get_published_pid(ptr, index), -1);
            assert_eq!(mqtt_event_list_get_subscribed_pid(ptr, index), -1);
            assert!(mqtt_event_list_get_message_topic(ptr, index).is_null());
            assert!(mqtt_event_list_get_message_payload(ptr, index, null_mut()).is_null());
            assert!(mqtt_event_list_get_error_message(ptr, index).is_null());
            assert!(mqtt_event_list_get_stream_close_reason(ptr, index).is_null());
            assert_eq!(mqtt_event_list_get_stream_closed_by_peer(ptr, index), -1);
            assert_eq!(mqtt_event_list_get_stream_id(ptr, index), 0);
            assert_eq!(mqtt_event_list_get_stream_error_code(ptr, index), 0);
        }
        assert_eq!(mqtt_event_list_len(null()), 0);
        mqtt_event_list_free(list);
    }
}

#[cfg(feature = "tls")]
#[test]
fn c_tls_handshake_output_and_configuration_errors_are_visible() {
    unsafe {
        // Use a repository fixture rather than depending on the host keychain.
        let ca = CString::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../ca.pem")).unwrap();
        let tls = MqttTlsOptionsC {
            ca_cert_file: ca.as_ptr(),
            client_cert_file: null(),
            client_key_file: null(),
            alpn: null(),
            insecure_skip_verify: 0,
            enable_key_log: 0,
        };
        let engine = mqtt_tls_engine_new(null(), 5, null(), &tls);
        assert!(!engine.is_null());
        assert_eq!(mqtt_tls_engine_is_connected(engine), 0);
        mqtt_tls_engine_connect(engine);
        mqtt_tls_engine_handle_tick(engine, 0);
        let mut len = 0;
        let data = mqtt_tls_engine_take_socket_data(engine, &mut len);
        let hello = take_bytes(data, len);
        assert!(!hello.is_empty());
        assert_eq!(hello[0], 0x16);
        assert!(mqtt_tls_engine_take_socket_data(engine, &mut len).is_null());
        assert_eq!(len, 0);
        let topic = CString::new("topic").unwrap();
        assert!(mqtt_tls_engine_publish(engine, topic.as_ptr(), b"a".as_ptr(), 1, 1) > 0);
        assert!(mqtt_tls_engine_subscribe(engine, topic.as_ptr(), 1) > 0);
        assert!(mqtt_tls_engine_unsubscribe(engine, topic.as_ptr()) > 0);
        mqtt_tls_engine_handle_socket_data(engine, b"invalid tls data".as_ptr(), 16);
        let events = mqtt_tls_engine_take_events_list(engine);
        assert!((0..mqtt_event_list_len(events)).any(|i| mqtt_event_list_get_tag(events, i) == 8));
        mqtt_event_list_free(events);
        mqtt_tls_engine_disconnect(engine);
        #[cfg(feature = "json")]
        take_string(mqtt_tls_engine_take_events(engine));
        mqtt_tls_engine_free(engine);
        assert!(mqtt_tls_engine_new(null(), 0, null(), null()).is_null());
        mqtt_tls_engine_free(null_mut());
        mqtt_tls_engine_connect(null_mut());
        mqtt_tls_engine_handle_tick(null_mut(), 0);
        mqtt_tls_engine_handle_socket_data(null_mut(), null(), 0);
        mqtt_tls_engine_disconnect(null_mut());
        assert_eq!(mqtt_tls_engine_is_connected(null_mut()), 0);
        assert!(mqtt_tls_engine_take_socket_data(null_mut(), null_mut()).is_null());
        assert!(mqtt_tls_engine_take_events_list(null_mut()).is_null());
    }
}

#[cfg(feature = "quic")]
#[test]
fn c_quic_datagram_ownership_addresses_and_errors_are_preserved() {
    let address = CString::new("127.0.0.1:14567").unwrap();
    let name = CString::new("localhost").unwrap();
    let invalid = CString::new("invalid address").unwrap();
    let topic = CString::new("topic").unwrap();
    unsafe {
        let engine = mqtt_quic_engine_new(name.as_ptr(), 5);
        assert!(!engine.is_null());
        assert_eq!(mqtt_quic_engine_is_connected(engine), 0);
        assert_eq!(
            mqtt_quic_engine_connect(engine, invalid.as_ptr(), name.as_ptr(), null()),
            -1
        );
        let events = mqtt_quic_engine_take_events_list(engine);
        assert_eq!(mqtt_event_list_get_tag(events, 0), 8);
        mqtt_event_list_free(events);
        let opts = MqttTlsOptionsC {
            ca_cert_file: null(),
            client_cert_file: null(),
            client_key_file: null(),
            alpn: null(),
            insecure_skip_verify: 1,
            enable_key_log: 0,
        };
        assert_eq!(
            mqtt_quic_engine_connect(engine, address.as_ptr(), name.as_ptr(), &opts),
            0
        );
        mqtt_quic_engine_handle_tick(engine, 0);
        let mut count = 0;
        let datagrams = mqtt_quic_engine_take_outgoing_datagrams(engine, &mut count);
        assert!(count > 0);
        assert!(!datagrams.is_null());
        for datagram in std::slice::from_raw_parts(datagrams, count) {
            assert_eq!(CStr::from_ptr(datagram.addr), address.as_c_str());
            assert!(datagram.data_len > 0);
            assert!(!datagram.data.is_null());
        }
        mqtt_quic_engine_free_datagrams(datagrams, count);
        assert!(mqtt_quic_engine_take_outgoing_datagrams(engine, &mut count).is_null());
        assert_eq!(count, 0);
        assert_eq!(
            mqtt_quic_engine_publish(engine, topic.as_ptr(), b"a".as_ptr(), 1, 1),
            -1
        );
        assert_eq!(mqtt_quic_engine_subscribe(engine, topic.as_ptr(), 1), -1);
        assert_eq!(mqtt_quic_engine_unsubscribe(engine, topic.as_ptr()), -1);
        mqtt_quic_engine_handle_datagram(engine, b"a".as_ptr(), 1, invalid.as_ptr());
        #[cfg(feature = "json")]
        assert!(take_string(mqtt_quic_engine_take_events(engine)).contains("Error"));
        mqtt_quic_engine_disconnect(engine);
        mqtt_quic_engine_free(engine);
        assert!(mqtt_quic_engine_new(null(), 0).is_null());
        assert_eq!(
            mqtt_quic_engine_connect(null_mut(), null(), null(), null()),
            -1
        );
        mqtt_quic_engine_free(null_mut());
        mqtt_quic_engine_free_datagrams(null_mut(), 0);
        mqtt_quic_engine_handle_datagram(null_mut(), null(), 0, null());
        mqtt_quic_engine_handle_tick(null_mut(), 0);
        mqtt_quic_engine_disconnect(null_mut());
        assert_eq!(mqtt_quic_engine_is_connected(null_mut()), 0);
        assert!(mqtt_quic_engine_take_events_list(null_mut()).is_null());
        assert!(mqtt_quic_engine_take_outgoing_datagrams(null_mut(), null_mut()).is_null());
    }
}

#[cfg(feature = "json")]
#[test]
fn checked_c_api_errors_ownership_and_operation_metadata() {
    use super::c_api::*;
    unsafe {
        let mut engine = null_mut();
        let mut error = null_mut();
        let config = br#"{"version":1,"connect":{"options":{"client_id":"c-checked","clean_start":false}},"runtime":{"peer":"tcp://broker:1883","operation_timeouts":{"connect_ms":0}}}"#;
        assert_eq!(
            mqtt_engine_new_v1(config.as_ptr(), config.len(), &mut engine, &mut error),
            0
        );
        assert!(error.is_null());
        let command = br#"{"command":"connect"}"#;
        assert_eq!(
            mqtt_engine_command_v1(
                engine,
                command.as_ptr(),
                command.len(),
                null_mut(),
                &mut error
            ),
            0
        );
        assert_ne!(
            mqtt_engine_command_v1(
                engine,
                command.as_ptr(),
                command.len(),
                null_mut(),
                &mut error
            ),
            0
        );
        assert!(!take_string(error).is_empty());
        mqtt_engine_handle_tick(engine, 10);
        let list = mqtt_engine_take_events_list(engine);
        assert_eq!(mqtt_event_list_get_tag(list, 0), 19);
        let mut json = null_mut();
        assert_eq!(mqtt_event_list_get_json(list, 0, &mut json, &mut error), 0);
        let event = take_string(json);
        assert!(
            event.contains("OperationFailed")
                && event.contains("Connect")
                && event.contains("Timeout")
        );
        assert!(event.contains("timeout_ms"));
        assert_eq!(
            mqtt_event_list_get_json(list, 999, &mut json, &mut error),
            1
        );
        assert!(json.is_null());
        take_string(error);
        mqtt_event_list_free(list);
        for (ptr, len) in [(null(), 10), (null(), 0), (config.as_ptr(), usize::MAX)] {
            let mut output = std::ptr::dangling_mut();
            assert_eq!(mqtt_engine_new_v1(ptr, len, &mut output, &mut error), 1);
            assert!(output.is_null());
            take_string(error);
        }
        #[cfg(feature = "durable-session")]
        {
            let mut out = std::ptr::dangling_mut();
            let mut len = 999;
            assert_eq!(
                mqtt_engine_snapshot_session(null(), &mut out, &mut len, &mut error),
                1
            );
            assert!(out.is_null());
            assert_eq!(len, 0);
            take_string(error);
            assert_eq!(
                mqtt_engine_restore_session_state(engine, b"bad".as_ptr(), 3, &mut error),
                1
            );
            assert!(!take_string(error).contains("bad"));
        }
        mqtt_engine_free(engine);
    }
}
