---
rfc: LI-0002
title: PCM / Line-In audio source (USB ADC, S/PDIF, analog capture)
status: draft            # draft | accepted | in-progress | done | superseded
version: 1.3.0           # v1.3: config aligned to snapdog conventions (Raw→resolve port via convention.rs)
created: 2026-06-21
updated: 2026-06-21
target_repo: snapdog
target_branch: main
requires_rfc: BT-0001    # builds on the shared live-input core (BT-DEC-24/26)
cross_platform: true     # Linux/ALSA, macOS/CoreAudio, Windows/WASAPI via cpal
feature_flags: [linein]
owners: [metaneutrons]
progress:                # keep in sync with the LI-LEDGER block (§13)
  total_tasks: 24
  done: 0
  in_progress: 0
  todo: 19
  deferred: 5            # Roadmap (§15): manual selection, discovery, runtime routing/fan-out, selection surfaces
---

# RFC LI-0002 — PCM / Line-In audio source

> **For AI agents:** single source of truth for this feature. IDs are stable
> (`LI-REQ-*`, `LI-DEC-*`, `LI-T*`). Track progress by updating the task checkbox +
> `status:` (§10), the **LI-LEDGER** YAML (§13), and the frontmatter `progress:`.
> Reference IDs in commits (e.g. `feat(linein): cpal capture (LI-T10)`). This RFC
> **depends on RFC BT-0001's live-input core** — read BT-0001 §5–§6 first; shared
> decisions are cited as `BT-DEC-*`. See §12 for the protocol.

## 1. Summary

Add a **Line-In source**: snapdog captures a local PCM audio input (USB ADC,
S/PDIF receiver, analog/onboard capture) and routes it into one or more zones.
It reuses the **shared live-input core defined in BT-0001** (`LiveInputHub`,
routing matrix, `activation`) — Line-In is the **second input kind** after
Bluetooth. The only kind-specific piece is **capture**: a cross-platform `cpal`
stream (vs BlueALSA for BT).

Unlike Bluetooth, Line-In is the *opposite* on fidelity: a 24-bit/96 kHz S/PDIF
or USB ADC can be your **highest-quality** input (true lossless). And unlike BT,
it's **not Linux-only** — `cpal` covers ALSA/CoreAudio/WASAPI.

> **MVP scope (v1.1, `LI-DEC-12`):** ship Line-In as **signal-sensing autodetect
> only** — like AirPlay/Spotify/Bluetooth. A config-**bound** port auto-takes-over
> its zone(s) when signal appears; the PCM format shows in the existing zone state.
> **Deferred to the Roadmap (§15):** manual selection (`play/linein`), the
> discovery API (shape still **open** — see BT-0001 §15), runtime routing/fan-out,
> and the KNX/MQTT *selection* surfaces.

## 2. Goals / Non-goals

### Goals (`LI-REQ-*`)
- `LI-REQ-01` Capture a local PCM input → F32 → existing zone pipeline.
- `LI-REQ-02` **Cross-platform** capture via `cpal` (Linux/macOS/Windows) — `LI-DEC-02`.
- `LI-REQ-03` **Auto-activation by signal detection** (configurable), so it can
  take over like Bluetooth/AirPlay when audio appears — `LI-DEC-03`.
- `LI-REQ-04` Reuse BT-0001's core: routing matrix, fan-out to a set of zones,
  `activation` model — `LI-DEC-01`.
- `LI-REQ-05` Multi-port from day one (multiple ADCs / S/PDIF inputs).
- `LI-REQ-06` Per-port **configurable friendly name** + stable id — `LI-DEC-08`.
- `LI-REQ-07` Surface the PCM **format** (rate/bit-depth) via the codec/quality
  field — `LI-DEC-10`.

### Non-goals
- `LI-NG-01` Metadata / transport control — a raw input has none (`LI-DEC-09`).
- `LI-NG-02` Navigation (next/prev) — not a collection; no-op.
- `LI-NG-03` A generic `play/input` verb — rejected in BT-0001 (`BT-DEC-25`).
- `LI-NG-04` Re-implementing the live-input core — it lives in BT-0001.

## 3. Background — builds on BT-0001

This RFC adds **one input kind** to the core BT-0001 establishes. Reused as-is:
- **`LiveInputHub`** (`BT-DEC-24`): input registry, routing matrix (input→{zones}),
  fan-out, take-over via `SessionStarted`.
