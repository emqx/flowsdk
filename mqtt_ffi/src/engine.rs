// SPDX-License-Identifier: MPL-2.0
#![allow(clippy::missing_safety_doc)]

use flowsdk::mqtt_client::commands::PublishCommand;
use flowsdk::mqtt_client::engine::{MqttEngine, MqttEvent};
use flowsdk::mqtt_client::opts::MqttClientOptions;
use libc::{c_char, size_t};
use std::ffi::{CStr, CString};
use std::time::{Duration, Instant};

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
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.connect();
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_incoming(
    ptr: *mut MqttEngineFFI,
    data: *const u8,
    len: size_t,
) {
    let wrapper = unsafe { &mut *ptr };
    let data = unsafe { std::slice::from_raw_parts(data, len) };
    let events = wrapper.engine.handle_incoming(data);
    wrapper.events.extend(events);
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_handle_tick(ptr: *mut MqttEngineFFI, now_ms: u64) {
    let wrapper = unsafe { &mut *ptr };
    let now = wrapper.start_time + Duration::from_millis(now_ms);
    let events = wrapper.engine.handle_tick(now);
    wrapper.events.extend(events);
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_next_tick_ms(ptr: *mut MqttEngineFFI) -> i64 {
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
    let wrapper = unsafe { &mut *ptr };
    if wrapper.events.is_empty() {
        return CString::new("[]").unwrap().into_raw();
    }
    let json = serde_json::to_string(&wrapper.events).unwrap_or_else(|_| "[]".to_string());
    wrapper.events.clear();
    CString::new(json).unwrap().into_raw()
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
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.disconnect();
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_is_connected(ptr: *mut MqttEngineFFI) -> i32 {
    let wrapper = unsafe { &mut *ptr };
    if wrapper.engine.is_connected() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_get_version(ptr: *mut MqttEngineFFI) -> u8 {
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.mqtt_version()
}

#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_auth(ptr: *mut MqttEngineFFI, reason_code: u8) {
    let wrapper = unsafe { &mut *ptr };
    wrapper.engine.auth(reason_code, Vec::new());
}
