# MED-0011 implementation plan: server first, then every consumer

Plan for [RFC MED-0011](0011-media-api-identity-search-management.md), version 0.2.0.
The RFC owns requirements and acceptance criteria (`MED-M*-A*`). This plan owns
the order of work across repositories, the changes per repository and which work
is tracked in issues. It states no dates, because none has a measured basis.

## 1. Order and rules

1. **The server comes first, and consumers follow its release.** Every consumer
   needs `playlist_id`, `index` and `playlist_index` from a released server, and
   the specifications they pin (`openapi.json`, `asyncapi.json`) come from that
   release. No consumer merges work that needs a field before the server release
   that carries it exists.
2. **Consumers first move off the deprecated names, then use the new features.**
   The guard of RFC 2.8 blocks SnapDog 1.0 while `id` and `ZoneChanged.playlist`
   are deprecated, so every consumer has to stop reading and sending them. That is
   its own exit criterion per consumer (section 4), separate from new features.
3. **One kind of change per pull request** (repository standard). Moving off `id`,
   adding a feature and updating pinned specifications are separate pull requests.
4. **Old clients keep working.** Until 1.0 nothing in this plan removes a field.
   Published apps (iOS, Android) cannot be updated together with the server, which
   is why the removal waits for 1.0 and the guard.

## 2. Findings that shape the plan

**`snapdog/tests/ci_guard.rs`** (56 lines, read on `main`). It holds two tests. One
reads `.github/workflows/ci.yml` with `include_str!` and checks that the CI still
runs four named feature-gated test files; the other checks that those files exist.
The release guard of RFC 2.8 fits the same pattern: another test in this file,
reading `../../Cargo.toml`, `../../.release-please-manifest.json`,
`../../openapi.json` and `../../docs/api/asyncapi.json` with `include_str!`. Facts
the guard design depends on:

- The workspace version is `0.29.1` in `Cargo.toml` and in
  `.release-please-manifest.json` on `main`. `toml` is already a workspace
  dependency of `snapdog`, so parsing needs nothing new. `serde_json` is used by the
  neighbouring tests.
- `openapi.json` carries `info.version = "1.0.0"`, and `openapi_contract.rs`
  asserts it. That is the version of the document, not the product. **The guard must
  read the product version and never `info.version`**, or it would fail today. A test
  for exactly this goes into the guard (counter-probe).
- `openapi.json` and `docs/api/asyncapi.json` are already compared against what the
  code generates (`openapi_contract.rs`, `asyncapi_contract.rs`), so scanning the
  checked-in files for `deprecated` scans the generated documents.
- The AsyncAPI document is built by hand in `api/asyncapi.rs`. `"deprecated": true`
  is written there by hand and the existing contract test compares it.
- The release workflow starts by `workflow_dispatch` from release-please only (its
  own comment says so). A tag pushed by hand starts nothing. The check in
  `CI Success` is therefore the one that cannot be bypassed, and the check in the
  release workflow is a second line. RFC 2.8 was corrected accordingly.
- **Not verified:** whether `utoipa` 6 emits `deprecated: true` for a field. The
  source of `utoipa-gen` 6.0.1 handles a `Deprecated` feature on named fields. A
  Rust `#[deprecated]` on the struct field would raise compiler warnings at every
  read and trip `-D warnings`, so the plan is a spike (WP-S1) that finds the
  attribute form that emits the flag without warnings.

**Consumers found by search** (files that mention playlists outside the server
crate, excluding docs and translations):

