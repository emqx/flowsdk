// SPDX-License-Identifier: MPL-2.0

use io_uring::opcode;
use io_uring::types::{Fd, SubmitArgs, Timespec};
use io_uring::IoUring;
use slab::Slab;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::os::fd::IntoRawFd;
use std::os::fd::RawFd;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::BenchConfig;
use crate::connection::{ConnState, Connection};
use crate::stats::{BenchStats, ErrorKind};
use crate::worker_common::{
    decode_user_data, encode_user_data, WorkerResult, OP_CONNECT, OP_RECV, OP_SEND,
};

/// Each Connection is Box-allocated so its buffers have stable addresses
/// for io_uring SQEs (Slab<T> may relocate entries on grow).
pub fn run_worker(
    worker_id: usize,
    conn_range: std::ops::Range<usize>,
    config: Arc<BenchConfig>,
    stats: Arc<BenchStats>,
) -> WorkerResult {
    let num_conns = conn_range.len();
    if num_conns == 0 {
        return WorkerResult {
            latency_samples: Vec::new(),
        };
    }

    let ring_size = 4096u32.min((num_conns as u32 * 3).next_power_of_two().max(64));
    let mut ring: IoUring = IoUring::builder()
        .build(ring_size)
        .unwrap_or_else(|e| panic!("worker {}: failed to create io_uring: {}", worker_id, e));

    let mut conns: Slab<Box<Connection>> = Slab::with_capacity(num_conns);

    // We need SockAddr to stay alive for the duration of connect SQEs.
    // Keep one per in-flight connect. Once connected, it can be dropped.
    let sock_addr = SockAddr::from(config.addr);
    // Pin the SockAddr in a Box so its address is stable for io_uring.
    let sock_addr_box = Box::new(sock_addr);
    let sa_ptr = sock_addr_box.as_ref().as_ptr();
    let sa_len = sock_addr_box.as_ref().len();

    let budget_per_sec = config.connect_rate / config.workers.max(1);
    let mut connect_budget: f64 = 0.0;
    let mut last_budget_tick = Instant::now();
    let mut pending_create: Vec<usize> = conn_range.collect();
    pending_create.reverse();

    let mut done_count: usize = 0;
    let mut ifaddr_idx: usize = 0;
    // Upper bound on idle wait — keeps Ctrl+C / shutdown responsive even when
    // no protocol timer is scheduled.
    let max_idle_wait = Duration::from_secs(1);

    loop {
        if stats.stopped.load(Ordering::Relaxed) {
            break;
        }

        let now = Instant::now();

        // Replenish connect budget using fractional accumulation
        let elapsed_secs = now.duration_since(last_budget_tick).as_secs_f64();
        if elapsed_secs >= 0.01 {
            connect_budget += budget_per_sec as f64 * elapsed_secs;
            if connect_budget > pending_create.len() as f64 {
                connect_budget = pending_create.len() as f64;
            }
            last_budget_tick = now;
        }

        // Create sockets and submit connect SQEs
        while connect_budget >= 1.0 {
            if let Some(client_idx) = pending_create.pop() {
                let bind_ip = if config.ifaddrs.is_empty() {
                    None
                } else {
                    let ip = config.ifaddrs[ifaddr_idx % config.ifaddrs.len()];
                    ifaddr_idx += 1;
                    Some(ip)
                };
                match create_socket(&config, bind_ip) {
                    Ok(fd) => {
                        let conn = Box::new(Connection::new(fd, client_idx, &config));
                        let key = conns.insert(conn);

                        let connect_e = opcode::Connect::new(Fd(fd), sa_ptr as *const _, sa_len)
                            .build()
                            .user_data(encode_user_data(key, OP_CONNECT));

                        unsafe {
                            if ring.submission().push(&connect_e).is_err() {
                                // SQ full, submit and retry
                                let _ = ring.submitter().submit();
                                let _ = ring.submission().push(&connect_e);
                            }
                        }
                        connect_budget -= 1.0;
                    }
                    Err(e) => {
                        eprintln!("worker {}: socket creation failed: {}", worker_id, e);
                        stats.record_error(ErrorKind::Socket);
                        stats.clients_done.fetch_add(1, Ordering::Relaxed);
                        done_count += 1;
                        connect_budget -= 1.0;
                    }
                }
            } else {
                break;
            }
        }

        // Wait for the soonest of: next protocol deadline, a CQE arrival, or
        // `max_idle_wait` (keeps shutdown responsive when nothing is scheduled).
        let min_deadline = conns
            .iter()
            .filter(|(_, c)| {
                c.state != ConnState::Done
                    && c.state != ConnState::Failed
                    && c.state != ConnState::TcpConnecting
            })
            .filter_map(|(_, c)| c.next_tick_at())
            .min();
        let wait_dur = match min_deadline {
            Some(d) => d.saturating_duration_since(now).min(max_idle_wait),
            None => max_idle_wait,
        };
        let wait_count = if conns.is_empty() || wait_dur.is_zero() {
            0
        } else {
            1
        };
        let ts = Timespec::new()
            .sec(wait_dur.as_secs())
            .nsec(wait_dur.subsec_nanos());
        let args = SubmitArgs::new().timespec(&ts);
        match ring.submitter().submit_with_args(wait_count, &args) {
            Ok(_) => {}
            Err(ref e) if e.raw_os_error() == Some(libc::EINTR) => continue,
            Err(ref e) if e.raw_os_error() == Some(libc::EBUSY) => continue,
            Err(ref e) if e.raw_os_error() == Some(libc::ETIME) => {} // deadline hit, fall through to tick
            Err(e) => {
                eprintln!("worker {}: io_uring submit error: {}", worker_id, e);
                break;
            }
        }

        // Process completions
        let cqes: Vec<_> = ring.completion().collect();
        for cqe in cqes {
            let (conn_key, op) = decode_user_data(cqe.user_data());
            let result = cqe.result();

            if !conns.contains(conn_key) {
                continue;
            }

            match op {
                OP_CONNECT => {
                    handle_connect_complete(&mut conns, conn_key, result, &mut ring, &stats);
                }
                OP_SEND => {
                    handle_send_complete(
                        &mut conns,
                        conn_key,
                        result,
                        &mut ring,
                        &config,
                        &stats,
                        &mut done_count,
                    );
                }
                OP_RECV => {
                    handle_recv_complete(
                        &mut conns,
                        conn_key,
                        result,
                        &mut ring,
                        &config,
                        &stats,
                        &mut done_count,
                    );
                }
                _ => {}
            }
        }

        // Try to publish on ready connections
        let keys: Vec<usize> = conns
            .iter()
            .filter(|(_, c)| c.state == ConnState::Publishing && !c.send_pending)
            .map(|(k, _)| k)
            .collect();
        for key in keys {
            let conn = &mut conns[key];
            if conn.try_publish(&config) {
                stats.messages_sent.fetch_add(1, Ordering::Relaxed);
                submit_send(&mut ring, key, conn);
            }
            conn.check_publish_complete();
            if conn.state == ConnState::Draining
                && !conn.send_pending
                && conn.check_drain_complete()
            {
                submit_send(&mut ring, key, conn);
            }
        }

        // Drive protocol timers. Connection::handle_tick is a cheap no-op if
        // the per-connection deadline hasn't been reached, so iterating every
        // loop pass is fine — and only happens when we either got a CQE or
        // crossed the global min_deadline computed above.
        let tick_keys: Vec<usize> = conns
            .iter()
            .filter(|(_, c)| {
                c.state != ConnState::Done
                    && c.state != ConnState::Failed
                    && c.state != ConnState::TcpConnecting
            })
            .map(|(k, _)| k)
            .collect();
        for key in tick_keys {
            let conn = &mut conns[key];
            let events = conn.handle_tick();
            if !events.is_empty() {
                let outcome = conn.process_events(events);
                for _ in 0..outcome.errors {
                    stats.record_error(ErrorKind::Mqtt);
                }
                if outcome.acked > 0 {
                    stats
                        .messages_acked
                        .fetch_add(outcome.acked, Ordering::Relaxed);
                }
            }
            // PINGREQ / retransmits produce outgoing bytes without any event.
            if conn.has_pending_send() && !conn.send_pending {
                submit_send(&mut ring, key, conn);
            }
        }

        if done_count >= num_conns && pending_create.is_empty() {
            break;
        }
    }

    // Collect latency samples
    let mut all_samples = Vec::new();
    for (_, conn) in conns.iter_mut() {
        all_samples.append(&mut conn.latency_samples);
    }

    // Close fds
    for (_, conn) in conns.iter() {
        unsafe {
            libc::close(conn.fd);
        }
    }

    drop(sock_addr_box);

    WorkerResult {
        latency_samples: all_samples,
    }
}

