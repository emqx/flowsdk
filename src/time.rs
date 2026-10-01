// SPDX-License-Identifier: MPL-2.0

//! Caller-supplied monotonic time for the portable MQTT engine.
//!
//! Every timestamp for one engine must use the same origin. For Zephyr, pass
//! nonnegative `k_uptime_get()` values to [`Timestamp::try_from_millis`]. Neither
//! this module nor the portable engine reads a platform clock.

use core::{fmt, time::Duration};

/// Nanoseconds since an application-selected monotonic origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(u64);

/// A timestamp or duration cannot be represented by the selected clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeError;

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("monotonic timestamp is out of range")
    }
}

impl core::error::Error for TimeError {}

impl Timestamp {
    pub const ZERO: Self = Self(0);

    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    pub fn try_from_millis(millis: u64) -> Result<Self, TimeError> {
        millis.checked_mul(1_000_000).map(Self).ok_or(TimeError)
    }

    /// Convert signed Zephyr uptime without wrapping a negative value.
    pub fn try_from_uptime_millis(millis: i64) -> Result<Self, TimeError> {
        Self::try_from_millis(u64::try_from(millis).map_err(|_| TimeError)?)
    }

    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Whole elapsed milliseconds, rounded down.
    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000_000
    }

    /// Absolute deadline in milliseconds, rounded up so a poll never fires early.
    pub const fn as_millis_ceil(self) -> u64 {
        self.0 / 1_000_000 + (self.0 % 1_000_000 != 0) as u64
    }

    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let nanos = u64::try_from(duration.as_nanos()).ok()?;
        self.0.checked_add(nanos).map(Self)
    }

    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0).map(Duration::from_nanos)
    }

    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Timestamp {}
    #[cfg(feature = "std")]
    impl Sealed for std::time::Instant {}
}

/// Supported time representations. Applications supply values, not clock readers.
pub trait TimePoint: sealed::Sealed + Copy + Ord + fmt::Debug {
    fn checked_add(self, duration: Duration) -> Option<Self>;
    fn saturating_duration_since(self, earlier: Self) -> Duration;
}

impl TimePoint for Timestamp {
    fn checked_add(self, duration: Duration) -> Option<Self> {
        self.checked_add(duration)
    }

    fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.saturating_duration_since(earlier)
    }
}

#[cfg(feature = "std")]
impl TimePoint for std::time::Instant {
    fn checked_add(self, duration: Duration) -> Option<Self> {
        std::time::Instant::checked_add(&self, duration)
    }

    fn saturating_duration_since(self, earlier: Self) -> Duration {
        std::time::Instant::saturating_duration_since(&self, earlier)
    }
}

/// Default host representation; portable callers should name [`Timestamp`].
#[cfg(feature = "std")]
pub type DefaultTime = std::time::Instant;
/// Default representation when the standard library is disabled.
#[cfg(not(feature = "std"))]
pub type DefaultTime = Timestamp;
