---
rfc: HAI-0007
title: A Home Assistant integration for SnapDog – what MQTT can carry, and a real integration for the rest
status: draft            # draft | accepted | in-progress | done | superseded
version: 0.1.0           # v0.1: scoping after the MQTT fix; nothing of stages 1 to 4 implemented
created: 2026-10-25
updated: 2026-10-25
target_repo: snapdog     # stage 3 lives in a new repository (see HAI-DEC-3)
target_branch: main
related: [CID-0005]
owners: [metaneutrons]
---

# RFC HAI-0007 – A Home Assistant integration for SnapDog

> **For AI agents:** this is a **scoping** RFC, not accepted. **Nothing in stages 1 to 4
> is implemented.** Stage 0 (the MQTT repair) is done and described in §2. Requirements
> are `HAI-REQ-*`, decisions `HAI-DEC-*`, tasks `HAI-T*`. SnapDog line numbers are
> approximate, symbol names are the anchor. Statements about Home Assistant come from the
> documentation as read on 2026-10-25 (§16 lists the pages and how each was read); they
> change with every monthly release, so check them again before building on them.

## 1. Summary

SnapDog's README promised "Home Assistant auto-discovery" and "zones appear as
`media_player` entities". That never worked: Home Assistant's MQTT integration has **no
`media_player` platform**, so the retained discovery messages SnapDog published at every
start produced no entity. Stage 0 removed them (and clears them from the broker) and
repaired the rest of the MQTT bridge, which until then also dropped every inbound command.

This RFC answers what a good Home Assistant (HA) experience for SnapDog looks like and
proposes two layers, in this order:

1. **Stage 1, MQTT device discovery.** One `homeassistant/device/.../config` message that
   exposes what MQTT discovery *can* express: volume, mute, shuffle, repeat and zone
   assignment as `number`, `switch` and `select`, transport as `button`, now-playing as
   `sensor`, clients as diagnostic entities. Cheap, works with the MQTT broker users
   already run, and gives automations their first hold on SnapDog. It cannot give a
   `media_player`, and with it no media card, no `tts.speak` or `play_media` target, no
   browsing, no voice assistant exposure.
2. **Stages 2 and 3, a custom integration** (`custom_components/snapdog`, installed through
   HACS today and the Marketplace built into HA 2026.11) that talks to SnapDog's REST and
   WebSocket API: one real `media_player` per zone with local push, found by zeroconf
   (`_snapdog._tcp`, already advertised), API-key authentication with reauth, diagnostics
   and repairs. It needs a handful of small additions to the SnapDog API first (§5).

Stage 4, moving the integration into HA core, is optional and not recommended before the
integration has run at Gold level for a while.

HA's own **Snapcast integration** does not replace this: it needs Snapcast's JSON-RPC control
port, which SnapDog's embedded server does not open, and where it works it would fight
SnapDog over groups and streams (§9). **Music Assistant** is not a replacement either, but
it should compose: a SnapDog zone that supports `media_player.play_media` can be a playback
target for it (§9; the feature flags its Home Assistant provider requires are not verified).

## 2. Where things stand (stage 0, done)

Verified in the code before this RFC (anchors in `snapdog/src/mqtt/`):

- **Commands were dead.** `config` normalises `base_topic` to end in `/`; the old
  `handle_command` stripped the base and then demanded a second `/`. Fixed at the root: one
  `Topics` type builds the base, the subscriptions and the parser from one route table.
- **Discovery was dead.** `homeassistant/media_player/snapdog_zone_{n}/config` is not a
  topic HA's MQTT integration subscribes to, because it has no such platform (§3). SnapDog
  now publishes **no** discovery message and clears the old ones on first connect.
- The bridge now runs in its own tasks, resubscribes and re-announces after every
  reconnect, ignores retained commands, validates payloads, rate limits per zone and client,
  supports TLS and mutual TLS, and removes a shortcoming that matters here: a graceful
  shutdown publishes `offline`, so HA greys the entities out at once.

Still true and relevant to this RFC:

