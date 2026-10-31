---
rfc: HAI-0010
title: The SnapDog Home Assistant integration, built and checkable – scope, API prerequisites, acceptance matrix
status: draft            # draft | accepted | in-progress | done | superseded
version: 0.1.0
created: 2026-11-01
updated: 2026-11-01
target_repo: snapdog     # RFC and API additions here; integration and library in two new repositories (HAI-DEC-3, confirmed 2026-11-01)
target_branch: main
related: [HAI-0007]
owners: [metaneutrons]
---

# RFC HAI-0010 – The SnapDog Home Assistant integration, built and checkable

> **For AI agents:** this RFC turns stages 2 and 3 of HAI-0007 into a build plan with a
> checkable definition of "complete". HAI-0007 stays the scoping document (why a
> `media_player` needs a custom integration, what MQTT discovery can and cannot do, security
> model, quality scale); this RFC does not repeat it. Requirements are `HAI-REQ-*` (continuing
> HAI-0007), decisions `HAI-DEC-*` (continuing at 8), API additions `S-*`, matrix rows
> `M-*`. Statements about SnapDog were read in this repository on 2026-11-01 (`openapi.json`,
> `snapdog/src/api/{ws,auth}.rs`, `snapdog/src/main.rs`, `docs/mqtt.md`). Statements about
> Home Assistant come from HAI-0007 §16 (read 2026-10-25) and from a read-only query of the
> maintainer's instance (§2). Anything marked **unverified** has to be checked in code before
> the matching work package starts.

## 1. Outcome and boundaries

**Outcome.** A Home Assistant (HA) user installs one custom integration, SnapDog is found by
zeroconf, and everything SnapDog exposes through its REST API and WebSocket is available in
HA: every zone as a `media_player` with local push, every client as a set of entities, the
server as a diagnostic device, services for what has no entity, translations in German and
English, diagnostics and repairs. "Complete" is defined by the acceptance matrix in §7, which
a script in CI checks against `openapi.json` and the WebSocket event list, so that a new
SnapDog endpoint or event without a matrix row fails the build.

**Route (decided in HAI-0007, confirmed here).** Both layers: the existing MQTT discovery
(stage 1, done, off by default) stays for users without a custom integration, and the native
custom component is the primary route. MQTT discovery published by the server cannot produce a
`media_player` (HAI-0007 §3), so it cannot be the primary route.

**Out of scope.** Core submission (HAI-DEC-5 of HAI-0007, after Gold). A Lovelace card. Music
Assistant provider code (a zone with `PLAY_MEDIA` is a target without it, HAI-0007 §9.2).
Changes to the MQTT bridge or to stage 1. Anything under `snapdog/src/config`, `snapdog/src/net` or
`snapdog/src/mqtt`; the interface-binding epic #377 (LBD-0008/0009) is not touched.

## 2. Baseline

- **SnapDog (`origin/main` at e6ffd2b).** REST API v1, 99 operations in `openapi.json`
  (zones, clients, media, system, knx, speakers). WebSocket `/ws` pushes `Notification`
  variants: `zone_changed` (full zone state, about once a second while a zone plays),
  `zone_volume_changed`, `zone_progress` (defined, **emitted nowhere**, HAI-0007 §2),
  `client_state_changed`, `zone_eq_changed`, `zone_presence_changed`, `playback_error`.
  Authentication is `Authorization: Bearer` (one key class), the query token only on `/ws`.
  mDNS `_snapdog._tcp` carries `api_version`, `server_id`, `snapcast_port`, `auth` (and
  `docker`, `tls`, `base_url` where they apply). The WebSocket has no hello and no snapshot:
  after a connect the client reads everything over REST. A client that falls behind is closed
  with code 1013 and has to reload.
