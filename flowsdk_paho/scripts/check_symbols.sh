#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# check_symbols.sh — Verify the built library exports every Paho ABI symbol
# listed in expected_symbols.txt.
#
# Usage:
#   scripts/check_symbols.sh [path/to/libflowsdk_paho.{dylib,so,a}]
#
# With no argument it looks for the debug build under ../../target/debug.
# Exits non-zero if any expected symbol is missing.

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
manifest="$script_dir/expected_symbols.txt"

# Locate the library to inspect.
lib="${1:-}"
if [ -z "$lib" ]; then
    for ext in dylib so a; do
        cand="$script_dir/../../target/debug/libflowsdk_paho.$ext"
        if [ -f "$cand" ]; then lib="$cand"; break; fi
    done
fi
if [ -z "$lib" ] || [ ! -f "$lib" ]; then
    echo "error: library not found; build with 'cargo build -p flowsdk_paho' or pass a path" >&2
    exit 2
fi

echo "Inspecting: $lib"

# Dump exported (defined, global) symbols, normalising the leading underscore
# that Mach-O prepends so the names match the manifest on both macOS and Linux.
dump_symbols() {
    # macOS nm: -g global, -U defined-only. GNU nm: -D dynamic, --defined-only.
    if nm -gU "$lib" >/dev/null 2>&1; then
        nm -gU "$lib" 2>/dev/null
    else
        nm -D --defined-only "$lib" 2>/dev/null || nm -g "$lib" 2>/dev/null
    fi | awk '{print $NF}' | sed 's/^_//'
}

exported=$(dump_symbols | sort -u)

missing=0
checked=0
while IFS= read -r line; do
    case "$line" in
        ''|\#*) continue ;;
    esac
    sym=$(printf '%s' "$line" | tr -d '[:space:]')
    [ -z "$sym" ] && continue
    checked=$((checked + 1))
    if printf '%s\n' "$exported" | grep -qx "$sym"; then
        :
    else
        echo "  MISSING: $sym"
        missing=$((missing + 1))
    fi
done < "$manifest"

echo "Checked $checked expected symbols."
if [ "$missing" -ne 0 ]; then
    echo "FAIL: $missing expected symbol(s) not exported." >&2
    exit 1
fi
echo "OK: all expected Paho symbols are exported."
