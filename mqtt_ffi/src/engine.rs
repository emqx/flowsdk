// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::missing_safety_doc)]

use flowsdk::mqtt_client::commands::PublishCommand;
use flowsdk::mqtt_client::engine::{MqttEngine, MqttEvent};
use flowsdk::mqtt_client::opts::MqttClientOptions;
use libc::{c_char, size_t};
use std::ffi::{CStr, CString};
use std::time::{Duration, Instant};

#[cfg(feature = "quic")]
use flowsdk::mqtt_client::engine::QuicMqttEngine;
#[cfg(feature = "tls")]
use flowsdk::mqtt_client::tls_engine::TlsMqttEngine;
#[cfg(feature = "quic")]
use std::net::SocketAddr;

pub struct MqttEngineFFI {
    engine: MqttEngine,
    start_time: Instant,
    events: Vec<MqttEvent>,
}

#[repr(C)]
pub struct MqttOptionsFFI {
    pub client_id: *const c_char,
    pub mqtt_version: u8,
    pub clean_start: u8,
    pub keep_alive: u16,
    pub username: *const c_char,
    pub password: *const u8,
    pub password_len: size_t,
    pub reconnect_base_delay_ms: u64,
    pub reconnect_max_delay_ms: u64,
    pub max_reconnect_attempts: u32,
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_new(
    client_id: *const c_char,
    mqtt_version: u8,
) -> *mut MqttEngineFFI {
    let client_id = if client_id.is_null() {
        "mqtt_client".to_string()
    } else {
        unsafe {
            CStr::from_ptr(client_id)
                .to_str()
                .unwrap_or("mqtt_client")
                .to_string()
        }
    };

    let options = MqttClientOptions::builder()
        .client_id(client_id)
        .mqtt_version(mqtt_version)
        .build();

    let engine = MqttEngine::new(options);
    let wrapper = Box::new(MqttEngineFFI {
        engine,
        start_time: Instant::now(),
        events: Vec::new(),
    });

    Box::into_raw(wrapper)
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_new_with_opts(
    opts_ptr: *const MqttOptionsFFI,
) -> *mut MqttEngineFFI {
    if opts_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let opts = unsafe { &*opts_ptr };

    let client_id = if opts.client_id.is_null() {
        "mqtt_client".to_string()
    } else {
        unsafe {
            CStr::from_ptr(opts.client_id)
                .to_str()
                .unwrap_or("mqtt_client")
                .to_string()
        }
    };

    let mut builder = MqttClientOptions::builder()
        .client_id(client_id)
        .mqtt_version(opts.mqtt_version)
        .clean_start(opts.clean_start != 0)
        .keep_alive(opts.keep_alive)
        .reconnect_base_delay_ms(opts.reconnect_base_delay_ms)
        .reconnect_max_delay_ms(opts.reconnect_max_delay_ms)
        .max_reconnect_attempts(opts.max_reconnect_attempts);

    if !opts.username.is_null() {
        if let Ok(username) = unsafe { CStr::from_ptr(opts.username).to_str() } {
            builder = builder.username(username);
        }
    }

    if !opts.password.is_null() && opts.password_len > 0 {
        let password = unsafe { std::slice::from_raw_parts(opts.password, opts.password_len) };
        builder = builder.password(password.to_vec());
    }

    let engine = MqttEngine::new(builder.build());
    let wrapper = Box::new(MqttEngineFFI {
        engine,
        start_time: Instant::now(),
        events: Vec::new(),
    });

    Box::into_raw(wrapper)
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_connection_lost(ptr: *mut MqttEngineFFI) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.handle_connection_lost();
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_free(ptr: *mut MqttEngineFFI) {
    if !ptr.is_null() {
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_connect(ptr: *mut MqttEngineFFI) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.connect();
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_incoming(
    ptr: *mut MqttEngineFFI,
    data: *const u8,
    len: size_t,
) {
    if ptr.is_null() || data.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    let data = unsafe { std::slice::from_raw_parts(data, len) };
    let events = wrapper.engine.handle_incoming(data);
    wrapper.events.extend(events);
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_tick(ptr: *mut MqttEngineFFI, now_ms: u64) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    let now = wrapper.start_time + Duration::from_millis(now_ms);
    let events = wrapper.engine.handle_tick(now);
    wrapper.events.extend(events);
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_next_tick_ms(ptr: *mut MqttEngineFFI) -> i64 {
    if ptr.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    match wrapper.engine.next_tick_at() {
        Some(tick) => {
            let duration = tick.duration_since(wrapper.start_time);
            duration.as_millis() as i64
        }
        None => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_take_outgoing(
    ptr: *mut MqttEngineFFI,
    out_len: *mut size_t,
) -> *mut u8 {
    if ptr.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }
    let wrapper = unsafe { &mut *ptr };
    let bytes = wrapper.engine.take_outgoing();
    if bytes.is_empty() {
        unsafe { *out_len = 0 };
        return std::ptr::null_mut();
    }

    unsafe { *out_len = bytes.len() };
    let mut boxed_slice = bytes.into_boxed_slice();
    let res = boxed_slice.as_mut_ptr();
    std::mem::forget(boxed_slice);
    res
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_free_bytes(ptr: *mut u8, len: size_t) {
    if !ptr.is_null() {
        unsafe {
            let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len));
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_take_events(ptr: *mut MqttEngineFFI) -> *mut c_char {
    if ptr.is_null() {
        return CString::new("[]").unwrap().into_raw();
    }
    let wrapper = unsafe { &mut *ptr };
    if wrapper.events.is_empty() {
        return CString::new("[]").unwrap().into_raw();
    }
    let json = serde_json::to_string(&wrapper.events).unwrap_or_else(|_| "[]".to_string());
    wrapper.events.clear();
    let c_string = CString::new(json).unwrap_or_else(|err| {
        eprintln!(
            "mqtt_engine_take_events: failed to create CString from JSON events: {}",
            err
        );
        // Fallback to an empty JSON array, which is known to be free of interior NUL bytes.
        CString::new("[]").expect("CString::new on a literal without NUL bytes should not fail")
    });
    c_string.into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_publish(
    ptr: *mut MqttEngineFFI,
    topic: *const c_char,
    payload: *const u8,
    payload_len: size_t,
    qos: u8,
) -> i32 {
    if ptr.is_null() || topic.is_null() || payload.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic = unsafe { CStr::from_ptr(topic).to_str().unwrap_or("").to_string() };
    let payload = unsafe { std::slice::from_raw_parts(payload, payload_len) }.to_vec();

    let command_res = PublishCommand::builder()
        .topic(topic)
        .payload(payload)
        .qos(qos)
        .build();

    let command = match command_res {
        Ok(c) => c,
        Err(_) => return -1,
    };

    match wrapper.engine.publish(command) {
        Ok(Some(pid)) => pid as i32,
        Ok(None) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_subscribe(
    ptr: *mut MqttEngineFFI,
    topic_filter: *const c_char,
    qos: u8,
) -> i32 {
    if ptr.is_null() || topic_filter.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic_filter = unsafe {
        CStr::from_ptr(topic_filter)
            .to_str()
            .unwrap_or("")
            .to_string()
    };

    let command = flowsdk::mqtt_client::commands::SubscribeCommand::single(topic_filter, qos);

    match wrapper.engine.subscribe(command) {
        Ok(pid) => pid as i32,
        Err(_) => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_unsubscribe(
    ptr: *mut MqttEngineFFI,
    topic_filter: *const c_char,
) -> i32 {
    if ptr.is_null() || topic_filter.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic_filter = unsafe {
        CStr::from_ptr(topic_filter)
            .to_str()
            .unwrap_or("")
            .to_string()
    };

    let command =
        flowsdk::mqtt_client::commands::UnsubscribeCommand::from_topics(vec![topic_filter]);

    match wrapper.engine.unsubscribe(command) {
        Ok(pid) => pid as i32,
        Err(_) => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_disconnect(ptr: *mut MqttEngineFFI) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.disconnect();
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_is_connected(ptr: *mut MqttEngineFFI) -> i32 {
    if ptr.is_null() {
        return 0;
    }
    let wrapper = unsafe { &mut *ptr };
    if wrapper.engine.is_connected() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_get_version(ptr: *mut MqttEngineFFI) -> u8 {
    if ptr.is_null() {
        return 0;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.mqtt_version()
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_auth(ptr: *mut MqttEngineFFI, reason_code: u8) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.auth(reason_code, Vec::new());
}

#[cfg(any(feature = "quic", feature = "tls"))]
use rustls::client::danger::{ServerCertVerified, ServerCertVerifier};
#[cfg(any(feature = "quic", feature = "tls"))]
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
#[cfg(any(feature = "quic", feature = "tls"))]
use std::sync::Arc;

#[cfg(feature = "quic")]
pub struct QuicMqttEngineFFI {
    engine: QuicMqttEngine,
    start_time: Instant,
    events: Vec<MqttEvent>,
}

#[cfg(any(feature = "quic", feature = "tls"))]
#[derive(Debug)]
struct InsecureServerCertVerifier;

#[cfg(any(feature = "quic", feature = "tls"))]
impl ServerCertVerifier for InsecureServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

#[cfg(any(feature = "quic", feature = "tls"))]
#[repr(C)]
pub struct MqttTlsOptionsFFI {
    pub insecure_skip_verify: u8,
    pub alpn: *const c_char,
    pub ca_cert_file: *const c_char,
    pub client_cert_file: *const c_char,
    pub client_key_file: *const c_char,
}

#[cfg(feature = "quic")]
#[repr(C)]
pub struct MqttDatagramFFI {
    pub remote_addr: *mut c_char,
    pub data: *mut u8,
    pub len: size_t,
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_new(
    client_id: *const c_char,
    mqtt_version: u8,
) -> *mut QuicMqttEngineFFI {
    let client_id = if client_id.is_null() {
        "mqtt_quic_client".to_string()
    } else {
        unsafe {
            CStr::from_ptr(client_id)
                .to_str()
                .unwrap_or("mqtt_quic_client")
                .to_string()
        }
    };

    let options = MqttClientOptions::builder()
        .client_id(client_id)
        .mqtt_version(mqtt_version)
        .build();

    let _ = rustls::crypto::ring::default_provider().install_default();

    match QuicMqttEngine::new(options) {
        Ok(engine) => {
            let wrapper = Box::new(QuicMqttEngineFFI {
                engine,
                start_time: Instant::now(),
                events: Vec::new(),
            });
            Box::into_raw(wrapper)
        }
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_free(ptr: *mut QuicMqttEngineFFI) {
    if !ptr.is_null() {
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_connect(
    ptr: *mut QuicMqttEngineFFI,
    server_addr: *const c_char,
    server_name: *const c_char,
    opts_ptr: *const MqttTlsOptionsFFI,
) -> i32 {
    if ptr.is_null() || server_addr.is_null() {
        return -1;
    }

    let wrapper = unsafe { &mut *ptr };
    let server_addr_str = unsafe { CStr::from_ptr(server_addr).to_str().unwrap_or("") };
    let server_name_str = if server_name.is_null() {
        // Fallback to address without port if possible
        server_addr_str.split(':').next().unwrap_or(server_addr_str)
    } else {
        unsafe { CStr::from_ptr(server_name).to_str().unwrap_or("") }
    };

    let addr: SocketAddr = match server_addr_str.parse() {
        Ok(a) => a,
        Err(_) => return -1,
    };

    let crypto_builder = rustls::ClientConfig::builder();

    if !opts_ptr.is_null() {
        let opts = unsafe { &*opts_ptr };

        let mut config = if opts.insecure_skip_verify != 0 {
            crypto_builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier))
                .with_no_client_auth()
        } else {
            // Handle CA certs
            let mut root_store = rustls::RootCertStore::empty();
            if !opts.ca_cert_file.is_null() {
                let ca_path = unsafe { CStr::from_ptr(opts.ca_cert_file).to_str().unwrap_or("") };
                if let Ok(file) = std::fs::File::open(ca_path) {
                    let mut reader = std::io::BufReader::new(file);
                    // Also load from file if provided
                    let certs = rustls_pemfile::certs(&mut reader)
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap_or_default();
                    for cert in certs {
                        root_store.add(cert).ok();
                    }
                }
            } else {
                for cert in rustls_native_certs::load_native_certs().unwrap_or_default() {
                    root_store.add(cert).ok();
                }
            }

            // Handle client cert/key
            let mut client_auth = None;
            if !opts.client_cert_file.is_null() && !opts.client_key_file.is_null() {
                let cert_path =
                    unsafe { CStr::from_ptr(opts.client_cert_file).to_str().unwrap_or("") };
                let key_path =
                    unsafe { CStr::from_ptr(opts.client_key_file).to_str().unwrap_or("") };

                if let (Ok(cert_file), Ok(key_file)) = (
                    std::fs::File::open(cert_path),
                    std::fs::File::open(key_path),
                ) {
                    let mut cert_reader = std::io::BufReader::new(cert_file);
                    let mut key_reader = std::io::BufReader::new(key_file);

                    let certs = rustls_pemfile::certs(&mut cert_reader)
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap_or_default();
                    let key = rustls_pemfile::private_key(&mut key_reader).ok().flatten();

                    if !certs.is_empty() && key.is_some() {
                        client_auth = Some((certs, key));
                    }
                }
            }

            let builder = crypto_builder.with_root_certificates(root_store);
            if let Some((certs, key)) = client_auth {
                builder
                    // SAFETY: We verified key is Some at line 579
                    .with_client_auth_cert(certs, key.expect("Missing private key"))
                    .unwrap()
            } else {
                builder.with_no_client_auth()
            }
        };

        if !opts.alpn.is_null() {
            let alpn = unsafe { CStr::from_ptr(opts.alpn).to_str().unwrap_or("mqtt") };
            config.alpn_protocols = vec![alpn.as_bytes().to_vec()];
        } else {
            config.alpn_protocols = vec![b"mqtt".to_vec()];
        }

        if wrapper
            .engine
            .connect(addr, server_name_str, config, Instant::now())
            .is_err()
        {
            return -1;
        }
    } else {
        // Default config
        let mut root_store = rustls::RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().unwrap_or_default() {
            root_store.add(cert).ok();
        }
        let mut config = crypto_builder
            .with_root_certificates(root_store)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"mqtt".to_vec()];

        if wrapper
            .engine
            .connect(addr, server_name_str, config, Instant::now())
            .is_err()
        {
            return -1;
        }
    }

    0
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_handle_datagram(
    ptr: *mut QuicMqttEngineFFI,
    data: *const u8,
    len: size_t,
    remote_addr: *const c_char,
) {
    if ptr.is_null() || data.is_null() || remote_addr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    let data = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    let remote_addr_str = unsafe { CStr::from_ptr(remote_addr).to_str().unwrap_or("") };
    let addr: SocketAddr = match remote_addr_str.parse() {
        Ok(a) => a,
        Err(_) => return,
    };

    wrapper.engine.handle_datagram(data, addr, Instant::now());
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_handle_tick(ptr: *mut QuicMqttEngineFFI, now_ms: u64) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    let now = wrapper.start_time + Duration::from_millis(now_ms);
    let events = wrapper.engine.handle_tick(now);
    wrapper.events.extend(events);
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_take_outgoing_datagrams(
    ptr: *mut QuicMqttEngineFFI,
    out_count: *mut size_t,
) -> *mut MqttDatagramFFI {
    if ptr.is_null() || out_count.is_null() {
        return std::ptr::null_mut();
    }
    let wrapper = unsafe { &mut *ptr };
    let mut datagrams = wrapper.engine.take_outgoing_datagrams();

    if datagrams.is_empty() {
        unsafe { *out_count = 0 };
        return std::ptr::null_mut();
    }

    let count = datagrams.len();
    unsafe { *out_count = count };

    let mut ffi_datagrams = Vec::with_capacity(count);
    for (addr, data) in datagrams.drain(..) {
        let addr_str = CString::new(addr.to_string()).unwrap();
        let mut data_box = data.into_boxed_slice();
        let data_ptr = data_box.as_mut_ptr();
        let data_len = data_box.len();
        std::mem::forget(data_box);

        ffi_datagrams.push(MqttDatagramFFI {
            remote_addr: addr_str.into_raw(),
            data: data_ptr,
            len: data_len,
        });
    }

    let mut boxed_slice = ffi_datagrams.into_boxed_slice();
    let res = boxed_slice.as_mut_ptr();
    std::mem::forget(boxed_slice);
    res
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_free_datagrams(ptr: *mut MqttDatagramFFI, count: size_t) {
    if !ptr.is_null() {
        let datagrams = unsafe { std::slice::from_raw_parts_mut(ptr, count) };
        for dg in datagrams {
            if !dg.remote_addr.is_null() {
                let _ = CString::from_raw(dg.remote_addr);
            }
            if !dg.data.is_null() {
                let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(dg.data, dg.len));
            }
        }
        let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, count));
    }
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_take_events(ptr: *mut QuicMqttEngineFFI) -> *mut c_char {
    if ptr.is_null() {
        return CString::new("[]").unwrap().into_raw();
    }
    let wrapper = unsafe { &mut *ptr };
    if wrapper.events.is_empty() {
        // Still need to call engine.take_events in case it has some
        let events = wrapper.engine.take_events();
        wrapper.events.extend(events);
    }

    if wrapper.events.is_empty() {
        return CString::new("[]").unwrap().into_raw();
    }

    let json = serde_json::to_string(&wrapper.events).unwrap_or_else(|_| "[]".to_string());
    wrapper.events.clear();
    let c_string = CString::new(json).unwrap_or_else(|_| CString::new("[]").unwrap());
    c_string.into_raw()
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_publish(
    ptr: *mut QuicMqttEngineFFI,
    topic: *const c_char,
    payload: *const u8,
    payload_len: size_t,
    qos: u8,
) -> i32 {
    if ptr.is_null() || topic.is_null() || payload.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic = unsafe { CStr::from_ptr(topic).to_str().unwrap_or("").to_string() };
    let payload = unsafe { std::slice::from_raw_parts(payload, payload_len) }.to_vec();

    let command = match PublishCommand::builder()
        .topic(topic)
        .payload(payload)
        .qos(qos)
        .build()
    {
        Ok(c) => c,
        Err(_) => return -1,
    };

    match wrapper.engine.publish(command) {
        Ok(Some(pid)) => pid as i32,
        Ok(None) => 0,
        Err(_) => -1,
    }
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_subscribe(
    ptr: *mut QuicMqttEngineFFI,
    topic_filter: *const c_char,
    qos: u8,
) -> i32 {
    if ptr.is_null() || topic_filter.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic_filter = unsafe {
        CStr::from_ptr(topic_filter)
            .to_str()
            .unwrap_or("")
            .to_string()
    };
    let command = flowsdk::mqtt_client::commands::SubscribeCommand::single(topic_filter, qos);

    match wrapper.engine.subscribe(command) {
        Ok(pid) => pid as i32,
        Err(_) => -1,
    }
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_unsubscribe(
    ptr: *mut QuicMqttEngineFFI,
    topic_filter: *const c_char,
) -> i32 {
    if ptr.is_null() || topic_filter.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic_filter = unsafe {
        CStr::from_ptr(topic_filter)
            .to_str()
            .unwrap_or("")
            .to_string()
    };
    let command =
        flowsdk::mqtt_client::commands::UnsubscribeCommand::from_topics(vec![topic_filter]);

    match wrapper.engine.unsubscribe(command) {
        Ok(pid) => pid as i32,
        Err(_) => -1,
    }
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_disconnect(ptr: *mut QuicMqttEngineFFI) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.disconnect();
}

#[cfg(feature = "quic")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_quic_engine_is_connected(ptr: *mut QuicMqttEngineFFI) -> i32 {
    if ptr.is_null() {
        return 0;
    }
    let wrapper = unsafe { &mut *ptr };
    if wrapper.engine.is_connected() {
        1
    } else {
        0
    }
}

#[cfg(feature = "tls")]
pub struct TlsMqttEngineFFI {
    engine: TlsMqttEngine,
    start_time: Instant,
    events: Vec<MqttEvent>,
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_new(
    client_id: *const c_char,
    mqtt_version: u8,
    server_name: *const c_char,
    opts_ptr: *const MqttTlsOptionsFFI, // Reuse same opts for CA/certs
) -> *mut TlsMqttEngineFFI {
    let client_id = if client_id.is_null() {
        "mqtt_tls_client".to_string()
    } else {
        unsafe {
            CStr::from_ptr(client_id)
                .to_str()
                .unwrap_or("mqtt_tls_client")
                .to_string()
        }
    };

    let server_name_str = if server_name.is_null() {
        return std::ptr::null_mut();
    } else {
        unsafe { CStr::from_ptr(server_name).to_str().unwrap_or("") }
    };

    let options = MqttClientOptions::builder()
        .client_id(client_id)
        .mqtt_version(mqtt_version)
        .build();

    let _ = rustls::crypto::ring::default_provider().install_default();

    let crypto_builder = rustls::ClientConfig::builder();

    let mut config = if !opts_ptr.is_null() {
        let opts = unsafe { &*opts_ptr };
        if opts.insecure_skip_verify != 0 {
            crypto_builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier))
                .with_no_client_auth()
        } else {
            let mut root_store = rustls::RootCertStore::empty();
            if !opts.ca_cert_file.is_null() {
                let ca_path = unsafe { CStr::from_ptr(opts.ca_cert_file).to_str().unwrap_or("") };
                if let Ok(file) = std::fs::File::open(ca_path) {
                    let mut reader = std::io::BufReader::new(file);
                    let certs = rustls_pemfile::certs(&mut reader)
                        .filter_map(|r| r.ok())
                        .collect::<Vec<_>>();
                    for cert in certs {
                        root_store.add(cert).ok();
                    }
                }
            } else {
                for cert in rustls_native_certs::load_native_certs().unwrap_or_default() {
                    root_store.add(cert).ok();
                }
            }

            // Handle client cert/key
            let mut client_auth = None;
            if !opts.client_cert_file.is_null() && !opts.client_key_file.is_null() {
                let cert_path =
                    unsafe { CStr::from_ptr(opts.client_cert_file).to_str().unwrap_or("") };
                let key_path =
                    unsafe { CStr::from_ptr(opts.client_key_file).to_str().unwrap_or("") };

                if let (Ok(cert_file), Ok(key_file)) = (
                    std::fs::File::open(cert_path),
                    std::fs::File::open(key_path),
                ) {
                    let mut cert_reader = std::io::BufReader::new(cert_file);
                    let mut key_reader = std::io::BufReader::new(key_file);

                    let certs = rustls_pemfile::certs(&mut cert_reader)
                        .filter_map(|r| r.ok())
                        .collect::<Vec<_>>();
                    let key = rustls_pemfile::private_key(&mut key_reader).ok().flatten();

                    if !certs.is_empty() && key.is_some() {
                        client_auth = Some((certs, key));
                    }
                }
            }

            let builder = crypto_builder.with_root_certificates(root_store);
            if let Some((certs, key)) = client_auth {
                builder
                    .with_client_auth_cert(certs, key.expect("Missing private key"))
                    .unwrap()
            } else {
                builder.with_no_client_auth()
            }
        }
    } else {
        let mut root_store = rustls::RootCertStore::empty();
        for cert in rustls_native_certs::load_native_certs().unwrap_or_default() {
            root_store.add(cert).ok();
        }
        crypto_builder
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };
    config.alpn_protocols = vec![b"mqtt".to_vec()];

    match TlsMqttEngine::new(options, server_name_str, Arc::new(config)) {
        Ok(engine) => {
            let wrapper = Box::new(TlsMqttEngineFFI {
                engine,
                start_time: Instant::now(),
                events: Vec::new(),
            });
            Box::into_raw(wrapper)
        }
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_free(ptr: *mut TlsMqttEngineFFI) {
    if !ptr.is_null() {
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_connect(ptr: *mut TlsMqttEngineFFI) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.connect();
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_handle_socket_data(
    ptr: *mut TlsMqttEngineFFI,
    data: *const u8,
    len: size_t,
) -> i32 {
    if ptr.is_null() || data.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let data_slice = unsafe { std::slice::from_raw_parts(data, len) };
    if wrapper.engine.handle_socket_data(data_slice).is_err() {
        return -1;
    }
    0
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_take_socket_data(
    ptr: *mut TlsMqttEngineFFI,
    out_len: *mut size_t,
) -> *mut u8 {
    if ptr.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }
    let wrapper = unsafe { &mut *ptr };
    let bytes = wrapper.engine.take_socket_data();
    if bytes.is_empty() {
        unsafe { *out_len = 0 };
        return std::ptr::null_mut();
    }
    unsafe { *out_len = bytes.len() };
    let mut boxed_slice = bytes.into_boxed_slice();
    let res = boxed_slice.as_mut_ptr();
    std::mem::forget(boxed_slice);
    res
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_handle_tick(ptr: *mut TlsMqttEngineFFI, now_ms: u64) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    let now = wrapper.start_time + Duration::from_millis(now_ms);
    let events = wrapper.engine.handle_tick(now);
    wrapper.events.extend(events);
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_take_events(ptr: *mut TlsMqttEngineFFI) -> *mut c_char {
    if ptr.is_null() {
        return CString::new("[]").unwrap().into_raw();
    }
    let wrapper = unsafe { &mut *ptr };
    if wrapper.events.is_empty() {
        let events = wrapper.engine.take_events();
        wrapper.events.extend(events);
    }
    if wrapper.events.is_empty() {
        return CString::new("[]").unwrap().into_raw();
    }
    let json = serde_json::to_string(&wrapper.events).unwrap_or_else(|_| "[]".to_string());
    wrapper.events.clear();
    let c_string = CString::new(json).unwrap_or_else(|_| CString::new("[]").unwrap());
    c_string.into_raw()
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_publish(
    ptr: *mut TlsMqttEngineFFI,
    topic: *const c_char,
    payload: *const u8,
    payload_len: size_t,
    qos: u8,
) -> i32 {
    if ptr.is_null() || topic.is_null() || payload.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic = unsafe { CStr::from_ptr(topic).to_str().unwrap_or("").to_string() };
    let payload = unsafe { std::slice::from_raw_parts(payload, payload_len) }.to_vec();
    let command = match PublishCommand::builder()
        .topic(topic)
        .payload(payload)
        .qos(qos)
        .build()
    {
        Ok(c) => c,
        Err(_) => return -1,
    };
    match wrapper.engine.publish(command) {
        Ok(Some(pid)) => pid as i32,
        Ok(None) => 0,
        Err(_) => -1,
    }
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_subscribe(
    ptr: *mut TlsMqttEngineFFI,
    topic_filter: *const c_char,
    qos: u8,
) -> i32 {
    if ptr.is_null() || topic_filter.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic_filter = unsafe {
        CStr::from_ptr(topic_filter)
            .to_str()
            .unwrap_or("")
            .to_string()
    };
    let command = flowsdk::mqtt_client::commands::SubscribeCommand::single(topic_filter, qos);
    match wrapper.engine.subscribe(command) {
        Ok(pid) => pid as i32,
        Err(_) => -1,
    }
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_unsubscribe(
    ptr: *mut TlsMqttEngineFFI,
    topic_filter: *const c_char,
) -> i32 {
    if ptr.is_null() || topic_filter.is_null() {
        return -1;
    }
    let wrapper = unsafe { &mut *ptr };
    let topic_filter = unsafe {
        CStr::from_ptr(topic_filter)
            .to_str()
            .unwrap_or("")
            .to_string()
    };
    let command =
        flowsdk::mqtt_client::commands::UnsubscribeCommand::from_topics(vec![topic_filter]);
    match wrapper.engine.unsubscribe(command) {
        Ok(pid) => pid as i32,
        Err(_) => -1,
    }
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_disconnect(ptr: *mut TlsMqttEngineFFI) {
    if ptr.is_null() {
        return;
    }
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.disconnect();
}

#[cfg(feature = "tls")]
#[no_mangle]
pub unsafe extern "C" fn mqtt_tls_engine_is_connected(ptr: *mut TlsMqttEngineFFI) -> i32 {
    if ptr.is_null() {
        return 0;
    }
    let wrapper = unsafe { &mut *ptr };
    if wrapper.engine.is_connected() {
        1
    } else {
        0
    }
}
