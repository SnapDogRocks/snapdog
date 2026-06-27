---
rfc: BT-0001
title: Bluetooth (A2DP sink) audio source
status: draft            # draft | accepted | in-progress | done | superseded
version: 1.2.0           # v1.2: adapters addressed by stable index, not MAC (BT-DEC-23)
created: 2026-06-21
updated: 2026-06-21
target_repo: snapdog
target_branch: main
linux_only: true
feature_flags: [bluetooth, bt-aptx]
owners: [metaneutrons]
progress:                # keep in sync with the BT-LEDGER block at the bottom
  total_tasks: 35
  done: 0
  in_progress: 0
  todo: 35
---

# RFC BT-0001 — Bluetooth (A2DP sink) audio source

> **For AI agents:** this document is the single source of truth for this feature.
> Every requirement (`BT-REQ-*`), decision (`BT-DEC-*`), and task (`BT-T*`) has a
> stable ID. To track progress, update (1) the task's checkbox and `status:` in
> §10, (2) the matching entry in the **BT-LEDGER** YAML block (§13), and (3) the
> `progress:` rollup in the frontmatter. Reference IDs in commits/PRs
> (e.g. `feat(bt): BlueALSA capture loop (BT-T11)`). Do not renumber IDs; mark
> dropped items `status: cancelled`. See §12 for the full protocol.

## 1. Summary

Add a **Bluetooth source** to snapdog: the device acts as a Bluetooth **A2DP
sink**, receiving audio from phones/laptops and routing it into one or more
zones. Positioned as a **universal "any device, any app, zero-setup" convenience
source** — explicitly **not** a high-fidelity source (see §4). Linux-only.

Decode/transport is delegated to **BlueZ + BlueALSA**; snapdog owns the control
plane (D-Bus), PCM capture, routing, metadata, and AVRCP control. A single
**`BluetoothHub`** owns all adapters and fans each adapter's stream out to a
configurable set of zones; per-zone integration reuses the existing
`ReceiverProvider` channels so the zone player is largely untouched.

## 2. Goals / Non-goals

### Goals (`BT-REQ-*`)
- `BT-REQ-01` Receive A2DP audio from arbitrary devices and play it in a zone.
- `BT-REQ-02` Codecs: SBC + AAC + aptX/aptX HD (best realistic sink set).
- `BT-REQ-03` Multi-adapter from day one (model for N, validate with 1).
- `BT-REQ-04` One adapter's stream is routable to a **set** of zones (fan-out).
- `BT-REQ-05` Two routing layers: **config binding** (default) + **runtime
  selection** (override) — see `BT-DEC-07`.
- `BT-REQ-06` Now-playing metadata + transport control via AVRCP, surfaced on
  REST/WS/MQTT/KNX with parity to AirPlay/Spotify.
- `BT-REQ-07` Open guest pairing by default; optional secured pairing mode.
- `BT-REQ-08` Honest quality signalling: expose the negotiated codec.
- `BT-REQ-09` Runs on SnapDog OS (buildroot) and on generic desktop Linux.