- **Generic per-zone `LiveInputReceiver` shim** (`BT-DEC-28`): the hub multiplexes
  Line-In into the **same** per-zone shim BT uses — **PCM adds no per-zone shim and
  no zone/runner changes** (only a capture backend + an input-kind registration).
- **Categories & selection** (`BT-DEC-25`): Line-In is in the **inputs/receivers**
  category; the eventual selection verb is **type-specific `play/linein`** (no
  `play/input`) — **deferred to Roadmap (§15)** for MVP (`LI-DEC-12`).
- **Discovery** — the API shape (unified `/inputs` vs per-kind) is **open** and
  **deferred** (BT-0001 §15); the autodetect MVP needs no discovery endpoint.
- **Activation** (`BT-DEC-26`): shared `activation = auto|manual`; Line-In's
  *trigger* is **signal detection** (vs BT's connect).
- **Addressing** (`BT-DEC-12/23`): config keyed by a **stable device id**; API/
  MQTT/KNX use a **stable index**.
- **On disconnect/inactivity** (`BT-DEC-02`): → `Idle`.

Code touchpoints are the same as BT-0001 §3 (`SourceType`, `ActiveSource`,
`ReceiverProvider`, `FileConfig`, runner select-loop, KNX/MQTT handlers).

## 4. Fidelity — the inverse of Bluetooth

Line-In carries whatever the source feeds, uncompressed:

| Input | Typical format | Fidelity |
|---|---|---|
| S/PDIF (optical/coax) | up to 24-bit / 96–192 kHz PCM | **lossless, hi-res** |
| USB ADC (good interface) | 24-bit / 48–192 kHz | **lossless, hi-res** |
| Onboard/analog line-in | 16–24-bit / 48 kHz | good |

So the quality badge (`BT-DEC-21` codec field) is reused but **flips meaning**:
it shows e.g. `PCM 96 kHz/24-bit` — a *selling point*, not the lossy warning BT
shows. Capture at the device's **native bit depth** (don't truncate 24→16).

## 5. Architecture

```
            ┌──────────────── snapdog process ────────────────┐
 ADC/SPDIF ──PCM──▶ cpal capture (ALSA/CoreAudio/WASAPI)        │
   │              → native depth → F32 → LiveInputHub            │
   │         ┌───────────────── LiveInputHub (BT-0001 core) ─────┤
   │         │  input registry: [bluetooth…, linein…]            │
   │         │  per-port: capture stream + LEVEL MONITOR         │
   │         │  activation(auto): RMS>threshold → SessionStarted │
   │         │  routing matrix: input → {zones}  → fan-out       │
   │         └──────┬────────────────────────────────────────── │
   │   per-zone shim│ (the same one BT uses) → audio_tx/event_tx │
   │                ▼  ZonePlayer select loop (unchanged)        │
   └─────────────────────────────────────────────────────────────┘
```

- **Capture (kind-specific):** `cpal` opens the configured device, gives the
  sample stream on every OS → convert to **F32 interleaved** → hand to the hub.
  `SessionStarted{format}` carries the native rate/channels for the resampler.
- **Activation (signal-sensing):** an *auto* port keeps its capture **open and
  monitored**; a **level (RMS) detector** with `threshold_db` + `hold_ms` (attack)
  + `release_ms` (release) emits active/inactive → the core's take-over / Idle.
  Level-sensing is the **universal** method (works for analog + every OS via the
  sample stream); **S/PDIF hardware lock** is an opt-in augmentation where the
  driver exposes it (`LI-DEC-04`).
- **Manual mode** (Roadmap §15): no monitoring; the port is selected explicitly
  via `play/linein` (`LI-DEC-07`) and released via `stop`.
- Everything else (routing, fan-out, per-zone wiring) is BT-0001's core — no new
  machinery. (Manual selection & discovery are Roadmap, §15.)

## 6. Decisions (resolved)