| Repository | Touchpoints for this plan |
|---|---|
| `snapdog` `webui` | `src/lib/api.ts` (sends `{ id: playlistIndex }`, reads `PlaylistInfo`), `components/PlaylistBrowser.tsx` (`pl.id` as key, selection, play, cover path), `lib/types.ts`, `stores/useAppStore.ts`, `components/ZoneDetail.tsx` |
| `snapdog` `demo-backend` | `server.js`: a mock of the API. Its zone state already has a field `playlist_id` that holds a **number** (the index, lines 397, 423, 441, 629) and a track list with `id`. It must be aligned or the mock contradicts the new contract |
| `snapdog-ios` | The shared packages `SnapDogKit` (`APIClient.swift` with `["id": playlistId, ...]`, `Models.swift`, `WsNotification.swift` with `playlist`, `AppState.swift`, `DemoCatalog.swift`) and `SnapDogUI` (`PlaylistBrowserView.swift` and the navigation views), tests. All Apple targets build on these packages: iOS, iPadOS, **macOS** (`SnapDogMac`, macOS 26; the packages declare macOS 14), watchOS and visionOS. `SnapDogMac` and `SnapDogVision` hold no playlist code of their own. The **watch app** does: `SnapDogWatchApp.swift` (`WatchPlaylistView` lists `PlaylistInfo`, passes `playlist.id` on as the index to `WatchTrackListView`, which calls `playPlaylist` and `listTracks(playlistId:)`) |
| `snapdog-client-android` | `data/api/KtorSnapDogApi.kt` (`mapOf("id" to playlistId, ...)`), `SnapDogApi.kt`, `domain/model/WsNotification.kt` (`playlist`), `SnapDogViewModel.kt`, `LibraryPanel.kt`, demo API and its test |
| `pysnapdog` | `client.py` (`{"id": playlist}`), `models.py`, `events.py` (`playlist`), pinned `spec/*.json`, tests |
| `snapdog-homeassistant` | `coordinator.py` (maps `event.playlist` to `playlist_index`), `media_player.py`, tests. Depends on `pysnapdog` |
| `snapdog-web` | API pages (`api-rest`, `api-websocket`, `api-reference`), `assets/openapi.json` |
| `snapdog` docs | `docs/mqtt.md`, `docs/knx-bridge.md`, `docs/upgrading.md`, `docs/api/asyncapi.json`, `openapi.json` |
| Not affected | `snapdog-os` (its `playlist` fields are KNX group-object names of the server configuration), the macOS **server** app in `snapdog/macos-helper` (it never calls the REST API; its only hit is the same KNX name list in `ConfigView.swift`), `snapdog-client` (D-Bus), `snapcast-rs`, `shairplay-rust` |

The Android client is missing from the project overview in the workspace
`CLAUDE.md`. It is a consumer and is in the plan.

## 3. Work packages

Size is relative and counts only what the search above found: **S** a few lines or
one file, **M** several files in one component with tests, **L** a new feature or
several components. None of it is a time estimate.

### Phase 0: preparation

| WP | Repository | Work | Size |
|---|---|---|---|
| WP-0a | `snapdog` | Push the RFC and this plan as a pull request, review, accept (`status: accepted`) | S |
| WP-0b | `snapdog` | Spike WP-S1: which `utoipa` 6 attribute emits `deprecated: true` on a field without compiler warnings. Result goes into the M1 issue | S |
| WP-0c | `snapdog` | Spike WP-S2: measure `search3` on Navidrome and Gonic (`songCount`, `albumCount`, paging) and playlist id stability across rescans. Answers two open questions of the RFC | S |

### Phase 1: server (`snapdog`), M1 split into three pull requests

| WP | Work | RFC | Depends | Size |
|---|---|---|---|---|
| WP-1a | **Identity.** `playlist_id` and `index` in `PlaylistInfo`, `PlaylistDetails`, `ZonePlaylistInfo`, zone state; `playlist_id` and `playlist_index` in `ZoneChanged`; play body accepts `index` and `id`; `by-id` endpoints (media reads, zone `playlist/by-id`, `play/playlist/by-id`); MQTT `playlist/by-id/set` and `playlist_id` in the state payload; deprecation flags and notice text in OpenAPI and AsyncAPI; regenerate both files; docs (`mqtt.md`, `knx-bridge.md`, `upgrading.md`) | 2.1, A1, A2, A2a, A6 | WP-0b | L |
| WP-1b | **Events and paging.** `PlaylistsChanged`, the refresh task with `[subsonic].refresh_secs`, `limit` and `offset` with `X-Total-Count`, a test that keeps the last good list across a refresh run | 2.2, 2.3, A3 to A5 | WP-1a (shares the cached list and the AsyncAPI file) | M |
| WP-1c | **Release guard.** Test in `tests/ci_guard.rs`, check in `release.yml`, probes for `1.0.0-rc.1` against a description with and without deprecated entries, and against `0.30.0`; the probe that `info.version` is ignored | 2.8, A7 | WP-1a | S |

