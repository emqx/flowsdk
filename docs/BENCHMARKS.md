# Microbenchmarks

The `flowsdk` crate uses [Criterion](https://github.com/criterion-rs/criterion.rs)
on stable Rust. All six suites run in memory without a broker, sockets, an async
runtime, or certificates. For broker and connection load testing, see
[`mqtt_ring_bench`](../mqtt_ring_bench/README.md).

## Run

```bash
# One suite, using default features and Cargo's optimized bench profile
cargo +stable bench -p flowsdk --bench parser

# Direct leveled APIs, without streaming feed/buffer overhead
cargo +stable bench -p flowsdk --bench leveled_parser

# Encoding only, across MQTT versions and packet fixtures (25 cases)
cargo +stable bench -p flowsdk --bench codec -- '^codec/(v3_1_1|v5)/encode/'

# All suites (Criterion's default warm-up, sampling, and measurement settings)
cargo +stable bench -p flowsdk --bench codec --bench parser --bench leveled_parser --bench queues --bench engine

# Filter to one case; benchmark IDs are regular expressions
cargo +stable bench -p flowsdk --bench parser -- 'parser/v5/full/complete/4096$'

# Compile and smoke-test every case without collecting measurements
cargo +stable bench -p flowsdk --bench codec --bench parser --bench leveled_parser --bench queues --bench engine --no-run
cargo +stable bench -p flowsdk --bench codec --bench parser --bench leveled_parser --bench queues --bench engine -- --test

# The core configuration, with protocol validation and without transport features
cargo +stable bench -p flowsdk --bench codec --bench parser --bench leveled_parser --bench queues --bench engine \
  --no-default-features --features std,strict-protocol-compliance -- --test
```

HTML reports are in `target/criterion/report/index.html`.


## Coverage and units

MQTT 3.1.1 uses wire version `4` and the benchmark label `v3_1_1`; MQTT 5 uses `5`
and `v5`. Inputs are deterministic, with a fixed topic and payload byte `0x5a`.

| Suite            | Cases                                                                                  | Throughput unit                                       |
|------------------|----------------------------------------------------------------------------------------|-------------------------------------------------------|
| `codec`          | Encode/decode CONNECT, SUBSCRIBE, PUBACK/PUBREC/PUBREL/PUBCOMP;                        | Encoded wire bytes                                    |
| `parser`         | All four parse levels; complete packets at the four payload sizes;                     | Total wire bytes fed                                  |
| `leveled_parser` | Direct full/header/raw/type-only parsing of QoS 1 PUBLISH at the four payload sizes.   | Wire bytes represented by each complete input frame   |
| `queues`         | Inflight lookup, insertion, acknowledgement, and expiration at depths 1, 32, and 1,024 | Operations, or entries retransmitted for `expire_all` |
| `engine`         | Send and receive a complete QoS 0/1/2 exchange with 256-byte and 4 KiB payloads        | Completed application messages                        |


## Compare changes

```bash
# Run on the reference revision after the benchmark suite has been added
cargo +stable bench -p flowsdk --bench parser -- --save-baseline before

# Run on the candidate revision, keeping target/criterion from the reference run
cargo +stable bench -p flowsdk --bench parser -- --baseline before
```

## Shared publish payloads

```bash
cargo bench -p flowsdk --bench shared_payload
# Short local comparison; use the defaults for more stable timing estimates.
cargo bench -p flowsdk --bench shared_payload -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2
```

The suite compares 4 KiB vectors, uniquely owned `Bytes`, shared `Bytes`, static
buffers, and shared 4 KiB slices of 64 KiB allocations. The `bytes` path uses
`PublishCommand::from_bytes`. `vec_conversion_baseline` explicitly reinstates the
removed `Bytes -> Vec` conversion before using the current command/packet types;
it isolates that conversion, rather than benchmarking an entire old SDK build.

Before timing, CSV rows report allocations and requested allocated bytes for
command-to-packet preparation, replacement payload bytes copied, and live heap
bytes retained with and without the application handle. Input allocation happens
before preparation counters start. Retained heap includes input backing storage,
the packet's topic, and shared-buffer metadata. It excludes stack/static storage,
wire output, allocator size-class overhead, and RSS. Dropping the packet must
return live heap bytes to the starting value. Replacement copy volume is inferred
from pointer identity on these known retain-or-copy conversion paths, not from
instrumenting arbitrary memcpy calls; wire encoding copies are excluded.

Criterion separately times preparation and a complete MQTT 5 QoS 1 encode/PUBACK
exchange. Input buffers and connected engines are prepared outside timed work.
The exchange includes serialization and completion; it has no transport I/O.
Elapsed time is a CPU-cost proxy, not a hardware cycle count. The counting global
allocator adds atomic-counter overhead to allocations in both timing paths.
Compare paths from the same run. A slice that saves a copy can retain much more
backing memory after the application releases its handle; the CSV makes that
tradeoff visible.

### Local measurements, 2026-11-03

Measured on aarch64 macOS 15.7 with `rustc 1.98.0-nightly (14210df0e 2026-05-31)`,
default crate features and the optimized bench profile. The final run used 30
samples, 0.2 s warm-up and 0.5 s measurement per case, with other build/test jobs
stopped. These are local measurements, not throughput guarantees.

Each cell below is **Bytes path / vector-conversion baseline**. Both paths retain
the same application handle for shared input. The one common preparation
allocation is the 9-byte topic, not a payload allocation.

| Input (4 KiB visible) | Preparation allocations | Requested bytes allocated | Replacement payload bytes copied | Retained heap after application handle drops |
|---|---:|---:|---:|---:|
| Vec | 1 / 1 | 9 / 9 | 0 / 0 | 4,105 / 4,105 |
| Unique Bytes | 1 / 1 | 9 / 9 | 0 / 0 | 4,105 / 4,105 |
| Shared Bytes | 1 / 2 | 9 / 4,105 | 0 / 4,096 | 4,129 / 4,105 |
| Shared slice, 64 KiB backing | 1 / 2 | 9 / 4,105 | 0 / 4,096 | 65,569 / 4,105 |
| Static Bytes | 1 / 2 | 9 / 4,105 | 0 / 4,096 | 9 / 4,105 |

While the application still holds its reference, shared input retains 4,129 /
8,225 heap bytes and sliced input retains 65,569 / 69,665 bytes. All cases return
to their initial live-heap count when both application and packet are dropped.

| Input | Preparation, Bytes (ns) | Preparation, baseline (ns) | Encode/ACK, Bytes (ns) | Encode/ACK, baseline (ns) |
|---|---:|---:|---:|---:|
| Vec | 25.0 | 24.7 | 775 | 766 |
| Unique Bytes | 18.6 | 25.2 | 759 | 771 |
| Shared Bytes | 19.2 | 150.3 | 769 | 860 |
| Shared slice, 64 KiB backing | 22.8 | 170.0 | 1,468 | 1,966 |
| Static Bytes | 17.6 | 118.3 | 1,042 | 996 |

These are Criterion's central estimates. Shared preparation had intervals of
19.04–19.34 ns / 149.27–151.48 ns; its encode/ACK intervals were 765–774 ns /
855–865 ns. The slice and static exchange measurements were noisy with
overlapping intervals (slice: 1,337–1,640 / 1,348–3,056 ns; static: 859–1,274 /
925–1,095 ns). Those exchange measurements do not establish a speedup. The
allocation and replacement-copy reductions are deterministic across both runs.

### Validation accompanying the payload change

* Workspace all-target compilation and Clippy with warnings denied passed with
  `durable-session,arbitrary,quic,rustls-tls` enabled.
* The focused payload tests cover pointer sharing, owner release, queue admission,
  both MQTT versions and every QoS, negative ACKs, quota-blocked replay, checkpoints,
  and shutdown with pending commands. Core wire/lifecycle/reliability suites pass.
* 41 FFI tests and 4 protobuf conversion tests pass; UniFFI records remain vectors.
  54 Rust doctests pass. Language-specific Swift/Kotlin/Python runners were not run.
* The no-std host harness passes with alloc-only SDK builds, strict validation
  both on and off, durable checkpoints, and the `portable-atomics` feature. These
  are host checks, not a new bare-metal target run. Both fuzz targets compile.
* Two native TLS tests fail on this macOS host with “The extended key usage is not
  valid”: `native_and_rustls_clients_use_custom_ca_and_complete_disconnect` and
  `tls_backends_verify_custom_roots_and_exchange_binary_data`. Both failures were
  reproduced from unchanged commit `dc932414`; they precede this payload change.
