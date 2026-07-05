---
rfc: KEA-0004
title: Applying ETS-programmed parameters to snapdog's running config
status: in-progress      # draft | accepted | in-progress | done | superseded
version: 0.2.0           # v0.2: Option A boot-time apply + self-restart landed (KEA-T2/T3/T4); state-seed + merge deferred
created: 2026-07-05
updated: 2026-07-05
target_repo: snapdog
target_branch: main
related: []
owners: [metaneutrons]
---

# RFC KEA-0004 — Applying ETS-programmed parameters to snapdog

> **For AI agents:** this is a **scoping** RFC, not yet accepted. It maps what it
> would take to make snapdog actually *use* the parameters ETS programs into its
> KNX device memory (today they are parsed and thrown away). Requirements are
> `KEA-REQ-*`, decisions `KEA-DEC-*`, tasks `KEA-T*`. Line numbers are approximate
> (verify before editing); **symbol names are the stable anchor.** Nothing here is
> implemented; the only shipped groundwork is the persist-on-restart hook
> (`fix(knx): persist ETS programming immediately on A_Restart`).

## 1. Summary

snapdog can run as a KNX **device** (tunnel server) that ETS programs. A download
lands a 2903-byte parameter segment in the device's memory, and snapdog already
parses it into an `EtsParams` struct (`knx/device.rs:150` / `:192`
`parse_ets_memory`) — zone/client/radio enable flags, volumes, names, MACs, zone
assignment, presence, `http_port`, `log_level`, subsonic + MQTT endpoints.