WP-1c can start as soon as WP-1a has an entry to point at, and has to be merged
**no later than** WP-1a's release, so that no release ever exists without it.
Already delivered: PR #406 (last good list on outage).

### Phase 2: server release

| WP | Work | Size |
|---|---|---|
| WP-2 | Release the next minor with M1 through the normal release-please flow, then the pins follow: `snapdog` OS bump PR if the chain requires it (existing release chain). Publishes `openapi.json` and `asyncapi.json` that all consumers take | S |

### Phase 3: consumers, first move off `id`, then new features

Phase 3 starts when WP-2 has shipped. Within it the order is dictated by
dependencies. Rows in the same group can run in parallel.

**Group A, libraries and docs** (no dependency among them)

| WP | Repository | Work | Size |
|---|---|---|---|
| WP-3a | `pysnapdog` | Refresh `spec/*.json` from the release. Models: `index`, `playlist_id` in `PlaylistInfo` and the zone playlist fields. Events: `playlist_index`, `playlist_id`, new `PlaylistsChanged` event. Client: send `index` in `play_playlist`, add `by-id` methods. Release 0.2.0 (still 0.x). Drops reads of `id` and `playlist` | M |
| WP-3b | `snapdog-web` | API pages and the bundled `openapi.json` for the new fields, endpoints, the event, the deprecation notice and the 1.0 break | M |
| WP-3c | `snapdog` `demo-backend` | Align the mock: numeric `playlist_id` becomes `playlist_index`, add `index`, `playlist_id` strings and `playlists_changed` | S |

**Group B, clients that must leave `id` before 1.0** (after A where they use the library)

| WP | Repository | Work | Size |
|---|---|---|---|
| WP-3d | `snapdog` `webui` | `api.ts` sends `index`, `PlaylistBrowser.tsx` keys and selects by `playlist_id`, types and store use `playlist_index` and `playlist_id`, handle `playlists_changed` by reloading the list | M |
| WP-3e | `snapdog-ios` (all Apple targets) | In the shared packages: `APIClient` sends `index`; `Models` and `WsNotification` read `index`, `playlist_index`, `playlist_id`; browser keyed by `playlist_id`; demo catalogue and tests. In the watch app: `WatchPlaylistView` and `WatchTrackListView` take `playlist_id` or `index` instead of `playlist.id`. Build and test every target (iOS, macOS, watchOS, visionOS), because they share the packages. New app version, the store release is a separate decision | M, with a wider qualification than the code change |
| WP-3f | `snapdog-client-android` | Same as iOS in `KtorSnapDogApi`, `WsNotification`, `SnapDogViewModel`, `LibraryPanel`, demo API and test | M |
| WP-3g | `snapdog-homeassistant` | Move the coordinator to `event.playlist_index`; depends on WP-3a (`pysnapdog==0.2.0`). First media browser: `BROWSE_MEDIA`, `play_media` and `select_source` by `playlist_id`, `app_name` only for the AirPlay and Spotify override | L |

### Phase 4: server M2 and M3, then their consumers

