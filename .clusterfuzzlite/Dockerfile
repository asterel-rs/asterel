FROM gcr.io/oss-fuzz-base/base-builder-rust
RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*
COPY . $SRC/asterel
WORKDIR $SRC/asterel
COPY .clusterfuzzlite/build.sh $SRC/