### Non-goals
- `BT-NG-01` LDAC reception (no open decoder — see §4).
- `BT-NG-02` macOS / Windows A2DP sink (not feasible without private APIs).
- `BT-NG-03` LE Audio / LC3 (revisit later; BlueALSA path doesn't preclude it).
- `BT-NG-04` Bluetooth **source** role (snapdog sending audio out) — out of scope.
- `BT-NG-05` True lossless/hi-res over Bluetooth (protocol can't deliver it).

## 3. Background — how snapdog sources work today (verified against code)

| Concept | Location | Notes |
|---|---|---|
| `ReceiverProvider` trait | `snapdog/src/receiver/mod.rs:143` | `name/start/stop/is_running`; `start(audio_tx, event_tx)` |
| `AudioSender = mpsc::Sender<Vec<f32>>` | `…/receiver/mod.rs:28` | **F32 interleaved**, any rate/channels |
| `ReceiverEvent` | `…/receiver/mod.rs:52` | `SessionStarted{format}`, `SessionEnded`, `Metadata`, `CoverArt`, `Progress`, `Volume`, `RemoteAvailable` |
| `RemoteControl` + `RemoteCommand` | `…/receiver/mod.rs:101,126` | Play/Pause/Next/Prev/Stop/SetVolume/ToggleShuffle/ToggleRepeat |
| Per-zone receiver wiring | `snapdog/src/player/runner.rs:236–293` | AirPlay always-on; Spotify `#[cfg(feature="spotify")]` |
| Zone select loop arms | `…/player/runner.rs:1070–1077` | `airplay_audio_rx`/`airplay_event_rx`, `spotify_*` |
| Audio consume + resample + EQ → Snapcast | `…/player/runner.rs` (handle macros) | F32 resample → per-zone EQ → `backend.send_audio` |
| `SourceType` enum | `snapdog/src/state/mod.rs:183` | `Idle, Radio, SubsonicPlaylist, SubsonicTrack, Url, AirPlay, Spotify, …` |
| `ActiveSource` enum | `snapdog/src/player/commands.rs:82` | per-zone active source (carries params) |
| Config `FileConfig` | `snapdog/src/config/types.rs:176` | `airplay` (201), `spotify: Option` (207); resolved `Config` ~1008 |
| Source config structs | `…/config/types.rs:655` (`AirplayConfig`), `:667` (`SpotifyConfig`) | pattern to mirror |
| AirPlay impl (in-proc) | `snapdog/src/receiver/airplay.rs` | `shairplay` crate |
| Spotify impl (in-proc) | `snapdog/src/receiver/spotify.rs` | `librespot` crate |
| Cargo features | `snapdog/Cargo.toml` | `default = ["snapcast-embedded","spotify","dbus"]`; `dbus` → `zbus` |

> Line numbers are approximate (verify before editing); the **names** are stable.

**Key divergence:** AirPlay/Spotify are **per-zone receiver instances**. Bluetooth
is fundamentally a **shared hardware input** (one phone per adapter) routable to
many zones — so it needs a shared-source layer (§5), not N independent instances.

**Verified external-API conventions (v1.1 — drive §8):**
- **No generic "set source."** Active sources are chosen by type-specific `Play*`
  commands (`api/routes/zones.rs`: `play/url`, `play/playlist`,
  `play/subsonic/{id}`); **AirPlay/Spotify are passive take-over with no select
  command**. BT follows this, not a uniform "select source."
- **No `codec`/quality field exists** in any DTO today (only `bitrate_kbps` in
  `TrackInfo`, `state/mod.rs:215`). The codec badge is a brand-new field.
- **One active source per zone** (`ActiveSource`). Fan-out = several zones each
  independently holding `Bluetooth(mac)`; the *routing* is the new part, not zone
  state.
- **WS is push-only** (`api/ws.rs`: `Notification`, `#[serde(tag="type", rename_all="snake_case")]`); commands never arrive over WS.
- **MQTT**: `{base}zones/{i}/{action}/set` (raw payloads) + retained `…/state` JSON.
- **KNX**: fixed `GoDefinition`s (`knx/group_objects.rs`) + per-zone GAs
  (`config/types.rs` `RawZoneKnxConfig`); enumerations ride DPT 5.010 indices.
  Adding a command/feedback touches config + listener + publisher + group_objects
  + device-mode count — and there's **no way to express device lists or pairing**.
- **Adding a source touches:** `SourceType`, `ActiveSource`/`ZoneCommand`, REST
  routes, MQTT handler, KNX (config/listener/publisher/group_objects); OpenAPI is
  auto-generated via utoipa.

## 4. The fidelity reality (rationale for §1 positioning)

Classic A2DP is **always lossy**, and a *sink* must *decode* what the source
sends — and the best codecs are proprietary:

| Codec | Quality | Sink-decodable on Linux | In scope |
|---|---|---|---|
| SBC / SBC-XQ | lossy baseline | ✅ mandatory | ✅ |
| AAC | lossy ~256 kbps (iPhones) | ✅ fdk-aac | ✅ |
| aptX | lossy 16/48 | ✅ libfreeaptx | ✅ (feature `bt-aptx`) |
| aptX HD | lossy **24/48** (ceiling) | ✅ libfreeaptx | ✅ (feature `bt-aptx`) |
| LDAC | lossy ≤24/96 | ❌ encoder-only, no sink decoder | ❌ `BT-NG-01` |
| aptX Adaptive/Lossless | proprietary | ❌ | ❌ |
| LC3/LC3plus (LE Audio) | newer | ⚠️ immature on Pi | ❌ `BT-NG-03` |

Conclusion: Bluetooth is the **lowest-fidelity** source; "hi-res over BT" tops out
at aptX HD (lossy 24/48). `BT-REQ-08` surfaces the codec so users see reality.

## 5. Architecture

### 5.1 Components
```
            ┌──────────────────────── snapdog process ───────────────────────┐
 phone ──A2DP──▶ bluetoothd (BlueZ)        BlueAlsa daemon                     │
   │           (pairing, AVRCP, adapters)  (A2DP decode → PCM)                 │
   │                  ▲  D-Bus(org.bluez)        ▲ ALSA "bluealsa" PCM         │
   │                  │   via zbus               │ via `alsa` crate           │
   │           ┌──────┴───────────────── BluetoothHub (global, Arc) ──────────┤
   │           │  • adapter registry (keyed by MAC)                           │
   │           │  • control plane: discoverable/pairable/agent, connect evts  │
   │           │  • per-adapter capture loop: PCM(S16/S24) → F32              │
   │           │  • AVRCP metadata/transport (org.bluez MediaPlayer1)         │
   │           │  • negotiated codec (org.bluez MediaTransport1)              │
   │           │  • ROUTING MATRIX: adapter → {zone set}                      │
   │           └───────┬───────────────────────────────────────────────────  │
   │   per-zone shim   │ fan-out: forwards PCM + ReceiverEvent to each        │
   │  BluetoothReceiver│ subscribed zone's (audio_tx, event_tx)               │
   │  (ReceiverProvider)▼                                                      │
   │            ZonePlayer select loop (UNCHANGED pattern; new arms)          │
   └──────────────────────────────────────────────────────────────────────────┘
```

### 5.2 Why a hub + per-zone shim
- The **`BluetoothHub`** is the shared source: it owns adapters, the BlueZ/BlueALSA
  plumbing, AVRCP, and the routing matrix. One instance per process.
- The **`BluetoothReceiver`** (implements existing `ReceiverProvider`) is a thin
  per-zone shim. On `start(audio_tx, event_tx)` it registers
  `(zone_index, audio_tx, event_tx)` with the hub. The hub forwards a routed
  adapter's PCM/events into those channels — so `runner.rs` only gains two
  `select!` arms (mirroring AirPlay/Spotify) and no other zone-player changes.
- **Take-over:** when an adapter with a route to zone Z gets a connection, the hub
  emits `SessionStarted{format}` into Z's event channel → existing take-over logic
  switches Z's `ActiveSource` to Bluetooth, exactly like AirPlay (`BT-DEC-01`).
- **Fan-out:** the matrix maps one adapter to N zones; the hub writes the same PCM
  frame to each subscribed zone's `audio_tx`. Snapcast keeps the zones in sync.

### 5.3 Capture & format
- BlueALSA decodes A2DP → PCM exposed as an ALSA `bluealsa` capture PCM.
- Capture at the codec's **native depth** (S24 for aptX HD; do **not** truncate to
  S16) → convert to **F32 interleaved** → `audio_tx`. Emit `SessionStarted` with
  the negotiated rate/channels so the zone player builds the right resampler.
- Alternative capture path (note, not chosen): BlueALSA D-Bus (`org.bluealsa`)
  hands a socket FD per PCM — more control, no ALSA plugin dep. ALSA-plugin path
  chosen for simplicity (`BT-DEC-art1`).

### 5.4 Control plane
- Use **`zbus`** (already a dep via the `dbus` feature) against `org.bluez`:
  adapter `Discoverable`/`Pairable`/`DiscoverableTimeout`, `Agent1` registration,
  `Device1` connect/disconnect, `MediaPlayer1` (AVRCP metadata + transport),
  `MediaTransport1` (negotiated codec). `bluer` crate is an option for ergonomics
  but avoided to keep the dep surface small (`BT-DEC-art2`).

## 6. Decisions (resolved)

| ID | Decision | Resolution |
|---|---|---|
| `BT-DEC-01` | Take-over on connect | **Auto** take-over for routed adapters (like AirPlay). Configurable `take_over = auto\|manual`, default `auto`. |
| `BT-DEC-02` | On disconnect | **Match AirPlay/Spotify** exactly → `SessionEnded` resets zone to `Idle` (no auto-revert). |
| `BT-DEC-03` | Codec set | SBC + AAC + aptX/aptX HD. aptX behind feature `bt-aptx`, **on by default**. LDAC excluded. |
| `BT-DEC-04` | Decode/transport stack | **BlueALSA** (+ bluetoothd). Not PipeWire (overkill), not in-proc (bluetoothd is mandatory regardless). |
| `BT-DEC-05` | Platform | **Linux only** (`feature = "bluetooth"` + `cfg(target_os="linux")`). |
| `BT-DEC-06` | Multi-adapter | Built in from the start; validated with one adapter. |
| `BT-DEC-07` | Routing model | One model: adapter = named input; matrix input→{zones}; config bindings = defaults, runtime selection = override. |
| `BT-DEC-08` | Concurrency | One device per adapter. Configurable: `on_second_device = reject\|replace` (default `reject` = first-come-holds) + `idle_timeout_s` (default 300, 0=off). |
| `BT-DEC-09` | Bonds | **Ephemeral** — forget device on disconnect (avoid unbounded guest-bond growth). |
| `BT-DEC-10` | Pairing modes | Support **both** `open` (just-works, zero friction) and `secured` (timed window + agent/app confirmation). Config-selectable, **default `open`**. |
| `BT-DEC-11` | Config granularity | Global defaults with **per-adapter override** for pairing/concurrency/take-over/name. |
| `BT-DEC-12` | Adapter identity | Key **config/persistence** by **MAC address** (stable), never `hciN`. The **API/MQTT/KNX address adapters by a stable index** (`BT-DEC-23`); MAC is surfaced as a field in `GET /bluetooth/adapters`. |
| `BT-DEC-13` | Cover art | **Best-effort** via AVRCP; ship if not a showstopper, else defer (non-blocking). |
| `BT-DEC-14` | Transport controls | Implement `RemoteControl` (AVRCP) in v1 — parity with AirPlay/Spotify. |
| `BT-DEC-15` | Quality signal | Expose negotiated codec (from `MediaTransport1`) as a badge in webui/apps + a subtle "lossy" hint. |
| `BT-DEC-16` | Onboard vs USB | Audio via **USB dongles**; Pi onboard radio discouraged/disabled (WiFi/BT coexistence). |
| `BT-DEC-17` | Enablement default | `bluetooth.enabled` default **off** (opt-in); per-adapter enable. |
| `BT-DEC-18` | `/var/lib/bluetooth` | Persist on `/data` (SnapDog OS) so bonds survive — same symlink pattern as other mutable config. |
| `BT-DEC-art1` | Capture path | ALSA `bluealsa` plugin via `alsa` crate (vs BlueALSA D-Bus FD). |
| `BT-DEC-art2` | BlueZ binding | `zbus` directly (existing dep) vs `bluer`. |
| `BT-DEC-19` | Source-selection convention | No generic "set-source." A **bound** adapter takes over its zone(s) **passively** (like AirPlay, no command); routing a **shared** adapter to a zone uses a `Play*`-style command (`play/bluetooth[/{adapter}]` → `ZoneCommand::PlayBluetooth`, adapter = index per `BT-DEC-23`). |
| `BT-DEC-20` | Adapter/device/pairing surface | Lives under a **new `/bluetooth` subsystem resource** (REST + WS + MQTT). **Not** per-zone, **not** KNX — device lists & pairing flows have no GA/DPT analog. KNX gets only per-zone select + status. |
| `BT-DEC-21` | Codec is a new field | No codec/quality field exists today. Add `codec: Option<String>` to `TrackInfo` and propagate to REST + MQTT + WS (optional KNX string GA) — a 4–5 surface change, not a free status cell. |
| `BT-DEC-22` | Fan-out vs one-source-per-zone | Fan-out = each target zone independently holds `ActiveSource::Bluetooth(adapter)`. **No change** to the one-active-source-per-zone model; only the routing command is new. |
| `BT-DEC-23` | Adapter addressing in the API | Adapters are addressed by a **stable index** in REST/MQTT/KNX (consistent with zones/clients; KNX needs DPT 5.010 indices, MACs can't traverse KNX). MAC (`BT-DEC-12`) stays the internal/config key and appears as a field in `GET /bluetooth/adapters`. Supersedes the `{mac}`-in-path from v1.1. |

**Legal note (aptX):** libfreeaptx is reverse-engineered; the `bt-aptx` feature
flag lets the **official SnapDog OS image** ship without it if legal advises,
without code changes (`BT-DEC-03`).

## 7. Config schema (proposed)

```toml
[bluetooth]
enabled = false                 # BT-DEC-17 (opt-in)
# global defaults (BT-DEC-11); each adapter may override
pairing = "open"                # "open" | "secured"            (BT-DEC-10)
take_over = "auto"              # "auto" | "manual"             (BT-DEC-01)
on_second_device = "reject"     # "reject" | "replace"          (BT-DEC-08)
idle_timeout_s = 300            # 0 = disabled                  (BT-DEC-08)

# One block per physical adapter, keyed by MAC (BT-DEC-12).
# Omit [[bluetooth.adapter]] entirely to auto-use the single present adapter.
[[bluetooth.adapter]]
mac = "AA:BB:CC:DD:EE:FF"
enabled = true
name = "SnapDog Living Room"    # advertised BT name; default "SnapDog <zone>"
bind_zones = ["Living Room"]    # default route(s) (BT-DEC-07); [] = unbound
# pairing/take_over/on_second_device/idle_timeout_s overridable here
```

`BluetoothConfig` mirrors `AirplayConfig`/`SpotifyConfig` in
`config/types.rs:655`; add `bluetooth: Option<BluetoothConfig>` to both
`FileConfig` (:176) and resolved `Config` (~:1008).

## 8. Control surfaces (reworked to match verified conventions, §3)

Two distinct layers — do **not** model everything as "endpoint + topic + GA per
action" (that was the v1.0 mistake):

**(A) Per-zone — reuses existing source/transport patterns.** A *bound* adapter
takes over passively (no command, like AirPlay); a *shared* adapter is routed via
a `Play*`-style command. Transport/now-playing reuse existing per-zone APIs.

| Action | REST | MQTT | KNX | WS |
|---|---|---|---|---|
| Route shared BT input → zone | `POST /zones/{z}/play/bluetooth[/{adapter}]` (`ZoneCommand::PlayBluetooth`; index optional → default/bound adapter) | `zones/{z}/bluetooth/set` = `<adapter-index>` or `off` | `bluetooth` (DPT 1.001, default adapter on/off) + opt `bluetooth_source` (DPT 5.010, select by index) | push only |
| Release (→ Idle) | existing `POST /zones/{z}/stop` | `zones/{z}/control/set` = `stop` | existing stop GA | — |
| Transport play/pause/next/prev/stop | existing endpoints (now also drive AVRCP) | existing `control/set` | existing GOs | — |
| Now-playing + **new `codec`** | existing `…/track/metadata` + `codec` field | `zones/{z}/state` + `codec` | `bluetooth_device_status`, opt `codec_status` (DPT 16.001) | `ZoneChanged` + `codec` |
| BT active / connected device (status) | zone state | `zones/{z}/state` | `bluetooth_active_status` (1.001), `bluetooth_device_status` (16.001) | `ZoneChanged` |

Passive take-over for a bound adapter needs **no** route command. Fan-out is just
several zones each holding `ActiveSource::Bluetooth(mac)` — no state-model change
(`BT-DEC-22`).

**(B) Bluetooth subsystem — NEW `/bluetooth` resource (REST + WS + MQTT; NOT KNX).**
Adapter/device/pairing are not per-zone source concerns.

| Action | REST | MQTT | WS |
|---|---|---|---|
| List adapters (**index**, mac, name, bound zones, connected device, codec) | `GET /bluetooth/adapters` | `bluetooth/adapters/{adapter}/state` (retained; payload carries mac+name) | `BluetoothChanged` (push) |
| List connected devices | `GET /bluetooth/devices` | (in adapter state) | `BluetoothChanged` |
| Disconnect current device | `POST /bluetooth/adapters/{adapter}/disconnect` | `bluetooth/adapters/{adapter}/disconnect/set` | — |
| Start `secured` pairing window | `POST /bluetooth/adapters/{adapter}/pairing` | `bluetooth/adapters/{adapter}/pairing/set` | `BluetoothChanged` (state) |

> `{adapter}` is the stable index from `GET /bluetooth/adapters` (which maps
> index ↔ mac ↔ name); MAC never appears in a path/topic/GA (`BT-DEC-23`).

**KNX is deliberately scope-limited** (`BT-DEC-20`): per-zone select + status
(BT-active / device-name / optional codec) only. Device lists, multi-adapter
inventory, and pairing flows are **not** exposed over KNX (no DPT/GA analog).

## 9. SnapDog OS / buildroot

- Packages: `bluez5_utils` (bluetoothd + tools), `bluez-alsa`, `sbc`; `fdk-aac`
  (AAC); `libfreeaptx` (aptX, gated by `bt-aptx`).
- systemd: enable `bluetooth.service` + `bluealsa.service` (configure enabled
  codecs); snapdog needs `netdev`/bluetooth group + the right D-Bus policy.
- Persistence: symlink `/var/lib/bluetooth` → `/data/bluetooth` in
  `board/raspberrypi/post-build.sh`; seed dir in `snapdog-data-init` (`BT-DEC-18`).
- Footprint: ~a few MB. Keep onboard BT off by default (`BT-DEC-16`).

## 10. Task breakdown (phased)

> Status legend: `todo` ▢ · `in-progress` ◐ · `done` ✅ · `blocked` ⛔ · `cancelled` ✗
> Update the checkbox **and** the `status:` token **and** the BT-LEDGER (§13).

### Phase 0 — Scaffolding
- [ ] `BT-T01` Add Cargo features `bluetooth`, `bt-aptx` (+ `alsa` dep, optional). `status: todo` · files: `snapdog/Cargo.toml` · deps: — · **AC:** builds with/without features; `default` includes `bluetooth` (Linux).
- [ ] `BT-T02` `BluetoothConfig` + wire into `FileConfig`/`Config`. `status: todo` · files: `config/types.rs` · deps: — · **AC:** §7 TOML parses; absent section = disabled.
- [ ] `BT-T03` Add `Bluetooth` to `SourceType` + `ActiveSource` (carry adapter id). `status: todo` · files: `state/mod.rs:183`, `player/commands.rs:82` · deps: — · **AC:** matches exhaustively; serializes to API.
- [ ] `BT-T04` Module skeleton `receiver/bluetooth/` (`mod.rs`, `hub.rs`, `bluez.rs`, `bluealsa.rs`, `avrcp.rs`), `cfg(all(feature="bluetooth", target_os="linux"))`. `status: todo` · deps: BT-T01.

### Phase 1 — Single-adapter MVP (audio end-to-end, open pairing)
- [ ] `BT-T10` `BluetoothHub` skeleton (adapter registry by MAC, routing matrix, lifecycle). `status: todo` · files: `receiver/bluetooth/hub.rs` · deps: BT-T04.
- [ ] `BT-T11` BlueALSA capture loop: ALSA `bluealsa` PCM → native depth → F32. `status: todo` · files: `…/bluealsa.rs` · deps: BT-T10 · **AC:** decoded PCM observed for a connected phone.
- [ ] `BT-T12` BlueZ control plane via `zbus`: adapter discoverable/pairable, just-works `Agent1`, connect/disconnect events. `status: todo` · files: `…/bluez.rs` · deps: BT-T10 · **AC:** phone pairs+connects with no prompt (`open`).
- [ ] `BT-T13` `BluetoothReceiver` (`ReceiverProvider`) per-zone shim registering channels with the hub. `status: todo` · files: `…/mod.rs` · deps: BT-T10.
- [ ] `BT-T14` Hub→zone fan-out of PCM + `SessionStarted`/`SessionEnded`. `status: todo` · deps: BT-T11, BT-T13 · **AC:** audio plays in the bound zone.
- [ ] `BT-T15` Wire receiver + select-loop arms into `runner.rs` (mirror AirPlay/Spotify). `status: todo` · files: `player/runner.rs:236–293,1070` · deps: BT-T13.
- [ ] `BT-T16` Take-over on connect + `Idle` on disconnect (`BT-DEC-01/02`). `status: todo` · deps: BT-T14, BT-T15 · **AC:** connecting preempts current source; disconnect → Idle.
- [ ] `BT-T17` Config binding (single adapter → zone), MAC-keyed (`BT-DEC-12`). `status: todo` · deps: BT-T02, BT-T14.
- [ ] `BT-T18` Concurrency policy: reject/replace + idle timeout (`BT-DEC-08`). `status: todo` · deps: BT-T12 · **AC:** 2nd device rejected (default); idle frees adapter.
- [ ] `BT-T19` Ephemeral bonds — forget on disconnect (`BT-DEC-09`). `status: todo` · deps: BT-T12.

### Phase 2 — Metadata, control, quality
- [ ] `BT-T20` AVRCP metadata → `Metadata`/`Progress` events (`MediaPlayer1`). `status: todo` · files: `…/avrcp.rs` · deps: BT-T12.
- [ ] `BT-T21` `RemoteControl` impl (AVRCP transport) (`BT-DEC-14`). `status: todo` · deps: BT-T20 · **AC:** play/pause/next reach the phone.
- [ ] `BT-T22` Add **new `codec` field** to `TrackInfo` (from `MediaTransport1`) and propagate to REST + MQTT + WS (+opt KNX) (`BT-DEC-15/21`). `status: todo` · deps: BT-T12 · **AC:** codec visible per-zone across REST/WS/MQTT.
- [ ] `BT-T23` Cover art via AVRCP (best-effort) (`BT-DEC-13`). `status: todo` · deps: BT-T20 · **AC:** ships if non-disruptive, else `status: cancelled` with note.

### Phase 3 — Routing (runtime selection + fan-out)
- [ ] `BT-T30` Routing matrix runtime API in hub (set/clear input→zone). `status: todo` · deps: BT-T14.
- [ ] `BT-T31` Zone routing via `Play*` convention (`ZoneCommand::PlayBluetooth` route + passive take-over for bound adapters); fan-out = independent per-zone selection (`BT-DEC-19/22`, `BT-REQ-04`). `status: todo` · deps: BT-T30.
- [ ] `BT-T32` Per-zone control surfaces §8(A): REST `play/bluetooth` + WS `ZoneChanged`(+codec). `status: todo` · deps: BT-T31.
- [ ] `BT-T33` MQTT (per-zone + `bluetooth/adapters/*`) and **KNX scope-limited** (per-zone select + BT-active/device/codec status; no device-list/pairing) §8 (`BT-DEC-20`). `status: todo` · deps: BT-T31, BT-T34.
- [ ] `BT-T34` **`/bluetooth` subsystem resource** §8(B): REST `GET /bluetooth/adapters` & `/devices`, `POST …/disconnect`, `POST …/pairing`; `BluetoothChanged` WS push (`BT-DEC-20`). `status: todo` · deps: BT-T12, BT-T30.

### Phase 4 — Multi-adapter
- [ ] `BT-T40` Adapter enumeration via BlueZ; per-adapter capture/control instances. `status: todo` · deps: BT-T11, BT-T12 · **AC:** 2 dongles → 2 independent inputs.
- [ ] `BT-T41` Per-adapter config + overrides + naming (`BT-DEC-11`). `status: todo` · deps: BT-T17, BT-T40.

### Phase 5 — Secured pairing
- [ ] `BT-T50` `secured` mode: timed discoverable window + agent confirmation; per-adapter (`BT-DEC-10/11`). `status: todo` · deps: BT-T12.
- [ ] `BT-T51` App-mediated allow/deny over WS (surface incoming request). `status: todo` · deps: BT-T50, BT-T32.

### Phase 6 — SnapDog OS build
- [ ] `BT-T60` Buildroot packages: bluez5_utils, bluez-alsa, sbc, fdk-aac, libfreeaptx (gated). `status: todo` · repo: **snapdog-os** · deps: BT-T01.
- [ ] `BT-T61` systemd units (bluetooth, bluealsa) + codec config + D-Bus/group policy. `status: todo` · repo: snapdog-os · deps: BT-T60.
- [ ] `BT-T62` `/var/lib/bluetooth` → `/data` symlink + data-init seed (`BT-DEC-18`). `status: todo` · repo: snapdog-os · files: `board/raspberrypi/post-build.sh`, `package/snapdog-server/snapdog-data-init`.

### Phase 7 — UI (webui + apps)
- [ ] `BT-T70` Source selector exposes BT inputs + connected device names. `status: todo` · repo: snapdog-web / apps · deps: BT-T32.
- [ ] `BT-T71` Codec quality badge + lossy hint (`BT-DEC-15`). `status: todo` · deps: BT-T22, BT-T32.
- [ ] `BT-T72` Disconnect-device + (secured) pairing controls. `status: todo` · deps: BT-T32, BT-T50.

### Cross-cutting
- [ ] `BT-T80` Tests: PCM-format conversion, routing-matrix logic, config parse, take-over/disconnect state. `status: todo` · deps: phase 1–3.
- [ ] `BT-T81` Docs: README source list, `snapdog.example.toml` `[bluetooth]`, hardware note (USB dongle, dongle-per-room for guest self-serve). `status: todo` · deps: BT-T17.

## 11. Definition of done (MVP = phases 0–3 + 6 + minimal 7)
- A phone connects to the device (open pairing), audio plays in the bound zone,
  preempting the prior source; disconnect → Idle.
- The same input is selectable on multiple zones (fan-out) at runtime.
- Now-playing + negotiated codec visible in webui; transport controls work.
- Runs on a SnapDog OS image with bonds persisted across reboot.

## 12. Progress-tracking protocol (for AI agents)
1. Pick a task whose `depends_on` are all `done`/`cancelled`.
2. Set it `◐ in-progress` (checkbox stays `[ ]`); mirror in BT-LEDGER; bump frontmatter `in_progress`.
3. Implement to the task's **AC**; reference the ID in commits.
4. On completion: `[x]` + `status: done` + BT-LEDGER + frontmatter rollup; set RFC `status: in-progress` once any task starts, `done` when all non-cancelled tasks are done.
5. New work discovered mid-flight → add `BT-T9x` (don't reuse IDs); scope cuts → `status: cancelled` + one-line reason.
6. Decisions that change → add a new `BT-DEC-*` superseding the old (mark old `superseded by …`); never silently rewrite history.

## 13. Machine-readable task ledger

<!-- BT-LEDGER-START (authoritative status; agents update here + the checkboxes above) -->
```yaml
rfc: BT-0001
updated: 2026-06-21
tasks:
  - { id: BT-T01, phase: 0, status: todo, depends_on: [] }
  - { id: BT-T02, phase: 0, status: todo, depends_on: [] }
  - { id: BT-T03, phase: 0, status: todo, depends_on: [] }
  - { id: BT-T04, phase: 0, status: todo, depends_on: [BT-T01] }
  - { id: BT-T10, phase: 1, status: todo, depends_on: [BT-T04] }
  - { id: BT-T11, phase: 1, status: todo, depends_on: [BT-T10] }
  - { id: BT-T12, phase: 1, status: todo, depends_on: [BT-T10] }
  - { id: BT-T13, phase: 1, status: todo, depends_on: [BT-T10] }
  - { id: BT-T14, phase: 1, status: todo, depends_on: [BT-T11, BT-T13] }
  - { id: BT-T15, phase: 1, status: todo, depends_on: [BT-T13] }
  - { id: BT-T16, phase: 1, status: todo, depends_on: [BT-T14, BT-T15] }
  - { id: BT-T17, phase: 1, status: todo, depends_on: [BT-T02, BT-T14] }
  - { id: BT-T18, phase: 1, status: todo, depends_on: [BT-T12] }
  - { id: BT-T19, phase: 1, status: todo, depends_on: [BT-T12] }
  - { id: BT-T20, phase: 2, status: todo, depends_on: [BT-T12] }
  - { id: BT-T21, phase: 2, status: todo, depends_on: [BT-T20] }
  - { id: BT-T22, phase: 2, status: todo, depends_on: [BT-T12] }
  - { id: BT-T23, phase: 2, status: todo, depends_on: [BT-T20] }
  - { id: BT-T30, phase: 3, status: todo, depends_on: [BT-T14] }
  - { id: BT-T31, phase: 3, status: todo, depends_on: [BT-T30] }
  - { id: BT-T32, phase: 3, status: todo, depends_on: [BT-T31] }
  - { id: BT-T33, phase: 3, status: todo, depends_on: [BT-T31, BT-T34] }
  - { id: BT-T34, phase: 3, status: todo, depends_on: [BT-T12, BT-T30] }
  - { id: BT-T40, phase: 4, status: todo, depends_on: [BT-T11, BT-T12] }
  - { id: BT-T41, phase: 4, status: todo, depends_on: [BT-T17, BT-T40] }
  - { id: BT-T50, phase: 5, status: todo, depends_on: [BT-T12] }
  - { id: BT-T51, phase: 5, status: todo, depends_on: [BT-T50, BT-T32] }
  - { id: BT-T60, phase: 6, status: todo, repo: snapdog-os, depends_on: [BT-T01] }
  - { id: BT-T61, phase: 6, status: todo, repo: snapdog-os, depends_on: [BT-T60] }
  - { id: BT-T62, phase: 6, status: todo, repo: snapdog-os, depends_on: [] }
  - { id: BT-T70, phase: 7, status: todo, repo: snapdog-web, depends_on: [BT-T32] }
  - { id: BT-T71, phase: 7, status: todo, repo: snapdog-web, depends_on: [BT-T22, BT-T32] }
  - { id: BT-T72, phase: 7, status: todo, repo: snapdog-web, depends_on: [BT-T32, BT-T50] }
  - { id: BT-T80, phase: x, status: todo, depends_on: [] }
  - { id: BT-T81, phase: x, status: todo, depends_on: [BT-T17] }
```
<!-- BT-LEDGER-END -->

## 14. Open questions
None blocking — all design decisions in §6 are resolved. `BT-T23` (cover art) is
the only conditional item and is non-blocking by definition.
