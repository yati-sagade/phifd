from rust

COPY ["src/", "/work/src"]
COPY ["Cargo.toml", "build.rs", "/work/"]
COPY ["proto/", "/work/proto"]
WORKDIR "/work"
RUN apt-get -y update && apt-get install -y protobuf-compiler
ENV RUST_BACKTRACE=1
RUN cargo build --release
RUN cp /work/target/release/phifd /
ENTRYPOINT ["/phifd"]
