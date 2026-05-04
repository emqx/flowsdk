// SPDX-License-Identifier: MPL-2.0

pub mod engine;

#[cfg(feature = "uniffi-bindings")]
uniffi::setup_scaffolding!("flowsdk_ffi");