- **Stage 1** (MQTT device discovery) is implemented behind `[mqtt.homeassistant]`.
- **The maintainer's Home Assistant** (`https://home.schmieder.eu`, read-only queries on
  2026-11-01 with the `HA_TOKEN` from fnox; nothing was written): version 2026.10.4, state
  `RUNNING`, not in safe mode. Loaded: `zeroconf`, `tts`, `media_player`. **Not loaded: `mqtt`
  and `hacs`**; no `snapcast`, no `snapdog`, no SnapDog or Snapcast entity. So today neither
  stage 1 nor a HACS install can be tried there without a change to that instance (§9).
- Stage 3 does not exist yet: no repository, no code.

## 3. Requirements and decisions

New requirements (HAI-REQ-1 to 5 stay):

- **HAI-REQ-6** Completeness is checkable: every REST operation, every WebSocket event type and
  every mDNS TXT key is either mapped to an HA entity or service with a test, or listed in an
  explicit exclusion table with a reason, and CI fails on the difference.
- **HAI-REQ-7** All user-facing text exists in German and English; a CI check fails on a
  missing key in either language.
- **HAI-REQ-8** The integration never relies on optimistic state: every state shown comes from
  the WebSocket or a REST read (HAI-0007 §13, risk 2).

Decisions (HAI-DEC-1 to 7 of HAI-0007 stay, with the outcomes below):

- **HAI-DEC-3 (confirmed).** Two new repositories under `SnapDogRocks`:
  `snapdog-homeassistant` (`custom_components/snapdog`, `hacs.json`, CI) and `pysnapdog` (async
  client, models, PyPI package of the same name). Both licensed Apache-2.0 (maintainer, 2026-11-01). Both are created locally
  first, with no remote and no push without consent per action. `pysnapdog` is developed in its
  own repository from the start and published to PyPI before the first integration release.
- **HAI-DEC-7 (decided: yes, as a separate design).** Zone linking (S-6) gets its own RFC
  before any code; `GROUPING` and the `join`/`unjoin` services depend on it and stay behind
  a capability flag until then.
- **HAI-DEC-8.** The integration needs no server change for covers. It fetches cover art itself
  with the header and hands it to HA through `async_get_media_image` and `media_image_hash`,
  so a protected server works. The signed cover URL proposed in HAI-0007 §5 item 2 is dropped
  from this plan: its only consumers are stage 1 (an MQTT matter, not touched here) and
  devices that cannot send a header (Cast, Music Assistant), which are not targets.
- **HAI-DEC-9.** Clients are entities on their own device, not `media_player` entities
  (HAI-0007 §6.4), with one exception to decide in §10 (open question 1).
- **HAI-DEC-10.** One config entry per server; zones and clients appear and disappear
  dynamically (`dynamic-devices`, `stale-devices`), driven by a REST reload when the zone or
  client set changes.
- **HAI-DEC-11.** The integration declares `iot_class: local_push`, minimum HA version 2026.10,
  Python as HA requires (3.14 for 2026.10).
- **HAI-DEC-12.** Entity identity is the server id plus the index (`unique_id` =
  `<server_id>_zone_<n>_<key>`). The order of `[[zone]]` and `[[client]]` in the configuration
  is the identity, as it already is for API paths, MQTT topics and KNX addresses. `snapdog.toml`
  defines the ids and they stay as they are until that file is edited: new blocks go at the
  end, while inserting, deleting or reordering a block shifts the ones behind it. The integration's documentation states this contract. This is a decision about
  the integration only. HAI-0007's HAI-DEC-6 concerns the MQTT entities and is untouched: stage
  1 shipped with indices and no `id`, and the MQTT bridge is not changed by this RFC.

## 4. Architecture

```text
snapdog (Rust)  --REST + /ws-->  pysnapdog (PyPI, async)  -->  custom_components/snapdog
S-1, S-3, S-4, S-6, S-7          models, client, WS runner        config flow, coordinator,
openapi.json, ws-events.json     coverage manifest                platforms, services, repairs
```

