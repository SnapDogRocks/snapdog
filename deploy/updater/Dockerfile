# syntax=docker/dockerfile:1
# docker:29-cli multi-arch manifest, pinned for reproducible updater builds.
FROM docker:29-cli@sha256:27a51d5ab1cd38d9eeaba7b415b8c07bc10c31e1cf1ec8d78f6413fcfab3f44f

RUN apk add --no-cache \
      bash \
      ca-certificates \
      coreutils \
      cosign \
      curl \
      docker-cli-compose \
      jq \
      nodejs \
    && update-ca-certificates

# The version is reported to the Web UI and lets the detached self-update helper
# decide whether a newer updater image is required. Bake it in rather than
# inspecting via the socket: no extra privilege and it works if the socket is
# temporarily unavailable.
ARG UPDATER_VERSION=0.0.0-dev
ENV UPDATER_VERSION=${UPDATER_VERSION}

COPY update.sh /usr/local/bin/snapdog-update
COPY control.mjs /usr/local/lib/snapdog-updater/control.mjs
COPY core.mjs /usr/local/lib/snapdog-updater/core.mjs

EXPOSE 8080
# The updater intentionally controls the Docker daemon and rewrites the stack's
# image pin. Docker-socket access is root-equivalent regardless of the numeric
# container user, so state the privilege explicitly instead of implying a false
# user boundary. The API is unexported, token-authenticated, and fail-closed.
USER root
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD node -e "fetch('http://127.0.0.1:8080/v1/status',{headers:{authorization:'Bearer '+process.env.UPDATER_TOKEN}}).then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

ENTRYPOINT ["node", "/usr/local/lib/snapdog-updater/control.mjs"]
