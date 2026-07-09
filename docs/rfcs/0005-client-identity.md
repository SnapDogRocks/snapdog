---
rfc: CID-0005
title: Robust client identification beyond a single MAC address
status: draft            # draft | accepted | in-progress | done | superseded
version: 0.1.0           # v0.1: initial scoping (reordered: root-cause MAC fix first, then optional ID layers)
created: 2026-07-09
updated: 2026-07-09
target_repo: snapdog     # also touches snapcast-rs (snapcast-client) and snapdog-client
target_branch: main
related: []
owners: [metaneutrons]
---

# RFC CID-0005 — Robust client identification beyond a single MAC

> **For AI agents:** this is a **scoping** RFC, not accepted, **nothing implemented**.
> It proposes making client→config binding reliable. Requirements are `CID-REQ-*`,
> decisions `CID-DEC-*`, tasks `CID-T*`. **Line numbers are approximate (verify before
> editing); symbol names are the stable anchor.** The work spans three repos: the
> root-cause MAC fix lives in **snapcast-rs** (`snapcast-client`), the id derivation in
> **snapdog-client**, the matching/config/UX in **snapdog**.

## 1. Summary

Today a `[[client]]` is bound to a Snapcast connection **only** by MAC
(`c.mac.to_lowercase() == hello.mac`, `snapdog/src/snapcast/events.rs:75`), with an
implicit `name == host_name` fallback used only when the MAC is empty
(`events.rs:68-71`). This is fragile in exactly the deployments SnapDog targets.

The headline finding — and the reason this RFC is **layered**: the primary breakage is
**not** the MAC itself, it is that the client sends **the first enumerated interface's
MAC** non-deterministically (`snapcast-rs/crates/snapcast-client/src/controller.rs:392`
`get_mac_address()` → `mac_address::get_mac_address()`), and that same value fills both
`hello.mac` **and**, by default, `hello.id` (`controller.rs:118-123`). On a Pi with
`eth0` **and** `wlan0` the reported MAC flips between boots/link states and the zone
binding silently breaks.

Therefore the recommendation is ordered **cheapest-root-cause-first**:

- **Layer 0 — deterministic NIC/MAC selection (root-cause fix).** Make the client always
  advertise *the same* permanent MAC of *the same* stable interface. This alone repairs
  the motivating bug and makes today's MAC key reliable. **Do this regardless.** (§3)
- **Layer 1 — multi-identifier matching (server).** Let a `[[client]]` also match by a
  stable `id` or `hostname`, via **one** precedence function shared by both backends and
  the reject allowlist. Additive, zero-breakage. Handles MAC-less clients, containers,
  and operator preference. (§4)
- **Layer 2 — deterministic client-id derivation (client).** Replace the random-UUID idea
  with a per-platform *deterministic* `id`, funnelled through `UUIDv5`, with user
  overrides. Only meaningful once Layer 1 exists. (§5)
- **Layer 3 — discovery/onboarding UX.** Surface a connecting client's advertised
  identifiers so the operator never types a UUID blind. (§6)

Layer 0 is **S–M** and independently shippable. Layers 1–3 are the robustness stack on
top; each is optional and additive. **Recommended first step: Layer 0 + Layer 1.**

## 2. Problem — why one MAC is not enough

Grounded failure modes (anchors approximate):

1. **Non-deterministic interface choice (the real bug).**
   `get_mac_address()` (`controller.rs:392`) returns the first NIC the `mac_address`
   crate enumerates; enumeration order is not stable, so a dual-NIC receiver (`eth0` +
   `wlan0`) advertises a different MAC across boots. `hello.id` defaults to that MAC
   (`controller.rs:119-123`), so **both** identity carriers flip at once.
2. **WiFi MAC randomization** — a randomized `wlan0` MAC changes the advertised value
   even on a single-NIC device.
3. **NIC / hardware swap, containers, VMs** — MAC changes or is shared/cloned.
4. **MAC-less clients** — C++/browser Snapcast send `""`; `snapcast-rs` sends
   `00:00:00:00:00:00` on failure (`controller.rs:397`). All such clients **collide onto
   one** `[[client]]` entry, and the MAC-only reject allowlist
   (`snapdog/src/snapcast/embedded.rs:63-80`) cannot admit them at all.