- **Library `pysnapdog`.** `SnapDogClient(session, host, port, api_key, tls)` with one method
  per operation, typed models generated from `openapi.json` (pinned copy, updated by script),
  `SnapDogSocket` with reconnect and backoff that yields typed events, and a
  `COVERAGE` manifest (operation → method) used by the CI check. It takes HA's `aiohttp`
  session (`inject-websession`).
- **Coordinator.** No `update_interval`. At setup: REST snapshot (`/zones`, `/clients`,
  `/media/playlists`, `/system/status`, `/system/version`, `/eq/presets`). Then the socket
  keeps `coordinator.data` current. After every reconnect, and after close code 1013, a new
  snapshot (S-3 replaces N calls with one message). Entities are unavailable while the socket
  is down, with one log line when lost and one when back.
- **Position.** `zone_changed` arrives about once a second while playing. The zone entity sets
  `media_position` and `media_position_updated_at` only when the position departs from the
  extrapolated value by more than a tolerance, or the track, state or duration changes
  (HAI-0007 §6.3). A unit test feeds a once-per-second stream and asserts a small number of
  state writes.
- **Discovery.** `manifest.json` `zeroconf: ["_snapdog._tcp.local."]`; the entry's `unique_id`
  is `server_id`; a changed address updates the entry. TXT `auth=true` leads to the API key
  step, `tls=true` to a verify switch.
- **Identity.** HAI-DEC-12: `unique_id` from `server_id` and the index, no migration layer.

## 5. API additions to SnapDog

Each is small and useful on its own. All are additive; API version stays 1 (open question 7
of HAI-0007 answered: additive means 1). None touches `snapdog/src/config`, `snapdog/src/net` or `snapdog/src/mqtt`.

| # | Addition | Needed for | Order |
|---|---|---|---|
| S-1 | `GET /api/v1/eq/presets` (names, and the band set of each) | `select_sound_mode`, client EQ preset select | first |
| S-3 | WebSocket `hello` (server id, version, `api_version`, capabilities such as `zone_links`) and `snapshot` (all zones, clients, EQ, presence, playlists), **sent only on opt-in** (`/ws?hello=1`, or a first client message), so the WebUI and other clients see no new message type | one-message resync (HAI-REQ-5, HAI-REQ-8) | first |
| S-4 | Optional API key scopes `read` and `control` | a HA key that cannot change configuration (HAI-0007 §10). A key without a scope keeps full rights, otherwise it would be breaking | after S-3 |
| S-6 | Zone linking (party mode): a runtime model that makes zones follow a leader's source | `GROUPING`, `join`, `unjoin` | own design RFC first |
| S-7 | A machine-readable description of the WebSocket events (`docs/api/ws-events.json`, generated from `Notification` by a test, so it cannot drift) and a golden-file test in SnapDog | HAI-REQ-6, shared fixtures for the library | first |

**Dropped from HAI-0007 §5, numbers kept so the discussion stays traceable.** S-2 (signed cover
URL): no consumer left, see HAI-DEC-8. S-5 (stable `id`): see HAI-DEC-12. Both would also have
touched code outside this RFC's boundary (MQTT state, `snapdog/src/config`).

**Breaking changes.** None. S-1, S-6 (once designed) and S-7 add endpoints, files or tests. S-3
is additive only because it is opt-in. S-4 is additive only while an unscoped key keeps all
rights.

**Emitting `zone_progress`** (HAI-T4) is dropped from the plan: the integration works with the
once-per-second `zone_changed`; the variant is either removed or documented as reserved as part
of S-7, so the events file does not list a dead event as live.

## 6. Entity and service model

Names are German-first in `translations/de.json`, English in `translations/en.json`; the
`media_player` has no name of its own (`has_entity_name`, device name is the zone name).

### 6.1 Devices

One `service` device for the server (diagnostics, update, KNX), one device per zone, one per
client, each with `via_device` to the server. A client's device is linked to its zone only
through the `zone` select, not through the device tree (a client moves).

