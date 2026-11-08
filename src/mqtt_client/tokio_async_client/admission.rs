// SPDX-License-Identifier: Apache-2.0

//! Local publish admission envelopes and bounded logical input accounting.

use super::{MqttClientError, Property, PublishCommand, Response};
use std::fmt;
use std::mem::size_of;
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, Notify};

/// Opt-in limits shared by single and batch local-admission requests.
///
/// Byte charges include visible payload/topic/property contents, one
/// `size_of::<PublishCommand>()` per command, and `size_of::<Property>()` per
/// property. They exclude spare allocation capacity, shared backing storage,
/// response storage, and inputs in caller-owned futures waiting for credits.
/// Engine queues and extracted transport output have separate limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishAdmissionConfig {
    pub max_batch_messages: usize,
    pub max_batch_bytes: usize,
    pub max_pending_messages: usize,
    pub max_pending_bytes: usize,
}

impl PublishAdmissionConfig {
    pub(super) fn validate(&self) -> Result<(), MqttClientError> {
        if self.max_batch_messages == 0
            || self.max_batch_bytes == 0
            || self.max_pending_messages == 0
            || self.max_pending_bytes == 0
            || self.max_batch_messages > self.max_pending_messages
            || self.max_batch_bytes > self.max_pending_bytes
        {
            return Err(MqttClientError::InvalidConfiguration {
                field: "publish_admission".into(),
                reason: "Limits must be nonzero and every maximum-size batch must fit the pending limits".into(),
            });
        }
        Ok(())
    }
}

/// Definitive local admission results, one per command in input order.
///
/// `Ok(None)` admits QoS 0; `Ok(Some(id))` admits QoS 1/2. An inner error means
/// that item was not admitted. Acceptance does not confirm transport or broker
/// delivery. A successful report may contain only rejections.
#[derive(Debug)]
#[must_use = "Each publication's admission result must be inspected"]
pub struct PublishBatchAdmission {
    pub results: Vec<Result<Option<u16>, MqttClientError>>,
}

/// Distinguishes a request that never entered the worker from a lost response.
#[derive(Debug)]
#[non_exhaustive]
pub enum PublishBatchAdmissionError {
    /// Nothing was submitted. The original commands can be split or retried.
    NotSubmitted {
        error: MqttClientError,
        commands: Vec<PublishCommand>,
    },
    /// Any subset, including all commands, may already have been admitted.
    /// Retrying the complete batch can duplicate accepted publications.
    OutcomeUnknown { error: MqttClientError },
}

impl fmt::Display for PublishBatchAdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSubmitted { error, commands } => {
                write!(
                    f,
                    "publish batch ({} commands) was not submitted: {error}",
                    commands.len()
                )
            }
            Self::OutcomeUnknown { error } => {
                write!(f, "publish batch admission outcome is unknown: {error}")
            }
        }
    }
}

impl std::error::Error for PublishBatchAdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotSubmitted { error, .. } | Self::OutcomeUnknown { error } => Some(error),
        }
    }
}

pub(super) fn channel_closed() -> MqttClientError {
    MqttClientError::ChannelClosed {
        channel: "async client admission".into(),
    }
}

fn overflow() -> MqttClientError {
    MqttClientError::InvalidConfiguration {
        field: "publish_admission".into(),
        reason: "Logical command byte charge overflow".into(),
    }
}

/// Calculate logical input size without serializing or copying payloads.
/// Exhaustively match properties so new owned property fields require an audit.
pub(super) fn command_bytes(command: &PublishCommand) -> Result<usize, MqttClientError> {
    let mut total = size_of::<PublishCommand>()
        .checked_add(command.payload.len())
        .and_then(|n| n.checked_add(command.topic_name.len()))
        .ok_or_else(overflow)?;
    for property in &command.properties {
        total = total
            .checked_add(size_of::<Property>())
            .ok_or_else(overflow)?;
        let bytes = match property {
            Property::ContentType(value)
            | Property::ResponseTopic(value)
            | Property::AssignedClientIdentifier(value)
            | Property::AuthenticationMethod(value)
            | Property::ResponseInformation(value)
            | Property::ServerReference(value)
            | Property::ReasonString(value) => value.len(),
            Property::CorrelationData(value) | Property::AuthenticationData(value) => value.len(),
            Property::UserProperty(key, value) => {
                key.len().checked_add(value.len()).ok_or_else(overflow)?
            }
            Property::PayloadFormatIndicator(_)
            | Property::MessageExpiryInterval(_)
            | Property::SubscriptionIdentifier(_)
            | Property::SessionExpiryInterval(_)
            | Property::ServerKeepAlive(_)
            | Property::RequestProblemInformation(_)
            | Property::WillDelayInterval(_)
            | Property::RequestResponseInformation(_)
            | Property::ReceiveMaximum(_)
            | Property::TopicAliasMaximum(_)
            | Property::TopicAlias(_)
            | Property::MaximumQoS(_)
            | Property::RetainAvailable(_)
            | Property::MaximumPacketSize(_)
            | Property::WildcardSubscriptionAvailable(_)
            | Property::SubscriptionIdentifierAvailable(_)
            | Property::SharedSubscriptionAvailable(_) => 0,
        };
        total = total.checked_add(bytes).ok_or_else(overflow)?;
    }
    Ok(total)
}

