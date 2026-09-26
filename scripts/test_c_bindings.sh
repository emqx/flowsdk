#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
c_flags=()
json=false
while [[ "${1:-}" == --* ]]; do
    case "$1" in
        --json) c_flags+=(-DFLOWSDK_JSON); json=true ;;
        --durable-session) c_flags+=(-DFLOWSDK_DURABLE_SESSION); json=true ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
lib_dir="${1:-target/debug}"
tests=(c_basic)
if [[ "$json" == true ]]; then
    tests+=(c_restart)
fi
for test in "${tests[@]}"; do
    cc ${c_flags[@]+"${c_flags[@]}"} -std=c11 -Wall -Wextra -Werror -I flowsdk_ffi/include "flowsdk_ffi/tests/$test.c" \
        -L "$lib_dir" -lflowsdk_ffi -Wl,-rpath,"$PWD/$lib_dir" -o "$lib_dir/ffi_$test"
    "$lib_dir/ffi_$test"
done
