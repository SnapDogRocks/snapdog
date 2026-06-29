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
  done: 6
  in_progress: 7
  todo: 34
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
- [ ] `IT-T08` Resolve the snapcast **0.16.1-vs-0.17.0 pin** (`IT-DEC-11`): record decision in an ADR; add a feature build-smoke matrix entry. `status: todo` · deps: —.

### Phase 1 — REST contract suite
- [x] `IT-T10` In-process REST driver: `oneshot` on `Router` + mock `AppState` (captured `ZoneCommand`/`SnapcastCmd` mpsc + broadcast tap). `status: todo` · deps: IT-T01 · **AC:** a GET returns 200 with no TCP socket; the suite **enumerates and asserts every mounted route group** (no hardcoded endpoint count).
- [x] `IT-T11` Zone endpoints contract (all): status+body+**exactly-one** command; boundaries (zone 0→404), seek XOR(→400), volume parse/clamp, repeat cycle, EQ band limits, cover placeholder etag. `status: todo` · deps: IT-T10, IT-T03 · **AC:** every zone endpoint returns documented status+body and captures exactly one `ZoneCommand`; zone 0→404; seek both→400 / exactly-one→200; volume parse/clamp; repeat cycles Off→All→One; EQ >10 bands→400 + band-edit clears preset; `GET …/cover`→200 PNG + ETag `"snapdog-placeholder"`.
- [ ] `IT-T12` Client endpoints contract: vol/mute/latency, zone-assign validation(+fade), EQ 422 if not snapdog, **GET/PUT `…/{client}/speaker`** (profile retrieval/apply, 404 unknown profile, validation error). `status: todo` · deps: IT-T10.
- [ ] `IT-T13` Media/Speakers/System/KNX(409 client-mode)/Health contract. `status: todo` · deps: IT-T10.
- [ ] `IT-T14` Auth middleware (401 w/o key) + **OpenAPI** response-schema contract validation. `status: todo` · deps: IT-T10.

### Phase 2 — WebSocket suite
- [x] `IT-T20` All **7** notification variants emitted on the right mutation; serde `tag`/snake_case round-trip; **a compile-time exhaustiveness match over all 7 `Notification` variants** (catch silent add/rename, mirrors IT-T52). `status: todo` · deps: IT-T10, IT-T03.
- [ ] `IT-T21` Ping cadence (30s via time::pause), close 1001 on shutdown, 65th conn→503. `status: todo` · deps: IT-T20, IT-T03.

### Phase 3 — MQTT suite
- [ ] `IT-T30` Routing/decode via `test_bridge`: 16 topics→captured cmds; volume 0–1 & 0–100; mute/repeat parse; client-zone validation. `status: todo` · deps: IT-T01.
- [ ] `IT-T31` Retained state JSON schema (`zones/{i}/state`,`clients/{i}/state`) + LWT online/offline + HA discovery payloads. `status: todo` · deps: IT-T30.
- [ ] `IT-T32` **(T2)** Real mosquitto via testcontainers: reconnect, QoS1 retained, LWT on ungraceful disconnect; loud-skip w/o Docker. `status: todo` · deps: IT-T05, IT-T30.

### Phase 4 — KNX suite
- [ ] `IT-T40` Routing/decode via `run_incoming`: every GA action→command; DPT decode bool/percent/u8/u16/dim-stepcode; unmapped GA ignored. `status: todo` · deps: IT-T01.
- [ ] `IT-T41` DPT/GA golden + **reuse `knx-rs-core` golden vectors** as dep-contract; `GroupAddress` round-trip; ASAP layout + 460-GO assert. `status: todo` · deps: IT-T06, IT-T40.
- [ ] `IT-T42` Publisher: status GOs on each notification w/ fixed DPTs; `track_progress` scaling. `status: todo` · deps: IT-T40.
- [ ] `IT-T43` Device-mode: `DeviceServer::start_at(:0)` loopback + raw `CemiFrame` exchange + `Bau.save()` byte-stability + CRC; prog-mode endpoint device-mode. `status: todo` · deps: IT-T04, IT-T40.

