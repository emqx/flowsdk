#!/bin/bash
set -e

# Persistence is opt-in; generated bindings must match the native library.
PROFILE="debug"
CARGO_PROFILE="dev"
TARGET_DIR="target/debug"
FEATURES="uniffi-bindings"
DURABLE_SESSION=false
TEST=false
COVERAGE=false
XCFRAMEWORK=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE="release"; CARGO_PROFILE="release"; TARGET_DIR="target/release" ;;
        --durable-session) FEATURES="$FEATURES,durable-session"; DURABLE_SESSION=true ;;
        --test) TEST=true ;;
        --coverage) COVERAGE=true ;;
        --xcframework) XCFRAMEWORK=true ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Temporary generation dir; final files are distributed below
SWIFT_GEN_DIR="swift/Generated"
# SPM target paths (must stay in sync with Package.swift)
SWIFT_C_TARGET="swift/Sources/flowsdk_ffi"
SWIFT_TARGET="swift/Sources/FlowSDK"
SWIFT_LIB_DIR="swift/lib"

echo "Building flowsdk_ffi ($PROFILE)..."
if [[ "$COVERAGE" == true ]]; then
    mkdir -p target/llvm-cov-target
    export RUSTFLAGS="-C instrument-coverage"
    export LLVM_PROFILE_FILE="$PWD/target/llvm-cov-target/swift-%p-%m.profraw"
fi
cargo build -p flowsdk_ffi --profile "$CARGO_PROFILE" --features "$FEATURES"

echo "Generating Swift bindings..."
mkdir -p "$SWIFT_GEN_DIR"
cargo run -p flowsdk_ffi --features "$FEATURES" --bin uniffi-bindgen generate \
    --library "$TARGET_DIR/libflowsdk_ffi.dylib" \
    --language swift \
    --out-dir "$SWIFT_GEN_DIR"

echo "Distributing generated files to SPM target directories..."
# C module target: only the header (SPM auto-generates the module map)
mkdir -p "$SWIFT_C_TARGET"
cp "$SWIFT_GEN_DIR/flowsdk_ffiFFI.h" "$SWIFT_C_TARGET/flowsdk_ffi.h"
# Swift target: the generated Swift bindings
mkdir -p "$SWIFT_TARGET"
cp "$SWIFT_GEN_DIR/flowsdk_ffi.swift" "$SWIFT_TARGET/"
# Normalize the SPM module name so consumers import `flowsdk_ffi` instead of `flowsdk_ffiFFI`.
perl -0pi -e 's/canImport\(flowsdk_ffiFFI\)/canImport(flowsdk_ffi)/g; s/import flowsdk_ffiFFI/import flowsdk_ffi/g' "$SWIFT_TARGET/flowsdk_ffi.swift"
# Clean up temp dir
rm -rf "$SWIFT_GEN_DIR"

echo "Copying library for Swift package..."
mkdir -p "$SWIFT_LIB_DIR"
cp "$TARGET_DIR/libflowsdk_ffi.dylib" "$SWIFT_LIB_DIR/"

if [[ "$XCFRAMEWORK" == true ]]; then
    echo "Building multi-arch XCFramework..."

    # Ensure required targets are installed
    rustup target add aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios 2>/dev/null || true

    # Build macOS slices
    echo "  Building aarch64-apple-darwin..."
    cargo build -p flowsdk_ffi --profile "$CARGO_PROFILE" --features "$FEATURES" --target aarch64-apple-darwin
    echo "  Building x86_64-apple-darwin..."
    cargo build -p flowsdk_ffi --profile "$CARGO_PROFILE" --features "$FEATURES" --target x86_64-apple-darwin

    # Build iOS device
    echo "  Building aarch64-apple-ios..."
    cargo build -p flowsdk_ffi --profile "$CARGO_PROFILE" --features "$FEATURES" --target aarch64-apple-ios

    # Build iOS simulator (universal: arm64 sim + x86_64 sim)
    echo "  Building aarch64-apple-ios-sim..."
    cargo build -p flowsdk_ffi --profile "$CARGO_PROFILE" --features "$FEATURES" --target aarch64-apple-ios-sim
    echo "  Building x86_64-apple-ios (simulator)..."
    cargo build -p flowsdk_ffi --profile "$CARGO_PROFILE" --features "$FEATURES" --target x86_64-apple-ios

    MACOS_ARM="target/aarch64-apple-darwin/$PROFILE/libflowsdk_ffi.a"
    MACOS_X86="target/x86_64-apple-darwin/$PROFILE/libflowsdk_ffi.a"
    IOS_ARM="target/aarch64-apple-ios/$PROFILE/libflowsdk_ffi.a"
    IOS_SIM_ARM="target/aarch64-apple-ios-sim/$PROFILE/libflowsdk_ffi.a"
    IOS_SIM_X86="target/x86_64-apple-ios/$PROFILE/libflowsdk_ffi.a"

    mkdir -p swift/xcframework-intermediates

    # Universal macOS .a
    echo "  Lipo-ing macOS slices..."
    lipo -create "$MACOS_ARM" "$MACOS_X86" \
        -output swift/xcframework-intermediates/libflowsdk_ffi-macos.a

    # Universal iOS simulator .a
    echo "  Lipo-ing iOS simulator slices..."
    lipo -create "$IOS_SIM_ARM" "$IOS_SIM_X86" \
        -output swift/xcframework-intermediates/libflowsdk_ffi-ios-sim.a

    echo "  Creating XCFramework..."
    rm -rf swift/FlowSDK.xcframework
    xcodebuild -create-xcframework \
        -library swift/xcframework-intermediates/libflowsdk_ffi-macos.a \
        -headers "$SWIFT_C_TARGET" \
        -library "$IOS_ARM" \
        -headers "$SWIFT_C_TARGET" \
        -library swift/xcframework-intermediates/libflowsdk_ffi-ios-sim.a \
        -headers "$SWIFT_C_TARGET" \
        -output swift/FlowSDK.xcframework

    echo "  XCFramework created at swift/FlowSDK.xcframework"
fi

if [[ "$TEST" == true ]]; then
    echo "Running Swift build verification..."
    SWIFT_FLAGS=()
    if [[ "$DURABLE_SESSION" == true ]]; then
        SWIFT_FLAGS=(-Xswiftc -DFLOWSDK_DURABLE_SESSION)
    fi
    LIBRARY_PATH="$PWD/$SWIFT_LIB_DIR" swift build --package-path swift "${SWIFT_FLAGS[@]}"
    LIBRARY_PATH="$PWD/$SWIFT_LIB_DIR" DYLD_LIBRARY_PATH="$PWD/$SWIFT_LIB_DIR" swift run --package-path swift "${SWIFT_FLAGS[@]}" FlowSDKTests
fi

if [[ "$COVERAGE" == true ]]; then
    echo "Running TcpClientExample to collect coverage data..."
    LIBRARY_PATH="$PWD/$SWIFT_LIB_DIR" timeout 5s swift run --package-path swift TcpClientExample broker.emqx.io 1883 || true
fi

echo "Done!"
