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


```