### 6.2 Zone `media_player`

As HAI-0007 §6.3, plus the decisions that were still open there:

- **State** `playing`, `paused`, `idle` (SnapDog `stopped` and `idle`); never `off`.
- **Features** `PLAY`, `PAUSE`, `STOP`, `NEXT_TRACK`, `PREVIOUS_TRACK`, `SEEK` (only when
  `seekable`), `VOLUME_SET`, `VOLUME_STEP`, `VOLUME_MUTE`, `SHUFFLE_SET`, `REPEAT_SET`,
  `SELECT_SOURCE`, `SELECT_SOUND_MODE`, `PLAY_MEDIA`, `BROWSE_MEDIA`, `MEDIA_ENQUEUE` is not
  offered. `GROUPING`, `join`, `unjoin` only when the `hello` lists `zone_links` (S-6).
- **Source list** the unified playlists from `/media/playlists` (index 0 radio, 1 and up
  Subsonic). AirPlay and Spotify are receivers that start themselves: they show as `source`
  attribute and `app_name`, not as selectable sources (**unverified**: confirm in the
  `state` code that no API call selects them).
- **`play_media`** `media_type` `url` → `POST /zones/{n}/play/url`; `playlist` →
  `.../play/playlist`; `track` → `.../play/playlist/{p}/track` or `.../play/subsonic/{id}`;
  `media_source://` ids are resolved by HA first. Announce is not supported.
- **Browse** root → playlists and radio → tracks; thumbnails through HA's proxy.
- **Attributes** `media_title/artist/album/album_artist/track/duration/position`,
  `media_content_type` (`music`), `media_playlist` (playlist name), `app_name` (source),
  plus extra state attributes for genre, year, disc, bitrate, content type, can-next/prev,
  playlist can-next/prev.

### 6.3 Other entities

| Object | Entities |
|---|---|
| Zone | `binary_sensor` presence; `switch` presence control (`config`); `number` presence timeout (`config`); `binary_sensor` presence timer (`diagnostic`); `switch` EQ enabled; `event` playback error (`playback_error`, with message, details, recoverable); `button` next and previous playlist; `sensor` source, bitrate, content type (`diagnostic`, disabled by default) |
| Client | `number` volume (respecting `max_volume`); `switch` mute; `number` latency (`config`, ms, ±10000); `select` zone; `select` speaker correction (options from `/speakers`); `select` EQ preset and `switch` EQ enabled (only when `is_snapdog`); `binary_sensor` connected (`diagnostic`); name and icon as device name and entity icon attribute, rename through a service |
| Server | `sensor` version, zone count, client count, radio count (`diagnostic`); `update` and `button` check for update, from `/system/updates` (only when the sidecar answers); `switch` KNX programming mode (only when KNX is enabled) |

### 6.4 Services

`snapdog.play_playlist_track` (zone, playlist, track), `snapdog.set_zone_eq` and
`snapdog.set_client_eq` (full band list, one band), `snapdog.move_client` (client, zone),
`snapdog.rename_client`, `snapdog.apply_speaker_profile`, `snapdog.check_for_update`. Every
service has a `services.yaml` entry, a schema, translations and an exception translation for
a rejected request (400 with the SnapDog `ErrorBody` message).

## 7. Acceptance matrix

A row is done when its test exists, passes in CI, and the row's "Live" cell (where set) has
been checked once against a real server. Test names are modules in the repositories named in
§8: `L:` library (`pysnapdog/tests`), `I:` integration
(`snapdog-homeassistant/tests`), `E:` end to end against the real `snapdog` demo backend,
`S:` in the `snapdog` repository. **Live** = checked by hand on a running HA (§9), `—` = not
needed.

### 7.1 REST operations

Group totals come from `openapi.json` (99 operations). The coverage script (§8, WP-1)
checks each operation individually; the table groups for reading.

