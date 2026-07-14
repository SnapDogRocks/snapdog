# syntax=docker/dockerfile:1.19.0@sha256:b6afd42430b15f2d2a4c5a02b919e98a525b785b1aaff16747d2f623364e39b6

# Base images are pinned to multi-architecture manifest digests. Dependabot or
# Renovate should update the readable tag and digest together.
ARG NODE_IMAGE="node:24-bookworm-slim@sha256:cb4e8f7c443347358b7875e717c29e27bf9befc8f5a26cf18af3c3dec80e58c5"
ARG RUST_IMAGE="rust:1.97.0-bookworm@sha256:7d0723df719e7f213b69dc7c8c595985c3f4b060cfbee4f7bc0e347a86fe3b6a"
ARG RUNTIME_IMAGE="debian:bookworm-slim@sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df"

# Which stage supplies the snapdog binary at /out/snapdog:
#   compile  (default) — build from source in this Dockerfile. Used by local
#              `docker build .` and the PR `container-images` CI job.
#   prebuilt           — copy a binary already built by the release `build` job
#              (compiled once against bookworm glibc, matching RUNTIME_IMAGE) from
#              the context under bins/<arch>/. The release pipeline sets this so the
#              image ships the exact same binary as the tarball, with no second compile.
ARG BIN_STAGE=compile

# ── WebUI build stage (compile path only) ─────────────────────
FROM ${NODE_IMAGE} AS webui-builder

ENV NEXT_TELEMETRY_DISABLED=1
WORKDIR /build/webui

COPY webui/package.json webui/package-lock.json ./
RUN --mount=type=cache,id=snapdog-npm,target=/root/.npm,sharing=locked \
    npm ci --no-audit --no-fund

COPY webui/ ./
RUN npm run build && test -f out/index.html

# ── Rust build stage (compile path only) ─────────────────────
FROM ${RUST_IMAGE} AS compile

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
# The prebuilt ETS product database is embedded into the binary via include_bytes!
# (api/routes/knx.rs → ../../../../knx/snapdog.knxprod), so it must be in the image build.
COPY knx/snapdog.knxprod knx/snapdog.knxprod
COPY --from=webui-builder /build/webui/out webui/out

RUN --mount=type=cache,id=snapdog-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=snapdog-cargo-git,target=/usr/local/cargo/git/db,sharing=locked \
    --mount=type=cache,id=snapdog-server-target,target=/build/target,sharing=locked \
    CARGO_PROFILE_RELEASE_LTO="${CARGO_RELEASE_LTO}" cargo build --locked --release -p snapdog && \
    install -Dm0755 target/release/snapdog /out/snapdog

# ── Prebuilt binary stage (release path only) ─────────────────
# The release `build` job compiles the binary once (bookworm glibc) and drops it in
# the context under bins/<arch>/. TARGETARCH is set by buildx per target platform, so one
# multi-arch build selects the right binary with no second compile. This stage only copies
# (no emulation needed); the runtime stage's apt/user setup is what runs under QEMU for the
# arm64 platform.
FROM scratch AS prebuilt
ARG TARGETARCH
COPY bins/${TARGETARCH}/snapdog /out/snapdog

# Select the binary source: `compile` (default) or `prebuilt`.
FROM ${BIN_STAGE} AS binary

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

COPY --from=binary --chmod=0755 /out/snapdog /usr/local/bin/snapdog

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
