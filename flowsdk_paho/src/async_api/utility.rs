// SPDX-License-Identifier: MPL-2.0
//! `MQTTAsync_waitForCompletion`, `MQTTAsync_getPendingTokens`,
//! `MQTTAsync_free`, `MQTTAsync_freeMessage`, `MQTTAsync_strerror`

use libc::{c_char, c_int, c_void};
use std::time::Duration;

use crate::common::async_structs::{MQTTAsync, MQTTAsync_message, MQTTAsync_token};
use crate::common::return_codes::{self, *};
use crate::common::structs;
use crate::inner::client_state::PahoClientInner;

/// Block until the operation identified by `token` completes, or `timeout` ms elapse.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_waitForCompletion(MQTTAsync handle, MQTTAsync_token token,
///                                 unsigned long timeout);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_waitForCompletion(
    handle: MQTTAsync,
    token: MQTTAsync_token,
    timeout: libc::c_ulong,
) -> c_int {
    if handle.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    let inner = &*(handle as *mut PahoClientInner);
    let duration = Duration::from_millis(timeout as u64);
    match inner.shared.token_tracker.wait_for_completion(token, duration) {
        Some(result) => result.rc,
        None => MQTTASYNC_FAILURE,
    }
}

/// Retrieve the list of tokens for operations not yet acknowledged.
///
/// # Paho C signature
/// ```c
/// int MQTTAsync_getPendingTokens(MQTTAsync handle, MQTTAsync_token** tokens);
/// ```
///
/// The returned array is `malloc`-allocated and terminated with `-1`; free it
/// with `MQTTAsync_free`.
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_getPendingTokens(
    handle: MQTTAsync,
    tokens: *mut *mut MQTTAsync_token,
) -> c_int {
    if handle.is_null() || tokens.is_null() {
        return MQTTASYNC_NULL_PARAMETER;
    }
    let inner = &*(handle as *mut PahoClientInner);
    let pending = inner.shared.token_tracker.get_pending_tokens();

    if pending.is_empty() {
        *tokens = std::ptr::null_mut();
        return MQTTASYNC_SUCCESS;
    }

    let array_size = (pending.len() + 1) * std::mem::size_of::<c_int>();
    let array = libc::malloc(array_size) as *mut c_int;
    if array.is_null() {
        return MQTTASYNC_FAILURE;
    }
    for (i, t) in pending.iter().enumerate() {
        *array.add(i) = *t;
    }
    *array.add(pending.len()) = -1;
    *tokens = array;
    MQTTASYNC_SUCCESS
}

/// Free memory allocated by the library (e.g. token arrays).
///
/// # Paho C signature
/// ```c
/// void MQTTAsync_free(void* ptr);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        libc::free(ptr);
    }
}

/// Free a message allocated by the library.
///
/// # Paho C signature
/// ```c
/// void MQTTAsync_freeMessage(MQTTAsync_message** message);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTAsync_freeMessage(message: *mut *mut MQTTAsync_message) {
    structs::free_paho_message(message);
}

/// Return a human-readable description of a return code.
///
/// # Paho C signature
/// ```c
/// const char* MQTTAsync_strerror(int code);
/// ```
#[no_mangle]
pub extern "C" fn MQTTAsync_strerror(code: c_int) -> *const c_char {
    return_codes::strerror(code)
}