### Phase 5 — snapcast contract firewall
- [ ] `IT-T50` `SnapcastBackend` trait double (mockall/hand-coded) capturing `execute(SnapcastCmd)` + injecting `SnapcastEvent`. `status: todo` · deps: IT-T01.
- [ ] `IT-T51` `reconcile_zone_groups` + pure helpers w/ `ServerStatus` fixtures; **sorted** `Group.SetClients`. `status: in-progress` · deps: IT-T50.
- [ ] `IT-T52` Event roundtrip `ServerEvent`→`SnapcastEvent`→state+WS; **exhaustiveness test fails on unmapped variant**. `status: todo` · deps: IT-T50.
- [x] `IT-T53` `GroupVolumeMode.effective()` table tests (Absolute/Relative/Compressed + clamp + max_volume). `status: todo` · deps: IT-T01.
- [ ] `IT-T54` **Golden JSON-RPC vectors** for the 17 methods + the **line-delimited-JSON TCP fake** (`IT-DEC-06`); assert request ser + response de. `status: in-progress` · deps: IT-T06, IT-T50.
- [ ] `IT-T55` Embedded `F32AudioSender::send(&[f32])` signature contract + `send_audio` path (inline mock server). `status: todo` · deps: IT-T50.
- [ ] `IT-T56` **(T2)** Real snapserver via repaired `SnapserverHandle`: control + per-zone TCP audio source end-to-end. `status: todo` · deps: IT-T07, IT-T05.

### Phase 6 — Audio pipeline suite
- [ ] `IT-T60` Golden PCM vectors: sine/silence/pink → decode→resample→EQ → hashed (tolerance); canonical fixtures. `status: todo` · deps: IT-T06.
- [ ] `IT-T61` Fade math (pure `fade_gain`): assert monotonic gain ramp + sample count = `sample_rate*fade_ms/1000` (±1) for 0→1 and 1→0 at a fixed `sample_rate`. `status: todo` · deps: IT-T01 · **AC:** ramp monotonic; count exact (±1); pure function — no tokio time needed.
- [ ] `IT-T62` EQ stability: `proptest` random audio+filter params → finite/bounded (NaN/Inf guard). `status: todo` · deps: IT-T01.
- [ ] `IT-T63` Subsonic prefetch cache hit vs miss + ICY metadata parse (`wiremock`). `status: todo` · deps: IT-T01.

### Phase 7 — AirPlay & Spotify seams
- [ ] `IT-T70` shairplay contract: `TestHandler` + `RaopServer::builder().port(0)` loopback + `send_rtsp`; `audio_init`→`SessionStarted`, volume/metadata/coverart→`ReceiverEvent`. `status: todo` · deps: IT-T01.
- [ ] `IT-T71` AirPlay volume **golden (corrected)** + `RemoteCommand` round-trip + AP2 SRP/`MemoryPairingStore`. `status: todo` · deps: IT-T70, IT-T06.
- [ ] `IT-T72` Spotify `ChannelSink` f64→f32 cast (assert librespot 0.8 samples are already normalized — **no** rescaling) + `PlayerEvent`→`ReceiverEvent` mapper (pure fns) + volume vectors. `status: todo` · deps: IT-T01.
- [ ] `IT-T73` Feature **build-smoke matrix**: `embedded`/`process` × `ap2` on/off × `spotify` on/off compile. `status: todo` · deps: IT-T05.

### Phase 8 — State machine & lifecycle
- [ ] `IT-T80` Zone-player transitions (track None iff Idle) + persistence roundtrip (restore subset, playback→Stopped). `status: todo` · deps: IT-T01, IT-T04.
- [ ] `IT-T81` Next/Prev/complete honoring repeat + shuffle (seeded) + prev-restart >3s. `status: todo` · deps: IT-T80, IT-T02.
- [ ] `IT-T82` Presence (fixed clock + auto-off via time::pause) + `source_conflict` LastWins/ReceiverWins. `status: todo` · deps: IT-T80, IT-T03.
- [ ] `IT-T83` Multi-zone isolation + crash restart (`ZONE_RESTART_DELAY` via time control); loom as a stretch note. `status: todo` · deps: IT-T80, IT-T03.
- [ ] `IT-T84` Headless boot `run_app(cfg)` (mdns off, ephemeral ports, TempEnv) + health endpoints + graceful shutdown. `status: todo` · deps: IT-T04, IT-T02.

