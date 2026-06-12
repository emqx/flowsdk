// SPDX-License-Identifier: MPL-2.0
//! Delivery token tracking for Paho-compatible `waitForCompletion`.
//!
//! Tracks pending MQTT operations by their packet ID (= delivery token),
//! providing `Condvar`-based blocking for the sync API's `waitForCompletion`.

use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Result of a completed delivery.
#[derive(Debug, Clone)]
pub struct CompletionResult {
    /// The delivery token (packet ID)
    pub token: i32,
    /// Whether the operation succeeded
    pub success: bool,
    /// Paho-style return code
    pub rc: i32,
}

/// Thread-safe delivery token tracker.
///
/// The I/O thread calls `mark_pending` when an operation starts and
/// `mark_completed` when acknowledgment arrives. The C caller thread
/// calls `wait_for_completion` which blocks on a `Condvar`.
pub struct TokenTracker {
    inner: Mutex<TokenTrackerInner>,
    notify: Condvar,
}

struct TokenTrackerInner {
    pending: HashSet<i32>,
    completed: HashMap<i32, CompletionResult>,
}

impl Default for TokenTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenTracker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(TokenTrackerInner {
                pending: HashSet::new(),
                completed: HashMap::new(),
            }),
            notify: Condvar::new(),
        }
    }

    /// Register a delivery token as pending.
    pub fn mark_pending(&self, token: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.insert(token);
        inner.completed.remove(&token);
    }

    /// Mark a delivery token as completed. Wakes any thread blocked in `wait_for_completion`.
    pub fn mark_completed(&self, result: CompletionResult) {
        let token = result.token;
        let mut inner = self.inner.lock().unwrap();
        inner.pending.remove(&token);
        inner.completed.insert(token, result);
        self.notify.notify_all();
    }

    /// Block until the given token completes or the timeout expires.
    /// Returns the completion result, or `None` on timeout.
    pub fn wait_for_completion(&self, token: i32, timeout: Duration) -> Option<CompletionResult> {
        let mut inner = self.inner.lock().unwrap();

        // Already completed?
        if let Some(result) = inner.completed.remove(&token) {
            return Some(result);
        }

        // Not pending? Must have already been collected or never existed.
        if !inner.pending.contains(&token) {
            return None;
        }

        // Wait with timeout
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return None; // Timeout
            }

            let (guard, wait_result) = self.notify.wait_timeout(inner, remaining).unwrap();
            inner = guard;

            if let Some(result) = inner.completed.remove(&token) {
                return Some(result);
            }

            if wait_result.timed_out() {
                return None;
            }
        }
    }

    /// Get all currently pending tokens. Used for `MQTTClient_getPendingDeliveryTokens`.
    pub fn get_pending_tokens(&self) -> Vec<i32> {
        let inner = self.inner.lock().unwrap();
        inner.pending.iter().copied().collect()
    }

    /// Clear all pending and completed tokens (e.g., on disconnect).
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending.clear();
        inner.completed.clear();
        self.notify.notify_all();
    }
}
