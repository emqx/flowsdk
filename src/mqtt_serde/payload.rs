// SPDX-License-Identifier: Apache-2.0

//! Keep the Vec-era sequence representation for commands, packets and checkpoints.
//! Bytes' default serde implementation uses serialize_bytes instead of a sequence.

use alloc::vec::Vec;
use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) fn serialize<S: Serializer>(payload: &Bytes, serializer: S) -> Result<S::Ok, S::Error> {
    payload.as_ref().serialize(serializer)
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Bytes, D::Error> {
    Vec::<u8>::deserialize(deserializer).map(Bytes::from)
}

#[cfg(feature = "arbitrary")]
pub(crate) fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Bytes> {
    u.arbitrary::<Vec<u8>>().map(Bytes::from)
}
