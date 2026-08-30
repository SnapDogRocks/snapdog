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