| WP | Work | RFC | Depends | Size |
|---|---|---|---|---|
| WP-4a | **Search and album.** `search3` in the Subsonic client, `GET /media/search`, `GET /media/cover/{cover_id}`, album playback (spike on the player queue first: the RFC's open question) | 2.4, M2 | WP-2 | L |
| WP-4b | **Management.** write calls in the Subsonic client, `POST/PATCH/PUT/DELETE`, `playlist_write` switch, 403/502 behaviour, configuration reference | 2.5, M3 | WP-2 | L |
| WP-4c | Release the next minor with M2 and M3 | | WP-4a, WP-4b | S |
| WP-4d | Consumers: `pysnapdog` search and management methods; HA search in the media browser and playlist services; web interface search; iOS and Android only if wanted | | WP-4c | L |

### Phase 5: close out

| WP | Work |
|---|---|
| WP-5a | Every consumer in section 4 meets its exit criterion; the guard stays in `CI Success` |
| WP-5b | Before 1.0, remove `id` and `ZoneChanged.playlist` in a `feat!:` pull request. The guard passes only then. `/api/v1` keeps its name |

## 4. Exit criterion per consumer: no deprecated names

Each consumer proves it with a test or a recorded search, so the claim does not
rest on reading code:

| Consumer | Evidence |
|---|---|
| `pysnapdog` | Tests build play bodies with `index` and models without `id`; a repository check that no source file reads the deprecated names |
| `snapdog-homeassistant` | The same through `pysnapdog` 0.2.0, plus the coordinator test with an event that has `playlist_index` and no `playlist` |
| `webui` | `grep` check in CI for `{ id:` in playlist calls, or a typed client that has no `id` field |
| `snapdog-ios` (packages and watch app), Android | A unit test that encodes the play body and asserts no `id` key; decode tests against a payload without `playlist`; the watch app has no use of `playlist.id` left. One test in `SnapDogKit` covers iOS, macOS and visionOS, which share it |
| Server docs | `openapi.json` marks the entries; the guard in WP-1c |

## 5. Tracking: where issues are worth it

Rule from the planning reference: one epic and one issue per milestone, an
ordinary link elsewhere. Small work stays a plain pull request.

| Record | Where | Why |
|---|---|---|
| **Epic** "MED-0011 media API", index of the issues below | `snapdog` | Cross-repository, six repositories involved |
| Milestone issue **M1** (WP-1a to WP-1c, spikes, release) | `snapdog` | Three pull requests and a release, a blocker for everything else |
| Milestone issue **M2** (WP-4a) and **M3** (WP-4b) | `snapdog` | Large, with an open question on the player queue |
| Consumer issue for **`pysnapdog`** (WP-3a, later WP-4d part) | `pysnapdog` | Two releases, other repositories depend on it |
| Consumer issue for **Home Assistant** (WP-3g, later WP-4d part) | `snapdog-homeassistant` | The largest consumer item, has its own acceptance |
| Consumer issue for **iOS** (WP-3e) | `snapdog-ios` | Store release is a separate decision and can take weeks |
| Consumer issue for **Android** (WP-3f) | `snapdog-client-android` | Same |
| Consumer issue for the **web interface** (WP-3d) | `snapdog` | Own acceptance, but ships with the server repository |
| **No issue** | WP-0a, WP-0b, WP-0c (inside the M1 issue as tasks), WP-2, WP-3b, WP-3c, WP-5a | Small, a pull request with a link is enough |

Consumer issues link to the epic and to the plan section; they do not copy the
acceptance criteria. Status lives in the issues, requirements in the RFC. Partial
pull requests use non-closing references.

Creating the epic and the issues needs the RFC and this plan to be public first
(WP-0a), because the issues link to them.

## 6. Risks and open questions

- **The deprecation window can be long.** iOS and Android releases depend on store
  review. Until they ship, the guard blocks 1.0, which is the purpose, but nothing
  schedules the consumer work. The epic makes the open consumers visible.
- **`demo-backend` already uses the name `playlist_id` for a number.** Anyone who
  reads that mock to learn the API learns the wrong type. WP-3c fixes it in the same
  release as the server change.
- **`playlist_id` stability on third-party Subsonic servers is unmeasured.** WP-0c
  answers it before M1 is accepted, because the identity rests on it.
- **Auto-merge is off in the smaller repositories** (`pysnapdog`,
  `snapdog-homeassistant`). Their pull requests merge by hand or through the merge
  queue now enabled there.
- **The macOS server app and `snapdog-os` use the word "playlist" only for KNX
  names.** They are unaffected and listed so nobody searches again. The Mac
  *client* is not in that group: it is a target of `snapdog-ios` and changes with WP-3e.