| Fact | Consequence |
|---|---|
| State is published per zone as **one JSON object** (`zones/{n}/state`, retained) and per client (`clients/{n}/state`) | One state topic can feed many HA entities through `value_template` (documented for `sensor`). |
| `volume_level` is a fraction, commands take a fraction **or** a percentage (`1` means 100 %) | A `number` with range 0..100 must send through a `command_template` that divides by 100. |
| `media_position` is the position at the last `ZoneChanged`. A decoded source (Subsonic, radio, URL) reports a new position once per second of media time and a receiver (AirPlay, Spotify) on each progress event, and every report is a full `ZoneChanged`. The `ZoneProgress` notification itself is consumed in three places and **emitted nowhere** | The MQTT state is republished about once a second while a zone plays, so the position is live over MQTT. HA's documentation asks for the opposite: set `media_position` and `media_position_updated_at` only at jumps and let the frontend extrapolate, because per-second updates only fill the database. A position sensor would do exactly that damage, so stage 1 does not offer one. |
| `media_image_url` is built from `[http].base_url`; the cover endpoint needs the API key when `api_keys` is set | HA's `image` entity downloads a URL without a header, so covers fail on a protected server (§5 item 2, HAI-T7). |
| Zone and client indices are **list positions** | Reordering `[[zone]]` renumbers topics and entity IDs (§11, HAI-DEC-6). |
| The MQTT transport is MQTT 3.1.1 | No reason codes, no `no_local`, no "do not send retained on subscribe" (§12). |

## 3. What MQTT discovery can and cannot express

### 3.1 What exists

HA's MQTT integration supports discovery for these 32 platforms (list as given in the
integration documentation): `alarm_control_panel`, `binary_sensor`, `button`, `camera`,
`cover`, `climate`, `date`, `datetime`, `device_tracker`, `device_trigger`, `event`, `fan`,
`humidifier`, `image`, `infrared`, `lawn_mower`, `light`, `lock`, `notify`, `number`,
`scene`, `select`, `sensor`, `siren`, `switch`, `update`, `tag`, `text`, `time`, `vacuum`,
`valve`, `water_heater`. **`media_player` is not among them**; `media_player.mqtt` is a 404 on
home-assistant.io, and core issue #152085 ("MQTT media_player entity not created despite
publishing config payload") was closed as *not planned* on 2025-09-11 with the maintainer's
answer "Media player is not a supported entity platform for MQTT". I found no document that
announces one, and no official statement that points to a custom integration instead.

**Device-based discovery** (HA 2024.11, 2024-11-06) puts a device and all its components in
one message:

```text
topic:   <discovery_prefix>/device/[<node_id>/]<object_id>/config        (prefix default: homeassistant)
payload: { "dev": {identifiers, name, manufacturer, model, sw_version, ...},   # required
           "o":   {name, sw_version, support_url},                              # required
           "cmps": { "<component id>": { "p": "<platform>", "unique_id": "...", <platform options> }, ... },
           "availability": ..., "state_topic": ..., "command_topic": ..., "qos": ..., "encoding": ... }  # shared defaults
```

Points that decide the design:

- Every entity component needs `p` (platform) and `unique_id`; `dev` and `o` cannot be
  overridden per component. Root-level `availability`, `state_topic`, `command_topic`, `qos`
  and `encoding` act as defaults for all components.
- `object_id` as an option was deprecated in 2025.10 and removed in 2026.4. Use
  `default_entity_id` (for example `sensor.living_room_title`). The `<object_id>` in the
  *topic* is a different thing and stays.
- Availability: one `availability` list with a `topic` per entry and `availability_mode`
  `all`/`any`/`latest`; defaults `online`/`offline`. SnapDog already publishes exactly that
  on `{base}status` (retained `online`, last will `offline`).
- `entity_category` is `config` or `diagnostic`; sensors can only be `diagnostic`.
- After an HA restart discovered entities stay unavailable until their discovery message
  arrives again. The documentation recommends subscribing to HA's birth message
  (`homeassistant/status` = `online`) and re-sending discovery after a random delay; retained
  discovery works too but is flagged as heavy on the broker for large installations. SnapDog
  should do both (HAI-DEC-2).
- Removal is an empty retained payload on the device topic. The documentation on the `next`
  branch (not yet released when read) changes how single components are removed from a
  device payload and warns that deleting a device-discovered entity in the UI does not clear
  the payload; the generator must therefore always publish the *complete* device payload and
  never rely on component-level removal.

### 3.2 What it cannot express

A `media_player` is one entity that carries transport, volume, metadata, a source list,
browsing and playback of arbitrary media. MQTT discovery has nothing for that, and the loss
is concrete:

| Capability | With MQTT entities | With a `media_player` |
|---|---|---|
| Media control card, more-info dialog | one card per entity, assembled by hand | built in |
| `media_player.play_media` (URL, playlist, radio) | not possible | yes, also from automations |
| `tts.speak` and announcements to a zone | **not possible**: the action's `media_player_entity_id` takes a media player | yes |
| Browse playlists and radio in the UI (`browse_media`) | no | yes |
| Exposure to Assist, Alexa, Google | no (not a media entity) | yes |
| Live position bar | no: a sensor per second, no extrapolation, heavy on the recorder | yes, HA extrapolates from `media_position` + `media_position_updated_at` |
| Grouping (`join`/`unjoin`, `group_members`) | no | yes, if SnapDog gets a model for it (§6.3) |
| Sound mode (EQ preset) | `select` only | `select_sound_mode` |