5. **Latent case bug** — the embedded join lowercases only the *config* side
   (`events.rs:75`), so an upper-case `hello.mac` would miss. Fixed for free by
   centralising the match (§4).

## 3. Layer 0 — Deterministic NIC / MAC selection (root-cause fix)

The MAC is not inherently unstable; "pick the first interface" is. Anchor to a stable
hardware position → the same MAC every boot. This is exactly what systemd's *Predictable
Interface Names* (`enp3s0`, `wlp2s0`, `end0`) do — name NICs by topology, not probe order.

### 3.1 Stable anchors (Linux)

| Anchor | sysfs / API | Meaning |
|---|---|---|
| Bus position | `/sys/class/net/<if>/device` → symlink `…/0000:03:00.0` (PCI) or `1-1.2` (USB port) | stable physical location |
| Predictable name | udev `ID_NET_NAME_PATH` / `_SLOT` / `_ONBOARD` / `_MAC` | path/slot/onboard-based name |
| MAC origin | `/sys/class/net/<if>/addr_assign_type` | `0`=permanent (burned-in), `1`=random, `3`=set → **only trust `0`** |
| Permanent MAC | ethtool `ETHTOOL_GPERMADDR` (no stable sysfs file) | burned-in MAC even if current one is randomized/overridden → **beats WiFi randomization** |
| Virtual? | missing `device` symlink | filters bridge/veth/lo/wg |

### 3.2 Deterministic selection (no config)

Enumerate `/sys/class/net/*`; keep only interfaces with a real `device` symlink **and**
`addr_assign_type == 0`; prefer **wired over wireless, onboard over pluggable**; break
ties by the stable bus-path string; take the first; use its **permanent** MAC
(`ETHTOOL_GPERMADDR`). Result: the same MAC every boot regardless of `eth0`/`wlan0`
order.

### 3.3 User override

`--nic <anchor>` where anchor = interface name (`eth0`), PCI address
(`pci:0000:03:00.0`), or keyword (`onboard` / `wired`). Deterministic **and** pinnable.

### 3.4 Where it lives (multi-repo)

- `get_mac_address()` is in **snapcast-client** (`snapcast-rs`); fixing it there is a
  generic improvement for all users of the library (**CID-DEC-1**: fix generically).
- Add a `settings.mac` / `settings.host_id` override path so **snapdog-client** can inject
  a deterministically-chosen value (it already passes `settings.host_id` →
  `controller.rs:119`).

### 3.5 Honest limits

Layer 0 makes the **existing** MAC deterministic; it does **not** mint a new identity.
NIC replacement / moving a USB dongle to another port still changes it — but that is the
intended "hardware replaced = re-pair" semantics. Containers/VMs with only random/virtual
MACs (no `addr_assign_type==0`) fall through → need Layer 1/2 (token or injected id).

> **Pi corroboration:** the Pi's built-in NIC MAC is *derived from the board serial*
> (`dc:a6:32:…` / `b8:27:eb:…` with serial bytes). So "permanent MAC of the onboard NIC"
> and "SoC serial" (§5.3) are the **same hardware anchor** on a Pi — one code path covers
> both, and it proves the MAC route is sound there.

## 4. Layer 1 — Multi-identifier matching (server)

Let a `[[client]]` declare any subset of `{id, mac, hostname}`, matched by **one** central
precedence function called from both backends and the reject allowlist.

### 4.1 Precedence

`match_config_client(clients, hello_id, hello_mac, hello_host) -> Option<index>`:

1. `id` set and `== hello_id` (exact string); else
2. `mac` set and case-insensitive `== hello_mac` (non-empty); else
3. `hostname` case-insensitive `== hello_host`; else
4. legacy: empty `hello_mac` → `name == hello_host`.

`id > mac` is deliberate (setting `--hostID` intentionally overrides hardware identity);
`hostname` loses to MAC. `hello.id` already reaches both join points — `session.rs:301`
makes it the canonical server key, delivered as the top-level `id` of
`SnapcastEvent::ClientConnected` and as `snap_client.id`. **No wire/protocol change.**

### 4.2 Sites touched (all from the map; verify line numbers)

- **Embedded join** — `snapdog/src/snapcast/events.rs:68-76`: replace the `if/else` with
  the precedence call; also lowercase `hello.mac` here (fixes §2.5).
- **Process join** — `snapdog/src/snapcast/mod.rs:339-346` (`sync_initial_state`) and
  ClientOnConnect (`mod.rs:549/556`): resolve via `id` / `host.mac` / `host.name`.