| ID | SnapDog operation(s) | HA entity or service | Tests | Live |
|---|---|---|---|---|
| M-Z01 | `GET /zones`, `/zones/{n}`, `/zones/count`, `/zones/{n}/name`, `/icon`, `/clients` | zone devices and their entity set, device name, icon attribute | L: `test_zones`; I: `test_setup_devices` | yes |
| M-Z02 | `POST .../play`, `/pause`, `/stop` | `media_player` play, pause, stop | L: `test_transport`; I: `test_media_player_transport` | yes |
| M-Z03 | `POST .../next`, `/previous` | `media_next_track`, `media_previous_track` | same | yes |
| M-Z04 | `PUT .../volume`, `GET .../volume` | `media_player.volume_set`, `volume_up/down` | L: `test_volume`; I: `test_media_player_volume` | yes |
| M-Z05 | `PUT .../mute`, `POST .../mute/toggle` | `volume_mute` (toggle is not needed: HA sends the absolute value) | same | yes |
| M-Z06 | `PUT .../shuffle`, `POST .../shuffle/toggle` | `shuffle_set` | I: `test_media_player_shuffle_repeat` | yes |
| M-Z07 | `PUT .../repeat`, `POST .../repeat/toggle` | `repeat_set` (`off`, `one`, `all`) | same | yes |
| M-Z08 | `PUT .../track/position`, `PUT .../track/progress`, `GET` of both | `media_seek` (position); progress derived, not exposed | I: `test_media_player_seek` | yes |
| M-Z09 | `GET .../playback`, `/track`, `/track/*` (title, artist, album, duration, metadata, playing, position, progress) | media attributes from the snapshot and `zone_changed`; REST reads only in the snapshot and library | L: `test_track_reads`; I: `test_media_player_state` | yes |
| M-Z10 | `GET/PUT .../playlist`, `/playlist/count`, `/info`, `/name`, `POST .../play/playlist`, `.../next/playlist`, `.../previous/playlist` | `select_source`, `source_list`, `media_playlist`, buttons next and previous playlist | I: `test_source_select`, `test_playlist_buttons` | yes |
| M-Z11 | `POST .../play/track`, `.../play/playlist/{p}/track`, `.../play/subsonic/{id}` | `play_media` `track`, `snapdog.play_playlist_track` | I: `test_play_media_track`, `test_service_play_playlist_track` | yes |
| M-Z12 | `POST .../play/url` | `play_media` `url`, `tts.speak` target, `media_source` | I: `test_play_media_url`, `test_tts_target` | yes (TTS) |
| M-Z13 | `GET .../cover` | `async_get_media_image`, `media_image_hash`; proxy through HA | I: `test_cover_with_and_without_key` | yes |
| M-Z14 | `GET/PUT .../eq`, `PUT .../eq/{band}`, `POST .../eq/preset` | `select_sound_mode`, `switch` EQ enabled, `snapdog.set_zone_eq` | I: `test_sound_mode`, `test_service_zone_eq` | yes |
| M-Z15 | `GET/PUT .../presence`, `/presence/enable`, `/presence/timeout`, `GET /presence/timer` | presence binary sensor, switch, number, timer sensor | I: `test_presence_entities` | yes |
| M-C01 | `GET /clients`, `/clients/{n}`, `/clients/count`, `/connected` | client devices, `binary_sensor` connected | L: `test_clients`; I: `test_client_devices` | yes |
| M-C02 | `GET/PUT .../volume` | `number` volume | I: `test_client_volume` | yes |
| M-C03 | `GET/PUT .../mute`, `POST .../mute/toggle` | `switch` mute | I: `test_client_mute` | yes |
| M-C04 | `GET/PUT .../latency` | `number` latency | I: `test_client_latency` | yes |
| M-C05 | `GET/PUT .../zone` | `select` zone, `snapdog.move_client` | I: `test_client_zone`, `test_service_move_client` | yes |
| M-C06 | `GET/PUT .../name`, `GET .../icon` | device name, `snapdog.rename_client`, icon attribute | I: `test_client_rename` | — |
| M-C07 | `GET/PUT .../speaker`, `GET /speakers`, `GET /speakers/{name}/profile` | `select` speaker correction, `snapdog.apply_speaker_profile`; profile body only in the library | L: `test_speakers`; I: `test_client_speaker` | yes |
| M-C08 | `GET/PUT .../eq`, `PUT .../eq/{band}`, `POST .../eq/preset` | `select` EQ preset, `switch` EQ enabled, `snapdog.set_client_eq`; absent when not `is_snapdog` | I: `test_client_eq`, `test_non_snapdog_has_no_eq` | yes |
| M-M01 | `GET /media/playlists`, `/{p}`, `/{p}/tracks`, `/{p}/tracks/{t}` | `browse_media`, `source_list` | I: `test_browse_media` | yes |
| M-M02 | `GET /media/playlists/{p}/cover`, `.../tracks/{t}/cover` | browse thumbnails through `async_get_browse_image` | I: `test_browse_thumbnail` | yes |
| M-S01 | `GET /system/status`, `/system/version` | server sensors, device `sw_version` | I: `test_server_sensors` | yes |
| M-S02 | `GET /system/updates`, `POST .../check`, `.../apply`, `.../config` | `update` entity (install), `button` check, `snapdog.check_for_update`; mode and window as `select` and `time` (`config`) | I: `test_update_entity` | when a sidecar runs |
| M-S03 | `GET /system/listeners` | diagnostics dump only | I: `test_diagnostics` | — |
| M-S04 | `GET/PUT /knx/programming-mode` | `switch` (only when KNX is enabled) | I: `test_knx_programming_mode` | when KNX runs |
| M-S05 | `GET /knx/knxprod`, `/knx/product-info` | **excluded**: a download for ETS, not a runtime state | coverage exclusion table | — |
| M-S06 | `GET /eq/presets` (S-1) | sound-mode and preset lists | L: `test_eq_presets` | yes |

