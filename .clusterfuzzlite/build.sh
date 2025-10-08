#!/bin/bash -eu

# Navigate to project root
cd $SRC/flowsdk

# Build all fuzz targets
cd fuzz

# Install cargo-fuzz if not present
cargo install cargo-fuzz --force

# Build each fuzz target
for target in fuzz_targets/*.rs; do
    target_name=$(basename "$target" .rs)
    echo "Building fuzz target: $target_name"
    cargo fuzz build "$target_name"
    
    # Copy the built binary to $OUT
    cp target/x86_64-unknown-linux-gnu/release/$target_name $OUT/
done

# Return to project root
cd ..

# Create seed corpus directories if they don't exist
mkdir -p $OUT/fuzz_parser_funs_seed_corpus
mkdir -p $OUT/fuzz_mqtt_packet_symmetric_seed_corpus

# If there are existing corpus files, copy them
if [ -d "fuzz/corpus/fuzz_parser_funs" ]; then
    cp -r fuzz/corpus/fuzz_parser_funs/* $OUT/fuzz_parser_funs_seed_corpus/ || true
fi

if [ -d "fuzz/corpus/fuzz_mqtt_packet_symmetric" ]; then
    cp -r fuzz/corpus/fuzz_mqtt_packet_symmetric/* $OUT/fuzz_mqtt_packet_symmetric_seed_corpus/ || true
fi
