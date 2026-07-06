# ── WebUI build stage ─────────────────────────────────────────
FROM node:25-slim AS webui-builder

WORKDIR /build/webui
COPY webui/package.json webui/package-lock.json ./
RUN npm ci --force
COPY webui/ .
RUN npm run build

# ── Rust build stage ──────────────────────────────────────────
FROM rust:1-bookworm AS builder

RUN apt-get update && apt-get install -y cmake pkg-config libasound2-dev libavahi-compat-libdnssd-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY assets/ assets/
COPY snapdog/ snapdog/
COPY snapdog-client/ snapdog-client/
COPY snapdog-common/ snapdog-common/
COPY snapdog-testkit/ snapdog-testkit/
COPY xtask/ xtask/
COPY --from=webui-builder /build/webui/out webui/out
RUN cargo build --release -p snapdog

# ── Runtime stage ─────────────────────────────────────────────
FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/metaneutrons/snapdog"
LABEL org.opencontainers.image.description="Multi-room audio system with KNX integration"
LABEL org.opencontainers.image.licenses="GPL-3.0-only"

RUN apt-get update && apt-get install -y --no-install-recommends \
    libasound2 libavahi-client3 libavahi-compat-libdnssd1 ca-certificates dumb-init curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/snapdog /usr/local/bin/

RUN useradd -r -s /bin/false snapdog && \
    mkdir -p /var/lib/snapdog /etc/snapdog && \
    chown snapdog:snapdog /var/lib/snapdog

USER snapdog
WORKDIR /var/lib/snapdog

# Persist ETS programming (knx-memory.bin), state, and EQ config across restarts
VOLUME /var/lib/snapdog

# HTTP API, Snapcast streaming, KNX/IP device
EXPOSE 5555 1704 3671/udp

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:5555/health || exit 1

ENTRYPOINT ["dumb-init", "snapdog"]
CMD ["--config", "/etc/snapdog/snapdog.toml"]
