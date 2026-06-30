<!-- SPDX-License-Identifier: GPL-3.0-only -->
# snapdog test suite

Deterministic integration tests for the snapdog server, per
[RFC IT-0003](../../docs/rfcs/0003-integration-test-suite.md) (the task ledger).
This README is the entry point: the tiers, how to run them, the shared **testkit**,
the golden-vector flow, and how to add a test.

## Tiers

| Tier | What | Services | Determinism |
|------|------|----------|-------------|
| **Tier-1** | In-process: the axum `Router` via `tower::oneshot`, real zone-player tasks, pure mappers/codecs, golden vectors | none (no sockets beyond loopback, no mDNS) | fully deterministic — a flaky tier-1 test is a bug |
| **Tier-2** | Real external services over loopback/Docker: a managed `snapserver`, `mosquitto`, Subsonic | a real binary / broker / server | **loud-skips** (prints `SKIP …`, passes) when the service is absent |

Tier-1 runs everywhere on default features. Tier-2 is gated behind
`--features snapcast-process` (the process backend) and/or reads credentials from
`.env.test`; without the service it skips loudly rather than failing.

## Running

```sh
# Tier-1 (default features) — the everyday suite.
cargo test -p snapdog
cargo nextest run -p snapdog              # same, via nextest (see .config/nextest.toml)

# Tier-2 (process backend): real snapserver control + per-zone TCP audio, JSON-RPC
# golden vectors, real-mosquitto MQTT. Serial — each spawns its own server.
cargo test    -p snapdog --no-default-features --features snapcast-process -- --test-threads=1
cargo nextest run -p snapdog --no-default-features --features snapcast-process   # serial group is automatic

# test-harness feature: the PCM-injection seam (drive decode→resample→EQ→send_audio).
cargo test -p snapdog --features test-harness --test zone_player

# Regenerate golden fixtures after an intentional change (then commit them).
UPDATE_GOLDEN=1 cargo test -p snapdog
```

Tier-2 services locally:
- `snapserver` / `snapclient`: `brew install snapcast` (macOS) or `apt-get install snapserver`.
- MQTT (`mqtt_tier2`): Docker/colima (testcontainers spins up mosquitto).
- Subsonic + system MQTT (`integration`): set `SNAPDOG_TEST_*` in `.env.test`
  (see the keys in `.github/workflows/ci.yml`).

The feature model is `snapcast-embedded` (default) **XOR** `snapcast-process` — a
`compile_error!` enforces exactly one. The process module (`SnapserverHandle`) and
the JSON-RPC `SnapcastClient` only exist under `snapcast-process`.

## The testkit (`tests/common`)

Shared, deterministic primitives — reusable across this suite and a stable surface
sibling suites (`BT-0001`, `LI-0002`) can lift:

- **App harness** — `test_app()` / `build_test_app(config)` → a `TestApp` whose
  command channels are captured; `app.request(method, uri, body)` drives the full
  router with no socket. `drain_zone` / `drain_snap` assert emitted commands.
- **Zone-player harness** — `spawn_zone_harness(config)` runs the *real* per-zone
  runner tasks (`start_receivers=false`, no-op `MockBackend`); `await_notification`
  is the sync barrier (never sleep-then-poll). `spawn_zone_harness_capturing`
  (feature `test-harness`) adds a `CapturingBackend` + per-zone PCM injection.
- **Ephemeral pool (IT-T02)** — `free_port()`, `bind_ephemeral() → (listener, addr)`,
  `alloc_ports(n)` (n *mutually-distinct* ports), `EphemeralNames::new(seed)`
  (splitmix64; same seed ⇒ same collision-free sequence).
- **Virtual time (IT-T03)** — under `#[tokio::test(start_paused = true)]`, drive the
  §5.2 timers with `advance(d)` / `advance_secs(s)`; a 300 s timer fires in µs of
  wall time.
- **Golden vectors (IT-T06)** — `assert_json_golden(name, &value)` (exact canonical
  JSON) and `assert_f64_golden_within(name, &vec, tol)` / `cmp_f64_within(...)`
  (per-element ±tol for audio / float DPT). Fixtures live in `tests/fixtures/`;
  `UPDATE_GOLDEN=1` regenerates and passes.

## Golden-vector flow

1. Write the assertion: `common::assert_json_golden("my_vector", &actual);`.
2. Create the fixture: `UPDATE_GOLDEN=1 cargo test -p snapdog --test <file>`.
3. Inspect `tests/fixtures/my_vector.json`, then commit it.
4. Thereafter the test compares against the committed golden; an intentional change
   means re-running step 2 and reviewing the fixture diff.

## Adding a test

- **Tier-1**: new `tests/<area>.rs` with `mod common;`; use `TestApp` for REST/WS,
  `spawn_zone_harness` for player transitions, the golden helpers for wire shapes.
  Keep it deterministic — no real sockets, no sleeps; use the virtual-time helpers.
- **Tier-2**: gate the file with `#![cfg(feature = "snapcast-process")]`; **loud-skip**
  (eprintln `SKIP …` + `return`) when the service/binary is absent (see
  `snapserver_e2e.rs` / `mqtt_tier2.rs`); use `alloc_ports`/`free_port` for ports;
  add a run line to the `integration-tests` CI job.

## nextest

`.config/nextest.toml` defines two profiles. `default`: `retries = 0`
(deterministic tier-1), slow-test warnings. `ci`: same plus `fail-fast = false` and
a JUnit report. Both put the real-service binaries (`integration`,
`snapserver_e2e`, `mqtt_tier2`) in a serial `tier2-real-service` group with
`retries = 2` + a hang-kill — bounded tolerance for service bring-up, never for
tier-1.

## Test files at a glance

| File | Tier | Covers |
|------|------|--------|
| `testkit.rs` | 1 | the testkit itself (ephemeral pool, virtual time, golden) |
| `rest_*.rs`, `auth.rs`, `openapi_contract.rs`, `config_contract.rs` | 1 | REST surfaces, auth layer, OpenAPI/config contracts |
| `ws*.rs` | 1 | WebSocket broadcast, ping cadence, connection-limit lifecycle |
| `zone_player.rs` | 1 | real zone-player transitions, presence auto-off, PCM injection |
| `audio.rs`, `knx_golden.rs`, `snapcast.rs` | 1 | DSP/codec goldens, KNX GA/DPT goldens, seam exhaustiveness |
| `headless_boot.rs` | 1 | `/health` + serve-over-socket → graceful shutdown |
| `snapcast_rpc.rs` | 2* | JSON-RPC golden vectors via an in-process TCP fake (*no real service) |
| `integration.rs` | 2 | real Subsonic + MQTT (`.env.test`) |
| `mqtt_tier2.rs` | 2 | real mosquitto via testcontainers (retained/LWT round-trip) |
| `snapserver_e2e.rs` | 2 | real snapserver: control (status/sync/reconcile) + TCP audio source |