| ID | Decision | Resolution |
|---|---|---|
| `LI-DEC-01` | Reuse the live-input core | Build on BT-0001's `LiveInputHub` — routing matrix, activation, fan-out, and the **generic per-zone `LiveInputReceiver`** (`BT-DEC-24/28`). Only *capture* is kind-specific (cpal); PCM adds **no** per-zone shim. |
| `LI-DEC-02` | Platform | **Cross-platform** via `cpal` (ALSA/CoreAudio/WASAPI). Not Linux-only (unlike BT). Feature `linein`, default **on**. |
| `LI-DEC-03` | Activation default | **`auto` (signal detection)** so Line-In can take over like BT/AirPlay; user-configurable `auto\|manual` (shared `activation`, `BT-DEC-26`). |
| `LI-DEC-04` | Detection method + tunables | **Level/RMS** with per-port `threshold_db`, `hold_ms`, `release_ms` (defaults ≈ −50 dBFS, 300 ms, 5 s). Auto ports are **continuously monitored** (capture stays open — small CPU cost). S/PDIF hardware-lock = opt-in augmentation where exposed. |
| `LI-DEC-05` | Contention | Take-over vs content via the existing `source_conflict` policy; among live inputs, **most-recent-trigger wins** (shared with BT). |
| `LI-DEC-06` | Auto-release target | Silence past `release_ms` → `SessionEnded` → **Idle** (consistent with `BT-DEC-02`). "Revert to previous content" is a possible later enhancement. |
| `LI-DEC-07` | Selection verb | **Type-specific `play/linein[/{port}]`** (`ZoneCommand::PlayLineIn`, port = index). No generic `play/input` (`BT-DEC-25`). **Deferred to Roadmap (§15)** for MVP (`LI-DEC-12`). |
| `LI-DEC-08` | Identity + naming | Config keyed by a **stable device id** (ALSA `by-id`/name, cpal device id, or USB path — never the volatile card number); API/MQTT/KNX use a **stable index** (`BT-DEC-23`). Per-port **friendly `name`** configurable (`SourceType::LineIn`). |
| `LI-DEC-09` | No metadata/transport | No AVRCP/remote. "Now-playing" = the port's friendly name; `Next/Previous` = no-op; not seekable; no `RemoteControl`. |
| `LI-DEC-10` | Quality field reuse | Reuse the `codec` field (`BT-DEC-21`) to show the **PCM format** (`PCM <rate>/<bits>`). Capture at native bit depth. |
| `LI-DEC-11` | Multi-port | Built in from the start (mirror BT multi-adapter); each port routable to a set of zones (fan-out). |
| `LI-DEC-12` | MVP scope: autodetect-only | **Ship signal-sensing auto-activation only** (like AirPlay/Spotify/BT): a config-**bound** port auto-takes-over on signal (`LI-DEC-03/04`); PCM format rides zone state (`LI-T14`). **Deferred to Roadmap (§15):** manual selection (`LI-DEC-07`), the **discovery API** (shape **open** — BT-0001 §15), runtime routing/fan-out, and the KNX/MQTT *selection* surfaces. Mirrors `BT-DEC-27`. |
| `LI-DEC-art1` | Capture lib | **`cpal`** (cross-platform) over direct ALSA — required by `LI-DEC-02`. |

## 7. Config schema (proposed)

```toml
[linein]
enabled = false                 # opt-in (mirror BT-DEC-17)
# global defaults; each port may override (mirror BT-DEC-11)
activation   = "auto"           # "auto" (signal-sensing) | "manual"   (LI-DEC-03)
threshold_db = -50              # auto trigger threshold                (LI-DEC-04)
hold_ms      = 300              # attack debounce
release_ms   = 5000             # silence → release

# One block per input port, keyed by a STABLE device id (LI-DEC-08). Omit
# [[linein.port]] to auto-use the single detected device (name + index then
# derived by convention.rs).
[[linein.port]]
id = "alsa:hw:CARD=Scarlett,DEV=0"   # stable id (by-id/name/USB path), NOT card number
name = "Turntable"                    # friendly name (LI-REQ-06); default derived
enabled = true
bind_zones = ["Ground Floor"]         # default route(s); [] = unbound
# activation/threshold_db/hold_ms/release_ms overridable per port
```

The top-level `LineInConfig` mirrors `BluetoothConfig`; add
`linein: Option<LineInConfig>` to `FileConfig` and resolved `Config`. Each
`[[linein.port]]` is an **array entity like `[[zone]]`/`[[client]]`**, so parse it
as a **`RawLineInPortConfig`** and resolve via **`config/convention.rs`** — where
the **friendly-name default** and **stable index** (`BT-DEC-23`) are derived.

## 8. Control surfaces (follows BT-0001 conventions)

