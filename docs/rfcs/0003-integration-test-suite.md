---
rfc: IT-0003
title: Integration & regression test suite for snapdog
status: in-progress      # draft | accepted | in-progress | done | superseded
version: 1.2.0           # v1.2: Phase-0 testkit + REST/WS/audio/config tier-1 suites landed; integration.rs snapcast helpers repaired
created: 2026-06-28
updated: 2026-06-28
target_repo: snapdog
target_branch: main
related: [BT-0001, LI-0002]   # reusable testkit is consumed by the Bluetooth / Line-In RFCs
feature_flags: [test-util]
owners: [metaneutrons]
progress:                # keep in sync with the IT-LEDGER block (§13)
  total_tasks: 47
  done: 35
  in_progress: 3
  todo: 9
---

# RFC IT-0003 — Integration & regression test suite for snapdog

> **For AI agents:** this is the single source of truth for snapdog's integration
> test suite. Every requirement (`IT-REQ-*`), decision (`IT-DEC-*`), and task
> (`IT-T*`) has a stable ID. To track progress, update (1) the task checkbox +
> `status:` in §10, (2) the matching entry in the **IT-LEDGER** YAML (§13), and
> (3) the `progress:` rollup in the frontmatter. Reference IDs in commits
> (e.g. `test(snapcast): golden JSON-RPC vectors (IT-T54)`). Line numbers in §3/§8/§9
> are approximate (verify before editing); **symbol names are the stable anchor.**
> See §12 for the protocol.

## 1. Summary

snapdog has rich control surfaces (REST, WebSocket, MQTT, KNX), an F32 audio
pipeline (decode → resample → zone-EQ → Snapcast), and three receiver seams
(AirPlay, Spotify, Snapcast) built on **recently upgraded, breaking-change
dependencies**: `snapcast-server/proto 0.16.1`, `knx-rs-core/ip/device 0.2`,
`shairplay 0.5 (+ap2)`, `librespot 0.8`. The existing test surface is thin and the
single integration test file is **already dead** — gated behind a feature and
carrying a TODO because a `SnapcastClient` refactor removed `init()`/`state()`
with no test to catch it (`tests/integration.rs:6-8`, ~22 tests `#[ignore]`d).

This RFC proposes a **world-class, deterministic, AI-implementable** integration
suite whose primary job is **regression prevention**, with the dependency seams as
the headline: a **crate contract firewall** (§9) of golden vectors + exhaustiveness
+ build-smoke tests that *fail loudly* when an upgraded crate's API or wire format
drifts — exactly the class of break that silently killed the current suite.

Three tiers (`IT-DEC-01`): a **deterministic core** (no Docker, no network, always
green, the CI gate), an **opt-in integration tier** (real `snapserver` + optional
Docker/testcontainers for Subsonic/MQTT, *loud-skip* when unavailable), and a
**manual e2e/hardware tier** (never in CI). Docker (colima on macOS) is **purely
optional** and never a prerequisite for the core (`IT-DEC-10`).

## 2. Goals / Non-goals

### Goals (`IT-REQ-*`)
- `IT-REQ-01` A **deterministic tier-1** suite: no Docker, no network, no
  wall-clock sleeps; always green; the CI gate.
- `IT-REQ-02` Cover **every control surface**: all REST endpoints, all 7 WS
  notifications, all MQTT command/state topics, all KNX GA actions.
- `IT-REQ-03` Cover the **audio pipeline** with golden PCM vectors (decode →
  resample → EQ) and fade/EQ-stability assertions.
- `IT-REQ-04` A **crate contract firewall** for the 3 upgraded seams
  (snapcast/knx/airplay+spotify) that **fails on breaking changes**.
- `IT-REQ-05` Cover the **zone-player state machine** + headless boot/lifecycle.
- `IT-REQ-06` An **optional Docker tier-2** (testcontainers) for real services;
  **loud-skip** (print why) when Docker is absent — never silently pass.
- `IT-REQ-07` **Reuse** existing in-source doubles; **build** the missing ones
  (line-delimited-JSON TCP fake, librespot mapper harness, time guard, ephemeral
  resource pool, golden PCM fixtures).
- `IT-REQ-08` **Deterministic by construction**: controlled time, seeded RNG,
  fixed clock, temp filesystem, no mDNS in tier 1, ephemeral ports,
  sort-before-compare, epsilon/golden tolerances.
- `IT-REQ-09` **CI**: nextest, tier separation, flake retries (tier-2 only),
  failure-artifact capture, OpenAPI contract validation, feature build-matrix.
- `IT-REQ-10` **Prerequisites**: repair the dead `tests/integration.rs` against the
  new `SnapcastClient` API; resolve the `0.16.1`/`0.17.0` snapcast pin.
- `IT-REQ-11` A **reusable testkit** consumable by RFC `BT-0001`/`LI-0002`.

### Non-goals
- `IT-NG-01` Automated **e2e/hardware** in CI (real KNX/IP gateway, physical
  AirPlay/Spotify clients, real audio output) — manual tier-3 only.
- `IT-NG-02` Testing the **internals** of librespot/shairplay/snapcast/knx-rs —
  only snapdog's seam against them.
- `IT-NG-03` **Performance / load / latency-SLA** testing (separate effort).
- `IT-NG-04` **mDNS/zeroconf discovery** determinism (excluded from tier 1; tier-3).
- `IT-NG-05` **Migrating** snapdog to snapcast `0.17` — separate work; this suite is
  the safety net *for* that migration (§14, §15).

## 3. Background — verified facts (cite-checked)

| Fact | Evidence |
|---|---|
| Workspace `0.20.0`, edition 2024, rust 1.85; crate at `snapdog/snapdog` | `Cargo.toml` |
| Consumed deps: snapcast-server/proto **0.16.1**, knx-rs-core/ip/device **0.2.0**, shairplay **0.5.0**, librespot **0.8.0** | `Cargo.lock` |
| **ADR-018**: own JSON-RPC `SnapcastClient` (snapcast-control removed); **17 JSON-RPC** method wrappers (+ `connect`/`from_config`/`subscribe` lifecycle), `sync_initial_state` (:328), `reconcile_zone_groups` (:422) | `snapdog/src/snapcast/mod.rs:55-498` |
| **Realized breakage**: `SnapcastClient.init()/.state()` removed; `tests/integration.rs` dead, ~22 `#[ignore]` | `snapdog/tests/integration.rs:6-8` |
| `SnapcastBackend` trait is the mock seam; `embedded` XOR `process` (compile_error if both/neither) | `snapdog/src/snapcast/backend.rs`, `embedded.rs:24,260,448` |
| WS `Notification` has **7** variants (not 8) | `snapdog/src/api/ws.rs:39,109,118,130,145,153,164`; `MAX_WS_CONNECTIONS=64` (:13) |
| AirPlay volume: `vol<=-144→0` else `((vol+30)/30*100).clamp(0,100)` | `snapdog/src/receiver/airplay.rs:27,29,162-165` |
| Spotify volume: in `(v*100)/u16::MAX`, out `(p*u16::MAX)/100`; no true Stop | `snapdog/src/receiver/spotify.rs:110,181,287` |
| KNX **460** group objects; `zone_asap`/`client_asap` deterministic layout | `snapdog/src/knx/group_objects.rs:19-25,522,723,744-756` |
| `run_app()` is `pub async` — bootable headless from tests | `snapdog/src/main.rs:183` |
| Dev-deps present: `mockall`, `tempfile`, `wiremock` (UNUSED), `dotenvy`, tokio `test-util` (time::pause) | `snapdog/Cargo.toml` |
| CI: fmt, webui, clippy, unit-tests (`cargo test --lib`), integration (gated `snapcast-process`), audit, knxprod, windows-check | `.github/workflows/ci.yml` |
| In-source doubles already exist (see §9.4) | `mqtt/mod.rs:74`, `knx/mod.rs:763-828`, `state/mod.rs:452`, `snapcast/mod.rs:500` |

