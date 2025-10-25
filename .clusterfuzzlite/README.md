# ClusterFuzzLite Configuration

This directory contains the configuration files for ClusterFuzzLite continuous fuzzing.

## Files

- **Dockerfile**: Defines the build environment for fuzz targets using the OSS-Fuzz base image for Rust
- **build.sh**: Build script that compiles all fuzz targets and prepares the corpus
- **project.yaml**: Project metadata and sanitizer configuration

## How It Works

1. GitHub Actions triggers the workflow based on events (PR, push, schedule)
2. ClusterFuzzLite uses the Dockerfile to set up the build environment
3. The build.sh script compiles all fuzz targets in the `fuzz/` directory
4. Fuzz targets are run with various sanitizers (address, undefined, memory)
5. Results are uploaded as SARIF files for security analysis

## Local Testing

To test the build locally (requires Docker):

```bash
# Build the Docker image
docker build -t flowsdk-fuzz -f .clusterfuzzlite/Dockerfile .

# Run a test build
docker run --rm -e OUT=/tmp/out -e SRC=/src flowsdk-fuzz
```

## References

- [ClusterFuzzLite Documentation](https://google.github.io/clusterfuzzlite/)
- [OSS-Fuzz Documentation](https://google.github.io/oss-fuzz/)
