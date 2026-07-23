# mqtt_ring_bench

High-connection-count MQTT publish/subscribe benchmark using `io_uring` and FlowSDK's sans-I/O client.

Designed for **100K+ concurrent connections** with minimal memory footprint. Traditional MQTT bench tools allocate ~256KB of kernel socket buffers per connection (25GB+ at 100K). This tool cuts that to **~8KB per connection** by combining:

- **`NoIoMqttClient`** (sans-I/O) -- MQTT protocol handling decoupled from I/O
- **`io_uring`** -- kernel-bypass async I/O with no per-syscall overhead
- **Tiny socket buffers** -- `SO_RCVBUF`/`SO_SNDBUF` set to 2KB (configurable)
- **Small parser buffers** -- 1.5KB (1 MTU) instead of the default 16KB

## Memory Budget

### TCP mode

| Component | Per-Connection | 100K Total |
|---|---|---|
| NoIoMqttClient (parser + engine) | ~3.5 KB | 350 MB |
| Kernel socket buffers | 4 KB | 400 MB |
| Connection state | ~0.2 KB | 20 MB |
| **Total** | **~8 KB** | **~770 MB** |

### QUIC mode

| Component | Per-Connection | 100K Total |
|---|---|---|
| QuicMqttEngine (MQTT + quinn-proto state) | ~10 KB | 1 GB |
| UDP socket kernel overhead | ~0.5 KB | 50 MB |
| Connection struct + send queue | ~1 KB | 100 MB |
| **Total** | **~11.5 KB** | **~1.15 GB** |

QUIC avoids TCP's mandatory send/recv buffers. The main cost is the quinn-proto `Connection` state (~6-8KB for TLS session + congestion control).

## Requirements

- **Linux 5.6+** (io_uring)
- Raise file descriptor limit: `ulimit -n 200000`
- For 100K+ connections to a single broker, use `--ifaddr` with multiple source IPs (each IP supports ~65K ephemeral ports)

## Build

Requires Rust 1.70+. Linux-only (io_uring).

The default Ctrl-C behavior exits immediately. Select `--shutdown-mode graceful`
to make each worker cancel and drain its outstanding socket operations first; a
second Ctrl-C then forces an immediate exit.

```bash
# Native build on Linux (TCP only)
cargo build -p mqtt_ring_bench --release

# With QUIC support (adds rustls + quinn-proto)
cargo build -p mqtt_ring_bench --features quic --release

# Cross-compile from macOS using zig
cargo install cargo-zigbuild
cargo zigbuild -p mqtt_ring_bench --target aarch64-unknown-linux-musl --release
cargo zigbuild -p mqtt_ring_bench --features quic --target aarch64-unknown-linux-musl --release
cargo zigbuild -p mqtt_ring_bench --target x86_64-unknown-linux-musl --release
```

The output is a statically linked binary (~800KB without QUIC, ~2.3MB with QUIC) with no runtime dependencies.

## Usage

```
mqtt_ring_bench [OPTIONS]

OPTIONS:
    --action <pub|sub>        Publish or subscribe benchmark [default: pub]
    --host <HOST>             Broker hostname [default: localhost]
    --port <PORT>             Broker port [default: 1883]
    --clients <N>             Concurrent connections [default: 1000]
    --messages <N>            Publish or receive target per client [default: 1000]
    --qos <0|1|2>             Publish or requested subscription QoS [default: 0]
    --topic <TOPIC>           Base publish topic or subscription filter [default: bench/test]
    --payload-size <BYTES>    Publish payload size [default: 256]
    --interval <MS>           Delay between publishes, 0 = max speed [default: 0]
    --keep-alive <SECS>       MQTT keep-alive [default: 60]
    --mqtt-version <3|4|5>    Protocol version [default: 5]
    --workers <N>             Worker threads [default: num_cpus]
    --connect-rate <N>        Connections/sec during ramp-up [default: 1000]
    --socket-buf <BYTES>      SO_RCVBUF/SO_SNDBUF per socket [default: 2048]
    --parser-buf <BYTES>      MQTT parser buffer per connection [default: 1500]
    --shutdown-mode <MODE>    Ctrl-C behavior: immediate or graceful [default: immediate]
    --ifaddr <ADDRS>          Source IPs to bind (round-robin)
    --quic                    Use MQTT over QUIC (UDP) instead of TCP
    --quic-insecure           Skip TLS certificate verification (testing only)
    --server-name <NAME>      TLS SNI server name [default: --host value]
    -h, --help                Print help
```

`--shutdown-mode immediate` exits on the first Ctrl-C and lets the kernel tear
down pending io_uring operations. Use `--shutdown-mode graceful` to explicitly
cancel and reap pending operations before closing sockets and exiting.

In `--action sub`, `--messages 0` keeps every subscriber active until shutdown.
The subscription filter is literal by default; include `{client}` to replace it
with each client's zero-based index. After SUBACK, QoS 0 deliveries use a
type-only parser. QoS 1/2 parse only the headers needed to send acknowledgements.
Payload bytes and SUBACK latency are not collected.

## Examples