- **Zone wiring** — `mod.rs:503` (`build_client_mac_map`) + `player/context.rs:216-231`
  (`setup_zone_group`): per-config resolver (id → mac → hostname) so id-/hostname-only
  clients still wire into their group.
- **Reject allowlist** — `embedded.rs:66-77`: `MacAllowlist` → `ClientAllowlist`,
  `accept(hello)` true on id **or** mac **or** hostname (admits id-/hostname-only clients).
- **Reverse lookup** — `snapdog/src/api/routes/zones.rs:1409-1421`: store→config index by
  `id` first, then `mac`.
- **Config/state** — `types.rs:942-947` `RawClientConfig`: `id`/`hostname` as
  `#[serde(default)] Option<String>`, `mac` `String` → `Option<String>`;
  `convention.rs:42/61` passes through; `config/mod.rs:236-248` `validate_raw_inputs`
  requires ≥1 identifier, parses/dedups MAC **only when present**, separate uniqueness
  sets for `id` and `hostname`; `state/mod.rs:117` + `292-314` `ClientState` +
  `from_config` carry `id`/`hostname`. Post-join, `snapcast_id` stays the routing key
  (`events.rs:248-313`) — only the one-time join sites move.

## 5. Layer 2 — Deterministic client-id derivation (client)

The `id` in §4 is only as good as its source. A **random** UUIDv4 on `/data` dies on a
`/data`-wipe and is not reproducible. Instead derive it **deterministically** per platform.

### 5.1 Three axes of "stable"

No single hardware source covers all three:

| Axis | Satisfied by |
|---|---|
| survives **reboot** | almost anything (persisted / from firmware) |
| survives **reflash / `/data`-wipe / fresh image** | **only firmware/silicon** sources (SoC serial, DMI product_uuid) |
| survives **hardware swap** | **only OS/software** sources (machine-id, MachineGuid) — **or a user token** |

Tension: reflash-survival needs firmware sources that on Linux are **root-only** (DMI
`product_uuid`, 0400) — off-limits to the non-root `DynamicUser`; non-root portability
gives only `/etc/machine-id`, which does **not** survive reflash. That is exactly why the
random `/data` UUID is weak.

### 5.2 Funnel + precedence — `derive_stable_id()`

Every chosen source is funnelled through **`UUIDv5(fixed SnapDog namespace,
"<tag>:<value>")`** → uniform UUID, no raw-serial leak (SHA-1, one-way), and a `tag`
prefix (`seed:`, `hw:rpi:`, `machine-id:`) prevents two sources collapsing to the same id.
Namespace is a hardcoded `const`. Order, highest first:

1. **`--hostID <uuid>` verbatim** — used 1:1 (no v5). Also the clean path for
   containers/VMs (inject the id).
2. **`--host-id-seed <token>` → `UUIDv5(ns,"seed:"+token)`** — **the portable axis**;
   survives reflash *and* hardware swap because nothing is read from the machine. Memorable
   string (`wohnzimmer-links`) suffices.
3. **Cache `/data/snapdog-client/host-id`** — derive once, then read.
4. **Platform hardware source → `UUIDv5(ns,"hw:<platform>:<serial>")`** — default on real
   receivers; survives `/data`-wipe and self-heals the cache.
5. **Last resort: random `UUIDv4`, persisted** — only if no valid hardware source.

Results of (2)/(4)/(5) are cached; (4) is re-computed each boot and verified against the
cache so a `/data`-wipe self-heals to the *same* id.

### 5.3 Platform table (web-verified; `Root?` = non-root readability under `DynamicUser`)

