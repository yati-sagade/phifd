from rust

COPY ["src/", "/src"]
COPY ["Cargo.toml", "build.rs", "/"]
COPY ["proto/", "/proto"]
WORKDIR "/"
RUN apt-get -y update && apt-get install -y protobuf-compiler
RUN cargo build --release
ENV RUST_BACKTRACE=1
ENTRYPOINT ["cargo", "run"]