fn create_socket(
    config: &BenchConfig,
    bind_addr: Option<std::net::IpAddr>,
) -> std::io::Result<RawFd> {
    let domain = if config.addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_nonblocking(true)?;
    socket.set_nodelay(true)?;
    socket.set_recv_buffer_size(config.socket_buf)?;
    socket.set_send_buffer_size(config.socket_buf)?;

    // Bind to a specific source IP; port 0 lets the OS pick an ephemeral port
    if let Some(ip) = bind_addr {
        let bind_sa = SockAddr::from(std::net::SocketAddr::new(ip, 0));
        socket.bind(&bind_sa)?;
    }

    Ok(socket.into_raw_fd())
}

fn handle_connect_complete(
    conns: &mut Slab<Box<Connection>>,
    key: usize,
    result: i32,
    ring: &mut IoUring,
    stats: &BenchStats,
) {
    let conn = &mut conns[key];
    if result < 0 {
        eprintln!(
            "connection {}: TCP connect failed: {}",
            conn.client_index,
            std::io::Error::from_raw_os_error(-result)
        );
        conn.state = ConnState::Failed;
        stats.record_error(ErrorKind::Connect);
        stats.clients_done.fetch_add(1, Ordering::Relaxed);
        return;
    }

    stats.clients_connected.fetch_add(1, Ordering::Relaxed);
    conn.initiate_mqtt_connect();
    submit_send(ring, key, conn);
    submit_recv(ring, key, conn);
}