**But that `EtsParams` is discarded** (`knx/mod.rs:120` binds it as `_ets_params`).
So an ETS-programmed device is fully *programmable* (as of the knx-rs 0.4 bump)
but its parameters have **no effect** — snapdog runs entirely off the TOML
`AppConfig`. `main.rs:198` already declares the intent ("--knx-device without
--config: start with defaults, ETS provides config") but nothing implements it.

This RFC scopes closing that gap. The headline findings:

- The **core missing piece** (needed by every option) is a converter
  `EtsParams → config`. None exists, and it is **not** a 1:1 map — several
  `EtsParams` fields have **no home** in `AppConfig` at all.
- **Boot-time apply** (use ETS memory as the config on startup) is **medium**
  effort and architecturally clean. **Recommended.**
- **Live in-session apply** (reconfigure a running system when ETS reprograms
  mid-run) is **very-large**: there is **no hot-reload primitive anywhere** in
  snapdog — config is an immutable `Arc<AppConfig>` cloned into every subsystem —
  so it means re-seeding state and respawning zone players / receivers / the
  snapcast backend. Not recommended as a first step.

## 2. Goals / Non-goals

### Goals (`KEA-REQ-*`)
- **KEA-REQ-1** — An ETS-programmed device applies its parameters without hand-written TOML.
- **KEA-REQ-2** — The apply path reuses the existing config validation + convention resolution (`config::load_raw*`), not a parallel code path.
- **KEA-REQ-3** — Malformed ETS input (bad MAC/URL, truncated names, all-inactive) degrades gracefully, never panics or wedges startup.
- **KEA-REQ-4** — A defined, documented precedence when both `--config` and persisted ETS memory exist.

### Non-goals
- Live reconfiguration of a running audio system (deferred; see §5.2).
- Per-zone audio format from ETS (`ZONE_SRATE`/`ZONE_BITD` exist in memory but `parse_ets_memory` never reads them, and `AudioConfig` is global — `group_objects.rs:656-658`, `types.rs:419`).
- Any change to the KNX group-address wiring — in ETS-programmed mode the addr/assoc tables come from BAU memory, and TOML KNX GAs are already ignored (`device.rs:353`). `EtsParams` carries **zero** group addresses.

## 3. Background — verified facts (cite-checked)

### 3.1 What `EtsParams` gives us
Flat, fixed-size parallel arrays (`device.rs:150-178`): `zone_active[10]`,
`zone_names`, `zone_default_volume`, `zone_max_volume`, `zone_airplay`,
`zone_spotify`, `zone_presence_enabled/timeout`; `client_active[10]`,
`client_names`, `client_macs`, `client_default_zone` (a **u8 index**),
`client_default_volume`, `client_max_volume`, `client_default_latency`;
`radio_active[20]`/`radio_names`/`radio_urls`; `http_port`, `log_level` (a raw
enum byte), `subsonic_url/user/pass`, `mqtt_broker/topic`.

### 3.2 Current state: parsed, then dropped
`start_device_transport` returns `Option<EtsParams>` (`device.rs:310,363`);
`start_device` discards it (`knx/mod.rs:120`) and never propagates it out of the
KNX module (`knx::start` returns `Result<Option<DeviceControlHandle>>`). The value
is computed at exactly the right place but goes nowhere.

### 3.3 Config architecture (the constraint that shapes everything)
- Two struct layers: **`FileConfig`** (raw TOML) → `config::load_raw*` runs
  convention resolution (sink/stream_name/tcp_source_port/airplay_name/zone_index)
  + validation → **`AppConfig`** (resolved). Hand-building `AppConfig` would
  duplicate all convention logic and skip validation (`config/convention.rs:16-67`,
  `config/mod.rs:36-151`). **Convert to `FileConfig`, not `AppConfig`.**
- Config becomes an immutable `Arc<AppConfig>` at `main.rs:275` and is cloned/moved
  by value or `Arc` into every subsystem. **There is no `ArcSwap`/`RwLock<AppConfig>`,
  no SIGHUP, no reload command, no config-mutation channel** anywhere in `src/`.
- The KNX device is spawned **last** (`main.rs:430-446`), after state store
  (`:347`), snapcast backend (`:374-383`), zone players + receivers (`:390`). So
  `EtsParams` is currently produced *after* every subsystem that would consume it.
- The state store seeds zone/client volume from a hardcoded `DEFAULT_VOLUME = 50`
  and latency `0` (`state/mod.rs:264,303-307`), **not** from config — so some ETS
  fields have no config path even in principle (see §4.2).

## 4. The core gap: no `EtsParams → config` converter

### 4.1 Fields that map cleanly → `FileConfig`
`http_port → http.port`; `log_level (u8) → SystemConfig.log_level` (needs a
`u8→LogLevel` mapping — **KEA-DEC-2**); `subsonic_* → Option<SubsonicConfig>` (only
if URL non-empty; `password` is `SecretString`); `mqtt_* → Option<MqttConfig>`
(only if broker non-empty; must append the trailing `/` to `base_topic` that the
TOML deserializer adds); `radio_active/names/urls → Vec<RawRadioConfig>` (filter by
active); `zone_active/zone_names → Vec<RawZoneConfig>{name}`;
`zone_presence_* → RawZoneConfig.presence`; `client_active/names/macs/max_volume →
Vec<RawClientConfig>`. **Gotcha:** `client_default_zone` is a **u8 index** but
`RawClientConfig.zone` is a zone **name string** — translate via `zone_names[idx]`
and handle out-of-range/inactive indices (`convention.rs:47-56`).

### 4.2 Fields with NO config home (the hard part)
These `EtsParams` fields have **no** field in `RawZoneConfig`/`ZoneConfig`/
`RawClientConfig`/`ClientConfig`/`RadioConfig` today:

| EtsParams field | Reality |
|---|---|
| `zone_default_volume`, `client_default_volume` | State seed only — store hardcodes `DEFAULT_VOLUME=50` (`state/mod.rs:264,303-305`) |
| `client_default_latency` | State seed only — hardcoded `0` (`state/mod.rs:307`) |
| `zone_max_volume` | No zone-level `max_volume` exists (only per-client) |
| `zone_airplay`, `zone_spotify` (per-zone enable) | AirPlay/Spotify are **global** receivers with only a per-zone name; no per-zone enable concept exists (`player/runner.rs:296-330`) |
| `radio_active` | `RadioConfig` has no `active` flag |

**Decision forced (`KEA-DEC-3`):** applying these requires *either* extending the
config structs *or* seeding the store / receiver-start directly from `EtsParams`.
This is the single biggest work item and is shared by every option.

## 5. Options

### 5.1 Option A — boot-time apply (**recommended**) — *medium*
On startup, if persisted ETS memory exists, turn it into the `AppConfig` **before**
subsystems spawn; the whole downstream is then unchanged (every subsystem
transparently gets the ETS-derived config). Matches `main.rs:198`'s stated intent
and the shipped persist-on-restart + "restart snapdog to apply" UX.

Work: (1) the `EtsParams → FileConfig` converter (§4); (2) lift the
load-memory + `bau.restore` + `parse_ets_memory` path out of `start_device_transport`
into a standalone helper callable before `state::init` (`main.rs:~199`) — the parse
is a pure function of the memory bytes (`device.rs:192-261`); (3) merge policy vs
TOML (**KEA-DEC-1**); (4) decide the §4.2 state-seed fields.

Constraints: `DeviceServer::start` binds UDP 3671 as a side-effect (`device.rs:314`)
— avoid a double-bind (split "start server + parse EtsParams" early from "spawn
publisher/listener bridge" late, which needs `store`/`zone_commands`). Persisted
`zones.json` overlays store state (`main.rs:347`) and can shadow ETS-changed
identity — needs an invalidate/migrate story (**KEA-DEC-4**).

### 5.2 Option B — live in-session apply — *very-large*
Reconfigure already-running subsystems when ETS reprograms mid-run. Blocked by the
immutable-`Arc` architecture: every subsystem owns an independent config snapshot
with no re-supply channel. Per-subsystem reality:

| Subsystem | Live-reconfig | Note |
|---|---|---|
| Zone volume/name | moderate | existing `ZoneCommand`/`SnapcastCmd` + `SharedState`; `max_volume` still baked at init |
| Add/remove zone/client, airplay/spotify toggle | **needs-restart** | no supervisor API to spawn/kill a zone task; receivers gated on global config at spawn |
| Snapcast (streams/codec/port) | **needs-restart** | streams fixed at `SnapServer::new`; can retarget existing groups, can't create a stream for a new zone (`snapcast/embedded.rs:45-114`) |
| Subsonic | moderate | client cheap to rebuild, but created from captured `Arc` inside each zone task |
| MQTT | **needs-restart** | broker/topic/client baked at `connect()`; a bounded reconnect is net-new code |
| HTTP API port | **needs-restart** | listener pre-bound at `main.rs:299-307` |

### 5.3 Option C — hybrid
Boot-time apply (A) for everything, **plus** a bounded live path for the *cheap
tunables only* (zone/client volume + name via existing channels), forcing a restart
for structural changes. Sensible **iff** ETS reprograms in practice change tunables
far more often than structure (**KEA-DEC-5**).

## 6. Recommendation

1. **Ship Option A (boot-time apply).** It delivers KEA-REQ-1..4, reuses validation,
   and the restart-based UX is already in place (persist-on-`A_Restart`).
2. For the §4.2 state-seed fields, **seed the store from `EtsParams`** rather than
   growing `AppConfig` with runtime-only fields — `state::init` already takes
   `&AppConfig` and is the natural insertion point (`main.rs:347`, `state/mod.rs:252-314`).
   (Revisit per **KEA-DEC-3**.)
3. Defer Option B. Reconsider a **Hybrid (C)** live-tunables path only after A ships
   and only if usage data justifies it.

## 7. Open decisions (`KEA-DEC-*`)

- **KEA-DEC-1** — Precedence when both `--config` and ETS memory exist: ETS-wins, TOML-wins, or merge (TOML base + ETS override)? `main.rs:198` implies ETS only when no `--config`; the persist-on-restart stopgap implies ETS should win. **Recommend: merge onto a base — ETS fields override, empty ETS fields fall back to TOML/defaults.**
- **KEA-DEC-2** — `log_level` u8→`LogLevel` ordering (what integer ETS assigns trace/debug/info/warn/error).
- **KEA-DEC-3** — §4.2 fields: extend config structs vs seed the store directly.
- **KEA-DEC-4** — Stale `zones.json` when ETS changes zone/client identity: detect-and-migrate vs last-persisted-wins.
- **KEA-DEC-5** — Do real ETS reprograms change structural fields (counts/ports) or only tunables? Drives whether Hybrid (C) is worth it.
- **KEA-DEC-6** — `ETS_MEMORY_PATH = "knx-memory.bin"` is CWD-relative (`device.rs:52`) while other state uses `config.system.state_dir` (`main.rs:329`). Fix to `state_dir` as part of this work? (Boot-time apply must read the same path the BAU task writes.)

## 8. Task breakdown (`KEA-T*`) — Option A

- **KEA-T1** — Resolve **KEA-DEC-1..3, 6**. *Partially resolved:* DEC-1 = ETS applies only in `--knx-device` **without** `--config` (base = defaults); DEC-3 = state-seed (deferred, T5); DEC-6 = kept CWD-relative for now (read matches write).
- **KEA-T2** — ✅ **Done** — `ets_params_to_file_config(&EtsParams, FileConfig)` in `knx/ets_config.rs`, with client default-zone index→name translation and empty-name fallbacks. Unit-tested (mapping, base fallback, out-of-range zone, full `load_raw_no_validate` resolution).
- **KEA-T3** — ✅ **Done** — `device::load_persisted_ets_params(ia)` restores a throwaway BAU (no `DeviceServer`, no UDP bind) and parses only if `configured()`. Double-bind avoided entirely (boot path never starts the server).
- **KEA-T4** — ✅ **Done** — `main.rs` `--knx-device` (no `--config`) branch derives config via `knx::ets_device_config()`.
- **KEA-T4b** — ✅ **Done** (added) — **self-restart**: on `A_Restart`, the BAU task persists synchronously and `process::restart_process()` re-execs the binary so the boot-time path applies the new config. Gated by `knx.restart_after_ets` (default true).
- **KEA-T5** — ⏳ **Deferred** — seed the store from ETS `zone/client default_volume` + `client default_latency` (§4.2 fields with no config home; per KEA-DEC-3).
- **KEA-T6** — ⏳ **Deferred** — `zones.json` migration/invalidation per KEA-DEC-4; and the `--config` + ETS merge (KEA-DEC-1 richer form).
- **KEA-T7** — ⏳ **Deferred** — an end-to-end integration test (persisted memory → subsystems see ETS zones/clients/endpoints).

## 9. Risks

- **Silent misconfiguration** — bad ETS strings that pass validation but produce a wrong-but-valid config. Mitigate with sanitize-then-fallback + a startup log of the derived config.
- **Persistence path mismatch** (KEA-DEC-6) — boot reads a different file than the BAU writes → ETS programming silently never applies.
- **Scope creep via §4.2** — "just apply the params" quietly becomes "add per-zone airplay/spotify enable + per-zone max-volume to the whole config + state model." Keep KEA-T5 to state-seeding to contain it.
- **`zones.json` shadowing** — the most likely source of "I reprogrammed but nothing changed" bug reports.
