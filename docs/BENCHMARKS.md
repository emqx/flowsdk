# Microbenchmarks

The `flowsdk` crate uses [Criterion](https://github.com/criterion-rs/criterion.rs)
on stable Rust. The six protocol/payload suites run in memory without a broker,
sockets, an async runtime, or certificates. The `publish_admission` suite uses a
local TCP broker stub and a Tokio runtime to exercise the public async APIs. For
external broker and connection load testing, use the standalone `mqtt_ring_bench`
crate, maintained outside this workspace.

## Run

```bash
# One suite, using default features and Cargo's optimized bench profile
cargo +stable bench -p flowsdk --bench parser

# Direct leveled APIs, without streaming feed/buffer overhead
cargo +stable bench -p flowsdk --bench leveled_parser

# Encoding only, across MQTT versions and packet fixtures (25 cases)
cargo +stable bench -p flowsdk --bench codec -- '^codec/(v3_1_1|v5)/encode/'

# General suites (Criterion's default warm-up, sampling, and measurement settings)
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

HTML reports are in `target/criterion/report/index.html`. These commands use owned
payload/parser buffers by default. Add `--features shared-payloads` to measure
shared storage; in the direct leveled suite, the input clone then shares storage
instead of copying a vector. Keep the feature set the same when comparing revisions.


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
cargo bench -p flowsdk --features shared-payloads --bench shared_payload
# Short local comparison; use the defaults for more stable timing estimates.
cargo bench -p flowsdk --features shared-payloads --bench shared_payload -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2
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
the shared payload implementation (now selected with `shared-payloads`) and
the optimized bench profile. The final run used 30
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
* The no-std host harness passes with owned and shared storage, strict validation
  both on and off, and durable checkpoints. The owned-only library also builds for
  `thumbv6m-none-eabi` without `bytes`, atomic emulation, or a critical-section
  provider. CI checks this target and dependency graph. Both fuzz targets compile
  in owned and shared modes (`--features shared-payloads` selects the latter).
* Two native TLS tests fail on this macOS host with “The extended key usage is not
  valid”: `native_and_rustls_clients_use_custom_ca_and_complete_disconnect` and
  `tls_backends_verify_custom_roots_and_exchange_binary_data`. Both failures were
  reproduced from unchanged commit `dc932414`; they precede this payload change.

## Batched publish admission

```bash
# Verify every case without collecting measurements. No external broker is used.
cargo bench -p flowsdk --no-default-features \
  --features std,async-client,strict-protocol-compliance \
  --bench publish_admission -- --test

# Example comparison: 32 separate MQTT 5 publications with 64-byte payloads.
cargo bench -p flowsdk --no-default-features \
  --features std,async-client,strict-protocol-compliance,shared-payloads \
  --bench publish_admission -- 'publish_admission/v5/payload64/.*/32$'
```

The suite compares sequential single-publication admissions, bounded-concurrent
single admissions, and one batch admission. A synchronous engine path provides an
in-memory reference. Cases cover MQTT 3.1.1 and MQTT 5, QoS 0, payload sizes 64
and 4096 bytes, and groups of 1, 8, and 32 separate publications. The three main
async paths enable the same admission budgets, with limits above the test group
sizes. An additional `legacy_sequential_single` case leaves `publish_admission`
unconfigured to expose overhead from opting into the new request budgets; it is
not a benchmark of an older SDK revision. There are 60 cases in total.

Each Criterion iteration times command submission until all admission results
are returned. Commands and payloads are prepared before the timer starts. A local
TCP broker stub parses and verifies every topic, payload, sequence number, and
flag. After timing, the iteration waits for the verifier to report completion
before starting the next iteration. This barrier catches lost or duplicate
publications and prevents accumulated output from changing later iterations. The
final verification wait and remaining drain time are excluded from the duration.
Each admission group still produces exactly that many MQTT PUBLISH packets; batch
admission never merges payloads.

The async cases use a current-thread Tokio runtime shared by the SDK worker and
broker stub. The timer includes scheduling, result collection, the common timeout
guard, and any transport/verifier work that happens before admission completion.
It is admission wall latency under this scheduling arrangement, not isolated CPU
time, broker acknowledgement latency, maximum throughput, or a VeloFlux result.
Concurrent singles also include `FuturesUnordered` bookkeeping. The direct engine
case excludes channels, runtime scheduling, and socket I/O, so it is a reference
for protocol work rather than an equivalent transport measurement. Criterion's
throughput unit is publications represented by the timed admission group.

The benchmark works with owned or shared payload storage; explicitly select the
same features for comparisons. It does not measure allocation counts, scheduler
wakeups, or retained memory; use an allocator/profiler and the full application
benchmark for those questions. QoS 1/2 and lifecycle correctness are covered by
the admission tests rather than this QoS 0 timing fixture.

### Local admission sample (2026-11-08)

Measured from `feat/batch-publish-admission`, based on upstream main `1a3988d3`,
on macOS 15.7 with `rustc 1.98.0-nightly (14210df0e 2026-05-31)`. These are
release-build Criterion point estimates and 95% confidence intervals for a group
of 32 MQTT 5 QoS 0 publications, each with a 64-byte payload and shared storage.
No other builds or test suites were run concurrently by this task.

```bash
cargo bench -p flowsdk --no-default-features \
  --features std,async-client,strict-protocol-compliance,shared-payloads \
  --bench publish_admission --locked --offline -- \
  'publish_admission/v5/payload64/.*/32$' \
  --sample-size 30 --warm-up-time 1 --measurement-time 3 --noplot
```

| Admission path | Time per group of 32 | 95% confidence interval |
| --- | ---: | ---: |
| Sequential singles, budgets disabled | 258.06 us | 248.24–263.06 us |
| Sequential singles, budgets enabled | 277.05 us | 276.21–277.82 us |
| Concurrent singles, budgets enabled | 81.626 us | 81.479–81.801 us |
| One batch, budgets enabled | 17.700 us | 17.657–17.749 us |
| Direct engine reference | 7.5857 us | 7.0155–8.4384 us |

Batch admission took about 1/15.7 of the budgeted sequential admission time and
1/4.6 of the budgeted concurrent admission time in this fixture. Enabling budgets
for sequential singles added about 7.4% versus unbudgeted singles in this run;
limits remain opt-in. These comparisons are within this SDK revision, not a
before/after-main regression benchmark. Each group was verified to produce all
32 distinct MQTT PUBLISH packets. The scheduling and measurement limits above
apply: these figures do not establish VeloFlux throughput or CPU savings.
