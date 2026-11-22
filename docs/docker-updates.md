# Verified Docker updates

SnapDog's production Compose stack uses a dedicated updater sidecar. The sidecar
is the only container with access to the Docker socket; its control API is not
published on the host network. SnapDog proxies a small authenticated API to it,
so the WebUI can check and apply updates without receiving the updater token.

Download `docker-compose.yml` and `snapdog.env.example` from the same stable
GitHub release and verify both with that release's `SHA256SUMS`. Copy the example
to `.env`, replace `UPDATER_TOKEN` with `openssl rand -hex 32`, create a local
`snapdog.toml`, then run:

```sh
chmod 600 .env
docker compose up -d
```

The updater pulls an exact release tag, resolves its registry digest, and accepts
it only when Cosign verifies that it was published by
`.github/workflows/release.yml` for a stable `vX.Y.Z` tag. It recreates only the
server service, waits for `/health/ready`, persists the successful image pin, and
automatically restores the previous image if readiness fails. The persistent
`snapdog-data` volume and read-only configuration mount are never replaced.

Automatic mode checks every 15 minutes and applies once per local calendar day
at `MAINTENANCE_TIME`. Manual mode checks without applying. The About dialog
shows the updater state and provides Check/Update actions when the sidecar is
available. Native, APT, Homebrew, AUR, and plain `docker run` installations do
not expose those controls.

The updater can replace itself only through a detached helper created from the
known-good old updater image. The helper verifies the candidate, swaps the
container, checks its health, and rolls back on failure. Docker socket access is
root-equivalent; do not expose the updater port or reuse its token elsewhere.

## Own stack files

The updater runs `docker compose` inside its container against the host's Docker
daemon. Two consequences for a stack file that is not the shipped
`docker-compose.yml`, for example one kept by a NAS container manager:

- Every `env_file` entry must read `${COMPOSE_ENV_FILE:-./.env}`, as in the
  shipped file. A plain `./.env` is resolved against the host stack directory,
  which does not exist inside the updater container, and `docker compose` cannot
  load the stack.
- The stack file is bind-mounted as a single file. A manager that saves it by
  replacing the file (a new inode) leaves the running updater on the old content.
  Restart the updater container after editing the stack file.

Before an update the updater loads the stack file the way the deployment will. If
that fails, it stops with "update not applied, the running release is unchanged:
the stack file cannot be read by the updater" and the first line of the
`docker compose` error, without touching the server. The same "unchanged" status
appears when `docker compose` fails while the old container keeps running.
