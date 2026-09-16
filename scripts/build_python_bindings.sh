#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Default to debug build
PROFILE="debug"
CARGO_PROFILE="dev"
TARGET_DIR="target/debug"

if [[ "${1:-}" == "--release" ]]; then
    PROFILE="release"
    CARGO_PROFILE="release"
    TARGET_DIR="target/release"
    shift
fi

# Platforms
OS="$(uname -s)"
case "${OS}" in
    Linux*)     LIB_FILE="libflowsdk_ffi.so";;
    Darwin*)    LIB_FILE="libflowsdk_ffi.dylib";;
    CYGWIN*|MINGW*|MSYS*) LIB_FILE="flowsdk_ffi.dll";;
    *)          LIB_FILE="libflowsdk_ffi.so";;
esac

echo "Building flowsdk_ffi ($PROFILE)..."
cargo build -p flowsdk_ffi --profile $CARGO_PROFILE

echo "Generating Python bindings..."
# Output direct to python/package/flowsdk
cargo run -p flowsdk_ffi --features=uniffi/cli --bin uniffi-bindgen generate \
    --library "$TARGET_DIR/$LIB_FILE" \
    --language python \
    --out-dir python/package/flowsdk

echo "Copying library for Python package..."
cp "$TARGET_DIR/$LIB_FILE" python/package/flowsdk/

if [[ "${1:-}" == "--test" ]]; then
    echo "Running Python verification..."
    export PYTHONPATH=$PWD/python/package
    python3 -c "import flowsdk; print('Import successful'); engine = flowsdk.MqttEngineFfi('test', 5); print('Engine created')"
    python3 -W error -m unittest discover -s python/tests -v
fi

echo "Done!"
