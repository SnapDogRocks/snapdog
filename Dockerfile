# syntax=docker/dockerfile:1.19.0@sha256:b6afd42430b15f2d2a4c5a02b919e98a525b785b1aaff16747d2f623364e39b6

# Base images are pinned to multi-architecture manifest digests. Dependabot or
# Renovate should update the readable tag and digest together.
ARG NODE_IMAGE="node:24-bookworm-slim@sha256:cb4e8f7c443347358b7875e717c29e27bf9befc8f5a26cf18af3c3dec80e58c5"
ARG RUST_IMAGE="rust:1.97.0-bookworm@sha256:7d0723df719e7f213b69dc7c8c595985c3f4b060cfbee4f7bc0e347a86fe3b6a"
ARG RUNTIME_IMAGE="debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df"

# ── WebUI build stage ─────────────────────────────────────────
FROM ${NODE_IMAGE} AS webui-builder

ENV NEXT_TELEMETRY_DISABLED=1
WORKDIR /build/webui

COPY webui/package.json webui/package-lock.json ./
RUN --mount=type=cache,id=snapdog-npm,target=/root/.npm,sharing=locked \
    npm ci --no-audit --no-fund

COPY webui/ ./
RUN npm run build && test -f out/index.html

# ── Rust build stage ──────────────────────────────────────────
FROM ${RUST_IMAGE} AS rust-builder

ARG CARGO_RELEASE_LTO=thin

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
      cmake \
      libasound2-dev \
      libavahi-compat-libdnssd-dev \
      pkg-config

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY assets/ assets/
COPY snapdog/ snapdog/
COPY snapdog-client/ snapdog-client/
COPY snapdog-common/ snapdog-common/
COPY snapdog-testkit/ snapdog-testkit/
COPY xtask/ xtask/
COPY --from=webui-builder /build/webui/out webui/out

RUN --mount=type=cache,id=snapdog-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=snapdog-cargo-git,target=/usr/local/cargo/git/db,sharing=locked \
    --mount=type=cache,id=snapdog-server-target,target=/build/target,sharing=locked \
    CARGO_PROFILE_RELEASE_LTO="${CARGO_RELEASE_LTO}" cargo build --locked --release -p snapdog && \
    install -Dm0755 target/release/snapdog /out/snapdog

# ── Runtime stage ─────────────────────────────────────────────
FROM ${RUNTIME_IMAGE} AS runtime

ARG BUILD_VERSION="0.0.0-dev"
ARG BUILD_REVISION="unknown"
ARG BUILD_CREATED="1970-01-01T00:00:00Z"
ARG SNAPDOG_UID=10001
ARG SNAPDOG_GID=10001

LABEL org.opencontainers.image.title="SnapDog" \
      org.opencontainers.image.description="Multi-room audio system with KNX integration" \
      org.opencontainers.image.source="https://github.com/SnapDogRocks/snapdog" \
      org.opencontainers.image.url="https://snapdog.rocks" \
      org.opencontainers.image.documentation="https://github.com/SnapDogRocks/snapdog#docker" \
      org.opencontainers.image.vendor="SnapDog" \
      org.opencontainers.image.licenses="GPL-3.0-only" \
      org.opencontainers.image.version="${BUILD_VERSION}" \
      org.opencontainers.image.revision="${BUILD_REVISION}" \
      org.opencontainers.image.created="${BUILD_CREATED}"

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      dumb-init \
      libasound2 \
      libavahi-client3 \
      libavahi-compat-libdnssd1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid "${SNAPDOG_GID}" snapdog \
    && useradd --uid "${SNAPDOG_UID}" --gid snapdog --no-log-init \
         --home-dir /var/lib/snapdog --no-create-home --shell /usr/sbin/nologin snapdog \
    && install -d -o snapdog -g snapdog -m 0750 /var/lib/snapdog \
    && install -d -o root -g snapdog -m 0750 /etc/snapdog

COPY --from=rust-builder --chmod=0755 /out/snapdog /usr/local/bin/snapdog

ENV SNAPDOG_HEALTHCHECK_URL="http://127.0.0.1:5555/health/live"

USER snapdog:snapdog
WORKDIR /var/lib/snapdog

# Persist ETS programming, application state, pairing data, and EQ settings.
VOLUME ["/var/lib/snapdog"]

# HTTP API/WebUI, Snapcast streaming, KNX/IP device.
EXPOSE 5555 1704 3671/udp

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 4 "${SNAPDOG_HEALTHCHECK_URL}" >/dev/null || exit 1

STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/bin/dumb-init", "--", "/usr/local/bin/snapdog"]
CMD ["--config", "/etc/snapdog/snapdog.toml"]