HA documents a YAML-only **universal media player** that combines existing entities into
one `media_player`. It could close part of the gap for users who stay on stage 1, but it is
per-zone boilerplate, has no config flow, and the page does not say that it forwards
title and artist. Not pursued here (HAI-DEC-1).

## 4. Stage 1: the MQTT entity set

### 4.1 Shape

SnapDog publishes **one device message per server**
(`homeassistant/device/snapdog_<server_id>/config`, `server_id` being the stable id it
already persists and advertises over mDNS), retained and again after HA's birth message.
Components per zone and per client follow §8. State comes from the existing JSON topics; no
new state topic is needed for stage 1 except where §8 says so.

Configuration, all under `[mqtt]` (names are proposals, HAI-DEC-2):

```toml
[mqtt.homeassistant]
discovery = true                 # off by default until stage 1 is approved; see HAI-DEC-2
discovery_prefix = "homeassistant"
```

### 4.2 What it will not do

No position sensor (§2), no cover art on a server with API keys (§5 item 2), no EQ preset
list (the API only reports presets in an error message), and no zone-to-client moves by name
without a `command_template` that maps names to indices (doable: the generator knows the
names). Entity IDs depend on list positions until HAI-DEC-6 is settled.

## 5. Stage 2: API prerequisites

The integration (and, for covers, stage 1) needs a few additions to SnapDog. Each is small
and useful on its own:

| # | Addition | Why |
|---|---|---|
| 1 | `GET /api/v1/eq/presets` (names) | sound-mode list, `select` options |
| 2 | Cover access that works without a header: short-lived signed URL or per-server cover token | HA's `image` entity and `media_image_url` fetch without credentials; with `api_keys` set they fail today |
| 3 | A WebSocket hello message with server id, version, API version, and a full snapshot on request | clean resync after a reconnect without N REST calls |
| 4 | Optional: API keys with a scope (read, control) | a Home Assistant key should not be able to change configuration (§10) |
| 5 | Optional: stable `id` for `[[zone]]` and `[[client]]` | entity unique ids that survive reordering (HAI-DEC-6, relates to CID-0005) |
| 6 | Optional: a model for linking zones | the only honest basis for `join`/`unjoin` (§6.3) |

Position needs no addition: `ZoneChanged` already carries `position_ms` about once a second
while playing (§2). The integration decides itself when a change is a jump (seek, new track,
pause) and only then sets `media_position` and `media_position_updated_at`.

## 6. Stage 3: a custom integration

### 6.1 Why a real integration

It is the only way to get a `media_player` (§3.2). The documentation says the same about
packaging: a custom integration "is built the same way as one in core" and the Marketplace
(HACS until 2026.10, built into HA from 2026.11) is the recommended place to start, also
when core is the goal.

### 6.2 Design

| Topic | Proposal |
|---|---|
| Domain, type | `snapdog`, `integration_type: hub` (one config entry per server), `iot_class: local_push` |
| Transport | REST for commands and the initial snapshot, WebSocket (`/ws`) for push. Authentication with `Authorization: Bearer` on both: the server accepts the header on `/ws` too (`api/auth.rs`), so the key never needs to appear in a URL. SnapDog allows 64 concurrent WebSocket connections and pings every 30 s |
| Library | A separate Python package (`snapdog-api`, PyPI) holds the client. HA's principles page asks for exactly that ("device logic lives in libraries") and core requires it; nothing in the documentation requires it for a custom integration, but doing it now keeps stage 4 open and the integration thin |
| Discovery | `manifest.json` `"zeroconf": ["_snapdog._tcp.local."]`. SnapDog advertises that type with the TXT keys `api_version`, `server_id`, `snapcast_port`, `auth` (and `docker`, `tls`, `base_url` when applicable). Config entry `unique_id` = `server_id`; rediscovery with a new address updates the entry through `_abort_if_unique_id_configured(updates={CONF_HOST: host})` |
| Config flow | `user` (host, port), `zeroconf` (confirm), then API key if TXT `auth=true`, TLS verification switch if `tls=true`. `reauth` on `ConfigEntryAuthFailed`, `reconfigure` for host and port. No options flow in the first version |
| Runtime | `entry.runtime_data` with a coordinator **without** `update_interval`; the WebSocket handler calls `async_set_updated_data`; reconnect with backoff; entities `available` follows the socket. `PARALLEL_UPDATES = 1` for the media player |
| Devices | One device for the server (`entry_type=service`), one device per zone and per client with `via_device_id` to the server |
| Diagnostics | `async_get_config_entry_diagnostics` with the API key, host and `base_url` redacted |
| Repairs | `api_version` newer than supported (fixable: no, link to docs), `[http].base_url` still `localhost` so cover URLs are unreachable (not fixable, explains the setting), WebSocket limit reached |
| Brand | `custom_components/snapdog/brand/icon.png` (HA 2026.3 and later read brand images from the integration; no entry in `home-assistant/brands` is needed) |