> **MVP vs Roadmap (`LI-DEC-12`):** the autodetect-only MVP's *active* surfaces are
> **signal-triggered take-over** (no command) and the **format/status** fields
> below (via `LI-T14` + zone state). The **route/select** command and **discovery**
> are **deferred to the Roadmap (§15)** — kept here as the eventual design.

**Per-zone (type-specific, like `play/bluetooth`):**

| Action | REST | MQTT | KNX | WS |
|---|---|---|---|---|
| Route Line-In → zone | `POST /zones/{z}/play/linein[/{port}]` (`ZoneCommand::PlayLineIn`; index optional → default/bound port) | `zones/{z}/linein/set` = `<port-index>` or `off` | `linein` (DPT 1.001, default port on/off) + opt `linein_source` (DPT 5.010, by index) | push only |
| Release (→ Idle) | existing `POST /zones/{z}/stop` | `zones/{z}/control/set` = `stop` | existing stop GA | — |
| Now-playing + **format** (`codec`) | existing `…/track/metadata` (`codec` = `PCM 96k/24`) | `zones/{z}/state` | opt `codec_status` (DPT 16.001) | `ZoneChanged` |
| Line-In active / port name (status) | zone state | `zones/{z}/state` | `linein_active_status` (1.001), `linein_name_status` (16.001) | `ZoneChanged` |

**Discovery (Roadmap §15):** the discovery API shape (unified `/inputs` vs
per-kind) is **open** (BT-0001 §15). The autodetect MVP needs none — bound ports
self-activate on signal.

**KNX scope-limited** (as BT-0001): per-zone select + active/format status only;
no port inventory over KNX.

## 9. SnapDog OS / build