fn handle_send_complete(
    conns: &mut Slab<Box<Connection>>,
    key: usize,
    result: i32,
    ring: &mut IoUring,
    config: &BenchConfig,
    stats: &BenchStats,
    done_count: &mut usize,
) {
    let conn = &mut conns[key];
    conn.send_pending = false;

    if result < 0 {
        let err_code = -result;
        if err_code != libc::ECONNRESET && err_code != libc::EPIPE {
            eprintln!(
                "connection {}: send failed: {}",
                conn.client_index,
                std::io::Error::from_raw_os_error(err_code)
            );
        }
        mark_failed(conn, stats, done_count, ErrorKind::Send);
        return;
    }

    conn.advance_send(result as usize);

    // Continue sending remaining bytes
    if conn.has_pending_send() {
        submit_send(ring, key, conn);
        return;
    }

    match conn.state {
        ConnState::Disconnecting => {
            conn.state = ConnState::Done;
            stats.clients_done.fetch_add(1, Ordering::Relaxed);
            *done_count += 1;
        }
        ConnState::Publishing => {
            if conn.try_publish(config) {
                stats.messages_sent.fetch_add(1, Ordering::Relaxed);
                submit_send(ring, key, conn);
            }
            conn.check_publish_complete();
        }
        _ => {}
    }
}

