// SPDX-License-Identifier: Apache-2.0
//! Opt-in operation receipts. Enqueueing transfers publication ownership to the worker.
use super::*;
use std::sync::{Arc, OnceLock};
use tokio::sync::Notify;
use tokio::time::Instant as Deadline;

#[derive(Debug)]
pub struct TrackedPublishError {
    /// Whether engine admission has been observed when this error is returned.
    /// On timeout, `false` does not prevent later admission or make retrying safe.
    pub admitted: bool,
    pub error: MqttClientError,
}

#[derive(Debug)]
pub(super) struct Completion {
    at: Deadline,
    result: Result<PublishResult, MqttClientError>,
}

#[derive(Debug)]
pub(super) struct Receipt {
    // Only the worker publishes these immutable milestones. Waiters never
    // change the request or contend with admission over mutable receipt state.
    pub generation: OnceLock<Arc<()>>,
    pub completion: OnceLock<Completion>,
    pub written: OnceLock<Result<(), MqttClientError>>,
    changed: Notify,
    pub deadline: Deadline,
}

impl Receipt {
    pub fn new(deadline: Deadline) -> Self {
        Self {
            generation: OnceLock::new(),
            completion: OnceLock::new(),
            written: OnceLock::new(),
            changed: Notify::new(),
            deadline,
        }
    }

    pub fn is_admitted(&self) -> bool {
        self.generation.get().is_some()
    }

    pub fn fail(&self, error: MqttClientError) {
        self.completion.get_or_init(|| Completion {
            at: Deadline::now(),
            result: Err(error.clone()),
        });
        self.written.get_or_init(|| Err(error));
        self.changed.notify_waiters();
    }

    pub fn written(&self, result: Result<(), MqttClientError>) {
        if self.written.set(result).is_ok() {
            self.changed.notify_waiters();
        }
    }

    pub fn complete(&self, result: Result<PublishResult, MqttClientError>) {
        // Preserve confirmation timing even if nobody is currently waiting.
        if self
            .completion
            .set(Completion {
                at: Deadline::now(),
                result,
            })
            .is_ok()
        {
            self.changed.notify_waiters();
        }
    }
}

/// One publication, independently identified from its reusable MQTT packet ID.
/// Dropping this handle or cancelling `wait` never retracts a queued publication.
pub struct PublishOperation {
    pub(super) receipt: Arc<Receipt>,
    timed_out: bool,
}

impl PublishOperation {
    pub(super) fn new(receipt: Arc<Receipt>) -> Self {
        Self {
            receipt,
            timed_out: false,
        }
    }

    pub fn drain_receipt(&self) -> PublishDrain {
        PublishDrain(self.receipt.clone())
    }

    /// QoS 0 completes at engine admission; QoS 1/2 at PUBACK/PUBCOMP (or rejection).
    /// Timeout stops only this wait; even a command awaiting admission may still
    /// publish afterward. A timeout is not evidence that retrying is safe.
    /// Cancelling this wait preserves its original deadline. Confirmations recorded
    /// before that deadline remain available when the wait resumes afterward.
    pub async fn wait(&mut self) -> Result<PublishResult, TrackedPublishError> {
        if self.timed_out {
            return Err(self.timeout_error());
        }
        loop {
            if let Some(result) = self.completed_result() {
                return result;
            }
            if Deadline::now() >= self.receipt.deadline {
                // A timely completion may have arrived between the first read
                // and observing the deadline. Prefer it over a local timeout.
                if let Some(result) = self.completed_result() {
                    return result;
                }
                // Cache timeout only on this handle. It neither changes worker
                // state nor poisons the independently observable write fence.
                self.timed_out = true;
                return Err(self.timeout_error());
            }
            // Register then recheck to avoid missing a concurrent completion.
            // Already completed receipts never need a notification waiter.
            let changed = self.receipt.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.receipt.completion.get().is_some() {
                continue;
            }
            let _ = tokio::time::timeout_at(self.receipt.deadline, changed).await;
        }
    }

    fn completed_result(&self) -> Option<Result<PublishResult, TrackedPublishError>> {
        self.receipt.completion.get().map(|completion| {
            if completion.at < self.receipt.deadline {
                completion
                    .result
                    .clone()
                    .map_err(|error| TrackedPublishError {
                        admitted: self.receipt.is_admitted(),
                        error,
                    })
            } else {
                Err(self.timeout_error())
            }
        })
    }

    fn timeout_error(&self) -> TrackedPublishError {
        TrackedPublishError {
            admitted: self.receipt.is_admitted(),
            error: MqttClientError::OperationTimeout {
                operation: "tracked publish".into(),
                timeout_ms: 0,
            },
        }
    }
}

/// A per-operation local transport write-and-flush fence.
/// Success does not prove broker delivery.
#[derive(Clone)]
pub struct PublishDrain(Arc<Receipt>);

impl PublishDrain {
    /// Compare the transports on which the operations were admitted. Every new
    /// transport, including an intentional reconnect, has a distinct generation.
    pub fn same_connection(&self, other: &Self) -> bool {
        match (self.0.generation.get(), other.0.generation.get()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn is_written(&self) -> bool {
        matches!(self.0.written.get(), Some(Ok(())))
    }

    /// Wait only through this operation, without disconnecting the shared client.
    pub async fn drain_through(&self, deadline: Deadline) -> Result<(), MqttClientError> {
        loop {
            if let Some(result) = self.0.written.get() {
                return result.clone();
            }
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.0.written.get().is_some() {
                continue;
            }
            tokio::time::timeout_at(deadline, changed)
                .await
                .map_err(|_| MqttClientError::OperationTimeout {
                    operation: "publish drain".into(),
                    timeout_ms: 0,
                })?;
        }
    }
}

#[derive(Debug)]
pub(super) struct TrackedCommand {
    pub command: PublishCommand,
    pub receipt: Arc<Receipt>,
}

impl Drop for TrackedCommand {
    fn drop(&mut self) {
        // Also wakes waiters when the worker is aborted with queued commands.
        if !self.receipt.is_admitted() {
            self.receipt.fail(MqttClientError::OperationCancelled {
                operation: "publish worker stopped before admission".into(),
            });
        }
    }
}

impl TokioAsyncMqttClient {
    /// Enqueue without yielding, so the caller can retain its receipt atomically.
    /// A full command channel has not admitted the publication and is retryable.
    /// After successful enqueueing, dropping the operation or reaching `deadline`
    /// does not cancel publication. The deadline bounds confirmation waiting only.
    pub fn try_publish_tracked(
        &self,
        command: PublishCommand,
        deadline: Deadline,
    ) -> Result<PublishOperation, MqttClientError> {
        let receipt = Arc::new(Receipt::new(deadline));
        self.command_tx
            .try_send(TokioClientCommand::PublishTracked(TrackedCommand {
                command,
                receipt: receipt.clone(),
            }))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => MqttClientError::BufferFull {
                    buffer_type: "tracked command queue".into(),
                    capacity: self._config.command_queue_size,
                },
                mpsc::error::TrySendError::Closed(_) => MqttClientError::ChannelClosed {
                    channel: "tracked command queue".into(),
                },
            })?;
        Ok(PublishOperation::new(receipt))
    }
}
