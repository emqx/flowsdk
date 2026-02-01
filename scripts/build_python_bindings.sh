#!/bin/bash
set -e

# Default to debug build
PROFILE="debug"
CARGO_PROFILE="dev"
TARGET_DIR="target/debug"

if [[ "$1" == "--release" ]]; then
    PROFILE="release"
    CARGO_PROFILE="release"
    TARGET_DIR="target/release"
    shift
fi

# Platforms
OS="$(uname -s)"
case "${OS}" in
    Linux*)     EXT="so";;
    Darwin*)    EXT="dylib";;
    CYGWIN*|MINGW*|MSYS*) EXT="dll";; # Windows-ish
    *)          EXT="so";;
esac

echo "Building flowsdk_ffi ($PROFILE)..."
cargo build -p flowsdk_ffi --profile $CARGO_PROFILE

echo "Generating Python bindings..."
# Output direct to python/flowsdk
cargo run -p flowsdk_ffi --features=uniffi/cli --bin uniffi-bindgen generate \
    --library "$TARGET_DIR/libflowsdk_ffi.$EXT" \
    --language python \
    --out-dir python/flowsdk

echo "Copying library for Python package..."
cp "$TARGET_DIR/libflowsdk_ffi.$EXT" python/flowsdk/

if [[ "$1" == "--test" ]]; then
    echo "Running Python verification..."
    export PYTHONPATH=$PWD/python
    # We use a temporary test script that imports from flowsdk
    python3 -c "import flowsdk; print('Import successful'); engine = flowsdk.MqttEngineFfi('test', 5); print('Engine created')"
fi

echo "Done!"