fn handle_recv_complete(
    conns: &mut Slab<Box<Connection>>,
    key: usize,
    result: i32,
    ring: &mut IoUring,
    config: &BenchConfig,
    stats: &BenchStats,
    done_count: &mut usize,
) {
    let conn = &mut conns[key];
    conn.recv_pending = false;

    if result <= 0 {
        if result < 0 {
            let err_code = -result;
            if err_code != libc::ECONNRESET {
                eprintln!(
                    "connection {}: recv error: {}",
                    conn.client_index,
                    std::io::Error::from_raw_os_error(err_code)
                );
            }
        }
        if conn.state != ConnState::Done && conn.state != ConnState::Disconnecting {
            mark_failed(conn, stats, done_count, ErrorKind::Receive);
        }
        return;
    }

    let nbytes = result as usize;
    // SAFETY: recv_buf is owned by the Box<Connection> which has a stable address.
    // The kernel wrote into recv_buf during the recv operation.
    // We must clone/copy the data before submitting the next recv on the same buffer.
    let data: Vec<u8> = conn.recv_buf[..nbytes].to_vec();
    let events = conn.handle_incoming(&data);

    if !events.is_empty() {
        let outcome = conn.process_events(events);
        if outcome.acked > 0 {
            stats
                .messages_acked
                .fetch_add(outcome.acked, Ordering::Relaxed);
        }
        for _ in 0..outcome.errors {
            stats.record_error(ErrorKind::Mqtt);
        }
    }

    let drain_complete = matches!(conn.state, ConnState::Draining) && conn.check_drain_complete();

    match conn.state {
        ConnState::Publishing => {
            if conn.has_pending_send() && !conn.send_pending {
                submit_send(ring, key, conn);
            } else if !conn.send_pending && conn.try_publish(config) {
                stats.messages_sent.fetch_add(1, Ordering::Relaxed);
                submit_send(ring, key, conn);
            }
            conn.check_publish_complete();
            submit_recv(ring, key, conn);
        }
        ConnState::MqttConnecting => {
            if conn.has_pending_send() && !conn.send_pending {
                submit_send(ring, key, conn);
            }
            submit_recv(ring, key, conn);
        }
        ConnState::Draining if drain_complete => {
            if conn.has_pending_send() && !conn.send_pending {
                submit_send(ring, key, conn);
            } else if !conn.has_pending_send() {
                conn.state = ConnState::Done;
                stats.clients_done.fetch_add(1, Ordering::Relaxed);
                *done_count += 1;
            }
        }
        ConnState::Draining => {
            submit_recv(ring, key, conn);
        }
        ConnState::Failed | ConnState::Done => {}
        _ => {
            submit_recv(ring, key, conn);
        }
    }
}

fn mark_failed(
    conn: &mut Connection,
    stats: &BenchStats,
    done_count: &mut usize,
    error_kind: ErrorKind,
) {
    if conn.state != ConnState::Failed && conn.state != ConnState::Done {
        conn.state = ConnState::Failed;
        stats.record_error(error_kind);
        stats.clients_done.fetch_add(1, Ordering::Relaxed);
        *done_count += 1;
    }
}

fn submit_send(ring: &mut IoUring, key: usize, conn: &mut Connection) {
    if !conn.has_pending_send() || conn.send_pending {
        return;
    }
    let slice = conn.pending_send_slice();
    let ptr = slice.as_ptr();
    let len = slice.len() as u32;

    let send_e = opcode::Send::new(Fd(conn.fd), ptr, len)
        .build()
        .user_data(encode_user_data(key, OP_SEND));

    conn.send_pending = true;
    unsafe {
        if ring.submission().push(&send_e).is_err() {
            let _ = ring.submitter().submit();
            let _ = ring.submission().push(&send_e);
        }
    }
}

fn submit_recv(ring: &mut IoUring, key: usize, conn: &mut Connection) {
    if conn.recv_pending {
        return;
    }
    let buf_ptr = conn.recv_buf.as_mut_ptr();
    let buf_len = conn.recv_buf.len() as u32;

    let recv_e = opcode::Recv::new(Fd(conn.fd), buf_ptr, buf_len)
        .build()
        .user_data(encode_user_data(key, OP_RECV));

    conn.recv_pending = true;
    unsafe {
        if ring.submission().push(&recv_e).is_err() {
            let _ = ring.submitter().submit();
            let _ = ring.submission().push(&recv_e);
        }
    }
}