- Dep: **`cpal`** (Linux backend → ALSA; pulls `alsa-sys`/`libasound`).
- Buildroot: ensure `alsa-lib` + ALSA capture for the target ADC/SPDIF (USB-audio
  / I2S / onboard S/PDIF kernel support). No extra daemon (unlike BT's bluealsa).
- No persistence needs (Line-In is stateless; config lives with the rest).
- Cross-platform: macOS/Windows builds get CoreAudio/WASAPI via cpal — no extra work.

## 10. Task breakdown (phased)

> Legend & update rules: identical to BT-0001 §10/§12.
> **Prerequisites (RFC BT-0001 core, MVP):** `BT-T04` (module pattern), `BT-T10`
> (`LiveInputHub`), `BT-T14` (hub→zone fan-out). (`BT-T30`/`BT-T34` — runtime
> routing/discovery — are needed only by the deferred Roadmap tasks, §15.) LI tasks
> list only intra-RFC `depends_on`; BT-core prereqs are implied.

### Phase 0 — Scaffolding
- [ ] `LI-T01` Cargo feature `linein` + `cpal` dep (cross-platform). `status: todo` · files: `snapdog/Cargo.toml` · deps: — · **AC:** builds with/without; default on.
- [ ] `LI-T02` `LineInConfig` + `RawLineInPortConfig`→resolved via `convention.rs` (derive name default + stable index, like `[[zone]]`/`[[client]]`); wire into `FileConfig`/`Config`. `status: todo` · files: `config/types.rs`, `config/convention.rs` · deps: — · **AC:** §7 TOML parses; absent = disabled.
- [ ] `LI-T03` Add `LineIn` to `SourceType` + `ActiveSource` (carry port index). `status: todo` · files: `state/mod.rs`, `player/commands.rs` · deps: —.
- [ ] `LI-T04` Module `receiver/linein/` (capture via cpal) registering as a `LiveInputHub` input kind. `status: todo` · deps: LI-T01.

### Phase 1 — Single-port MVP (capture + bind; autodetect path)
- [ ] `LI-T10` cpal capture loop: open device → native depth → F32 → hub; emit `SessionStarted{format}`. `status: todo` · files: `receiver/linein/capture.rs` · deps: LI-T04 · **AC:** captured PCM observed from a real ADC/SPDIF.
- [ ] `LI-T11` Register Line-In ports as hub input kind (registry + routing) — **no per-zone shim**, reuses the generic `LiveInputReceiver` (`BT-DEC-28`). `status: todo` · deps: LI-T10.
- [ ] `LI-T12` Manual selection `play/linein[/{port}]` (`ZoneCommand::PlayLineIn`) → take-over via core ⟶ **ROADMAP (§15)**. `status: deferred` · deps: LI-T11, LI-T03.
- [ ] `LI-T13` Config binding (port → zone(s)), stable-id keyed (`LI-DEC-08`); core fans to bound zones. `status: todo` · deps: LI-T02, LI-T11.
- [ ] `LI-T14` Report PCM **format** via the `codec` field (`LI-DEC-10`). `status: todo` · deps: LI-T10 · **AC:** zone shows `PCM <rate>/<bits>`.

### Phase 2 — Signal-sensing activation (auto)
- [ ] `LI-T20` Level/RMS monitor → active/inactive (`SessionStarted`/`SessionEnded`). `status: todo` · files: `receiver/linein/detect.rs` · deps: LI-T10.
- [ ] `LI-T21` `activation=auto` wiring: signal → take-over; silence → Idle (`BT-DEC-26/02`). `status: todo` · deps: LI-T20, LI-T13 · **AC:** feeding audio auto-switches the bound zone; silence releases.
- [ ] `LI-T22` Per-port tunables (`threshold_db`/`hold_ms`/`release_ms`) + continuous-monitor lifecycle (`LI-DEC-04`). `status: todo` · deps: LI-T20.
- [ ] `LI-T23` S/PDIF hardware-lock augmentation, opt-in where exposed (`LI-DEC-04`). `status: todo` · deps: LI-T20 · **AC:** ships if available, else `status: cancelled` with note.

### Phase 3 — Routing, selection & discovery  ⟶ **ROADMAP (deferred, §15)**
> Deferred per `LI-DEC-12` — selection/discovery API needs the BT-0001 §15 design pass.
- [ ] `LI-T30` Discovery — Line-In in the discovery endpoint (**shape open**: `/inputs` vs per-kind, BT-0001 §15). `status: deferred` · deps: LI-T11.
- [ ] `LI-T31` Runtime multi-zone fan-out + runtime selection via core (`LI-REQ-05`; config-level fan-out already ships via LI-T13). `status: deferred` · deps: LI-T11.
- [ ] `LI-T32` Selection surfaces §8: REST `play/linein` + WS; MQTT `zones/{z}/linein/set`; KNX scope-limited. `status: deferred` · deps: LI-T31.

### Phase 4 — Multi-port
- [ ] `LI-T40` Enumerate capture devices (cpal); per-port instances; stable-id→index. `status: todo` · deps: LI-T10 · **AC:** 2 ADCs → 2 independent inputs.
- [ ] `LI-T41` Per-port config/overrides/naming (`LI-DEC-11`). `status: todo` · deps: LI-T13, LI-T40.

### Phase 5 — Platforms
- [ ] `LI-T50` Validate capture on Linux/ALSA, macOS/CoreAudio, Windows/WASAPI (`LI-DEC-02`). `status: todo` · deps: LI-T10.

### Phase 6 — SnapDog OS build
- [ ] `LI-T60` Buildroot: alsa-lib + USB-audio/S-PDIF kernel support; cpal cross-compiles for aarch64. `status: todo` · repo: **snapdog-os** · deps: LI-T01.

### Phase 7 — UI
- [ ] `LI-T70` Line-In in the merged Sources picker (selection UI) ⟶ **ROADMAP (§15)**. `status: deferred` · repo: snapdog-web/apps · deps: LI-T30.
- [ ] `LI-T71` Format/quality badge reuse (shows hi-res PCM) in now-playing (`LI-DEC-10`). `status: todo` · repo: snapdog-web · deps: LI-T14.

### Cross-cutting
- [ ] `LI-T80` Tests: level detection (threshold/hold/release), F32 conversion, config parse, routing/take-over. `status: todo` · deps: LI-T20.
- [ ] `LI-T81` Docs: README source list, `snapdog.example.toml` `[linein]`, hardware note (USB ADC / S-PDIF, hi-res capable). `status: todo` · deps: LI-T13.

## 11. Definition of done (MVP = autodetect-only; phases 0–2, 4, 6 + minimal 7)
- A configured Line-In port, on `activation=auto`, **auto-takes-over** its bound
  zone(s) when signal appears and releases to Idle on silence.
- Multiple ports each auto-serve their bound zone(s) (config-level fan-out).
- Zone shows the PCM format (e.g. `PCM 96 kHz/24-bit`).
- Captures on Linux (SnapDog OS); macOS/Windows builds capture via cpal.

> Manual selection, runtime fan-out, and the discovery API are **Roadmap (§15)**.

## 12. Progress-tracking protocol
Identical to BT-0001 §12 (pick deps-satisfied task → `in-progress` in all three
places → implement to AC → `done`; new work = `LI-T9x`; cuts = `cancelled`;
changed decisions = new `LI-DEC-*` superseding the old).

## 13. Machine-readable task ledger

<!-- LI-LEDGER-START (authoritative status; agents update here + the checkboxes above) -->
```yaml
rfc: LI-0002
updated: 2026-06-21
requires_rfc: BT-0001          # MVP prereqs BT-T04/T10/T14 (BT-T30/T34 only for §15 roadmap; not in depends_on)
tasks:
  - { id: LI-T01, phase: 0, status: todo, depends_on: [] }
  - { id: LI-T02, phase: 0, status: todo, depends_on: [] }
  - { id: LI-T03, phase: 0, status: todo, depends_on: [] }
  - { id: LI-T04, phase: 0, status: todo, depends_on: [LI-T01] }
  - { id: LI-T10, phase: 1, status: todo, depends_on: [LI-T04] }
  - { id: LI-T11, phase: 1, status: todo, depends_on: [LI-T10] }
  - { id: LI-T12, phase: 1, status: deferred, depends_on: [LI-T11, LI-T03] }  # Roadmap §15
  - { id: LI-T13, phase: 1, status: todo, depends_on: [LI-T02, LI-T11] }
  - { id: LI-T14, phase: 1, status: todo, depends_on: [LI-T10] }
  - { id: LI-T20, phase: 2, status: todo, depends_on: [LI-T10] }
  - { id: LI-T21, phase: 2, status: todo, depends_on: [LI-T20, LI-T13] }
  - { id: LI-T22, phase: 2, status: todo, depends_on: [LI-T20] }
  - { id: LI-T23, phase: 2, status: todo, depends_on: [LI-T20] }
  - { id: LI-T30, phase: 3, status: deferred, depends_on: [LI-T11] }  # Roadmap §15 (discovery API open)
  - { id: LI-T31, phase: 3, status: deferred, depends_on: [LI-T11] }  # Roadmap §15
  - { id: LI-T32, phase: 3, status: deferred, depends_on: [LI-T31] }  # Roadmap §15
  - { id: LI-T40, phase: 4, status: todo, depends_on: [LI-T10] }
  - { id: LI-T41, phase: 4, status: todo, depends_on: [LI-T13, LI-T40] }
  - { id: LI-T50, phase: 5, status: todo, depends_on: [LI-T10] }
  - { id: LI-T60, phase: 6, status: todo, repo: snapdog-os, depends_on: [LI-T01] }
  - { id: LI-T70, phase: 7, status: deferred, repo: snapdog-web, depends_on: [LI-T30] }  # Roadmap §15
  - { id: LI-T71, phase: 7, status: todo, repo: snapdog-web, depends_on: [LI-T14] }
  - { id: LI-T80, phase: x, status: todo, depends_on: [LI-T20] }
  - { id: LI-T81, phase: x, status: todo, depends_on: [LI-T13] }
```
<!-- LI-LEDGER-END -->

## 14. Open questions
None blocking the MVP — §6 decisions are resolved. `LI-T23` (S/PDIF hardware lock)
is the only conditional MVP item (non-blocking). The discovery-API shape is the one
**open** design question, deferred with its feature (§15, shared with BT-0001 §15).

## 15. Roadmap (deferred — manual selection & discovery API)

Deferred from the MVP per `LI-DEC-12`, for the shared design pass in **BT-0001 §15**.
The **MVP ships the audio path** (signal-sensing autodetect, like AirPlay/Spotify/BT);
these add user-driven control:

- **Manual selection** — `play/linein[/{port}]` (`LI-DEC-07`). Task: `LI-T12`.
- **Discovery API — OPEN** (unified `/inputs` vs per-kind; see BT-0001 §15). Task:
  `LI-T30`.
- **Runtime routing/fan-out** — runtime input→zone changes via API (config-level
  fan-out already ships in MVP via `LI-T13`). Task: `LI-T31`.
- **Selection surfaces** — REST/MQTT/KNX *selection* (status/format ship in MVP via
  `LI-T14` + zone state). Task: `LI-T32`.
- **Selection UI** — Line-In in the source picker. Task: `LI-T70`.

These IDs stay in the ledger with `status: deferred`; promote one by flipping it to
`todo` per §12.
