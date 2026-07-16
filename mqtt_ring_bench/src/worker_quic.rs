// SPDX-License-Identifier: MPL-2.0

use io_uring::opcode;
use io_uring::types::{Fd, SubmitArgs, Timespec};
use io_uring::IoUring;
use slab::Slab;
use socket2::{Domain, SockAddr, Socket, Type};
use std::os::fd::IntoRawFd;
use std::os::fd::RawFd;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crate::config::BenchConfig;
use crate::connection::{QuicConnState, QuicConnection};
use crate::stats::{BenchStats, ErrorKind};
use crate::worker_common::{decode_user_data, encode_user_data, WorkerResult, OP_RECV, OP_SEND};

pub fn run_quic_worker(
    worker_id: usize,
    conn_range: std::ops::Range<usize>,
    config: Arc<BenchConfig>,
    stats: Arc<BenchStats>,
    crypto: Arc<rustls::ClientConfig>,
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

    let mut conns: Slab<Box<QuicConnection>> = Slab::with_capacity(num_conns);

    let server_name = config
        .server_name
        .as_deref()
        .unwrap_or(&config.host)
        .to_string();

    let budget_per_sec = config.connect_rate / config.workers.max(1);
    let mut connect_budget: f64 = 0.0;
    let mut last_budget_tick = Instant::now();
    let mut pending_create: Vec<usize> = conn_range.collect();
    pending_create.reverse();

    let mut done_count: usize = 0;
    let mut ifaddr_idx: usize = 0;

    loop {
        if stats.stopped.load(Ordering::Relaxed) {
            break;
        }

        let now = Instant::now();

        // Replenish connect budget
        let elapsed_secs = now.duration_since(last_budget_tick).as_secs_f64();
        if elapsed_secs >= 0.01 {
            connect_budget += budget_per_sec as f64 * elapsed_secs;
            if connect_budget > pending_create.len() as f64 {
                connect_budget = pending_create.len() as f64;
            }
            last_budget_tick = now;
        }

        // Create UDP sockets, initiate QUIC handshake
        while connect_budget >= 1.0 {
            if let Some(client_idx) = pending_create.pop() {
                let bind_ip = if config.ifaddrs.is_empty() {
                    None
                } else {
                    let ip = config.ifaddrs[ifaddr_idx % config.ifaddrs.len()];
                    ifaddr_idx += 1;
                    Some(ip)
                };
                match create_udp_socket(&config, bind_ip) {
                    Ok(fd) => {
                        // "connect" the UDP socket so Send/Recv work without sendto/recvfrom
                        let ret = udp_connect(fd, &config.addr);
                        if ret < 0 {
                            let e = std::io::Error::last_os_error();
                            eprintln!(
                                "worker {}: UDP connect failed for client {}: {}",
                                worker_id, client_idx, e
                            );
                            unsafe {
                                libc::close(fd);
                            }
                            stats.record_error(ErrorKind::Connect);
                            stats.clients_done.fetch_add(1, Ordering::Relaxed);
                            done_count += 1;
                            connect_budget -= 1.0;
                            continue;
                        }

                        match QuicConnection::new(fd, client_idx, &config, config.addr) {
                            Ok(mut conn) => {
                                let crypto_clone = (*crypto).clone();
                                match conn.initiate_quic_connect(crypto_clone, &server_name, now) {
                                    Ok(()) => {
                                        let key = conns.insert(Box::new(conn));
                                        // Submit initial handshake datagrams + recv
                                        submit_front_send(&mut ring, key, &mut conns[key]);
                                        submit_recv(&mut ring, key, &mut conns[key]);
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "worker {}: QUIC connect failed for client {}: {}",
                                            worker_id, client_idx, e
                                        );
                                        unsafe {
                                            libc::close(fd);
                                        }
                                        stats.record_error(ErrorKind::Connect);
                                        stats.clients_done.fetch_add(1, Ordering::Relaxed);
                                        done_count += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "worker {}: QuicConnection::new failed for client {}: {}",
                                    worker_id, client_idx, e
                                );
                                unsafe {
                                    libc::close(fd);
                                }
                                stats.record_error(ErrorKind::Connect);
                                stats.clients_done.fetch_add(1, Ordering::Relaxed);
                                done_count += 1;
                            }
                        }
                        connect_budget -= 1.0;
                    }
                    Err(e) => {
                        eprintln!("worker {}: UDP socket creation failed: {}", worker_id, e);
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

        // Wait up to 10ms for completions (QUIC needs frequent ticking)
        let ts = Timespec::new().nsec(10_000_000); // 10ms
        let args = SubmitArgs::new().timespec(&ts);
        let wait_count = if conns.is_empty() { 0 } else { 1 };
        match ring.submitter().submit_with_args(wait_count, &args) {
            Ok(_) => {}
            Err(ref e) if e.raw_os_error() == Some(libc::EINTR) => continue,
            Err(ref e) if e.raw_os_error() == Some(libc::EBUSY) => continue,
            Err(ref e) if e.raw_os_error() == Some(libc::ETIME) => {} // timeout, continue
            Err(e) => {
                eprintln!("worker {}: io_uring submit error: {}", worker_id, e);
                break;
            }
        }

        // Process completions
        let now = Instant::now();
        let cqes: Vec<_> = ring.completion().collect();
        for cqe in cqes {
            let (conn_key, op) = decode_user_data(cqe.user_data());
            let result = cqe.result();

            if !conns.contains(conn_key) {
                continue;
            }

            match op {
                OP_SEND => {
                    handle_send_complete(
                        &mut conns,
                        conn_key,
                        result,
                        &mut ring,
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

        // Drive QUIC + MQTT state machines. The io_uring wait timeout (10ms)
        // provides the cadence; quinn-proto's loss detection and pacing need
        // ticks at roughly that rate regardless of CQE arrivals.
        {
            let keys: Vec<usize> = conns
                .iter()
                .filter(|(_, c)| c.state != QuicConnState::Done && c.state != QuicConnState::Failed)
                .map(|(k, _)| k)
                .collect();
            for key in keys {
                let conn = &mut conns[key];

                // Drive QUIC state machine
                let events = conn.handle_tick(now);
                if !events.is_empty() {
                    let outcome = conn.process_events(events);
                    if outcome.connected {
                        stats.clients_connected.fetch_add(1, Ordering::Relaxed);
                    }
                    if outcome.acked > 0 {
                        stats
                            .messages_acked
                            .fetch_add(outcome.acked, Ordering::Relaxed);
                    }
                    for _ in 0..outcome.errors {
                        stats.record_error(ErrorKind::Mqtt);
                    }
                }

                // Try publish
                if conn.state == QuicConnState::Publishing {
                    if conn.try_publish(&config, now) {
                        stats.messages_sent.fetch_add(1, Ordering::Relaxed);
                    }
                    conn.check_publish_complete();
                }

                // Check drain
                if conn.state == QuicConnState::Draining {
                    conn.check_drain_complete(now);
                }

                // Check if disconnecting is done (no more sends pending)
                if conn.state == QuicConnState::Disconnecting
                    && !conn.has_pending_send()
                    && !conn.send_pending
                {
                    conn.state = QuicConnState::Done;
                    stats.clients_done.fetch_add(1, Ordering::Relaxed);
                    done_count += 1;
                }

                // Submit any new outgoing datagrams
                if conn.has_pending_send() && !conn.send_pending {
                    submit_front_send(&mut ring, key, conn);
                }
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

    WorkerResult {
        latency_samples: all_samples,
    }
}

fn create_udp_socket(
    config: &BenchConfig,
    bind_addr: Option<std::net::IpAddr>,
) -> std::io::Result<RawFd> {
    let domain = if config.addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, None)?;
    socket.set_nonblocking(true)?;
    socket.set_recv_buffer_size(config.socket_buf)?;
    socket.set_send_buffer_size(config.socket_buf)?;

    let bind_sa = match bind_addr {
        Some(ip) => SockAddr::from(std::net::SocketAddr::new(ip, 0)),
        None => {
            // Bind to 0.0.0.0:0 so the OS assigns an ephemeral port
            if config.addr.is_ipv4() {
                SockAddr::from(std::net::SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
                    0,
                ))
            } else {
                SockAddr::from(std::net::SocketAddr::new(
                    std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED),
                    0,
                ))
            }
        }
    };
    socket.bind(&bind_sa)?;

    Ok(socket.into_raw_fd())
}

/// Connect a UDP socket to the given address using libc::connect.
/// This sets the default peer so Send/Recv work like on a connected socket.
fn udp_connect(fd: RawFd, addr: &std::net::SocketAddr) -> i32 {
    let sa = SockAddr::from(*addr);
    unsafe { libc::connect(fd, sa.as_ptr(), sa.len() as libc::socklen_t) }
}

fn handle_send_complete(
    conns: &mut Slab<Box<QuicConnection>>,
    key: usize,
    result: i32,
    ring: &mut IoUring,
    stats: &BenchStats,
    done_count: &mut usize,
) {
    let conn = &mut conns[key];
    conn.send_pending = false;

    if result < 0 {
        let err_code = -result;
        eprintln!(
            "connection {}: UDP send failed: {}",
            conn.client_index,
            std::io::Error::from_raw_os_error(err_code)
        );
        mark_failed(conn, stats, done_count, ErrorKind::Send);
        return;
    }

    // Datagram sent successfully; pop it from the queue
    conn.pop_front_datagram();

    // If more datagrams in queue, submit next send
    if conn.has_pending_send() {
        submit_front_send(ring, key, conn);
    }
}

fn handle_recv_complete(
    conns: &mut Slab<Box<QuicConnection>>,
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
            // ECONNREFUSED is common for UDP when the remote port is closed
            if err_code != libc::ECONNREFUSED {
                eprintln!(
                    "connection {}: UDP recv error: {}",
                    conn.client_index,
                    std::io::Error::from_raw_os_error(err_code)
                );
            }
        }
        if conn.state != QuicConnState::Done && conn.state != QuicConnState::Disconnecting {
            mark_failed(conn, stats, done_count, ErrorKind::Receive);
        }
        return;
    }

    let nbytes = result as usize;
    let now = Instant::now();

    // Feed datagram to QUIC engine
    let data: Vec<u8> = conn.recv_buf[..nbytes].to_vec();
    let events = conn.handle_datagram(&data, now);

    if !events.is_empty() {
        let outcome = conn.process_events(events);
        if outcome.connected {
            stats.clients_connected.fetch_add(1, Ordering::Relaxed);
        }
        if outcome.acked > 0 {
            stats
                .messages_acked
                .fetch_add(outcome.acked, Ordering::Relaxed);
        }
        for _ in 0..outcome.errors {
            stats.record_error(ErrorKind::Mqtt);
        }
    }

    // Try to publish after receiving (may have transitioned to Publishing)
    if conn.state == QuicConnState::Publishing {
        if conn.try_publish(config, now) {
            stats.messages_sent.fetch_add(1, Ordering::Relaxed);
        }
        conn.check_publish_complete();
    }

    if conn.state == QuicConnState::Draining {
        conn.check_drain_complete(now);
    }

    // Submit outgoing datagrams
    if conn.has_pending_send() && !conn.send_pending {
        submit_front_send(ring, key, conn);
    }

    // Continue receiving (unless done/failed)
    if conn.state != QuicConnState::Done && conn.state != QuicConnState::Failed {
        submit_recv(ring, key, conn);
    }
}

fn mark_failed(
    conn: &mut QuicConnection,
    stats: &BenchStats,
    done_count: &mut usize,
    error_kind: ErrorKind,
) {
    if conn.state != QuicConnState::Failed && conn.state != QuicConnState::Done {
        conn.state = QuicConnState::Failed;
        stats.record_error(error_kind);
        stats.clients_done.fetch_add(1, Ordering::Relaxed);
        *done_count += 1;
    }
}

fn submit_front_send(ring: &mut IoUring, key: usize, conn: &mut QuicConnection) {
    if conn.send_pending {
        return;
    }
    let datagram = match conn.front_send_datagram() {
        Some(d) => d,
        None => return,
    };
    let ptr = datagram.as_ptr();
    let len = datagram.len() as u32;

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

fn submit_recv(ring: &mut IoUring, key: usize, conn: &mut QuicConnection) {
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