| Platform | best stable source | access | Root? | Reflash | HW-swap | Caveat |
|---|---|---|---|---|---|---|
| **Raspberry Pi** | SoC/board serial | `/sys/firmware/devicetree/base/serial-number` · `/proc/cpuinfo` `Serial` | no | ✅ | ❌ | non-root gold standard; dies only on board swap (intended) |
| **other ARM SoCs** | probe `soc0/serial_number` → DT `serial-number` → eFuse (`sunxi-sid`/`imx-ocotp`/Rockchip/Amlogic) | sysfs/nvmem | no* | ✅ | ❌ | eFuse non-root only if `CONFIG_NVMEM_SYSFS=y`; nvmem index unstable → match by driver name; **blacklist** constants (all-zero, H3 bug) |
| **x86/ARM Linux generic** | `/etc/machine-id` | file | no | ❌ | ✅ | only non-root cross-arch source but **per-install** → dies on reflash; golden-image → collision; `product_uuid` is reflash-safe but **root-only** |
| **macOS** | IOPlatformUUID / `gethostuuid(2)` | `ioreg -rd1 -c IOPlatformExpertDevice` | no | ✅ | ❌ | **VM clones share it**; invisible in Linux containers |
| **Windows** | SMBIOS UUID (`Win32_ComputerSystemProduct.UUID`) | WMI / `GetSystemFirmwareTable` | no | ✅ | ❌ | **blacklist sentinels** (all-zero/-F, AMI default); VM template clones share it; fallback MachineGuid (survives HW-swap, not reflash) |
| **container / VM** | **none reliable** | — | — | — | — | machine-id host-shared/empty, DMI shared → **inject** id (`--hostID`) |

Generic x86-Linux, containers and VMs have **no** good hardware source → the user token
(2) or explicit id (1) is the only clean path.

### 5.4 Rust

- **`uuid` 1.23** funnel: `Uuid::new_v5(&SNAPDOG_NS, name)` — **feature `v5` required**
  (pure-Rust `sha1_smol`, no C-dep/root). Namespace `const`.
- **Read sysfs/DT serial directly** — no crate reads the Pi/ARM SoC serial; small ordered
  probe module, each stage with a validity/blacklist check.
- **`machine-uid` 0.6** only as fallback reader for macOS/Windows — **not** the Pi default
  (returns machine-id on Linux = wrong axis; returns raw value → always run through v5).
- **Avoid:** `mid`, `machineid-rs`, `hardware-id` (shell out / unmaintained / root-only DMI).
- Home: a `stable_id` module in **snapdog-client**, `cfg`-gated; result fills `hello.id`.
  If reflash-safe-*without*-board-binding is ever needed, a **root one-shot at first boot**
  reads `product_uuid`, computes v5, caches to `/data/…/host-id` (0444); the non-root client
  only reads the file.

## 6. Layer 3 — Discovery & onboarding UX

The operator must never invent/type a UUID blind. Note: the current `/api/v1/clients`
endpoint lists **only configured** clients and exposes only `mac`
(`snapdog/src/api/routes/clients.rs:19-30` `ClientInfo`) — a freshly connected unknown
device does not appear at all. So a discovery surface is new work either way.

- **On the receiver (source):** the id lives at `/data/snapdog-client/host-id`. The
  **snapdog-ctrl setup portal** (snapdog-os) already shows device/hostname/version → add a
  "Client-ID" line; allow overriding it with a memorable **token** (§5.2 stage 2), so the
  user picks a self-documenting stable seed instead of a UUID. Plus a client-startup
  `info!` log line and the D-Bus `ClientDbusState`.