### 6.3 The zone `media_player`

| Item | Mapping |
|---|---|
| Device class | `speaker` |
| State | `playing`, `paused`, and `idle` for both SnapDog's `stopped` and `idle`; never `off` (zones have no power) |
| Features | `PLAY`, `PAUSE`, `STOP`, `NEXT_TRACK`, `PREVIOUS_TRACK`, `SEEK` (when the track is seekable), `VOLUME_SET`, `VOLUME_STEP`, `VOLUME_MUTE`, `SHUFFLE_SET`, `REPEAT_SET`, `SELECT_SOURCE`, `SELECT_SOUND_MODE`, `PLAY_MEDIA`, `BROWSE_MEDIA`; later `SEARCH_MEDIA` |
| Source list | SnapDog's unified playlist model: index 0 is the radio list (from config), 1 and up are Subsonic playlists. `select_source` = `POST /zones/{n}/play/playlist` |
| `play_media` | `media_type: url` (any audio URL, also from `media_source`) = `POST .../play/url`; `playlist` and `track` with SnapDog ids. This is the hook that makes a zone a playback target for TTS and for Music Assistant |
| Browse | root = playlists and radio from `/api/v1/media/playlists` and their tracks, covers through HA's proxy (`get_browse_image_url`, never a URL as `media_image_id`) |
| Position | set `media_position` and `media_position_updated_at` only at a jump (seek, track change, pause, resume), detected from the once-per-second `ZoneChanged` reports; HA's frontend extrapolates in between |
| Sound mode | EQ presets (stage 2 item 1) |
| Grouping | **Not in the first version.** HA's `join`/`unjoin` take other `media_player` entities, and a SnapDog zone is fixed by configuration: clients move between zones, zones do not join. A faithful `GROUPING` needs zone linking in SnapDog (stage 2 item 6). Until then client assignment is a `select` on the client device |
| Not supported | `MEDIA_ANNOUNCE` (SnapDog has no ducking), `TURN_ON/OFF`, `CLEAR_PLAYLIST` |

### 6.4 Client entities

Clients are not `media_player` entities: they are speakers inside a zone. Per client:
`number` volume, `switch` mute, `number` latency (`config`), `select` zone, `binary_sensor`
connectivity (`diagnostic`). The core Snapcast integration models clients as players and
lets them `join` groups; SnapDog's model is zone-first, which is why the mapping differs.

### 6.5 Quality scale and what Gold and Platinum need

The scale has four tiers: Bronze 20 rules, Silver 10, Gold 21, Platinum 3. For a custom
integration it is a guide: hassfest skips the `quality_scale.yaml` check for non-core
integrations, and the manifest may carry `custom`. Following it is still the cheapest way to
keep stage 4 open.