**Existing test surface is thin:** ~19 of ~57 source files have `mod tests`; **zero**
use of `insta`/`proptest`/`loom`; the one integration file is feature-gated and
largely dead.

## 4. The regression problem (why now)

The user upgraded `snapcast-rs`, `knx-rs`, and `shairplay-rust` with **breaking
changes** and needs to verify snapdog against them. The current suite cannot do
this — and worse, it already *failed silently*: the `SnapcastClient` refactor
removed methods the integration tests called, and nothing went red because those
tests were `#[ignore]`d. This is the canonical "missing characterization test let a
breaking change through" failure.

Two facts shape the design:
1. **Pin contradiction (`IT-DEC-11`, §14).** snapdog's `Cargo.lock` pins snapcast at
   **0.16.1** (crates.io), but the local `snapcast-rs` workspace is **0.17.0**, and
   its powerful `snapcast-tests` harness (`crates/snapcast-tests/src/lib.rs`:
   `TestServer`/`TestClient`/`expect_event`) is the **0.17 API** and is **not** a
   dev-dep of snapdog. Adopting it requires deciding the pin first.
2. **No shared types across the JSON-RPC seam.** snapdog hand-rolls its snapcast
   control client, so server/client wire drift is invisible to the compiler — the
   highest-drift, must-have-a-test surface.

The firewall (§9) targets exactly these.

## 5. Architecture

### 5.1 Three tiers (`IT-DEC-01`)
```
 ┌── TIER 1 — DETERMINISTIC (default; the CI gate; no Docker, no net, no sleeps) ──┐
 │  REST  : axum tower::oneshot on Router + captured cmd senders + broadcast tap   │
 │  WS    : subscribe the broadcast::Sender directly; time::pause for ping/limit   │
 │  MQTT  : MqttBridge::test_bridge() (disconnected) → handle_command → cap chans  │
 │  KNX   : run_incoming() routing + golden CEMI/DPT vectors + DeviceServer:0 loop │
 │  snapcast: mocked SnapcastBackend trait + golden JSON-RPC vs custom TCP fake    │
 │  audio : golden PCM (sine/silence/pink) hashed; fade math; proptest EQ-stable   │
 │  AirPlay: shairplay TestHandler + RaopServer::builder().port(0) loopback        │
 │  Spotify: ChannelSink i16→f32 + PlayerEvent→ReceiverEvent as pure functions     │
 │  state : drive ZoneCommand mpsc + tokio time control; tempfile state_dir        │
 └────────────────────────────────────────────────────────────────────────────────┘
 ┌── TIER 2 — INTEGRATION (opt-in; Docker OPTIONAL; loud-skip when absent) ────────┐
 │  real snapserver (SnapserverHandle) · testcontainers: Navidrome, mosquitto      │
 │  real knx-rs DeviceServer loopback · reuses free_port/test_config harness       │
 └────────────────────────────────────────────────────────────────────────────────┘
 ┌── TIER 3 — E2E / HARDWARE (manual; NEVER in CI) ────────────────────────────────┐
 │  real KNX/IP gateway · physical AirPlay/Spotify clients · real audio output     │
 └────────────────────────────────────────────────────────────────────────────────┘
```

