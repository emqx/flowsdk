#!/bin/bash -eu

# Navigate to project root
cd $SRC/flowsdk

# Build all fuzz targets using cargo-fuzz
cd fuzz

# List all fuzz targets
targets=$(ls fuzz_targets/*.rs | xargs -n 1 basename | sed 's/\.rs$//')

# Build each fuzz target
for target in $targets; do
    echo "Building fuzz target: $target"
    
    # Build the fuzz target
    cargo fuzz build --release "$target"
    
    # Copy the built binary to $OUT
    if [ -f "target/x86_64-unknown-linux-gnu/release/$target" ]; then
        cp "target/x86_64-unknown-linux-gnu/release/$target" "$OUT/"
        echo "Copied $target to $OUT/"
    else
        echo "Warning: Binary not found for $target"
    fi
done

# Return to project root
cd ..

# Create seed corpus directories if they don't exist
for target in $targets; do
    mkdir -p "$OUT/${target}_seed_corpus"
    
    # If there are existing corpus files, copy them
    if [ -d "fuzz/corpus/$target" ]; then
        cp -r "fuzz/corpus/$target"/* "$OUT/${target}_seed_corpus/" 2>/dev/null || true
        echo "Copied corpus for $target"
    fi
done

echo "Build completed successfully"
