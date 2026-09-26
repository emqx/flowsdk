// SPDX-License-Identifier: MPL-2.0
//! Checked C APIs; JSON commands/events require the `json` feature.
use super::*;

#[cfg(feature = "json")]
mod json;
#[cfg(feature = "json")]
pub use json::*;

/// # Safety
/// engine must be null or a live TCP engine. This reports core output only,
/// not host socket buffers; null returns false.
#[no_mangle]
pub unsafe extern "C" fn mqtt_engine_has_pending_output(engine: *const MqttEngineFFI) -> bool {
    engine
        .as_ref()
        .is_some_and(MqttEngineFFI::has_pending_output)
}