| Tier | What SnapDog's integration has to deliver beyond the design above |
|---|---|
| Bronze | UI config flow with tests (100 % of `config_flow.py`), `runtime_data`, unique ids (entity and config entry), `has_entity_name`, `test-before-configure` and `test-before-setup`, no duplicate entries, documentation (install, remove, actions), `brands`, `dependency-transparency` |
| Silver | `reauthentication-flow`, `entity-unavailable`, `log-when-unavailable` (once when lost, once when back), `parallel-updates`, `config-entry-unloading`, an integration owner, more than 95 % test coverage |
| Gold | `discovery` and `discovery-update-info` (zeroconf, above), `devices`, `diagnostics`, `dynamic-devices` (new zones and clients after setup) and `stale-devices`, `entity-category`, `entity-device-class`, `entity-disabled-by-default` (diagnostic entities), `entity-translations`, `icon-translations`, `exception-translations`, `reconfiguration-flow`, `repair-issues`, and the documentation rules (data update, known limitations, supported functions, troubleshooting, use cases, examples) |
| Platinum | `async-dependency` (the library is async, as planned), `inject-websession` (use HA's `aiohttp` session, `async_get_clientsession`), `strict-typing` |

### 6.6 Tests

`pytest-homeassistant-custom-component` extracts HA core's test plugins; its version maps
one-to-one to an HA release (0.13.375 pins `homeassistant==2026.10.4`, needs Python 3.14).
Fixtures that matter: `hass`, `enable_custom_integrations`, `MockConfigEntry`,
`aioclient_mock`, `snapshot` (syrupy) and the registries. The library gets its own tests
against a small fake server (aiohttp) with recorded fixtures; the integration mocks the
library, not HA. An end-to-end job runs the real `snapdog` binary (the demo backend exists)
against the library on every release. Cost: the test dependency moves with every HA
release, so the pin needs a monthly bump.

### 6.7 Packaging

One integration per repository, `hacs.json` at the root (`name`, `homeassistant` as the
minimum HA version, `zip_release` if wanted), `custom_components/snapdog/` with
`manifest.json` (`version`, `issue_tracker`, `documentation`, `codeowners`, `requirements`),
`translations/en.json` (custom integrations ship finished translations, `strings.json` and
`[%key:...%]` do not apply). CI: `hassfest` and the HACS action, both required for the
catalog. Releases tagged with the manifest version. The catalog is the same for HACS and the
built-in Marketplace.

A monorepo does not fit: the integration needs its own repository (HAI-DEC-3).

## 7. Stage 4: core

Needs a PyPI library with tagged releases and automated publishing, an OSI-approved licence
for it and its dependencies (SnapDog is GPL-3.0-only; the library can be separate and
permissive, a decision for the maintainer), Gold-level documentation on home-assistant.io,
and a code owner who reviews pull requests for years. Not recommended before the
integration has proven itself; revisit after Gold (HAI-DEC-5).

## 8. Entity model

`{b}` is the base topic, `<id>` is `server_id`. Stage 1 is MQTT discovery, stage 3 the
integration. "Availability" lists what makes the entity unavailable.

| SnapDog object | Stage 1: platform, source | Stage 3: platform, source | Availability |
|---|---|---|---|
| Server | `sensor` diagnostic: none new (version from `o`/`dev`) | `sensor` diagnostic: version, zone and client counts (`GET /system/status`) | S1: `{b}status`. S3: WebSocket |
| Zone playback | `sensor` enum `playing/paused/stopped/idle`: `zones/{n}/state` `.state` | `media_player` state | as above |
| Zone volume | `number` 0..100, state `volume_level * 100`, `command_template: {{ value / 100 }}` to `zones/{n}/volume/set` | `media_player` `volume_level` | as above |
| Zone mute | `switch`: state `is_volume_muted`, payloads `true`/`false` to `zones/{n}/mute/set` | `media_player` `is_volume_muted` | as above |
| Zone shuffle | `switch` via `.../shuffle/set` | `media_player` `shuffle` | as above |
| Zone repeat | `select` `off/one/all` via `.../repeat/set` (matches the accepted payloads) | `media_player` `repeat` | as above |
| Zone transport | five `button`s, `payload_press` `play`/`pause`/`stop`/`next`/`previous` to `.../control/set` | `media_player` features | as above |
| Now playing | `sensor` title, artist, album from `media_title` etc. | `media_player` attributes | as above; empty when idle |
| Track duration | `sensor` `duration` (s) | `media_duration` | as above |
| Track position | not offered (per-second updates, see §2) | `media_position` + `media_position_updated_at`, set at jumps | as above |
| Cover art | `image` with `url_topic` `media_image_url`; **only without API keys** | `media_image_url` through HA's proxy, or `async_get_media_image` (needs §5 item 2 on protected servers) | as above |
| Source / playlist | `number` (box) for the playlist index, or none; names are not available over MQTT | `select_source`, `source_list` from `/media/playlists` | as above |
| Active source type | `sensor` enum (`radio`, `airplay`, `spotify`, ...) from `source` | `app_name` | as above |
| Presence | `binary_sensor` (presence), `switch` (enabled), `number` (timeout, `config`) | same, through the API | as above |
| EQ preset | none (no list, API only) | `select_sound_mode` | as above |
| Client volume, mute | `number`, `switch` via `clients/{n}/volume/set`, `.../mute/set` | same, through the API | S1: `{b}status` and the client's `connected` (`availability_mode: all`) |
| Client latency | `number` (`config`), ms, via `clients/{n}/latency/set` (accepted range ±10 s) | same | as above |
| Client zone | `select` of zone names, `command_template` maps name to index | `select` | as above |
| Client connected | `binary_sensor` `connectivity`, `diagnostic` | same | `{b}status` only |

## 9. Coexistence

### 9.1 HA's Snapcast integration

What it is (core, `local_push`; as read in its source on the `dev` branch): one `media_player`
per Snapcast **client**, no group entity, features volume, mute, source (the group's stream)
and grouping, `join` adds a client to the group. It connects to the Snapcast **control port** (JSON-RPC, 1705 by default).

What that means for SnapDog:

- SnapDog's default (embedded) server opens only the streaming port. The control port number
  `snapcast.jsonrpc_port` is read by the *process* backend (a build with the
  `snapcast-process` feature), not by the embedded one; I found no JSON-RPC listener in
  `snapcast/embedded.rs`. The released binary uses the embedded server, so the core
  integration finds nothing to connect to.
- Where it does connect (process backend), its `select_source` would repoint a group's
  stream and its `join` would move clients between groups, both of which SnapDog treats as
  its own state (zone assignment, one stream per zone). Two writers on one model.
- Recommendation: do not use it for a SnapDog server. The SnapDog integration covers the
  same speakers with the zone model SnapDog actually has. The documentation of stage 3 says so.

### 9.2 Music Assistant

Music Assistant (MA) exposes **its** players to HA (`media_player` per MA player) and has a
Snapcast player provider. The provider's manifest says *unmaintained*; it can use an
external Snapserver but needs the JSON-RPC control port, a stream named `default`, TCP stream
sources on ports 4953 to 5153, and it renames groups and reassigns streams on the server.
SnapDog cannot offer the first requirement (§9.1) and would not want the last.

The clean route is the other way round: MA's "Home Assistant Media Players" provider turns
HA `media_player` entities into MA targets (documented as the fallback for devices without a
native provider; I did not verify which feature flags it requires, expect `PLAY_MEDIA`). A
SnapDog zone with `PLAY_MEDIA` is then a destination for MA, which hands it a stream URL
through `play_media`. This costs SnapDog nothing beyond the integration.

### 9.3 With stage 1

Stages 1 and 3 can run side by side (different unique-id namespaces), at the price of two
entity sets per zone. The integration's documentation should say how to switch MQTT
discovery off (`[mqtt.homeassistant] discovery = false`) once the integration is set up, and
the integration should offer a repair if both are active.

## 10. Security model

| Surface | Model |
|---|---|
| MQTT (stage 1) | Broker authentication and ACL decide who may write `.../set`. TLS and mutual TLS are supported since stage 0; the example config shows an ACL that confines SnapDog to its base topic. Discovery payloads contain no secrets. Anybody who may publish to a `/set` topic can control the audio |
| REST/WebSocket (stage 3) | One API key class today (all or nothing, constant-time comparison, `Authorization: Bearer`; the query token is accepted on `/ws` only). The integration uses the header on both and never logs the key. Recommend a dedicated key for HA, revocable on its own |
| Where the key lives | In the config entry (`ConfigEntry.data`); redacted from diagnostics (`async_redact_data`). I did not check how HA protects config entries at rest |
| TLS | If TXT `tls=true`, the integration verifies the certificate by default and offers an explicit "do not verify" switch for self-signed certificates, stored per entry and shown in diagnostics |
| Cover art | Never put the API key into a URL (it ends in logs and in HA's state machine). Use the signed short-lived URL (§5 item 2) or HA's proxy |
| Scope | With one key class, a compromised HA can reconfigure SnapDog. A read/control scope (§5 item 4) limits that |

## 11. Migration notes

**From the dead `media_player` discovery (stage 0, done).** SnapDog publishes an empty
retained message to `homeassistant/media_player/snapdog_zone_{n}/config` for `n` from 1 to
the larger of the zone count and 16, once per process start, and clears `{base}/status`
(double slash). HA never created entities from those messages, so there are no registry
entries to clean. Orphans for zone numbers above 16 (or from another discovery prefix) stay
on the broker; remove them by hand:

```sh
mosquitto_pub -r -n -t 'homeassistant/media_player/snapdog_zone_20/config'
```

If the broker ACL forbids the publish, the cleanup silently does nothing; it is harmless.

**From stage 1 to stage 3.** Unique ids differ (`snapdog_<server_id>_...` for MQTT, the
integration's own for the API), so both coexist; entity ids may clash by name (HA adds
`_2`). The integration's documentation lists the renames; there is no automatic migration.

**Reordering zones.** Today topics and unique ids follow list positions. Moving a `[[zone]]`
moves its entities onto another zone. Either document this or give zones and clients an
optional `id` (HAI-DEC-6) before stage 1 ships.

## 12. MQTT 5

Evaluated for stage 0, not adopted. rumqttc 0.25.1 ships a complete v5 client
(`rumqttc::v5`): reason codes in CONNACK/SUBACK/PUBACK, session expiry, user properties,
enhanced authentication fields, and per-subscription options `nolocal` and
`RetainForwardRule::Never` ("do not send retained messages on subscribe"). The last one would
make the retained-command guard unnecessary at the broker; `nolocal` avoids echoes. It does
not change anything stage 1 needs: availability, retained state and a last will are
identical, and discovery is plain 3.1.1/5-agnostic. Against it: a second type universe in
the bridge (every `AsyncClient`, `Event`, `Publish` differs), a protocol switch with a v3.1.1
fallback for brokers that refuse CONNECT (older Mosquitto, some embedded brokers), and no
way for me to test EMQX, HiveMQ or the Home Assistant Mosquitto add-on here (only Mosquitto
2.0.18 was run). The retained guard, the reconnect logic and the clear error messages
already cover what v5 would add. Revisit when a concrete need appears (for example, shared
subscriptions or message expiry on commands). A switch of client library would also reopen
ADR-022 (rustls).

## 13. Effort, risks, phasing

Estimates are for one developer who knows the code, in working days, without review waits.

| Stage | Content | Estimate |
|---|---|---|
| 0 | MQTT repair, TLS, validation, this RFC | done |
| 1 | Device discovery generator, birth-message handling, config keys, tests (golden payloads, a mock HA through `mqtt_mock`-style schema checks), docs | 4 to 6 |
| 2 | API additions §5 items 1 to 3 (items 4 to 6 optional, 2 to 5 days each) | 3 to 5 |
| 3a | Library `snapdog-api` (REST, WebSocket, models, tests) | 4 to 6 |
| 3b | Integration alpha: zeroconf, config flow, reauth, zone `media_player`, client entities | 6 to 9 |
| 3c | Bronze to Gold polish: diagnostics, repairs, translations, docs, test coverage | 5 to 8 |
| Upkeep | monthly HA release: test pin bump, deprecation reading | about 0.5 per month |
| 4 | Core submission, if wanted | 10 or more, mostly review waiting |

Risks:

1. **HA moves monthly.** Deprecations are announced a year ahead on the contributor blog and
   a custom integration gets no free pass; the cost is real and permanent.
2. **Two writers.** A second controller (the WebUI, KNX, MQTT, HA) is normal in SnapDog; the
   integration must treat the WebSocket as the source of truth and never assume optimistic
   state.
3. **The cover gap** (§5 item 2) and a position that is written too often (§2) show up as
   "broken" or "heavy" in the HA UI long before they are noticed in SnapDog; do the API work
   first and let the integration write position state only at jumps.
4. **Entity identity** (HAI-DEC-6): positions as ids bite on the first reorder.
5. **Maintainer load.** Silver requires an integration owner; with one maintainer the
   integration, the library and SnapDog all hang on the same person.
6. **Version drift.** HA 2026.10.x needs Python 3.14 or newer; the integration cannot choose.

## 14. Requirements, decisions, tasks

### Requirements

- **HAI-REQ-1** A zone must be controllable and observable from HA without hand-written YAML.
- **HAI-REQ-2** SnapDog must never publish a discovery message that HA cannot use.
- **HAI-REQ-3** TTS and announcements must be able to target a zone.
- **HAI-REQ-4** No credential travels in a URL or lands in a log or a diagnostics file.
- **HAI-REQ-5** An HA restart or a SnapDog restart must restore all entities without user action.

### Decisions

- **HAI-DEC-1** Do not build `media_player` over MQTT (impossible) and do not promote the
  universal media player; offer stage 1 entities plus the integration. *Recommended.*
- **HAI-DEC-2** Stage 1 publishes one device message, retained and after HA's birth message;
  `[mqtt.homeassistant] discovery` defaults to **off** until the entity set is approved, then
  on. *Open for the maintainer.*
- **HAI-DEC-3** The integration and its library live in separate repositories. *Recommended.*
- **HAI-DEC-4** The integration uses the REST and WebSocket API with the header for
  authentication, zeroconf discovery, no polling. *Recommended.*
- **HAI-DEC-5** Core submission is deferred until after Gold. *Recommended.*
- **HAI-DEC-6** Zones and clients get an optional stable `id` before stage 1 ships.
  *Open for the maintainer.*
- **HAI-DEC-7** No `GROUPING` in the first integration version; zone linking is a separate
  SnapDog design. *Open for the maintainer.*

### Tasks

- **HAI-T0** ✅ MQTT repair, TLS, validation (stage 0).
- **HAI-T1** Resolve HAI-DEC-2, -6, -7 with the maintainer.
- **HAI-T2** Stage 1: discovery generator with golden tests, birth-message resend, config.
- **HAI-T3** `GET /api/v1/eq/presets`.
- **HAI-T4** Remove the unused `ZoneProgress` notification or give it a job (a tick marker the
  integration can use instead of detecting jumps).
- **HAI-T5** WebSocket hello and snapshot message.
- **HAI-T6** `snapdog-api` library (separate repository).
- **HAI-T7** Cover access without a header (signed URL).
- **HAI-T8** Integration: config flow, zeroconf, reauth, zone `media_player`, client entities.
- **HAI-T9** Diagnostics, repairs, translations, documentation (Silver, Gold).
- **HAI-T10** Optional: API key scopes, stable ids, zone linking (§5 items 4 to 6).

## 15. Open questions for the maintainer

1. Is stage 1 wanted at all, or should the effort go straight to the integration? (It is
   cheap, and the only option for users without a custom integration, but it adds a second
   entity set that must be retired later.)
2. Stable ids for zones and clients: accept `id` in `[[zone]]` and `[[client]]`, and what
   happens to existing installations (positions stay the fallback)?
3. Who maintains the library and the integration, and is a permissive licence for the library
   acceptable (it matters only for stage 4)?
4. Is zone linking (party mode) on the roadmap? It decides whether `GROUPING` is ever honest.
5. Should API keys get scopes before an integration holds one?
6. Which HA versions to support: the integration can only follow HA's own Python and
   `DeviceInfo` changes; a floor of the Marketplace release (2026.11) simplifies tests.
7. Does the WebSocket snapshot (§5 item 3) belong in API version 1 or a version 2?

## 16. Sources and how they were read

Read on 2026-10-25. **Raw** = the page text downloaded and read in full; **summary** = the
answer of a page-reading tool that summarises (verified against a second source where noted).
Where a statement rests on a summary only, it is worded as "reported" or marked in the text.

Home Assistant user documentation
- https://www.home-assistant.io/integrations/mqtt/ (raw `current` markdown and page; the 32 platforms, device discovery, availability, birth message, removal) – raw and summary, consistent
- https://raw.githubusercontent.com/home-assistant/home-assistant.io/current/source/_integrations/mqtt.markdown – raw; platform list re-read by me, `media_player` absent
- https://www.home-assistant.io/integrations/{number,switch,select,sensor,binary_sensor,button,image,text,event,device_trigger,update,scene}.mqtt/ – summary
- https://www.home-assistant.io/integrations/media_player.mqtt/ – HTTP 404
- https://github.com/home-assistant/core/issues/152085 – summary, re-read by me (closed as not planned, 2025-09-11)
- https://www.home-assistant.io/blog/2024/11/06/release-202411/ and core pull request 118757 – device discovery in 2024.11
- https://www.home-assistant.io/blog/2026/04/01/release-20264/ – `object_id` removal; https://www.home-assistant.io/changelogs/core-2025.10/ (deprecation)
- https://www.home-assistant.io/integrations/universal/ – universal media player; https://www.home-assistant.io/integrations/snapcast/, https://www.home-assistant.io/integrations/music_assistant/ – summary
- https://www.home-assistant.io/integrations/tts/ – `tts.speak` takes `data.media_player_entity_id` – summary, read by me

Home Assistant developer documentation (moved from developers.home-assistant.io to contributors.home-assistant.io; all raw)
- https://contributors.home-assistant.io/core/entities/media-player/ (features, states, position, grouping, browse)
- https://contributors.home-assistant.io/core/integrations/{config-flow,reauth-reconfigure,options-flow,config-entries,manifest,create,file-structure,discovery,network-discovery,fetching-data,api-library,diagnostics,repairs,device-registry,setup-failures,media-source,translations}/
- https://contributors.home-assistant.io/core/entities/{entity-class,naming,registry,state-updates,lifecycle}/ and https://contributors.home-assistant.io/architecture/principles/
- https://contributors.home-assistant.io/core/quality-scale/ (tiers, rules, one page per rule; summary of the rule lists, counts re-added by hand: 20, 10, 21, 3)
- https://contributors.home-assistant.io/marketplace/ (guides/integrations, reference/repository-structure, validation, hacs-json) and https://contributors.home-assistant.io/glossary/custom-integration/ ("Marketplace, built into Home Assistant starting with 2026.11", read by me)
- https://contributors.home-assistant.io/core/development/testing/ ; https://github.com/MatthewFlamm/pytest-homeassistant-custom-component and its PyPI metadata (summary)
- Core source on the `dev` branch: `homeassistant/components/media_player/{const,entity,__init__,browse_media,services}.py`, `services.yaml`; `homeassistant/components/snapcast/*`; `homeassistant/components/music_assistant/*` (raw or reported by the reading tool, as marked in the research notes)

Music Assistant
- https://music-assistant.io/player-support/ , /player-support/snapcast/ , /integration/ , /faq/groups/ – summary

SnapDog (this repository, read in the code): `snapdog/src/main.rs` (mDNS TXT keys), `snapdog/src/api/{auth,ws}.rs`, `openapi.json`, `snapdog/src/snapcast/embedded.rs`, `snapdog/src/mqtt/*`, `docs/rfcs/0005-client-identity.md`.
