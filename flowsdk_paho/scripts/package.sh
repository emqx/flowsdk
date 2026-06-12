#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# package.sh — Build flowsdk_paho and stage it as a Paho C drop-in replacement.
#
# Produces a `dist/` tree containing the headers and the library under the
# canonical Paho names, so existing build systems that link `-lpaho-mqtt3c`
# or `-lpaho-mqtt3a` can use FlowSDK unchanged:
#
#   dist/
#     include/   MQTTClient.h, MQTTAsync.h, MQTTProperties.h
#     lib/       libpaho-mqtt3c.{dylib,so}  libpaho-mqtt3a.{dylib,so}
#                libpaho-mqtt3c.a           libpaho-mqtt3a.a
#
# Both names resolve to the same FlowSDK library, which implements the sync
# (MQTTClient_*) and async (MQTTAsync_*) APIs together.
#
# Usage:
#   scripts/package.sh [--release|--debug] [--out DIR]

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
workspace_dir=$(CDPATH= cd -- "$crate_dir/.." && pwd)

profile=release
out="$crate_dir/dist"
while [ $# -gt 0 ]; do
    case "$1" in
        --release) profile=release ;;
        --debug)   profile=debug ;;
        --out)     shift; out="$1" ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

# Dynamic library extension by platform.
case "$(uname -s)" in
    Darwin) dyn=dylib ;;
    *)      dyn=so ;;
esac

echo "Building flowsdk_paho ($profile)..."
if [ "$profile" = release ]; then
    ( cd "$workspace_dir" && cargo build -p flowsdk_paho --release )
    target_dir="$workspace_dir/target/release"
else
    ( cd "$workspace_dir" && cargo build -p flowsdk_paho )
    target_dir="$workspace_dir/target/debug"
fi

src_dyn="$target_dir/libflowsdk_paho.$dyn"
src_a="$target_dir/libflowsdk_paho.a"
[ -f "$src_dyn" ] || { echo "error: $src_dyn not found" >&2; exit 1; }

echo "Staging into: $out"
rm -rf "$out"
mkdir -p "$out/include" "$out/lib"

cp "$crate_dir/include/MQTTClient.h" "$out/include/"
cp "$crate_dir/include/MQTTAsync.h" "$out/include/"
cp "$crate_dir/include/MQTTProperties.h" "$out/include/"

# Copy the dynamic + static libs under both Paho names.
for name in paho-mqtt3c paho-mqtt3a; do
    cp "$src_dyn" "$out/lib/lib$name.$dyn"
    [ -f "$src_a" ] && cp "$src_a" "$out/lib/lib$name.a"
done

echo ""
echo "Done. To compile a Paho program against FlowSDK:"
echo "  cc -I$out/include your_app.c -L$out/lib -lpaho-mqtt3c -o your_app"
echo "  (use -lpaho-mqtt3a for the asynchronous API)"

# Best-effort symbol audit on the packaged dynamic library.
if [ -x "$script_dir/check_symbols.sh" ]; then
    echo ""
    "$script_dir/check_symbols.sh" "$out/lib/libpaho-mqtt3c.$dyn" || true
fi