```bash
# Quick test: 100 clients, 100 messages each, QoS 0
mqtt_ring_bench --host broker.emqx.io --clients 100 --messages 100

# QoS 1 with latency measurement
mqtt_ring_bench --host 10.0.0.1 --clients 1000 --messages 500 --qos 1

# 10K subscribers on one shared filter, receiving until Ctrl-C
mqtt_ring_bench --action sub --host 10.0.0.1 --clients 10000 \
    --topic 'bench/events/#' --messages 0 --qos 0

# One filter per client; an external publisher must publish matching messages
mqtt_ring_bench --action sub --host 10.0.0.1 --clients 1000 \
    --topic 'bench/{client}/events' --messages 100 --qos 1

# 100K connections with multiple source IPs
mqtt_ring_bench --host 10.0.0.1 --clients 100000 --messages 10 \
    --ifaddr 192.168.1.100-200 --connect-rate 5000

# Max throughput: large payload, QoS 0, 8 workers
mqtt_ring_bench --host 10.0.0.1 --clients 5000 --messages 100000 \
    --payload-size 1024 --workers 8

# QUIC: connect to EMQX QUIC port (requires --features quic build)
mqtt_ring_bench --quic --quic-insecure --host broker.emqx.io --port 14567 \
    --clients 10 --messages 100 --qos 1

# QUIC: with custom TLS SNI name
mqtt_ring_bench --quic --host 10.0.0.1 --port 14567 \
    --server-name mqtt.example.com --clients 1000 --messages 500
```

### Source IP binding (`--ifaddr`)

A single source IP can only open ~65K connections to the same destination. Use `--ifaddr` to distribute connections across multiple IPs:

```bash
# Single IP
--ifaddr 192.168.1.100

# Comma-separated list
--ifaddr 192.168.1.100,192.168.1.101,192.168.1.102

# Range (expands last octet)
--ifaddr 192.168.1.100-200    # 101 IPs -> up to ~6.5M connections
```

Connections are round-robin distributed across the source addresses.

## Output

### Live stats (printed every second)

```
[  5s] Connected: 5000/100000 | Sent: 0/10000000 | Acked: 0 | PubAckNoMatch: 0 | Errors: 0 (socket:0 connect:0 send:0 recv:0 mqtt:0 [conn:0 pub:0 client:0 disc:0]) | Rate: 0 msg/s
[105s] Connected: 100000/100000 | Sent: 4500000/10000000 | Acked: 4499800 | PubAckNoMatch: 125 | Errors: 3 (socket:0 connect:1 send:1 recv:0 mqtt:1 [conn:0 pub:0 client:0 disc:1]) | Rate: 52000 msg/s
```

### Final summary

```
============================================================
                    MQTT Bench Results
============================================================
  Broker:          10.0.0.1:1883
  MQTT Version:    5
  Clients:         100000
  Workers:         8
  QoS:             1
  Payload Size:    256 bytes
  Messages/Client: 100
  Socket Buffers:  2048 bytes
  Parser Buffer:   1500 bytes
------------------------------------------------------------
  Total Sent:      10000000
  Total Acked:     9999997
  PubAckNoMatch:   125
  Errors:          3
    Socket:        0
    Connect:       1
    Send:          1
    Receive:       0
    MQTT:          1
      CONNECT:     0
      PUBLISH:     0
      Client:      0
      Disconnect:  1
  Duration:        192.34 s
  Throughput:      51991.23 msg/s
------------------------------------------------------------
  Latency (send -> ACK):
    Min:           0.08 ms
    Max:           45.21 ms
    Avg:           1.87 ms
    P50:           1.12 ms
    P95:           5.43 ms
    P99:           12.76 ms
============================================================
```

## Architecture

```
main thread
  ├── parse args, resolve broker address
  ├── (if --quic) build TLS config (shared Arc<rustls::ClientConfig>)
  ├── spawn N worker threads (std::thread, one per core)
  │     ├── Worker 0: io_uring instance + connections [0..K)
  │     ├── Worker 1: io_uring instance + connections [K..2K)
  │     └── ...
  ├── spawn 1 stats reporter thread (prints every 1s)
  └── join all, print final summary
```

No async runtime (no tokio). Each worker runs a synchronous `io_uring` event loop managing thousands of connections.

**TCP mode** (default): Each connection wraps a `NoIoMqttClient` for protocol handling and feeds bytes in/out via io_uring send/recv operations.

**QUIC mode** (`--quic`): Each connection wraps a `QuicMqttEngine` (sans-I/O QUIC+MQTT engine) with its own UDP socket. The UDP socket is "connected" via `libc::connect()` so the same io_uring `Send`/`Recv` opcodes work for both TCP and UDP. The QUIC state machine is driven by 10ms timer ticks (vs 500ms for TCP).

### Connection state machine

**TCP:**
```
TcpConnecting -> MqttConnecting -> Publishing -> Draining -> Disconnecting -> Done
                              \-> Subscribing -> Receiving -> Disconnecting -> Done
```

**QUIC:**
```
Handshaking -> Publishing -> Draining -> Disconnecting -> Done
           \-> Subscribing -> Receiving -> Disconnecting -> Done
```
No separate TCP connect phase — QUIC handshake happens via UDP datagrams. The engine internally handles QUIC handshake + MQTT CONNECT once the QUIC connection is established.

### Latency measurement

- **QoS 1/2**: Measures wall-clock time from `publish()` call to `MqttEvent::Published` (PUBACK/PUBCOMP). FIFO ordering guaranteed by MQTT spec.
- **QoS 0**: Measures enqueue time (no broker ack). Labeled separately in output.

## Kernel Tuning (for 100K+)

```bash
# File descriptor limit
ulimit -n 500000

# Ephemeral port range
sysctl -w net.ipv4.ip_local_port_range="1024 65535"

# Connection backlog
sysctl -w net.core.somaxconn=65535

# TCP memory (optional, reduces per-socket overhead further)
sysctl -w net.ipv4.tcp_mem="8388608 8388608 8388608"
sysctl -w net.ipv4.tcp_rmem="1024 2048 4096"
sysctl -w net.ipv4.tcp_wmem="1024 2048 4096"
```

## License

MPL-2.0
