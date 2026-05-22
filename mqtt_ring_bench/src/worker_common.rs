// SPDX-License-Identifier: MPL-2.0

use std::time::Duration;

pub const OP_CONNECT: u64 = 0;
pub const OP_SEND: u64 = 1;
pub const OP_RECV: u64 = 2;

pub fn encode_user_data(conn_key: usize, op: u64) -> u64 {
    (op << 48) | (conn_key as u64)
}

pub fn decode_user_data(data: u64) -> (usize, u64) {
    let conn_key = (data & 0x0000_FFFF_FFFF_FFFF) as usize;
    let op = data >> 48;
    (conn_key, op)
}

pub struct WorkerResult {
    pub latency_samples: Vec<Duration>,
}
