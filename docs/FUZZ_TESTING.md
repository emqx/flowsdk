# Fuzz Testing with ClusterFuzzLite

This document describes the fuzz testing setup for flowSDK using [ClusterFuzzLite](https://google.github.io/clusterfuzzlite/).

## Overview

FlowSDK uses continuous fuzz testing to discover bugs, security vulnerabilities, and edge cases in the MQTT protocol implementation. Fuzzing is automated through GitHub Actions and runs on:

- **Pull Requests**: Quick fuzzing to catch issues early (5 minutes per target)
- **Main Branch**: Batch fuzzing on every push (10 minutes per target)
- **Daily Schedule**: Long continuous fuzzing runs (1 hour per target)

## Fuzz Targets

### fuzz_parser_funs
Tests individual MQTT packet parsing functions:
- `parse_remaining_length`: Variable byte integer parsing
- `parse_utf8_string`: UTF-8 string validation
- `parse_topic_name`: Topic name parsing
- `parse_packet_id`: Packet identifier parsing
- `parse_vbi`: Variable byte integer parsing
- `parse_binary_data`: Binary data parsing
- `packet_type`: Packet type detection

**Location**: `fuzz/fuzz_targets/fuzz_parser_funs.rs`

### fuzz_mqtt_packet_symmetric
Tests the symmetry of MQTT packet serialization/deserialization:
- Generates random valid MQTT packets
- Serializes them to bytes
- Deserializes back to packets
- Verifies the result matches the original

This ensures that every packet that can be created can be correctly serialized and deserialized.

**Location**: `fuzz/fuzz_targets/fuzz_mqtt_packet_symmetric.rs`

## Running Fuzz Tests Locally

### Prerequisites

1. Install Rust nightly toolchain:
```bash
rustup install nightly
rustup default nightly
```

2. Install cargo-fuzz:
```bash
cargo install cargo-fuzz
```

### Basic Fuzzing

```bash
# Navigate to fuzz directory
cd fuzz

# Run a fuzz target for 60 seconds
cargo fuzz run fuzz_parser_funs -- -max_total_time=60

# Run with more iterations
cargo fuzz run fuzz_mqtt_packet_symmetric -- -runs=1000000

# Run until a crash is found
cargo fuzz run fuzz_parser_funs
```

### Coverage Analysis

```bash
# Generate coverage report
cargo fuzz coverage fuzz_parser_funs

# View the coverage report
cargo cov -- show target/x86_64-unknown-linux-gnu/coverage/x86_64-unknown-linux-gnu/release/fuzz_parser_funs \
    --format=html -instr-profile=fuzz/coverage/fuzz_parser_funs/coverage.profdata \
    > coverage.html
```

### Corpus Management

The corpus (collection of interesting test inputs) is automatically managed by ClusterFuzzLite, but you can also work with it locally:

```bash
# Run with a specific corpus directory
cargo fuzz run fuzz_parser_funs fuzz/corpus/fuzz_parser_funs

# Minimize the corpus
cargo fuzz cmin fuzz_parser_funs

# Merge new findings into corpus
cargo fuzz run fuzz_parser_funs fuzz/corpus/fuzz_parser_funs -- -merge=1
```

## GitHub Actions Integration

### Workflow File

The ClusterFuzzLite integration is configured in `.github/workflows/clusterfuzzlite.yml`.

### Jobs

1. **PR Fuzzing** (`pr-fuzzing`)
   - Triggers on pull requests
   - Runs with AddressSanitizer
   - 5 minutes per fuzz target
   - Reports findings as PR comments

2. **Batch Fuzzing** (`batch-fuzzing`)
   - Triggers on pushes to main
   - Runs with AddressSanitizer and UndefinedBehaviorSanitizer
   - 10 minutes per fuzz target

3. **Continuous Fuzzing** (`continuous-fuzzing`)
   - Triggers daily at 2 AM UTC or manual dispatch
   - Runs with AddressSanitizer, UndefinedBehaviorSanitizer, and MemorySanitizer
   - 1 hour per fuzz target

4. **Corpus Pruning** (`prune`)
   - Triggers daily on schedule
   - Minimizes corpus to reduce redundancy
   - 10 minutes per target

5. **Coverage Report** (`coverage`)
   - Triggers on pushes to main
   - Generates code coverage from fuzzing
   - 10 minutes per target

### Sanitizers

- **AddressSanitizer (ASan)**: Detects memory errors like buffer overflows, use-after-free
- **UndefinedBehaviorSanitizer (UBSan)**: Detects undefined behavior in C/C++ code (Rust FFI)
- **MemorySanitizer (MSan)**: Detects use of uninitialized memory

## Configuration Files

### `.clusterfuzzlite/Dockerfile`
Defines the build environment for fuzz targets.

### `.clusterfuzzlite/build.sh`
Build script that compiles fuzz targets and prepares the corpus.

### `.clusterfuzzlite/project.yaml`
Project metadata and configuration.

## Interpreting Results

### Crashes Found

When a crash is discovered:

1. **GitHub Security Alert**: A SARIF file is uploaded and appears in the Security tab
2. **Issue Creation**: ClusterFuzzLite may create an issue with crash details
3. **Reproducible Test Case**: The crashing input is saved

### Reproducing Crashes Locally

If a crash is found, download the crash input and reproduce:

```bash
# Download the crash file from the issue/alert
wget <crash-file-url> -O crash-input

# Reproduce the crash
cargo fuzz run fuzz_parser_funs crash-input
```

### Debugging

```bash
# Run with debugging symbols
cargo fuzz run fuzz_parser_funs crash-input --dev

# Use with debugger
rust-lldb target/x86_64-unknown-linux-gnu/debug/fuzz_parser_funs crash-input
```

## Adding New Fuzz Targets

1. Create a new file in `fuzz/fuzz_targets/`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Your fuzzing logic here
    your_function(data);
});
```

2. Add the target to `fuzz/Cargo.toml`:

```toml
[[bin]]
name = "fuzz_new_target"
path = "fuzz_targets/fuzz_new_target.rs"
test = false
doc = false
bench = false
```

3. Test locally:
```bash
cd fuzz
cargo fuzz run fuzz_new_target -- -max_total_time=60
```

4. The new target will be automatically picked up by ClusterFuzzLite on the next run.

## Best Practices

1. **Keep Targets Focused**: Each fuzz target should test a specific component or functionality
2. **Use Structured Fuzzing**: Use `arbitrary` crate for structured input generation when testing complex data structures
3. **Fast Feedback**: Ensure fuzz targets execute quickly (< 1ms per iteration) for maximum coverage
4. **Meaningful Assertions**: Add assertions that catch logic errors, not just crashes
5. **Monitor Corpus Growth**: A growing corpus indicates the fuzzer is finding new code paths

## Performance Tips

- **Corpus Seeds**: Add good initial inputs to `fuzz/corpus/<target_name>/` to improve coverage
- **Dictionary**: Create a dictionary file with common tokens to guide fuzzing
- **Parallel Fuzzing**: Use `-fork=N` to run multiple fuzzing processes in parallel

## Resources

- [ClusterFuzzLite Documentation](https://google.github.io/clusterfuzzlite/)
- [cargo-fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz.html)
- [libFuzzer Documentation](https://llvm.org/docs/LibFuzzer.html)
- [OSS-Fuzz](https://github.com/google/oss-fuzz)

## Troubleshooting

### Build Failures

If the fuzz build fails in CI:
1. Check the Dockerfile is correctly configured
2. Verify build.sh has execution permissions
3. Ensure all dependencies are available in the base image

### No Crashes Found

This is good! It means the code is robust. Consider:
1. Adding more diverse seed inputs
2. Creating additional fuzz targets for untested code paths
3. Increasing fuzz time for deeper exploration

### Slow Fuzzing

If fuzzing is too slow:
1. Profile the fuzz target to find bottlenecks
2. Reduce work per iteration
3. Avoid I/O operations in fuzz targets
4. Use simpler data structures for intermediates
