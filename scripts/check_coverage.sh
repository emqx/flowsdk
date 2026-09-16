#!/usr/bin/env bash
# Reproducible Linux coverage gate; all network peers are local test processes.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PYTHON="${PYTHON:-python3}"
command -v mosquitto >/dev/null
"$PYTHON" -m coverage --version
cargo llvm-cov --version

mkdir -p target/coverage
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --all-features --tests --no-report

# Use the same instrumentation for binaries and the library loaded by Python.
(
    export CARGO_TARGET_DIR="$PWD/target/llvm-cov-target"
    eval "$(cargo llvm-cov show-env --export-prefix)"
    cargo build --workspace --all-features --bins
    cargo build -p flowsdk_ffi --all-features --lib
)
export LLVM_PROFILE_FILE="$PWD/target/llvm-cov-target/network-%p-%m.profraw"
export FLOWSDK_COVERAGE_BIN="$PWD/target/llvm-cov-target/debug"
export PYTHONPATH="$PWD/target/coverage/python"

# Isolate the coverage library from the normal Python package and wheel builds.
"$PYTHON" - <<'PY'
from pathlib import Path

root = Path.cwd()
package = root / "target/coverage/python/flowsdk"
package.mkdir(parents=True, exist_ok=True)
sources = list((root / "python/package/flowsdk").glob("*.py"))
sources = [path for path in sources if path.name != "flowsdk_ffi.py"]
sources.append(root / "target/llvm-cov-target/debug/libflowsdk_ffi.so")
for source in sources:
    destination = package / source.name
    destination.unlink(missing_ok=True)
    destination.symlink_to(source)
PY
"$FLOWSDK_COVERAGE_BIN/uniffi-bindgen" generate \
    --library "$FLOWSDK_COVERAGE_BIN/libflowsdk_ffi.so" \
    --language python --out-dir "$PYTHONPATH/flowsdk" --no-format

"$PYTHON" -W error -m coverage run --rcfile=python/.coveragerc \
    -m unittest discover -s python/tests -v
"$PYTHON" -W error -m coverage run --rcfile=python/.coveragerc --append \
    scripts/test_coverage_network.py -v
"$PYTHON" -m coverage xml --rcfile=python/.coveragerc
"$PYTHON" -m coverage report --rcfile=python/.coveragerc | tee target/coverage/python-summary.txt

# Exclude test source only. Every production workspace crate stays in the gate.
cargo llvm-cov report --ignore-filename-regex '(/tests/|_tests\.rs$)' \
    --lcov --output-path target/coverage/rust.lcov --fail-under-lines 80
cargo llvm-cov report --ignore-filename-regex '(/tests/|_tests\.rs$)' \
    | tee target/coverage/rust-summary.txt