pub(super) fn batch_bytes(
    commands: &[PublishCommand],
    config: &PublishAdmissionConfig,
) -> Result<usize, MqttClientError> {
    if commands.len() > config.max_batch_messages {
        return Err(MqttClientError::BufferFull {
            buffer_type: "admission batch messages".into(),
            capacity: config.max_batch_messages,
        });
    }
    let bytes = commands.iter().try_fold(0usize, |total, command| {
        total
            .checked_add(command_bytes(command)?)
            .ok_or_else(overflow)
    })?;
    if bytes > config.max_batch_bytes {
        return Err(MqttClientError::BufferFull {
            buffer_type: "admission batch bytes".into(),
            capacity: config.max_batch_bytes,
        });
    }
    Ok(bytes)
}

#[derive(Debug, Default)]
struct Usage {
    messages: usize,
    bytes: usize,
}

#[derive(Debug)]
pub(super) struct AdmissionBudget {
    config: PublishAdmissionConfig,
    usage: Mutex<Usage>,
    changed: Notify,
}

impl AdmissionBudget {
    pub(super) fn new(config: PublishAdmissionConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            usage: Mutex::new(Usage::default()),
            changed: Notify::new(),
        })
    }

    pub(super) async fn reserve(
        self: &Arc<Self>,
        messages: usize,
        bytes: usize,
    ) -> Result<AdmissionPermit, MqttClientError> {
        for (requested, capacity, kind) in [
            (
                messages,
                self.config.max_pending_messages,
                "admission pending messages",
            ),
            (
                bytes,
                self.config.max_pending_bytes,
                "admission pending bytes",
            ),
        ] {
            if requested > capacity {
                return Err(MqttClientError::BufferFull {
                    buffer_type: kind.into(),
                    capacity,
                });
            }
        }
        loop {
            // Register before observing capacity. notify_waiters wakes every
            // contender; registering first prevents losing a concurrent release.
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut used = self.usage.lock().expect("admission budget mutex poisoned");
                if messages <= self.config.max_pending_messages - used.messages
                    && bytes <= self.config.max_pending_bytes - used.bytes
                {
                    used.messages += messages;
                    used.bytes += bytes;
                    return Ok(AdmissionPermit {
                        budget: Arc::clone(self),
                        messages,
                        bytes,
                    });
                }
            }
            notified.await;
        }
    }
}

#[derive(Debug)]
pub(super) struct AdmissionPermit {
    budget: Arc<AdmissionBudget>,
    messages: usize,
    bytes: usize,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        {
            let mut used = self
                .budget
                .usage
                .lock()
                .expect("admission budget mutex poisoned");
            used.messages -= self.messages;
            used.bytes -= self.bytes;
        }
        self.budget.changed.notify_waiters();
    }
}

/// One bounded lane for both request shapes. Credits live with each envelope
/// until all of its commands are processed or the envelope is dropped.
#[derive(Debug)]
pub(super) enum PublishAdmission {
    Single {
        command: PublishCommand,
        response_tx: Response<Option<u16>>,
        _permit: Option<AdmissionPermit>,
    },
    Batch {
        commands: Vec<PublishCommand>,
        response_tx: oneshot::Sender<PublishBatchAdmission>,
        _permit: Option<AdmissionPermit>,
    },
}

impl PublishAdmission {
    pub(super) fn respond(
        self,
        mut publish: impl FnMut(PublishCommand) -> Result<Option<u16>, MqttClientError>,
    ) {
        match self {
            Self::Single {
                command,
                response_tx,
                _permit,
            } => {
                let _ = response_tx.send(publish(command));
            }
            Self::Batch {
                commands,
                response_tx,
                _permit,
            } => {
                let results = commands.into_iter().map(&mut publish).collect();
                let _ = response_tx.send(PublishBatchAdmission { results });
            }
        }
    }

    pub(super) fn reject(self, error: MqttClientError) {
        self.respond(|_| Err(error.clone()));
    }
}
