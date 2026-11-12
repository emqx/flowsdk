// SPDX-License-Identifier: Apache-2.0
//! Opt-in operation receipts. Queue cancellation and engine admission share a lock.
use super::*;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio::time::Instant as Deadline;

#[derive(Debug)]
pub struct TrackedPublishError {
    pub admitted: bool,
    pub error: MqttClientError,
}

#[derive(Debug)]
pub(super) struct State {
    pub generation: Option<Arc<()>>,
    pub admitted: bool,
    pub cancelled: bool,
    pub result: Option<Result<PublishResult, MqttClientError>>,
    pub written: Option<Result<(), MqttClientError>>,
}

#[derive(Debug)]
pub(super) struct Receipt {
    pub state: Mutex<State>,
    pub changed: watch::Sender<()>,
    pub deadline: Deadline,
}

impl Receipt {
    pub fn fail(&self, error: MqttClientError) {
        let mut state = self.state.lock().unwrap();
        if state.result.is_none() {
            state.result = Some(Err(error.clone()));
        }
        if state.written.is_none() {
            state.written = Some(Err(error));
        }
        self.changed.send_replace(());
    }

    pub fn written(&self, result: Result<(), MqttClientError>) {
        self.state.lock().unwrap().written.get_or_insert(result);
        self.changed.send_replace(());
    }

    pub fn complete(&self, result: Result<PublishResult, MqttClientError>) {
        let mut state = self.state.lock().unwrap();
        // A cancelled wait may leave nobody polling the deadline when an ACK
        // arrives. Only confirmations recorded before that deadline can win.
        state.result.get_or_insert_with(|| {
            if Deadline::now() >= self.deadline {
                Err(MqttClientError::OperationTimeout {
                    operation: "tracked publish".into(),
                    timeout_ms: 0,
                })
            } else {
                result
            }
        });
        self.changed.send_replace(());
    }
}

/// One publication, independently identified from its reusable MQTT packet ID.
/// Keep this handle across cancellation of `wait`; dropping it cancels only a
/// command which the worker has not admitted yet.
pub struct PublishOperation {
    pub(super) receipt: Arc<Receipt>,
}

impl Drop for PublishOperation {
    fn drop(&mut self) {
        let mut state = self.receipt.state.lock().unwrap();
        if !state.admitted {
            state.cancelled = true;
        }
    }
}

impl PublishOperation {
    pub fn drain_receipt(&self) -> PublishDrain {
        PublishDrain(self.receipt.clone())
    }

    /// QoS 0 completes at engine admission; QoS 1/2 at PUBACK/PUBCOMP (or rejection).
    /// Timeout atomically cancels a command still awaiting admission. An admitted
    /// exchange is never retracted by a timeout or by dropping its waiter.
    /// Cancelling this wait preserves its original deadline. Confirmations recorded
    /// before that deadline remain available when the wait resumes afterward.
    pub async fn wait(&mut self) -> Result<PublishResult, TrackedPublishError> {
        let mut changed = self.receipt.changed.subscribe();
        loop {
            {
                let mut state = self.receipt.state.lock().unwrap();
                if let Some(result) = state.result.clone() {
                    return result.map_err(|error| TrackedPublishError {
                        admitted: state.admitted,
                        error,
                    });
                }
                if Deadline::now() >= self.receipt.deadline {
                    if !state.admitted {
                        state.cancelled = true;
                    }
                    let error = MqttClientError::OperationTimeout {
                        operation: "tracked publish".into(),
                        timeout_ms: 0,
                    };
                    state.result = Some(Err(error.clone()));
                    if !state.admitted {
                        state.written = Some(Err(error.clone()));
                    }
                    self.receipt.changed.send_replace(());
                    return Err(TrackedPublishError {
                        admitted: state.admitted,
                        error,
                    });
                }
            }
            let _ = tokio::time::timeout_at(self.receipt.deadline, changed.changed()).await;
        }
    }
}

/// A per-operation local transport write-and-flush fence.
/// Success does not prove broker delivery.
#[derive(Clone)]
pub struct PublishDrain(Arc<Receipt>);

impl PublishDrain {
    pub fn same_connection(&self, other: &Self) -> bool {
        let generation = self.0.state.lock().unwrap().generation.clone();
        match (generation, &other.0.state.lock().unwrap().generation) {
            (Some(a), Some(b)) => Arc::ptr_eq(&a, b),
            _ => false,
        }
    }
    pub fn is_written(&self) -> bool {
        matches!(self.0.state.lock().unwrap().written, Some(Ok(())))
    }

    /// Wait only through this operation, without disconnecting the shared client.
    pub async fn drain_through(&self, deadline: Deadline) -> Result<(), MqttClientError> {
        let mut changed = self.0.changed.subscribe();
        loop {
            if let Some(result) = self.0.state.lock().unwrap().written.clone() {
                return result;
            }
            tokio::time::timeout_at(deadline, changed.changed())
                .await
                .map_err(|_| MqttClientError::OperationTimeout {
                    operation: "publish drain".into(),
                    timeout_ms: 0,
                })?
                .map_err(|_| MqttClientError::ChannelClosed {
                    channel: "publish drain".into(),
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
        if !self.receipt.state.lock().unwrap().admitted {
            self.receipt.fail(MqttClientError::OperationCancelled {
                operation: "queued publish".into(),
            });
        }
    }
}

impl TokioAsyncMqttClient {
    /// Enqueue without yielding, so the caller can retain its receipt atomically.
    /// A full command channel has not admitted the publication and is retryable.
    pub fn try_publish_tracked(
        &self,
        command: PublishCommand,
        deadline: Deadline,
    ) -> Result<PublishOperation, MqttClientError> {
        let (changed, _) = watch::channel(());
        let receipt = Arc::new(Receipt {
            state: Mutex::new(State {
                generation: None,
                admitted: false,
                cancelled: false,
                result: None,
                written: None,
            }),
            changed,
            deadline,
        });
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
        Ok(PublishOperation { receipt })
    }
}