### 7.2 WebSocket, discovery, lifecycle

| ID | Capability | HA behaviour | Tests | Live |
|---|---|---|---|---|
| M-W01 | `zone_changed` | updates the zone state; position written at jumps only | I: `test_ws_zone_changed`, `test_position_write_rate` | yes |
| M-W02 | `zone_volume_changed` | volume and mute without a full state | I: `test_ws_volume` | yes |
| M-W03 | `client_state_changed` | client volume, mute, connected, zone; device appears or disappears | I: `test_ws_client_state`, `test_dynamic_devices`, `test_stale_devices` | yes |
| M-W04 | `zone_eq_changed` | EQ entities | I: `test_ws_eq` | yes |
| M-W05 | `zone_presence_changed` | presence entities | I: `test_ws_presence` | yes |
| M-W06 | `playback_error` | `event` entity and a persistent repair-free log line; recoverable flag in the event data | I: `test_ws_playback_error` | yes |
| M-W07 | `zone_progress` | **excluded** until emitted (S-7 decides); the event file marks it reserved | coverage exclusion table | — |
| M-W08 | `hello` and `snapshot` (S-3, requested by the integration on connect) | version and `api_version` check, capability flags, full resync | L: `test_hello_snapshot`; I: `test_resync_after_reconnect` | yes |
| M-W09 | Socket loss, close 1001 and 1013, 64-connection limit (HTTP 503) | entities unavailable, one log line each way, backoff, repair for a refused connection | I: `test_reconnect`, `test_too_slow_close`, `test_ws_limit_repair` | yes |
| M-D01 | mDNS `_snapdog._tcp` | zeroconf flow, confirm step, `unique_id` = `server_id` | I: `test_zeroconf` | yes |
| M-D02 | TXT `server_id` change of address | entry updated, no duplicate | I: `test_zeroconf_updates_host` | yes |
| M-D03 | TXT `auth`, `tls`, `base_url`, `snapcast_port`, `docker`, `api_version` | API key step, TLS verify switch, repair for a `localhost` base URL, diagnostics, version check | I: `test_flow_auth`, `test_flow_tls`, `test_repair_base_url`, `test_api_version_repair` | yes |
| M-A01 | Bearer key, 401 | key step, `reauth` flow on `ConfigEntryAuthFailed`, `reconfigure` for host and port | I: `test_reauth`, `test_reconfigure` | yes |
| M-A02 | Key scopes (S-4) | the flow tells which scope the key has; a `read` key makes controls unavailable with an explanation | I: `test_scoped_key` | when S-4 ships |
| M-X01 | Diagnostics | `async_get_config_entry_diagnostics`, key, host and `base_url` redacted | I: `test_diagnostics_snapshot` (syrupy) | yes |
| M-X02 | Both MQTT discovery and the integration active | repair that names the way to switch discovery off | I: `test_repair_double_entities` | when MQTT is present |
| M-X04 | Zone linking (S-6) | `GROUPING`, `join`, `unjoin`, `group_members` when the capability is set | I: `test_grouping` | when S-6 ships |
| M-X05 | Translations | `de.json` and `en.json` with equal keys, entity, service, exception, issue and flow strings | CI: `check_translations.py` | yes (UI language switch) |