### 5.2 Determinism doctrine (`IT-DEC-02`) — the flake firewall
- **Time:** `tokio::test(start_paused = true)` + `time::advance` for **every**
  timer — WS 30s ping, auto-save 5s, presence auto-off (configurable
  `auto_off_delay`, default 900s / inactive 86400s), volume coalescing 50ms,
  `SNAPSERVER_STARTUP_DELAY` 1s, `CONNECT_RETRY_DELAY` 500ms / `MAX_RECONNECT_DELAY`
  5s, `ZONE_RESTART_DELAY` 5s, playlist cache 60s, fades.
  *(Current integration suite burns 20–30s of real sleeps — the #1 flake source.)*
- **Randomness:** pre-write `state_dir/server_id` for a stable UUID; seed/mock
  `fastrand` (shuffle) and the Subsonic auth salt; assert JSON-RPC request-ids by
  **structure/correlation**, not literal value (or inject a seeded id generator).
- **Clock:** inject a fixed clock for presence schedule resolution
  (`chrono::Local::now`, `runner.rs:1130`), or use always-matching windows.
- **Filesystem:** `tempfile::TempDir` per test for `state_dir`
  (`zones.json`/`eq.json`/`snapcast.json`/`server_id`); or `persist_path=None` to
  disable the 5s auto-save loop.
- **Network:** tier-1 sets `mdns.enabled=false`; receivers driven at the
  **handler/event boundary**, never via discovery.
- **HTTP:** `wiremock` for Subsonic/radio/cover-art with fixed (non-updating)
  fixtures and fail-fast (no retry backoff).
- **Ports/parallelism:** `EphemeralResource` pool (ports via `TcpListener :0`,
  unique mDNS/zone names, seeded RNG); tier-2 real-service tests run serial.
- **Comparisons:** sort before compare (reconciliation already sorts); float/audio
  via epsilon or hashed golden vectors (rubato/biquad differ per platform); DPT
  float (9/14) ±1 LSB; assert **final state**, not intermediate event ordering.

### 5.3 Harness primitives (built in Phase 0)
`testkit` (a `tests/common/` module behind a `test-util` feature, `IT-DEC-08`) exports:
`TokioTimeGuard` (`IT-T03`), `EphemeralResource` pool (`IT-T02`), `TempEnv`
(TempDir + seeded server_id + mdns-off, `IT-T04`), the golden helper (`IT-T06`),
the in-process REST harness (`IT-T10`), the **line-delimited-JSON TCP fake** for
`SnapcastClient` (`IT-T54`), and the `SnapcastBackend` double (`IT-T50`).

## 6. Decisions (resolved)

| ID | Decision | Resolution |
|---|---|---|
| `IT-DEC-01` | Three-tier architecture | **(1) deterministic** (no Docker/net, always-green gate), **(2) integration** (opt-in, real snapserver + optional Docker, loud-skip), **(3) e2e/hardware** (manual, never CI). Tier 1 is the contract; the others are additive. |
| `IT-DEC-02` | Determinism doctrine | Controlled time (`tokio test-util`), seeded RNG, fixed clock, temp fs, no mDNS in tier 1, ephemeral ports, sort-before-compare, golden/epsilon tolerances (§5.2). Tier-1 must be **flake-free with zero retries**. |
| `IT-DEC-03` | In-process drivers | Drive control surfaces **without real sockets** in tier 1: axum `oneshot`, broadcast tap, `MqttBridge::test_bridge`, `run_incoming`. Real sockets only in tier 2. |
| `IT-DEC-04` | `SnapcastBackend` as mock seam | Test backend-agnostic logic (reconcile/volume/event-roundtrip) against the **trait** (mockall/hand-coded double) so a single pass covers logic and we avoid a 2× `embedded`/`process` feature matrix. |
| `IT-DEC-05` | Crate contract firewall | The headline (§9): golden vectors + **exhaustiveness** tests (fail, don't silently drop, on a new/renamed variant) + **build-smoke** matrix for the 3 upgraded seams. Rationale: this is precisely what would have caught the `start_system`/`init` break. |
| `IT-DEC-06` | Custom TCP fake for JSON-RPC | `wiremock` is HTTP-only and **cannot** stand in for the line-delimited-JSON snapcast control socket → build a small custom TCP fake (`IT-T54`). Real `snapserver` only in tier 2. |
| `IT-DEC-07` | Golden-vector policy | Store bytes/hashes under `tests/fixtures/`; regenerate via `UPDATE_GOLDEN=1`; ±1 LSB for float DPT, hash-with-tolerance for audio. A golden change must be a reviewed diff. |
| `IT-DEC-08` | Layout & reuse | Tests in `snapdog/tests/` (integration) + `tests/common/` (harness) + `tests/fixtures/`; reusable doubles behind a `test-util` feature so `BT-0001`/`LI-0002` reuse them (`IT-REQ-11`). |
| `IT-DEC-09` | Runner = cargo-nextest | nextest for isolation/retry/artifacts; `test-groups` serialize real-service tests; **retries only in tier 2** (tier 1 stays retry-free to surface real flakes). `cargo test` still works. |
| `IT-DEC-10` | Docker is optional | testcontainers-rs for tier-2 services; **never** a tier-1 prerequisite. Absence → **loud skip** (print why), not silent pass. colima available but not required. Docker config stays inside `snapdog/`. |
| `IT-DEC-11` | Version-pin & dead-suite prerequisites | The suite is **version-agnostic** via the `SnapcastBackend` trait + golden JSON-RPC vectors, so it does not block on the `0.16.1`/`0.17.0` decision. **But** `tests/integration.rs` must be **repaired** against the new `SnapcastClient` API first (`IT-T07`), and the pin (adopt `snapcast-tests` 0.17 vs keep 0.16.1 vectors) decided + recorded (`IT-T08`). |
| `IT-DEC-12` | Characterization vs specification | Where behavior is **unspecified** (the oracle gaps, §14), tests **record current behavior** (golden) and are labelled *characterization* — they catch regressions while leaving the spec free to change *deliberately* (update the golden in the same PR). |
| `IT-DEC-13` | mDNS excluded from tier 1 | Discovery (AirPlay `RaopServer` mDNS, Spotify `Discovery`, snapdog `_snapdog._tcp`) is non-deterministic (multicast/zeroconf timing) → driven at the handler boundary in tier 1; covered manually in tier 3. |
| `IT-DEC-14` | CI shape | Tier-1 job = always-green gate; tier-2 job = gated + loud-skip + **artifact capture** (server logs/state dumps) on failure; **OpenAPI contract** step; **feature build-matrix** smoke (`embedded`/`process`, `ap2` on/off, `spotify` on/off). |

## 7. Repository layout, tooling & fixtures

```
snapdog/
  src/**                         # in-source #[cfg(test)] unit + pure-fn table tests
  src/testing/  (feature=test-util)   # reusable doubles exported to BT/LI RFCs
  tests/
    common/                      # harness: time guard, ephemeral pool, TempEnv, REST driver, TCP fake
    fixtures/                    # golden.json (PCM hashes), jsonrpc/*.json, cemi/*.bin, covers/*
    rest_*.rs  ws.rs  mqtt.rs  knx.rs  snapcast.rs  audio.rs  receivers.rs  state.rs  boot.rs
    integration_tier2.rs         # cfg(feature="integration-external"), loud-skip
  .config/nextest.toml           # test-groups (serial real-service), tier-2 retries, slow-timeout
  .env.test.example              # SNAPDOG_TEST_SUBSONIC_URL / _MQTT_BROKER (tier-2)
Cargo.toml                       # [features] test-util, integration-external; dev-deps below
```
New dev-deps: keep `mockall`/`tempfile`/`wiremock`/`dotenvy`; add `insta` (snapshot),
`proptest` (EQ/property), `testcontainers` (tier-2, optional), `assert-json-diff`,
and `cargo-nextest` (tool). `wiremock` (currently unused) becomes the HTTP double.

## 8. Per-surface test matrix & assertion recipes

> Tier 1 unless noted. Each row's behaviors are the **assertion targets**; full
> evidence in §3 and the research brief. Tasks link in the last column.

| Surface | Behaviors to assert | Driver / double | Tasks |
|---|---|---|---|
| **REST** (axum, all endpoints under `/api/v1`) | route groups mount + auth; zone play/pause/stop/next/prev, seek (`position_ms` XOR `offset_ms`, both→400), volume (`VolumeValue` absolute/`+5`/`-10`+clamp), repeat cycle Off→All→One, EQ (≤10 bands, 11→400, band-edit clears preset), cover placeholder etag; client vol/mute/latency/zone(+fade)/EQ-422-if-not-snapdog; media unified playlist (radio idx0), speakers exactly-one-of, system/version, knx prog-mode 409 in client mode, health; **boundary** zone 0→404 | `tower::ServiceExt::oneshot` on `Router` + mock `AppState` w/ captured `ZoneCommand`/`SnapcastCmd` mpsc + broadcast tap; assert status+JSON(values)+**exactly one** command captured | `IT-T10`–`IT-T14` |
| **WebSocket** (`/ws`, 7 variants) | `ZoneChanged`/`ZoneVolumeChanged`/`ZoneProgress`/`ClientStateChanged`/`ZoneEqChanged`(serde flatten)/`ZonePresenceChanged`/`PlaybackError` emitted on the right mutation; serde `tag="type"` snake_case round-trip; ping 30s; close 1001; 65th conn→503 | subscribe the `broadcast::Sender` directly; `time::pause` for ping/limit | `IT-T20`,`IT-T21` |
| **MQTT** (16 cmd topics, 2 state) | all `/set` topics → correct command; volume 0.0–1.0 & 0–100; mute/repeat parsing; client zone validation (reject 0/unknown); retained `zones/{i}/state`+`clients/{i}/state` JSON; LWT online/offline; HA discovery | `MqttBridge::test_bridge()` (disconnected) → `handle_command` → captured chans (`zone_channels`/`snap_channel`/`test_state_with_client`); **tier-2** real mosquitto | `IT-T30`–`IT-T32` |
| **KNX** (460 GOs) | every GA action → command; DPT decode bool/percent(5.001)/u8(5.010)/u16(7.005)/dim(3.007 stepcode→%); publisher status GOs + DPTs + progress scaling; `zone_asap`/`client_asap` layout; unmapped GA ignored | `run_incoming` + `ga()`/`encode_*` helpers + `zone_ga_map`/`client_ga_map`; golden CEMI/DPT (reuse `knx-rs-core/tests`); **DeviceServer::start_at(:0)** loopback for device-mode | `IT-T40`–`IT-T43` |
| **Audio pipeline** | golden PCM (sine/silence/pink) through decode→resample→EQ; fade sample-count + gain ramp; EQ stability (no NaN/Inf); prefetch cache hit vs miss; ICY metadata parse | golden hashes (tolerance); mock `sample_rate`; `proptest`; `wiremock` for HTTP | `IT-T60`–`IT-T63` |
| **AirPlay** seam | `audio_init`→`SessionStarted`; `on_volume`/`on_metadata`/`on_coverart`→`ReceiverEvent`; volume golden (**corrected**: −144→0, 0→100, +30→100, −30→0); `RemoteCommand` round-trip; AP2 SRP | shairplay `TestHandler` + `RaopServer::builder().port(0)` + `send_rtsp` + `MemoryPairingStore` (`#[serial]`) | `IT-T70`,`IT-T71` |
| **Spotify** seam | `AudioPacket::Samples(Vec<f64>)`→f32 cast (librespot 0.8 is **already normalized** [-1,1] — assert **no** rescaling); `PlayerEvent`→`ReceiverEvent` mapping; volume `(v*100)/u16::MAX` vectors | pure-function mappers (no upstream harness; discovery excluded) | `IT-T72`,`IT-T73` |
| **snapcast** seam | reconcile_zone_groups (sorted `Group.SetClients`); event roundtrip + **exhaustiveness fails on unmapped** `ServerEvent`; `GroupVolumeMode.effective` (Absolute/Relative/Compressed+clamp+max_volume); golden JSON-RPC for 17 methods; embedded `F32AudioSender::send(&[f32])` | mocked `SnapcastBackend` + pure helpers + **custom line-delimited-JSON TCP fake**; **tier-2** real snapserver | `IT-T50`–`IT-T56` |
| **State machine / lifecycle** | transitions (track None iff Idle); persistence roundtrip (restore subset, playback→Stopped); next/prev/complete repeat+shuffle(seeded), prev-restart >3s; presence (fixed clock + auto-off via time::pause); `source_conflict` LastWins/ReceiverWins; multi-zone isolation; headless `run_app` boot | drive `ZoneCommand` mpsc + time control + `TempEnv` | `IT-T80`–`IT-T84` |

## 9. Crate contract firewall (headline — catches the breaking upgrades)

### 9.1 snapcast (`snapcast-server`/`proto`/`client`) — risk **HIGH, already realized**
**Seam:** embedded `snapcast_server::{SnapServer::new, add_f32_stream→F32AudioSender,
ServerConfig/Command/Event/Status, Hello, CustomMessage}`; process: hand-rolled
`SnapcastClient` JSON-RPC over `snapcast-proto` status types (17 methods:
`Server.GetStatus`, `Client.SetVolume`, `Group.SetClients`, `Stream.*`, …).
**Risks:** (a) `F32AudioSender::send` signature/error drift; (b) silent serde field
renames in `ServerStatus`/`Group`/`Client`; (c) **JSON-RPC method/param drift** (no
shared types — highest); (d) added/renamed `ServerEvent` variants silently dropped
(`embedded.rs:242`); (e) custom-protocol `type_id`/`CustomMessage` change. *(The
`init()`/`state()` removal already bit.)*
**Strategy:** golden JSON-RPC request/response vectors vs the TCP fake (`IT-T54`);
event-mapping **exhaustiveness** test that fails on unmapped variants (`IT-T52`);
F32 sender signature contract (`IT-T55`); mock `SnapcastBackend` for logic (`IT-T50`).
**Prereq:** repair `integration.rs` (`IT-T07`); resolve pin (`IT-T08`/`IT-DEC-11`).

### 9.2 knx-rs (`core`/`ip`/`device` 0.2) — risk **MEDIUM-HIGH** (0.1→0.2 split)
**Seam:** `core::{address, dpt::{encode,decode,DPT_*}, cemi::CemiFrame}`;
`ip::{Multiplexer, tunnel_server::{DeviceServer, ServerEvent}, KnxIpError}`;
`device::bau::Bau` (process/poll/save/restore/tables). Wrapped by snapdog's
`KnxPublisher`/`KnxListener`/`KnxDeviceControl` traits.
**Risks:** DPT semantics drift (scaling rounding, 3.007 stepcode, 14-byte string
pad); address-table big-endian format change corrupts ETS programming; `CemiFrame`
ctor/parse change; `Bau` save/restore byte format change (`PERSIST_MAGIC "SDKM"`);
`DeviceServer::start` signature; **watch the `KnxIpError`→`KnxIpParseError` rename**
flagged in the knx-rs audit (0.2.0 still names it `KnxIpError` — not yet landed).
**Strategy:** reuse `knx-rs-core/tests/golden_{cemi,dpt}.rs` as a **dependency
contract** (`IT-T41`); snapdog-side DPT vectors via `encode_*` helpers; `GroupAddress`
round-trip; device-mode `DeviceServer::start_at(:0)` + raw `CemiFrame` exchange +
`Bau.save()` byte-stability + CRC (`IT-T43`).

### 9.3 shairplay 0.5 (+ap2) & librespot 0.8 — risk **MEDIUM / HIGH**
**Seam:** shairplay `RaopServer::builder()` + `AudioHandler`/`AudioSession`/
`RemoteControl` traits; librespot `Discovery`/`Session`/`Player`/`Spirc` + custom
`ChannelSink` (`AudioPacket::Samples(Vec<f64>)`→f32, already normalized) + `PlayerEvent`.
**Risks:** librespot 0.7→0.8 removed public APIs (Sink signature, `PlayerEvent`
fields, `Spirc::new` tuple); shairplay 0.5 trait/`TrackMetadata` path/`RemoteCommand`
drift; AirPlay volume math is snapdog-local (frequent silent regression).
**Strategy:** shairplay contract via `TestHandler` + `RaopServer::builder().port(0)`
loopback + `send_rtsp` (`IT-T70`); corrected volume golden + remote round-trip
(`IT-T71`); librespot `ChannelSink`/`PlayerEvent` **pure-function** mappers + volume
vectors (`IT-T72`); **feature build-smoke** matrix to catch gross signature breaks
(`IT-T73`). Live discovery/Spirc excluded (tier-3).

### 9.4 Reuse vs build (doubles)
**Reuse (exist):** `MqttBridge::test_bridge()` + `zone_channels`/`snap_channel`/
`test_state_with_client` (`mqtt/mod.rs:74,559-589`); KNX `test_state`/`run_incoming`/
`zone_ga_map`/`client_ga_map`/`encode_*`/`ga` (`knx/mod.rs:718-828`) + `build_tables_from_minimal_config`/`persist_roundtrip` (`device.rs`); `SnapcastBackend`
trait + pure helpers `build_client_mac_map`/`build_group_ids`/`build_group_clients`
(`snapcast/mod.rs:500-520`) + inline mock server (`embedded.rs:448`); `state/mod.rs
test_config` + `config::load_raw[_no_validate]`; pure fns `GroupVolumeMode::effective`,
`fade_gain`, volume mappers; shairplay `TestHandler`/`RaopServer::builder().port(0)`/
`MemoryPairingStore`/`send_rtsp`; knx-rs golden vectors + `create_demo_bau` +
`tunnel_integration` frame builders; integration harness `free_port`/`test_config`/
`SnapserverHandle` (tier-2, **after repair**).
**Build (gaps):** line-delimited-JSON **TCP fake** for `SnapcastClient`; librespot
`ChannelSink`+`PlayerEvent` mapper harness; `TokioTimeGuard`; `EphemeralResource`
pool; golden **PCM** fixtures; the in-process REST driver. *(Do **not** rely on
`snapcast-rs` 0.17 `snapcast-tests` until the pin is resolved — `IT-DEC-11`.)*

## 10. Task breakdown (phased)

> Status legend: `todo` ▢ · `in-progress` ◐ · `done` ✅ · `blocked` ⛔ · `cancelled` ✗.
> Update the checkbox **and** the `status:` token **and** the IT-LEDGER (§13).
> Tier-2 tasks are marked **(T2)**; everything else is the deterministic tier-1 gate.

### Phase 0 — Foundations & prerequisites
- [x] `IT-T01` `testkit` scaffold: `tests/common/` + `test-util` feature exporting reusable doubles (`IT-DEC-08`). `status: todo` · deps: — · **AC:** `cargo test` discovers `tests/common`; `--features test-util` compiles.
- [ ] `IT-T02` `EphemeralResource` pool (ports `TcpListener :0`, unique mDNS/zone names, seeded RNG) for safe parallelism. `status: todo` · deps: IT-T01 · **AC:** `allocate_port` returns unique ports across N concurrent tasks; name allocation is collision-free and reproducible under a fixed seed.
- [ ] `IT-T03` `TokioTimeGuard` (pause/advance helpers for the named timers §5.2). `status: todo` · deps: IT-T01 · **AC:** a 300s presence auto-off test completes in <50ms.
- [x] `IT-T04` `TempEnv` fixture (TempDir `state_dir`, pre-seeded `server_id`, `persist_path` control, `mdns.enabled=false`). `status: todo` · deps: IT-T01 · **AC:** `TempEnv::new()` makes a TempDir `state_dir`, pre-writes a fixed `server_id` UUID, supports `persist_path=None` (disables auto-save), sets `mdns.enabled=false`, cleans up on drop.
- [ ] `IT-T05` Adopt cargo-nextest + `.config/nextest.toml` (test-groups serial for real-service, retries **tier-2 only**, slow-timeout). `status: todo` · deps: — · **AC:** `cargo nextest run` green; tier-1 has 0 retries.
- [ ] `IT-T06` Golden-vector harness: `tests/fixtures/` + load/compare helper, `UPDATE_GOLDEN=1`, ±tolerance for float DPT/audio (`IT-DEC-07`). `status: todo` · deps: IT-T01 · **AC:** compare returns Ok iff actual is within tolerance of golden; `UPDATE_GOLDEN=1` regenerates fixtures; one round-trippable golden vector exists.
- [ ] `IT-T07` **PREREQ**: repair `tests/integration.rs` `start_system`/`start_system_with_api` vs the new `SnapcastClient` API (`sync_initial_state`, no `init`/`state`); un-ignore the revived tests. `status: todo` · deps: — · **AC:** previously-`#[ignore]`d integration tests compile and pass under tier-2; **includes any `SnapserverHandle` updates the new API needs (feeds IT-T56)**.
- [x] `IT-T08` Snapcast **0.16.1 pin** decision recorded as **ADR-019** (stay pinned until the firewall + `IT-T73` are green, then `IT-NG-05`); build-smoke matrix is `IT-T73`. `status: done` · deps: —.

### Phase 1 — REST contract suite
- [x] `IT-T10` In-process REST driver: `oneshot` on `Router` + mock `AppState` (captured `ZoneCommand`/`SnapcastCmd` mpsc + broadcast tap). `status: todo` · deps: IT-T01 · **AC:** a GET returns 200 with no TCP socket; the suite **enumerates and asserts every mounted route group** (no hardcoded endpoint count).
- [x] `IT-T11` Zone endpoints contract (all): status+body+**exactly-one** command; boundaries (zone 0→404), seek XOR(→400), volume parse/clamp, repeat cycle, EQ band limits, cover placeholder etag. `status: todo` · deps: IT-T10, IT-T03 · **AC:** every zone endpoint returns documented status+body and captures exactly one `ZoneCommand`; zone 0→404; seek both→400 / exactly-one→200; volume parse/clamp; repeat cycles Off→All→One; EQ >10 bands→400 + band-edit clears preset; `GET …/cover`→200 PNG + ETag `"snapdog-placeholder"`.
- [x] `IT-T12` REST command-capture (zone actions + client vol/mute/latency) + EQ 400/422/404 + client-EQ-422-not-snapdog + client-speaker 404/422. (Speaker-apply-200 / zone-assign-fade need a SnapDog-client fixture — deferred.) `status: done` · deps: IT-T10.
- [x] `IT-T13` Media `[]`/index-404, client-speaker 404/422, KNX programming-mode 409 (device-mode inactive), system gaps. Network 200 paths out of scope. `status: done` · deps: IT-T10.
- [x] `IT-T14` Auth middleware 401 (no/wrong Bearer key; `/health` unauth) **+ caught/fixed a real auth-bypass** + **OpenAPI** structural contract. `status: done` · deps: IT-T10.

### Phase 2 — WebSocket suite
- [x] `IT-T20` All **7** notification variants emitted on the right mutation; serde `tag`/snake_case round-trip; **a compile-time exhaustiveness match over all 7 `Notification` variants** (catch silent add/rename, mirrors IT-T52). `status: todo` · deps: IT-T10, IT-T03.
- [x] `IT-T21` Ping cadence (30s via time::pause) + 65th conn→503 (real socket). Close-1001-on-shutdown is unreachable in prod → `IT-NG-08`. `status: done` · deps: IT-T20, IT-T03.

### Phase 3 — MQTT suite
- [x] `IT-T30` Routing/decode via `test_bridge`: 16 topics→captured cmds; volume 0–1 & 0–100; mute/repeat parse; client-zone validation. `status: done` · deps: IT-T01.
- [x] `IT-T31` Retained state JSON schema (`zones/{i}/state`,`clients/{i}/state`) + LWT online/offline + HA discovery payloads. `status: done` · deps: IT-T30.
- [x] `IT-T32` **(T2)** Real mosquitto via testcontainers: retained online, QoS1 retained state round-trip, LWT offline on ungraceful disconnect; loud-skip w/o Docker. `status: done` · deps: IT-T05, IT-T30.

### Phase 4 — KNX suite
- [x] `IT-T40` Routing/decode via `run_incoming`: GA action→command (repeat/track_repeat/presence_timeout-u16/shuffle/playlist_next/client-latency/client-zone happy path) + explicit-byte decode goldens incl. `decode_u16` + `build_*_ga_map`. `status: done` · deps: IT-T01.
- [x] `IT-T41` knx-rs-core dep contract: `GroupAddress` round-trip + DPT byte goldens + public 460-GO/ASAP export (`tests/knx_golden.rs`). `status: done` · deps: IT-T06, IT-T40.
- [x] `IT-T42` Publisher: `track_progress_pct` scaling + `publish_zone_state`/`publish_zone_track` status GOs with fixed DPTs (recording mock). `status: done` · deps: IT-T40.
- [x] `IT-T43` Device-mode deterministic core: `Bau.save()` envelope byte-layout + CRC golden + `resolve_go_update` asap→GA + parse/tables/persist. Live `DeviceServer::start_at(:0)` loopback + raw `CemiFrame` is a knx-rs dep contract (its own tests). `status: done` · deps: IT-T04, IT-T40.

### Phase 5 — snapcast contract firewall
- [x] `IT-T50` `SnapcastBackend` trait double (hand-coded no-op `MockBackend`) + `ZoneHarness`/`spawn_zone_harness` driving real `spawn_zone_players` (receivers off). `status: done` · deps: IT-T01.
- [x] `IT-T51` `reconcile_zone_groups` + pure helpers w/ `ServerStatus` fixtures; **sorted** `Group.SetClients`. `status: done` · deps: IT-T50.
- [x] `IT-T52` Event mapping `ServerEvent`→`SnapcastEvent` behavioral firewall (round-trip coverage + silently-dropped pins). Compile-time exhaustiveness N/A — `ServerEvent` is `#[non_exhaustive]` (foreign enum). `status: done` · deps: IT-T50.
- [x] `IT-T53` `GroupVolumeMode.effective()` table tests (Absolute/Relative/Compressed + clamp + max_volume). `status: todo` · deps: IT-T01.
- [x] `IT-T54` **Golden JSON-RPC vectors** for the 17 methods + the **line-delimited-JSON TCP fake** (`IT-DEC-06`); assert request ser + response de. `status: done` · deps: IT-T06, IT-T50.
- [x] `IT-T55` `send_audio` signature contract (compile-time) + behavioral PCM-injection path via `test_pcm_rx` seam → `CapturingBackend` (feature `test-harness`). `status: done` · deps: IT-T50.
- [ ] `IT-T56` **(T2)** Real snapserver via repaired `SnapserverHandle`: control + per-zone TCP audio source end-to-end. `status: todo` · deps: IT-T07, IT-T05.

### Phase 6 — Audio pipeline suite
- [ ] `IT-T60` Golden PCM vectors: resample (passthrough exact + 48k→24k) + EQ goldens **done**; sine/silence/pink **decode**-fixture chain hash deferred → `IT-NG-07` (rubato sinc not bit-exact). `status: in-progress` · deps: IT-T06.
- [x] `IT-T61` Fade math (pure `fade_gain`): monotonic gain ramp + sample count = `sample_rate*fade_ms/1000` for 0→1 and 1→0; `ZoneFade` total/zero-duration/per-frame. `status: done` · deps: IT-T01.
- [x] `IT-T62` EQ stability: deterministic filter-grid (random-equivalent, no proptest dep) → finite/bounded (NaN/Inf guard) + 0 dB identity + determinism. `status: done` · deps: IT-T01.
- [x] `IT-T63` Subsonic prefetch cache miss→fetch→hit (`wiremock`) + ICY metadata parse + cache LRU/eviction. `status: done` · deps: IT-T01.

### Phase 7 — AirPlay & Spotify seams
- [x] `IT-T70` AirPlay handler callback→`ReceiverEvent` mappers (`audio_init`→`SessionStarted`+PCM, metadata/coverart/progress/disconnect) on shairplay 0.5.0. RTSP-loopback e2e deferred → `IT-NG-09`. `status: done` · deps: IT-T01.
- [x] `IT-T71` AirPlay volume **golden** (0dB + slope + ±inf) + `RemoteCommand` 8-arm round-trip. AP2 SRP/pairing-store deferred → `IT-NG-09`. `status: done` · deps: IT-T70, IT-T06.
- [x] `IT-T72` Spotify `ChannelSink` f64→f32 (no rescale) + volume + `PlayerEvent`→`ReceiverEvent` mapper goldens (progress / Track-Episode-Local metadata / unhandled). `status: done` · deps: IT-T01.
- [x] `IT-T73` Feature **build-smoke matrix** (CI `build-smoke` job): embedded {default,minimal,full} + process {minimal,full} `cargo check`. `status: done` · deps: IT-T05.

### Phase 8 — State machine & lifecycle
- [x] `IT-T80` Zone-player transitions (track None iff Idle) + persistence roundtrip (restore subset, playback→Stopped). `status: done` · deps: IT-T01, IT-T04.
- [x] `IT-T81` Next/Prev/complete honoring repeat + shuffle + prev-restart >3s — pure `player::nav` extraction + in-source matrix (shuffle deterministic via injected draw). `status: done` · deps: IT-T80, IT-T02.
- [x] `IT-T82` `source_conflict` LastWins/ReceiverWins + command→state transitions + presence auto-off via `start_paused`+`advance` (ZoneHarness). `status: done` · deps: IT-T80, IT-T03.
- [ ] `IT-T83` Multi-zone isolation **done** (ZoneHarness); crash restart (`ZONE_RESTART_DELAY`) still blocked (no realistic `run()` Err path). `status: in-progress` · deps: IT-T80, IT-T03.
- [x] `IT-T84` Serve lifecycle: in-process `/health` + real ephemeral-port `api::serve` → health → graceful shutdown (cooperative future) → listener closed. Full `run_app(Config)` extraction deferred → `IT-NG-06`. `status: done` · deps: IT-T04, IT-T02.

### Phase 9 — CI & docs
- [x] `IT-T90` CI **tier-1 gate**: `cargo test --workspace` runs lib + tier-1 integration targets (always-green; nextest/retries deferred to IT-T05). `status: done` · deps: IT-T05, IT-T11, IT-T20, IT-T30, IT-T40, IT-T50, IT-T60.
- [ ] `IT-T91` **(T2)** CI tier-2 job: services (snapserver/navidrome/mosquitto via testcontainers), **loud-skip** when absent, **artifact capture** on failure. `status: todo` · deps: IT-T05, IT-T32, IT-T56.
- [ ] `IT-T92` OpenAPI contract step + coverage (`cargo-llvm-cov`) + thresholds + flake quarantine. `status: todo` · deps: IT-T14, IT-T90.
- [ ] `IT-T93` Docs: `tests/README` (tiers, how to run, how to add a test, golden-update flow) + test policy. `status: todo` · deps: IT-T90.
- [ ] `IT-T94` Export `testkit` reuse hooks for `BT-0001`/`LI-0002` (ReceiverEvent capture, time guard, ephemeral pool). `status: todo` · deps: IT-T01.

## 11. Definition of done (coverage goals)
- **Tier-1 is the gate**: green on every push, no Docker/network, no retries, runs in
  seconds (no wall-clock sleeps).
- **Every** mounted REST route (enumerated from the router, not a hardcoded count),
  all **7** WS variants, all **16** MQTT command topics + **2** state topics, and
  every KNX GA action have a tier-1 contract test.
- The **3 crate seams** each have a contract firewall (§9) that goes red on an API or
  wire-format change; the previously-dead integration suite is **revived** (`IT-T07`).
- The audio pipeline has golden PCM + fade + EQ-stability coverage.
- Tier-2 reproduces the formerly silent-skip Subsonic/MQTT/snapserver tests and
  **loud-skips** without Docker; tier-3 is documented (manual).
- CI runs nextest, captures failure artifacts, validates the OpenAPI contract, and
  build-smokes the feature matrix.

## 12. Progress-tracking protocol (for AI agents)
1. Pick a task whose `depends_on` are all `done`/`cancelled` (start with Phase 0).
2. Set it `◐ in-progress` (checkbox stays `[ ]`); mirror in IT-LEDGER; bump frontmatter `in_progress`.
3. Implement to the task's **AC**; reference the ID in commits (`test(...): … (IT-T..)`).
4. On completion: `[x]` + `status: done` + IT-LEDGER + frontmatter rollup; set RFC `status: in-progress` once any task starts, `done` when all non-cancelled tasks are done.
5. New work discovered mid-flight → add `IT-T9x`/`IT-T1xx` (don't reuse IDs); cuts → `status: cancelled` + reason.
6. Decisions that change → add a new `IT-DEC-*` superseding the old (mark old `superseded by …`); golden changes are reviewed diffs (`IT-DEC-07`).

## 13. Machine-readable task ledger

<!-- IT-LEDGER-START (authoritative status; agents update here + the checkboxes above) -->
```yaml
rfc: IT-0003
updated: 2026-06-28
tiers: { "1": deterministic-gate, "2": integration-docker-optional, "3": e2e-hardware-manual }
tasks:
  - { id: IT-T01, phase: 0, status: done, depends_on: [] }       # tests/common/mod.rs
  - { id: IT-T02, phase: 0, status: todo, depends_on: [IT-T01] }
  - { id: IT-T03, phase: 0, status: todo, depends_on: [IT-T01] }
  - { id: IT-T04, phase: 0, status: done, depends_on: [IT-T01] }   # build_test_app: TempDir + persist=None + mdns-off
  - { id: IT-T05, phase: 0, status: todo, depends_on: [] }
  - { id: IT-T06, phase: 0, status: todo, depends_on: [IT-T01] }
  - { id: IT-T07, phase: 0, status: in-progress, depends_on: [] }  # snapcast helpers repaired; tier-2 bodies need rewrite (see file TODO)
  - { id: IT-T08, phase: 0, status: done, depends_on: [] }   # ADR-019 (docs/architecture/decisions.md): stay pinned snapcast 0.16.1 until the seam firewall (IT-T52/T54/T55 + IT-T73 build-smoke) is complete+green, then upgrade as IT-NG-05 behind it; resolves the dangling README link too
  - { id: IT-T10, phase: 1, status: done, depends_on: [IT-T01] }   # api::build_router + TestApp::request (oneshot)
  - { id: IT-T11, phase: 1, status: done, depends_on: [IT-T10, IT-T03] }   # tests/rest_zones.rs (10 tests)
  - { id: IT-T12, phase: 1, status: done, depends_on: [IT-T10] }   # tests/rest_commands.rs: zone-action command capture (shuffle/repeat/toggle/seek abs+rel+400) + client vol/mute/latency capture (needs snapcast_id) + EQ 400(>10 bands)/422(serde-shape)/404 matrix + client-EQ-422-not-snapdog. Speaker-profile-apply-200 + zone-assign-fade paths need a snapdog-client fixture (deferred)
  - { id: IT-T13, phase: 1, status: done, depends_on: [IT-T10] }   # tests/rest_surfaces.rs: media playlists []/index-404, client-speaker 404/422, knx programming-mode 409 (device-mode inactive), system radios/version/name gaps. Network 200 paths (subsonic/spinorama) out of scope
  - { id: IT-T14, phase: 1, status: done, depends_on: [IT-T10] }   # tests/openapi_contract.rs (structural: 3.1.0/title/version + key paths + op-count floor 85 + component schemas) + tests/auth.rs (Bearer 401 without/wrong key, 200 with, /health unauth) — the auth test CAUGHT + now guards a real auth-bypass (Extension/middleware layer-order in build_router)
  - { id: IT-T20, phase: 2, status: done, depends_on: [IT-T10, IT-T03] }   # tests/ws.rs (serde + exhaustiveness + tap)
  - { id: IT-T21, phase: 2, status: done, depends_on: [IT-T20, IT-T03] }   # tests/ws_lifecycle.rs (real socket): keepalive ping-on-connect + 64-conn cap → 65th handshake 503; tests/ws_ping.rs (start_paused, isolated binary for the ACTIVE_CONNECTIONS global): 30s ping cadence via time::advance. close-1001-on-shutdown is unreachable in production (api::serve holds a NotifySender for the server lifetime → the broadcast never closes) → roadmap IT-NG-08
  - { id: IT-T30, phase: 3, status: done, depends_on: [IT-T01] }   # existing in-source mqtt routing tests: routes_zone_{volume,mute,control,playlist,seek} + routes_client_{volume,mute,zone_change}
  - { id: IT-T31, phase: 3, status: done, depends_on: [IT-T30] }   # zone+client state schema + HA-discovery payload golden + availability_topic==LWT topic (FIXED a snapdog//status double-slash bug). LWT runtime fire-on-disconnect = tier-2 (IT-T32).
  - { id: IT-T32, phase: 3, status: done, tier: 2, depends_on: [IT-T05, IT-T30] }   # tests/mqtt_tier2.rs: real mosquitto via testcontainers — retained "online", QoS1 retained zone-state round-trip, LWT "offline" on ungraceful disconnect (drop the bridge → broker fires LWT immediately). Loud-skips (no panic) when the Docker socket is absent. Verified passing against colima (DOCKER_HOST). dev-deps: testcontainers 0.25 + testcontainers-modules 0.13 (mosquitto)
  - { id: IT-T40, phase: 4, status: done, depends_on: [IT-T01] }   # knx/mod.rs in-source: handle_incoming routing via run_incoming for the uncovered actions (repeat/track_repeat→RepeatMode, presence_timeout→SetAutoOffDelay u16, shuffle, playlist_next, client latency, client zone-change happy path) + explicit-byte decode goldens incl. decode_u16 (zero coverage before) + build_zone/client_ga_map construction from config. (Existing module already covered play/volume/mute/toggle/playlist/dim/client.)
  - { id: IT-T41, phase: 4, status: done, depends_on: [IT-T06, IT-T40] }   # tests/knx_golden.rs: knx-rs-core dependency contract — GroupAddress 3-level round-trip (1/2/3↔0x0A03↔Display) + DPT byte goldens (1.001/5.001/7.x) + public-surface 460-GO/ASAP layout export. Catches a wire-format break on the knx-rs upgrade
  - { id: IT-T42, phase: 4, status: done, depends_on: [IT-T40] }   # knx/mod.rs in-source: track_progress_pct scaling golden + publish_zone_state status GOs with fixed DPTs (volume 5.001, mute/repeat 1.001, repeat-Playlist vs track-repeat-Track mutual exclusion, control_status==track_playing) + publish_zone_track (14-byte DPT16 title + scaled progress) via a recording KnxPublisher mock
  - { id: IT-T43, phase: 4, status: done, depends_on: [IT-T04, IT-T40] }   # device.rs deterministic core: Bau.save() envelope byte-layout golden (magic/version/LE-len/LE-crc32 — §9.2 drift guard) + resolve_go_update asap→GA translation + existing parse_ets_memory/build_tables/persist round-trip+corruption+version+truncation. Live DeviceServer::start_at(:0) loopback + raw CemiFrame is a knx-rs dependency contract (knx-rs-ip tunnel_integration.rs), out of snapdog scope
  - { id: IT-T50, phase: 5, status: done, depends_on: [IT-T01] }   # tests/common: hand-coded no-op SnapcastBackend double (MockBackend) + ZoneHarness/spawn_zone_harness driving real spawn_zone_players; runner emits snap cmds via snap_tx (captured there), backend.execute unused in the zone loop
  - { id: IT-T51, phase: 5, status: done, depends_on: [IT-T50] }   # build_* + ServerStatus golden + reconcile_zone_groups sorted Group.SetClients per diverged zone (FIXED unsorted HashMap-order wire payload)
  - { id: IT-T52, phase: 5, status: done, depends_on: [IT-T50] }   # SnapcastEvent+SnapcastCmd exhaustiveness (tests/snapcast.rs) + embedded ServerEvent→SnapcastEvent map coverage (embedded.rs: latency/name/custom-message type+payload extraction, group/stream collapse, silently-dropped StreamMeta/StreamControl pins). Compile-time exhaustiveness impossible — ServerEvent is #[non_exhaustive] (foreign), so behavioral firewall + documented caveat
  - { id: IT-T53, phase: 5, status: done, depends_on: [IT-T01] }   # tests/config_contract.rs (GroupVolumeMode + config)
  - { id: IT-T54, phase: 5, status: done, depends_on: [IT-T06, IT-T50] }   # tests/snapcast_rpc.rs: line-delimited-JSON TCP fake + golden vectors for ALL 17 JSON-RPC methods (incl. mute/streamUri traps) + framing + response-deser
  - { id: IT-T55, phase: 5, status: done, depends_on: [IT-T50] }   # tests/zone_player.rs: send_audio signature contract (compile-time, default gate) + behavioral PCM-injection via test_pcm_rx seam → CapturingBackend (feature test-harness); embedded F32AudioSender drift caught by embedded.rs compile
  - { id: IT-T56, phase: 5, status: todo, tier: 2, depends_on: [IT-T07, IT-T05] }
  - { id: IT-T60, phase: 6, status: in-progress, depends_on: [IT-T06] }   # f32→PCM golden + resample (passthrough exact-identity, 48k→24k ≈half within band, all-finite) + EQ goldens done; symphonia decode-fixture golden (sine/silence/pink) + full-chain hash deferred (rubato sinc not bit-exact) → IT-NG-07
  - { id: IT-T61, phase: 6, status: done, depends_on: [IT-T01] }   # snapdog-common fade_gain: monotonic/complementary/bounded ramp; runner.rs ZoneFade: total = sr*ms/1000 golden, zero-duration passthrough, per-frame stereo gain
  - { id: IT-T62, phase: 6, status: done, depends_on: [IT-T01] }   # audio/eq.rs: 0dB-peaking≈identity, bit-identical determinism, deterministic filter-grid (5 types × freq/gain/q) NaN/Inf guard (grid instead of proptest — no new dep)
  - { id: IT-T63, phase: 6, status: done, depends_on: [IT-T01] }   # ICY parse (icy.rs parse_icy_metadata + helpers.rs parse_icy_title) + TrackCache hit/miss/LRU/eviction (cache.rs ~15 tests) already covered; added wiremock subsonic prefetch_one miss→fetch→hit end-to-end (first wiremock use)
  - { id: IT-T70, phase: 7, status: done, depends_on: [IT-T01] }   # airplay.rs in-source: handler callback→ReceiverEvent mappers on shairplay 0.5.0 — audio_init→SessionStarted + PCM forward, on_metadata (+all-None), on_coverart, on_progress (44.1k→ms), on_client_disconnected→SessionEnded. RTSP-loopback e2e (RaopServer::port(0)+send_rtsp, #[serial]) deferred → IT-NG-09
  - { id: IT-T71, phase: 7, status: done, depends_on: [IT-T70, IT-T06] }   # airplay.rs in-source: volume golden (0dB + slope -7.5→75/-22.5→25 + ±inf) + RemoteCommand 8-arm round-trip to shairplay via a Fake RemoteControl. AP2 SRP/pairing-store + RemoteAvailable-via-loopback deferred → IT-NG-09
  - { id: IT-T72, phase: 7, status: done, depends_on: [IT-T01] }   # spotify.rs in-source: ChannelSink f64→f32 no-rescale + volume + PlayerEvent→ReceiverEvent mapper goldens (progress Playing/Paused/Seeked/PositionCorrection; TrackChanged Track/Episode/Local metadata; unhandled→nothing) on librespot 0.8
  - { id: IT-T73, phase: 7, status: done, depends_on: [IT-T05] }   # ci.yml build-smoke job: flat matrix (embedded default/minimal/full + process minimal/full) cargo check — gross-signature firewall across feature combos. All 12 combos verified to compile locally; the embedded XOR process compile_error! guards keep it a flat include-list (no --all-features)
  - { id: IT-T80, phase: 8, status: done, depends_on: [IT-T01, IT-T04] }   # state/mod.rs: persist/load roundtrip + transient-reset (existing 5 tests) + SourceType/PlaybackState wire-format golden; track-None-iff-Idle holds via reset
  - { id: IT-T81, phase: 8, status: done, depends_on: [IT-T80, IT-T02] }   # extracted pure player::nav (next_index/prev_index/complete_index + radio wrap) from helpers.rs handlers; in-source matrix covers repeat Off/Track/Playlist, end-of-list stop/wrap, CD-player >3s prev, Complete-vs-Next asymmetry, shuffle determinism via injected draw (no fastrand in tested path). Subsonic behavioral path needs network (out of scope)
  - { id: IT-T82, phase: 8, status: done, depends_on: [IT-T80, IT-T03] }   # source_conflict may_start_local_playback matrix + command->state transitions (ZoneHarness) + presence auto-off timer via start_paused+tokio::time::advance (tests/zone_player.rs presence_auto_off_stops_zone_after_delay; direct-seed precondition, zone_presence_changed fire barrier)
  - { id: IT-T83, phase: 8, status: in-progress, depends_on: [IT-T80, IT-T03] }   # multi-zone isolation DONE (tests/zone_player.rs transitions_are_isolated_per_zone via ZoneHarness); crash-restart still blocked (inline in spawn closure; run() has no realistic Err path)
  - { id: IT-T84, phase: 8, status: done, depends_on: [IT-T04, IT-T02] }   # api::serve gained a cooperative shutdown future + with_graceful_shutdown (plain HTTP); tests/headless_boot.rs: in-process /health oneshot + real loopback ephemeral-port serve→/health→graceful-shutdown→listener-closed. Full run_app(Config) extraction deferred by choice → roadmap IT-NG-06 (entry-point blast radius); this covers the genuinely-untested serve+shutdown-over-socket path
  - { id: IT-T90, phase: 9, status: done, depends_on: [IT-T05, IT-T11, IT-T20, IT-T30, IT-T40, IT-T50, IT-T60] }   # ci.yml unit-tests: cargo test --lib -> --workspace (runs tier-1 integration); nextest/retries = IT-T05
  - { id: IT-T91, phase: 9, status: todo, tier: 2, depends_on: [IT-T05, IT-T32, IT-T56] }
  - { id: IT-T92, phase: 9, status: todo, depends_on: [IT-T14, IT-T90] }
  - { id: IT-T93, phase: 9, status: todo, depends_on: [IT-T90] }
  - { id: IT-T94, phase: 9, status: todo, depends_on: [IT-T01] }
```
<!-- IT-LEDGER-END -->

## 14. Open questions

**Blocking prerequisites (resolve first):**
- **Snapcast pin** (`IT-DEC-11`, `IT-T08`): adopt local `snapcast-rs` 0.17 (path-deps +
  the `snapcast-tests` harness) or keep 0.16.1 + own golden vectors? The suite is
  designed to work either way, but tier-2 reuse of `snapcast-tests` depends on it.
- **`SnapcastClient` new contract** (`IT-T07`): confirm `sync_initial_state` is the
  replacement for the removed `init()/state()` before reviving `integration.rs`.

**Oracle gaps (characterization, `IT-DEC-12`) — record current behavior, flag for spec:**
`TrackInfo.track_index` 0- vs 1-based in `ZoneChanged`; `cover_url` absolute vs
relative; `PlaybackError.recoverable` heuristic; whether speaker-correction and zone
EQ compose or are exclusive; whether `max_volume` is enforced in `GroupVolumeMode`
scaling or only at Snapcast set-volume; two concurrent AirPlay clients on one zone;
`Group.SetClients` with empty list (dissolve vs pending); custom-protocol `type_id`
range + `CustomMessage` size limit; mid-stream sample-rate change (44.1k AirPlay →
48k Snapcast).

## 15. Roadmap / out of scope (deferred)
- **snapcast 0.17 migration** (`IT-NG-05`) — separate work; this suite is its safety net.
- **Tier-3 hardware e2e** (`IT-NG-01`) — real KNX/IP gateway, physical AirPlay/Spotify,
  real audio out; documented manual runbook, never CI.
- **Concurrency model checking** with `loom` (zone-player + shared state) — stretch.
- **Performance / latency-SLA** (`IT-NG-03`) — MQTT/KNX→audio budget, separate RFC.
- **`cargo public-api` / semver-checks** on the 3 deps as an early-warning CI step.
- **Full headless boot via `start_system(Config)`** (`IT-NG-06`, deferred from `IT-T84`) —
  extract a Cli-independent `start_system(config) -> StartedSystem { addr, shutdown, handle }`
  from `run_app` (cut after `Arc::new(app_config)`), with a cooperative shutdown token
  replacing the signal-only `process::exit` path, logging-init + the force-exit watchdog
  kept in `run_app`, and an `AppConfig.start_receivers` (`#[serde(skip)]`, default `true`)
  seam to silence receivers. Unlocks an end-to-end real-boot test (embedded snapcast on
  ephemeral ports, mDNS off, no MQTT/KNX) covering full subsystem assembly + teardown.
  Deferred for production-entry-point blast radius; `IT-T84` shipped the
  serve + graceful-shutdown-over-socket path instead.
- **WebSocket graceful-close on shutdown** (`IT-NG-08`, latent, found in `IT-T21`) — the
  `handle_socket` 1001 ("Going Away") close only fires when the notification broadcast
  channel closes, but `api::serve` keeps a `NotifySender` in `AppState` for the whole
  server lifetime, so a real shutdown never closes the channel — live WS clients get an
  abrupt TCP drop, not a clean 1001. Fix would thread the cooperative shutdown token
  (added in `IT-T84`) into `handle_socket` to send 1001 before teardown. Low severity
  (clients reconnect); flagged for a follow-up.
- **AirPlay RTSP-loopback + AP2-SRP receiver tests** (`IT-NG-09`, deferred from `IT-T70`/`IT-T71`) —
  drive the callback→`ReceiverEvent` mappers through a real loopback `RaopServer::builder().port(0)`
  + `send_rtsp` (SET_PARAMETER volume/metadata/coverart/progress; SETUP+DACP → `RemoteAvailable`),
  plus the AP2 `FilePairingStore` round-trip. Needs the `#[serial]` + `CI=1` (mDNS-off) loopback
  harness and the shairplay 0.6 pairing-store APIs (`load_identity`/`save_identity`). The pure
  callback mappers are already covered (`IT-T70`/`IT-T71`); this is the end-to-end layer. Also file
  the `on_progress` `current - start` u32 underflow (airplay.rs) — guard with `saturating_sub`.
- **Decode-fixture audio-chain golden** (`IT-NG-07`, deferred from `IT-T60`) — golden
  vectors for `symphonia` decode of canonical fixtures (sine/silence/pink in FLAC/MP3)
  through the full decode→resample→EQ chain. Whole-stream hashing is not bit-exact on
  the rubato sinc path (f32→f64→f32 + warm-up), so this needs a tolerance/feature-fingerprint
  approach + committed fixtures. `IT-T60` already covers the resample + EQ stages as units
  (passthrough exact-identity, 48k→24k ≈half, all-finite; EQ identity/determinism/grid).
