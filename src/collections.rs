// SPDX-License-Identifier: MPL-2.0

#[cfg(not(feature = "std"))]
pub(crate) use alloc::collections::{BTreeMap as Map, BTreeSet as Set};
#[cfg(feature = "std")]
pub(crate) use std::collections::{HashMap as Map, HashSet as Set};