### 7.3 MQTT and stage 1

Out of the matrix. The MQTT bridge and stage 1 are finished (HAI-0007 §2, §4.3, `1d00fd6`),
keep their own tests in `snapdog` and are not changed by this RFC. The integration does not use
MQTT. The only link is M-X02: a repair when both entity sets are active.

### 7.4 What "100 %" means, mechanically

1. `pysnapdog` CI runs `scripts/check_coverage.py`: every `operationId` or
   path+method in the pinned `openapi.json` must appear in `COVERAGE` (library method) **and**
   in the integration's `matrix.yaml` (row id, or an exclusion with a reason). The same for
   every `type` in `ws-events.json` (S-7).
2. `snapdog-homeassistant` CI runs `scripts/check_matrix.py`: each row id in
   `docs/matrix.yaml` names a test that exists (collected by pytest); a row without a test, or
   a test marked `@pytest.mark.row("M-…")` for no row, fails.
3. The matrix in this RFC and `matrix.yaml` must list the same ids; a CI step in
   `snapdog-homeassistant` compares them with a copy of this table pinned at the RFC version.
4. Coverage: more than 95 % overall, 100 % of `config_flow.py` (Bronze and Silver).
5. "Live" cells are checked once per release candidate and recorded in the release notes.

## 8. Work packages and order

API additions first, because the integration reads their output. Nothing here is pushed or
opened as a pull request without the maintainer's yes for that action.

| WP | Content | Where | Depends on |
|---|---|---|---|
| WP-0 | This RFC accepted; HAI-0007 status line and §14 updated to point here | `snapdog` branch `docs/rfc-ha-integration` | – |
| WP-1 | S-1, S-3, S-7 with golden tests (`S:` rows) | `snapdog`, own branch, no config or net files | WP-0 |
| WP-2 | Library skeleton: client, models from `openapi.json`, socket, fake-server tests, `check_coverage.py` | `pysnapdog` | S-3, S-7 for the full set; starts on REST |
| WP-3 | Integration alpha: config flow, zeroconf, reauth, coordinator, zone `media_player` (transport, volume, mute, shuffle, repeat, position) | `snapdog-homeassistant` | WP-2 |
| WP-4 | Sources, `play_media`, browse, covers, sound mode, TTS target | same | S-1 |
| WP-5 | Client entities, presence, EQ, speaker, server, update, KNX, services, `event` | same | WP-3 |
| WP-6 | Diagnostics, repairs, translations, dynamic and stale devices, docs, hassfest and HACS actions, matrix scripts | same | WP-5 |
| WP-8 | S-4 (scopes) | `snapdog` | WP-1 |
| WP-10 | S-6: own RFC, then code and `GROUPING` | `snapdog`, integration | RFC accepted |
| WP-11 | End-to-end job against the real `snapdog` demo backend; Live pass | CI and a HA | WP-6 |

