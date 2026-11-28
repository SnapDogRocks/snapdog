FROM rust:1.100.0-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS builder
RUN apt-get update && apt-get install -y libasound2-dev pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
RUN cargo build --release -p snapdog-client

FROM debian:bookworm-slim@sha256:e67eeead736ff7bbe258ef088bff683635c561b1fcfefad0806c1df4ab49ce0e
RUN apt-get update && apt-get install -y --no-install-recommends \
    libasound2 ca-certificates dumb-init \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/snapdog-client /usr/local/bin/
ENTRYPOINT ["dumb-init", "snapdog-client"]
CMD ["--url", "tcp://host.docker.internal"]
