// SPDX-License-Identifier: Apache-2.0

//! Publish and parser storage, selected by the `shared-payloads` feature.

use alloc::vec::Vec;
use core::ops::Range;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Shared immutable storage when `shared-payloads` is enabled.
#[cfg(feature = "shared-payloads")]
pub type Payload = bytes::Bytes;

/// Owned storage with no atomics requirement (the default).
#[cfg(not(feature = "shared-payloads"))]
pub type Payload = Vec<u8>;

// Preserve the Vec-era sequence representation for commands, packets and
// checkpoints. Bytes' own serde implementation uses serialize_bytes instead.
pub(crate) fn serialize<S: Serializer>(
    payload: &Payload,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    payload[..].serialize(serializer)
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Payload, D::Error> {
    Vec::<u8>::deserialize(deserializer).map(Payload::from)
}

#[cfg(feature = "arbitrary")]
pub(crate) fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Payload> {
    u.arbitrary::<Vec<u8>>().map(Payload::from)
}

/// Retain a range of a consumed frame. Shared storage keeps a view; owned
/// storage compacts in place without allocating a replacement buffer.
pub(crate) fn into_range(mut payload: Payload, range: Range<usize>) -> Payload {
    #[cfg(feature = "shared-payloads")]
    {
        payload = payload.slice(range);
    }
    #[cfg(not(feature = "shared-payloads"))]
    {
        payload.truncate(range.end);
        payload.drain(..range.start);
    }
    payload
}
