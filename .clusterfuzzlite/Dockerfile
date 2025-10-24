# Build stage for fuzz targets
FROM gcr.io/oss-fuzz-base/base-builder-rust

# Install libssl-dev
RUN apt-get update && apt-get install -y libssl-dev pkg-config

# Copy the project files
COPY . $SRC/flowsdk

# Set working directory
WORKDIR $SRC/flowsdk

# Build script for fuzz targets
COPY .clusterfuzzlite/build.sh $SRC/

# Make build script executable
RUN chmod +x $SRC/build.sh