Rough effort for one developer who knows the code: WP-1 3 to 4 days, WP-2 4 to 6, WP-3 5 to 7,
WP-4 3 to 4, WP-5 5 to 7, WP-6 4 to 6, WP-8 2, WP-10 design plus 6 to 10,
WP-11 2 (about 35 to 50 working days with S-6, 25 to 35 without). These are estimates, not
measurements.

### CI and quality gates

- **Library:** `ruff`, `mypy --strict`, `pytest` against an `aiohttp` fake server with
  recorded fixtures, `check_coverage.py`, a release job with PyPI trusted publishing (needs the
  maintainer to configure the PyPI project; not done by the agent).
- **Integration:** `ruff`, `mypy`, `pytest-homeassistant-custom-component` (pin follows the HA
  release: 0.13.375 for 2026.10.4, HAI-0007 §6.6), `home-assistant/actions/hassfest`,
  `hacs/action` (category `integration`), `check_translations.py`, `check_matrix.py`, syrupy
  snapshots for entities and diagnostics. A monthly job bumps the pin and runs the suite.
- **End to end:** a job starts the `snapdog` binary (demo backend, HAI-0007 §6.6) and runs the
  library and the integration's `E:` tests against it.
- Conventional commits, release-please, `commitlint` (body at most 100 characters per line),
  as in the other SnapDog repositories.

## 9. Verification against the real Home Assistant

The maintainer's instance is 2026.10.4 without `mqtt` and `hacs`.

1. **Read-only first (done on 2026-11-01).** Version, state and loaded components. Repeat
   before every Live pass; it needs only `HA_TOKEN` from fnox and writes nothing.
2. **Development target.** A local HA 2026.10.4 container (as used for stage 1) with
   `custom_components/snapdog` mounted, against a local `snapdog` and the demo backend. All
   automated Live rows run here.
3. **The maintainer's instance, only with a decision from the maintainer.** Installing the
   component means copying files into its config directory or adding HACS, and entering the
   SnapDog API key into a config flow. The agent does not do those steps from chat. What the
   agent may do afterwards, read-only, with `HA_TOKEN`: list the SnapDog entities and their
   states, read the entity registry and the diagnostics download, and read the error log.
   Service calls that play audio (a write) run only after a yes for that call and against a
   zone the maintainer names.
4. **Stage 1 on that instance** needs the MQTT integration and a broker; that is the
   maintainer's change to make, not this RFC's.

## 10. Risks and open questions

Risks as in HAI-0007 §13, plus:

1. **Reordering the configuration.** The index is the identity (HAI-DEC-12). If a user reorders `[[zone]]`, HA attaches history and areas to the wrong zone without an error. The documentation says so; if reports come in, S-5 can be revisited.
2. **S-6 is a design, not a task.** If zone linking is not wanted, M-X04 becomes an exclusion
   and "complete" is declared without grouping. This is the only row that can fall out of 100 %.
3. **The matrix counts operations, not behaviour.** A row that passes with a mock can still be
   wrong against the real server. The `E:` tests and the Live column exist for that; they are
   thinner than the mocks.
4. **Python and HA drift** (HAI-0007 risk 6) applies to both repositories.

Open questions for the maintainer:

1. Should a client also be offered as a `media_player` (volume, mute, source of its zone) for
   voice assistants? It doubles entities; the default here is no (accepted 2026-11-01).
2. How is the maintainer's HA hosted (OS, container, core)? The maintainer wants to test against
   `https://home.schmieder.eu` as well, so a way to place `custom_components/snapdog` there is
   needed (§9, item 3). Until then: local container, read-only checks of the real instance.
3. Minimum HA version 2026.10 (accepted 2026-11-01).
