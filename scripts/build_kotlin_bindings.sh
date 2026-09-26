#!/bin/bash
set -e

# Persistence is opt-in; generated bindings must match the native library.
PROFILE="debug"
CARGO_PROFILE="dev"
TARGET_DIR="target/debug"
FEATURES="uniffi-bindings"
DURABLE_SESSION=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE="release"; CARGO_PROFILE="release"; TARGET_DIR="target/release" ;;
        --durable-session) FEATURES="$FEATURES,durable-session"; DURABLE_SESSION=true ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

# Platforms
OS="$(uname -s)"
case "${OS}" in
    Linux*)     EXT="so";;
    Darwin*)    EXT="dylib";;
    CYGWIN*|MINGW*|MSYS*) EXT="dll";; # Windows-ish
    *)          EXT="so";;
esac

echo "Building flowsdk_ffi ($PROFILE)..."
cargo build -p flowsdk_ffi --profile $CARGO_PROFILE --features "$FEATURES"

echo "Generating Kotlin bindings..."
# Create package directory if it doesn't exist
mkdir -p kotlin/package/src/main/kotlin

# Output direct to kotlin/package/src/main/kotlin
cargo run -p flowsdk_ffi --features "$FEATURES" --bin uniffi-bindgen generate \
    --library "$TARGET_DIR/libflowsdk_ffi.$EXT" \
    --language kotlin \
    --out-dir kotlin/package/src/main/kotlin

echo "Copying library for Kotlin package..."
# JNA looks for libraries in the resource directory or system paths
mkdir -p kotlin/package/src/main/resources
cp "$TARGET_DIR/libflowsdk_ffi.$EXT" kotlin/package/src/main/resources/

echo "Building Kotlin package..."
cd kotlin
# We now have a multi-module project (package + examples)
./gradlew build -PdurableSession="$DURABLE_SESSION"

echo "Done!"
