#!/usr/bin/env bash
set -ex

# Ensure we are in the project root
cd "$(dirname "$0")/.."

# Clean previous coverage data
cargo +stable llvm-cov clean --workspace
mkdir -p target/llvm-cov-target

# 1. Collect coverage from Rust tests and examples
cargo +stable llvm-cov --workspace --no-report --tests
cargo +stable llvm-cov --workspace --no-report --examples --all-features
cargo +stable llvm-cov report --lcov --output-path lcov.info

# 2. Collect coverage from C example calling the Rust FFI
# Build instrumented library
cargo +stable llvm-cov clean --workspace
export RUSTFLAGS="-C instrument-coverage"
cargo build -p mqtt_ffi --all-features
make -C examples/c_ffi_example clean
make -C examples/c_ffi_example
# Run C examples with profile file output
export LLVM_PROFILE_FILE="target/llvm-cov-target/ffi-%p-%m.profraw"
LD_LIBRARY_PATH=target/debug/ timeout 5s ./examples/c_ffi_example/out/mqtt_c_example || true
LD_LIBRARY_PATH=target/debug/ timeout 5s ./examples/c_ffi_example/out/quic_c_example broker.emqx.io 14567 || true
LD_LIBRARY_PATH=target/debug/ timeout 5s ./examples/c_ffi_example/out/tls_c_example broker.emqx.io 8883 || true


# 3. Merge all raw profile data (*.profraw) into the unified profdata
# Find the llvm-profdata tool via cargo llvm-cov
PROFDATA_VAR=$(cargo +stable llvm-cov show-env | grep LLVM_PROFDATA) || true
# Extract the path, handling both quoted and unquoted formats
PROFDATA_TOOL=$(echo "$PROFDATA_VAR" | cut -d= -f2- | tr -d '"' | tr -d "'" | xargs)

if [ -z "$PROFDATA_TOOL" ] || [ ! -x "$PROFDATA_TOOL" ]; then
    # Fallback to searching in path
    export PATH="$(rustc --print=target-libdir)/../bin:$PATH"
    PROFDATA_TOOL=$(which llvm-profdata 2>/dev/null || true)
fi

if [ -z "$PROFDATA_TOOL" ] || [ ! -x "$PROFDATA_TOOL" ]; then
    echo "Error: LLVM_PROFDATA tool not found. Make sure llvm-tools-preview is installed."
    echo "Attempted to find via: cargo llvm-cov show-env"
    echo "PROFDATA_VAR was: '$PROFDATA_VAR'"
    exit 1
fi

echo "Using PROFDATA_TOOL: $PROFDATA_TOOL"
# Also find standard rust tests profraws if any were missed by the tool automatically
find . -name "*.profraw" -not -path "./target/llvm-cov-target/*" -exec cp {} target/llvm-cov-target/ \; 2>/dev/null || true

# Check if any profraw files were collected
if [ "$(ls -A target/llvm-cov-target/*.profraw 2>/dev/null)" ]; then
    "$PROFDATA_TOOL" merge -sparse target/llvm-cov-target/*.profraw -o target/llvm-cov-target/cargo-llvm-cov2.profdata
else
    echo "Warning: No .profraw files found to merge."
fi

# Determine library extension based on OS
LIB_EXT="so"
if [[ "$OSTYPE" == "darwin"* ]]; then
    LIB_EXT="dylib"
fi

llvm-cov export -format=lcov --instr-profile target/llvm-cov-target/cargo-llvm-cov2.profdata -object target/debug/deps/libmqtt_ffi.${LIB_EXT} > lcov2.info

# 4. Collect coverage from integration tests (Proxy binaries)
set -a
source <(cargo llvm-cov show-env --export-prefix)
set +a
cargo +stable llvm-cov clean --workspace
cargo build --workspace --bins --all-features --all

# Set profile output for integration tests
export LLVM_PROFILE_FILE="target/llvm-cov-target/integration-%p-%m.profraw"
cd mqtt_grpc_duality && ./run_integration_tests.sh Test.test_basic && cd ..
"$PROFDATA_TOOL" merge -sparse target/llvm-cov-target/*.profraw -o target/llvm-cov-target/integration-llvm-cov2.profdata
llvm-cov export -format=lcov --instr-profile target/llvm-cov-target/integration-llvm-cov2.profdata -object target/debug/r-proxy target/debug/s-proxy > lcov3.info

echo "Coverage report generated at lcov.info, lcov2.info and lcov3.info"