### Phase 9 — CI & docs
- [ ] `IT-T90` CI **tier-1 gate**: `nextest run` (lib + tier-1 integration), always-green, no retries. `status: todo` · deps: IT-T05, IT-T11, IT-T20, IT-T30, IT-T40, IT-T50, IT-T60.
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
  - { id: IT-T08, phase: 0, status: todo, depends_on: [] }
  - { id: IT-T10, phase: 1, status: done, depends_on: [IT-T01] }   # api::build_router + TestApp::request (oneshot)
  - { id: IT-T11, phase: 1, status: done, depends_on: [IT-T10, IT-T03] }   # tests/rest_zones.rs (10 tests)
  - { id: IT-T12, phase: 1, status: in-progress, depends_on: [IT-T10] }   # tests/rest_surfaces.rs: client GET contract+boundaries; cmd-capture/EQ-422 pending
  - { id: IT-T13, phase: 1, status: in-progress, depends_on: [IT-T10] }   # tests/rest_surfaces.rs: system+health done; media/speakers/knx-409 pending
  - { id: IT-T14, phase: 1, status: todo, depends_on: [IT-T10] }
  - { id: IT-T20, phase: 2, status: done, depends_on: [IT-T10, IT-T03] }   # tests/ws.rs (serde + exhaustiveness + tap)
  - { id: IT-T21, phase: 2, status: todo, depends_on: [IT-T20, IT-T03] }
  - { id: IT-T30, phase: 3, status: todo, depends_on: [IT-T01] }
  - { id: IT-T31, phase: 3, status: todo, depends_on: [IT-T30] }
  - { id: IT-T32, phase: 3, status: todo, tier: 2, depends_on: [IT-T05, IT-T30] }
  - { id: IT-T40, phase: 4, status: todo, depends_on: [IT-T01] }
  - { id: IT-T41, phase: 4, status: todo, depends_on: [IT-T06, IT-T40] }
  - { id: IT-T42, phase: 4, status: todo, depends_on: [IT-T40] }
  - { id: IT-T43, phase: 4, status: todo, depends_on: [IT-T04, IT-T40] }
  - { id: IT-T50, phase: 5, status: todo, depends_on: [IT-T01] }
  - { id: IT-T51, phase: 5, status: in-progress, depends_on: [IT-T50] }   # tests/snapcast_rpc.rs: build_* + ServerStatus golden; reconcile_zone_groups pending
  - { id: IT-T52, phase: 5, status: in-progress, depends_on: [IT-T50] }   # tests/snapcast.rs: SnapcastEvent+SnapcastCmd exhaustiveness; ServerEvent map + golden JSON-RPC pending (process feature)
  - { id: IT-T53, phase: 5, status: done, depends_on: [IT-T01] }   # tests/config_contract.rs (GroupVolumeMode + config)
  - { id: IT-T54, phase: 5, status: in-progress, depends_on: [IT-T06, IT-T50] }   # tests/snapcast_rpc.rs: line-delimited-JSON TCP fake + golden vectors 6/17 (incl. mute/streamUri traps) + framing/response-deser; remaining 11 mechanical
  - { id: IT-T55, phase: 5, status: todo, depends_on: [IT-T50] }
  - { id: IT-T56, phase: 5, status: todo, tier: 2, depends_on: [IT-T07, IT-T05] }
  - { id: IT-T60, phase: 6, status: in-progress, depends_on: [IT-T06] }   # tests/audio.rs: f32→PCM golden done; full decode→resample→EQ chain pending
  - { id: IT-T61, phase: 6, status: todo, depends_on: [IT-T01] }
  - { id: IT-T62, phase: 6, status: todo, depends_on: [IT-T01] }
  - { id: IT-T63, phase: 6, status: todo, depends_on: [IT-T01] }
  - { id: IT-T70, phase: 7, status: todo, depends_on: [IT-T01] }
  - { id: IT-T71, phase: 7, status: todo, depends_on: [IT-T70, IT-T06] }
  - { id: IT-T72, phase: 7, status: todo, depends_on: [IT-T01] }
  - { id: IT-T73, phase: 7, status: todo, depends_on: [IT-T05] }
  - { id: IT-T80, phase: 8, status: todo, depends_on: [IT-T01, IT-T04] }
  - { id: IT-T81, phase: 8, status: todo, depends_on: [IT-T80, IT-T02] }
  - { id: IT-T82, phase: 8, status: todo, depends_on: [IT-T80, IT-T03] }
  - { id: IT-T83, phase: 8, status: todo, depends_on: [IT-T80, IT-T03] }
  - { id: IT-T84, phase: 8, status: todo, depends_on: [IT-T04, IT-T02] }
  - { id: IT-T90, phase: 9, status: todo, depends_on: [IT-T05, IT-T11, IT-T20, IT-T30, IT-T40, IT-T50, IT-T60] }
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
