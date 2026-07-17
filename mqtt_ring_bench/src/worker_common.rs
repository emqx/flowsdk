// SPDX-License-Identifier: MPL-2.0

use io_uring::opcode;
use io_uring::types::{SubmitArgs, Timespec};
use io_uring::IoUring;
use std::collections::HashSet;
use std::io;
use std::time::Duration;

pub const OP_CONNECT: u64 = 0;
pub const OP_SEND: u64 = 1;
pub const OP_RECV: u64 = 2;

const CANCEL_USER_DATA_BIT: u64 = 1 << 63;
const SHUTDOWN_WAIT: Duration = Duration::from_millis(100);

pub fn encode_user_data(conn_key: usize, op: u64) -> u64 {
    (op << 48) | (conn_key as u64)
}

pub fn decode_user_data(data: u64) -> (usize, u64) {
    let conn_key = (data & 0x0000_FFFF_FFFF_FFFF) as usize;
    let op = data >> 48;
    (conn_key, op)
}

pub fn pending_user_data(
    conn_key: usize,
    connect_pending: bool,
    send_pending: bool,
    recv_pending: bool,
) -> impl Iterator<Item = u64> {
    [
        connect_pending.then(|| encode_user_data(conn_key, OP_CONNECT)),
        send_pending.then(|| encode_user_data(conn_key, OP_SEND)),
        recv_pending.then(|| encode_user_data(conn_key, OP_RECV)),
    ]
    .into_iter()
    .flatten()
}

/// Cancel and reap every operation before its socket and backing buffers are dropped.
pub fn cancel_pending_operations(ring: &mut IoUring, pending: Vec<u64>) -> io::Result<()> {
    let mut shutdown = ShutdownCompletions::new(&pending);

    for target in pending {
        let cancel_data = cancel_user_data(target);
        let cancel = opcode::AsyncCancel::new(target)
            .build()
            .user_data(cancel_data);

        loop {
            if unsafe { ring.submission().push(&cancel) }.is_ok() {
                shutdown.cancellations.insert(cancel_data);
                break;
            }
            submit_shutdown_batch(ring, &mut shutdown)?;
        }
    }

    submit_shutdown_batch(ring, &mut shutdown)?;
    while !shutdown.is_empty() {
        drain_shutdown_completions(ring, &mut shutdown);
        if shutdown.is_empty() {
            break;
        }

        let ts = Timespec::new()
            .sec(SHUTDOWN_WAIT.as_secs())
            .nsec(SHUTDOWN_WAIT.subsec_nanos());
        let args = SubmitArgs::new().timespec(&ts);
        match ring.submitter().submit_with_args(1, &args) {
            Ok(_) => {}
            Err(ref error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::EINTR | libc::EBUSY | libc::ETIME)
                ) => {}
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

fn submit_shutdown_batch(ring: &mut IoUring, shutdown: &mut ShutdownCompletions) -> io::Result<()> {
    loop {
        match ring.submitter().submit() {
            Ok(_) => break,
            Err(ref error) if matches!(error.raw_os_error(), Some(libc::EINTR | libc::EBUSY)) => {
                drain_shutdown_completions(ring, shutdown);
            }
            Err(error) => return Err(error),
        }
    }
    drain_shutdown_completions(ring, shutdown);
    Ok(())
}

fn drain_shutdown_completions(ring: &mut IoUring, shutdown: &mut ShutdownCompletions) {
    for cqe in ring.completion() {
        shutdown.record(cqe.user_data(), cqe.result());
    }
}

struct ShutdownCompletions {
    operations: HashSet<u64>,
    cancellations: HashSet<u64>,
}

impl ShutdownCompletions {
    fn new(pending: &[u64]) -> Self {
        Self {
            operations: pending.iter().copied().collect(),
            cancellations: HashSet::with_capacity(pending.len()),
        }
    }

    fn is_empty(&self) -> bool {
        self.operations.is_empty() && self.cancellations.is_empty()
    }

    fn record(&mut self, user_data: u64, result: i32) {
        if let Some(target) = cancel_target(user_data) {
            self.cancellations.remove(&user_data);
            // ENOENT means no request with this user_data remains in the ring.
            if result == -libc::ENOENT {
                self.operations.remove(&target);
            }
        } else {
            self.operations.remove(&user_data);
        }
    }
}

fn cancel_user_data(target: u64) -> u64 {
    debug_assert_eq!(target & CANCEL_USER_DATA_BIT, 0);
    target | CANCEL_USER_DATA_BIT
}

fn cancel_target(user_data: u64) -> Option<u64> {
    (user_data & CANCEL_USER_DATA_BIT != 0).then_some(user_data & !CANCEL_USER_DATA_BIT)
}

pub struct WorkerResult {
    pub latency_samples: Vec<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_user_data_contains_only_active_operations() {
        assert_eq!(
            pending_user_data(42, true, false, true).collect::<Vec<_>>(),
            vec![
                encode_user_data(42, OP_CONNECT),
                encode_user_data(42, OP_RECV),
            ]
        );
    }

    #[test]
    fn cancellation_tag_round_trips_target() {
        let target = encode_user_data(123, OP_SEND);
        let tagged = cancel_user_data(target);

        assert_eq!(cancel_target(tagged), Some(target));
        assert_eq!(cancel_target(target), None);
    }

    #[test]
    fn shutdown_waits_for_cancel_after_operation_completion() {
        let target = encode_user_data(123, OP_RECV);
        let cancel = cancel_user_data(target);
        let mut shutdown = ShutdownCompletions::new(&[target]);
        shutdown.cancellations.insert(cancel);

        shutdown.record(target, -libc::ECANCELED);
        assert!(!shutdown.is_empty());

        shutdown.record(cancel, 0);
        assert!(shutdown.is_empty());
    }

    #[test]
    fn shutdown_waits_for_operation_after_cancel_completion() {
        let target = encode_user_data(456, OP_SEND);
        let cancel = cancel_user_data(target);
        let mut shutdown = ShutdownCompletions::new(&[target]);
        shutdown.cancellations.insert(cancel);

        shutdown.record(cancel, 0);
        assert!(!shutdown.is_empty());

        shutdown.record(target, -libc::ECANCELED);
        assert!(shutdown.is_empty());
    }

    #[test]
    fn cancel_enoent_completes_missing_operation() {
        let target = encode_user_data(789, OP_CONNECT);
        let cancel = cancel_user_data(target);
        let mut shutdown = ShutdownCompletions::new(&[target]);
        shutdown.cancellations.insert(cancel);

        shutdown.record(cancel, -libc::ENOENT);

        assert!(shutdown.is_empty());
    }
}
