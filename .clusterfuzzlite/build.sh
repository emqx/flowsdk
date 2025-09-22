#!/bin/bash -eu

# Build the fuzz targets
cd fuzz
cargo fuzz build -O
cp target/x86_64-unknown-linux-gnu/release/fuzz_* $OUT/