- **On the server (where it's needed):** the join event already carries the advertised
  `id`/`mac`/`hostname`. Add a **"new/unknown clients"** view in the server WebUI listing
  connected-but-unconfigured devices with those three fields and an **"assign to zone"**
  button that writes the `[[client]]` entry — one-click onboarding, mirroring what
  `unknown_clients = "accept"` already does in runtime state.

## 7. Config shape & migration

```toml
# Existing MAC-only client — UNCHANGED, still MAC-matched (Layer 0 makes that MAC stable)
[[client]]
name = "Living Room"
mac  = "aa:bb:cc:dd:ee:ff"
zone = "Living Room"

# Recommended: stable id primary, MAC fallback (belt & suspenders)
[[client]]
name = "Kitchen"
id   = "b3f1c2a4-8d5e-4c7a-9f21-6a0e2d4c1b77"   # from /data/snapdog-client/host-id
mac  = "11:22:33:44:55:66"                       # survives a /data wipe
zone = "Kitchen"

# id-only: device with no stable MAC
[[client]]
name = "Bathroom"
id   = "7c9a1f30-2b44-4e88-b1a2-90ff5577cc10"
zone = "Bathroom"

# hostname instead of id — for stock/3rd-party clients without --hostID
[[client]]
name     = "Studio"
hostname = "studio-pi"
zone     = "Studio"
```

**Backward-compat:** `id`/`hostname` are `#[serde(default)] Option`; old TOML deserialises
unchanged; `mac` relaxes from *required* to *"required unless id/hostname set"* — a no-op
today since every config has a MAC. Precedence `id > mac` means no client that matches by
MAC today is re-bound. Reject configs stay valid (allowlist becomes a superset
macs ∪ ids ∪ hostnames). State persistence (by positional index, `state/mod.rs:369`) is
untouched → **no state migration**. Server and client upgrade independently.

## 8. Effort, risks, phasing

| Building block | Layer | Effort |
|---|---|---|
| Deterministic NIC/MAC selection (`get_mac_address()` in snapcast-client) + `--nic` | 0 | **S–M** |
| Precedence fn + embedded (`events.rs`) | 1 | **S** |
| Process path (`mod.rs`, `context.rs`) + reverse lookup (`zones.rs`) | 1 | **M** |
| Reject allowlist (`embedded.rs`) | 1 | **S** |
| Config/state schema + validation | 1 | **S–M** |
| `stable_id` module + UUIDv5 funnel + `--host-id-seed`/`--hostID` | 2 | **S–M** |
| Server "unknown clients" view + portal id/token surface | 3 | **M** |

**Risks:** (1) both backends must call the *one* precedence fn or endpoints drift again;
(2) `id` defaults to MAC without `--hostID` and is exact-string — warn if an `id` parses as
a MacAddress; (3) empty/zero MAC stays the sore point → such devices need `id`/`hostname`
(now admissible under Reject); (4) `id` survives only while `/data` does → keep MAC too;
(5) golden-image cloning of `/etc/machine-id` collides → ship images with empty machine-id;
(6) `instance` is **not** folded into `hello.id` (`controller.rs:138`) → assign `id` per
instance.

**Recommended phasing:** **Phase 1 = Layer 0 + Layer 1** (root-cause fix makes the MAC key
reliable; multi-identifier makes it robust and zero-breakage). **Phase 2 = Layer 2 + 3**
(deterministic id derivation + onboarding UX). macOS/Windows sources stay behind `cfg`
gates until a client actually ships there.

## 9. Requirements / Decisions / Tasks

### Requirements
- **CID-REQ-1** A client must advertise the same identity across boots on unchanged
  hardware (fixes the eth0/wlan0 flap).
- **CID-REQ-2** Existing MAC-only configs must keep binding byte-identically.
- **CID-REQ-3** MAC-less clients (empty/zero MAC) must be bindable and admissible under the
  reject policy.
- **CID-REQ-4** The operator must be able to establish a stable identity themselves
  (reproducible token, or explicit id) without inventing a UUID blind.
- **CID-REQ-5** Identity derivation must work non-root (`DynamicUser`) on snapdog-os.

### Decisions
- **CID-DEC-1** Fix `get_mac_address()` generically in snapcast-client (benefits all
  library users) + add an override path snapdog-client can inject.
- **CID-DEC-2** Single central `match_config_client()` precedence `id > mac > hostname >
  legacy-name`; called from both backends + reject allowlist.
- **CID-DEC-3** All id sources funnelled through `UUIDv5(fixed namespace, "<tag>:<value>")`.
- **CID-DEC-4** Default id source: Pi = SoC serial; generic = user token → machine-id →
  random. macOS/Windows behind `cfg`.
- **CID-DEC-5** `mac` relaxed to optional; ≥1 of `{id, mac, hostname}` required per client.

### Tasks (phase 1 first)
- **CID-T1** (Layer 0) Deterministic permanent-MAC selection in snapcast-client + `--nic`.
- **CID-T2** (Layer 1) `match_config_client()` + wire into embedded/process/zone/allowlist/
  reverse-lookup; centralise + fix the `events.rs:75` case bug.
- **CID-T3** (Layer 1) Config/state schema: `id`/`hostname` optional, `mac` optional,
  validation + uniqueness sets.
- **CID-T4** (Layer 2) `stable_id` module in snapdog-client (UUIDv5 funnel, precedence,
  Pi serial, `--host-id-seed`/`--hostID`, `/data` cache).
- **CID-T5** (Layer 3) Server "unknown clients" WebUI view + "assign to zone"; snapdog-ctrl
  portal Client-ID/token surface.
- **CID-T6** Docs: snapdog-web `snapdog-os.mdx` client-setup section for id/token/`--nic`.
