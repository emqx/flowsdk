// SPDX-License-Identifier: MPL-2.0
//! `MQTTClient_strerror`, `MQTTClient_free`, `MQTTClient_freeMessage`

use libc::{c_char, c_int, c_void};

use crate::common::return_codes;
use crate::common::structs;

/// Get a human-readable error message for a return code.
///
/// # Paho C signature
/// ```c
/// const char* MQTTClient_strerror(int code);
/// ```
///
/// Returns a pointer to a static string (do not free).
#[no_mangle]
pub extern "C" fn MQTTClient_strerror(code: c_int) -> *const c_char {
    return_codes::strerror(code)
}

/// Free memory allocated by the library.
///
/// # Paho C signature
/// ```c
/// void MQTTClient_free(void* ptr);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        libc::free(ptr);
    }
}

/// Free a message allocated by the library (e.g., from `MQTTClient_receive`).
///
/// # Paho C signature
/// ```c
/// void MQTTClient_freeMessage(MQTTClient_message** msg);
/// ```
#[no_mangle]
pub unsafe extern "C" fn MQTTClient_freeMessage(msg: *mut *mut structs::MQTTClient_message) {
    structs::free_paho_message(msg);
}
