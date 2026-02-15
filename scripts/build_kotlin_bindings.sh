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

echo "Generating Kotlin bindings..."
# Create package directory if it doesn't exist
mkdir -p kotlin/package/src/main/kotlin

# Output direct to kotlin/package/src/main/kotlin
cargo run -p flowsdk_ffi --features=uniffi/cli --bin uniffi-bindgen generate \
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
./gradlew build

echo "Done!"
