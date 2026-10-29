# Testing
```bash
# Run all tests in both workspaces
cargo test --workspace

# Library tests with different feature combinations
cargo test --lib                              # Standard tests
cargo test --lib --features protocol-testing  # With raw packet API

# Integration tests
cargo test --test '*'

# Run ignored tests (require live broker)
cargo test -- --ignored

# Or individually
cargo test                             # Main library tests
cd mqtt_grpc_duality && cargo test     # Proxy workspace tests
```

## Feature gates

`.github/workflows/feature-gates.yml` checks packages independently to prevent
workspace feature unification from hiding missing dependencies. It follows
[Quinn's feature CI](https://github.com/quinn-rs/quinn/blob/main/.github/workflows/rust.yml).

Install the same tool version used in CI:

```bash
cargo +stable install cargo-hack --version 0.6.44 --locked
```

Disable incremental compilation for sweeps, as CI does. Otherwise Rust retains a
separate incremental cache for each feature set and can exhaust runner disk space:

```bash
export CARGO_INCREMENTAL=0
```

PRs check combinations of up to three requested public features, plus minimal,
default, and all-feature configurations, on Linux and macOS. Public features
such as `arbitrary` are included; internal optional dependencies hidden
by `dep:` are not exposed as additional features.

```bash
cargo +stable hack check -p flowsdk --lib --features alloc \
  --feature-powerset --optional-deps --depth 3 --no-dev-deps --keep-going
cargo +stable hack check -p flowsdk_ffi --lib \
  --feature-powerset --optional-deps --depth 3 --no-dev-deps --keep-going
```

**Run `--no-dev-deps` sweeps in their own checkout.** cargo-hack temporarily removes
dev dependencies from manifests and restores them afterward. Do not edit manifests
or run test/example jobs in that checkout during the sweep. CI uses separate
checkouts for these jobs. Do not add `--all-targets` to the sweep with dev dependencies
removed; compile examples and tests separately.

Every Friday, Linux runs the full powerset without the depth limit. To reproduce:

```bash
cargo +stable hack check -p flowsdk --lib --features alloc \
  --feature-powerset --optional-deps --no-dev-deps --keep-going
cargo +stable hack check -p flowsdk_ffi --lib \
  --feature-powerset --optional-deps --no-dev-deps --keep-going
```

The workflow also accepts manual dispatch with `exhaustive: true`. Sweeps use two
partitions (`--partition 1/2` and `--partition 2/2`). There are no feature exclusions;
new public features automatically enter the sweep.

The core compile sweep always enables `alloc`, which FlowSDK requires. The contract
check verifies the diagnostic when both `alloc` and `std` are absent. An isolated
`tests/no_std` powerset tests portable operation with strict validation, persistence,
`std`, and portable atomics toggled, without host workspace feature unification.

Runtime jobs compile all targets and run library/integration tests under minimal host (`std`),
strict, default, async-only, individual TLS and QUIC backends, mixed QUIC, persistence,
and combined transport configurations. Ignored public-broker tests stay in the
existing integration workflow; transport checks use local fixtures.

The workspace lockfile is untracked; run `cargo +stable fetch` before commands
using `--locked`. The contract script resolves it automatically.

```bash
cargo +stable check -p flowsdk --locked --no-default-features --features std --all-targets
cargo +stable test -p flowsdk --locked --no-default-features --features std --lib --tests
cargo +stable test -p flowsdk --locked --no-default-features \
  --features strict-protocol-compliance,tls,rustls-tls,quic-openssl,durable-session \
  --lib --tests
cargo +stable hack test -p flowsdk_ffi --locked --lib --feature-powerset --optional-deps --keep-going
cargo +stable hack test --manifest-path tests/no_std/Cargo.toml --locked --feature-powerset --keep-going
python3 scripts/check_feature_contracts.py
```

The contract script compiles external Rust consumers against exact production
artifacts, checks normal dependency graphs, and inspects optional C/UniFFI exports.
Use `--offline` after fetching dependencies. `tests/crypto_provider.rs` runs in a
separate executable without installing a global Rustls provider, so another test's
initialization cannot conceal provider-selection failures.

`quic-openssl` implies `quic` and selects the fork for the async transport;
`quic-proto-openssl` implies `quic-proto` and selects the fork for the protocol engine.
QUIC Rustls configurations explicitly use OpenSSL when `quic-proto-openssl` is
active, and ring otherwise. The `rustls-tls` TCP transport always uses ring,
independently of QUIC backend selection. Both underlying
QUIC dependencies may remain in the build graph because Cargo features are additive.
`protocol-testing` implies `async-client` because its test client uses Tokio TCP.
`arbitrary` implies `std` because the derive macro uses host recursion tracking.
FFI `durable-session` implies `json`. Existing C, Python, Kotlin, Swift, and
`release-small` jobs remain in the main CI workflow.

All matrix jobs must pass for the aggregate **Feature gates** status to succeed.
Repository administrators can select this status in the branch protection/ruleset
for `main`; committing a workflow does not itself change repository settings.
Failed jobs upload their commands, logs, and feature trees as artifacts.

## Fuzz Testing

The project includes comprehensive fuzz testing infrastructure for protocol robustness:

**Fuzz Targets**:
- `fuzz_mqtt_packet_symmetric` - Round-trip encode/decode testing for all MQTT packet types
- `fuzz_parser_funs` - Parser functions with random input validation

**Running Fuzz Tests**:
```bash
# Install cargo-fuzz (one-time setup)
cargo install cargo-fuzz

# Run specific fuzz target
cd fuzz
cargo fuzz run fuzz_mqtt_packet_symmetric

# Run with specific number of iterations
cargo fuzz run fuzz_mqtt_packet_symmetric -- -runs=1000000

# Run all fuzz targets
cargo fuzz run fuzz_parser_funs
```

**Corpus & Coverage**:
- Pre-built corpus in `fuzz/corpus/` for faster fuzzing
- Coverage data in `fuzz/coverage/`
- Artifacts (crashes/hangs) saved in `fuzz/artifacts/`

**Viewing Fuzz Coverage**:
```bash
cd fuzz
cargo fuzz coverage fuzz_mqtt_packet_symmetric
```

The fuzzing infrastructure uses `libfuzzer-sys` and `arbitrary` crate for property-based testing, ensuring protocol parsers handle malformed input gracefully.
