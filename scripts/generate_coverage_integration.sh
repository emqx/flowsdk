#!/bin/bash 
set -euo pipefail

cd mqtt_grpc_duality

export PATH="$(rustc --print=target-libdir)/../bin:$PATH"
export PROFDATA_TOOL=$(which llvm-profdata 2>/dev/null || true)

# Clean AFTER setting environment (this is the correct order per llvm-cov docs)
cargo +stable llvm-cov clean --workspace

eval `cargo +stable llvm-cov show-env --export-prefix`
echo "LLVM_PROFILE_FILE=${LLVM_PROFILE_FILE}"
export CARGO_LLVM_COV=1

# Build proxy binaries WITH instrumentation (regular cargo build picks up the env vars)
cargo build --workspace --bins --all-features --all


# Set profile output for integration tests
 ./run_integration_tests.sh --mqtt-ver=both

 cd ../

# Merge and export coverage for proxy binaries
"$PROFDATA_TOOL" merge -sparse target/*.profraw -o target/integration-llvm-cov2.profdata
llvm-cov export -format=lcov --instr-profile target/integration-llvm-cov2.profdata -object target/debug/r-proxy -object target/debug/s-proxy > lcov3.info
