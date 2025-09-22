FROM gcr.io/oss-fuzz-base/base-builder-rust

RUN apt-get update && apt-get install -y build-essential

COPY . /src
WORKDIR /src
