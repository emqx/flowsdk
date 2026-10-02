# Microbenchmarks

The `flowsdk` crate uses [Criterion](https://github.com/criterion-rs/criterion.rs)
on stable Rust. All five suites run in memory without a broker, sockets, an async
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
  --no-default-features --features strict-protocol-compliance -- --test
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
