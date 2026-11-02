---
rfc: MED-0011
title: Stable playlist identity, search and playlist management in the media API
status: draft            # draft | accepted | in-progress | done | superseded
version: 0.2.0           # v0.2: index next to id, by-id paths, WebSocket, release guard
created: 2026-11-02
updated: 2026-11-02
target_repo: snapdog
target_branch: main
related: [HAI-0007]
owners: [metaneutrons]
---

# RFC MED-0011 — Stable playlist identity, search and playlist management in the media API

Epic: not created yet, opened on acceptance
Decision state: draft, open for the owner's review

> This document owns requirements and acceptance criteria (`MED-M*-A*`); the
> epic and milestone issues own execution state and evidence. Do not keep a
> second checklist in issues.

## 1. Outcome and boundaries

**Outcome.** A client can address a playlist in a way that survives changes to
the playlist list, learns about such changes without polling, pages through large
playlists, searches the music library and the radio list, and, where the operator
allows it, creates and edits Subsonic playlists. The first consumers are the
Home Assistant integration and the web interface. Neither can offer a media
browser or a search that holds up without this.

**Baseline (read from the code of `main` on 2026-11-02).**

| Topic | Today |
|---|---|
| Playlist model | One unified list. The radio stations from `[[radio]]` form a single playlist, followed by the Subsonic playlists in the order Subsonic returns them. `GET /media/playlists`, `api/routes/media.rs` |
| Position of the radio list | Index 0 **only if** at least one radio is configured (`has_radio_playlist`). Without radios the Subsonic playlists start at 0 |
| Identity | The `id` of a playlist in every response is its unified **index** (`PlaylistInfo.id: usize`, `PlaylistDetails.id`). The play command takes the same index (`PlaylistPayload { id: usize, track: usize }`). The WebSocket event `ZoneChanged` carries it as `playlist`, the zone state and the MQTT payload as `playlist_index` |
| Track identity | Subsonic tracks carry the Subsonic track id. A radio station gets `radio_{i}`, its position in the configuration |
| Change detection | None. The Subsonic list is cached for one minute (`PLAYLIST_CACHE_TTL`). When Subsonic cannot be reached, `get_playlists().await.unwrap_or_default()` caches an **empty** list, so for up to a minute every Subsonic playlist is gone and the indices behind it are different |
| Events | The WebSocket `Notification` enum has zone, client, presence and error events and nothing about media |
| Paging | `GET /media/playlists/{p}/tracks` returns every track |
| Search | None. The Subsonic client implements `ping`, `getPlaylists`, `getPlaylist`, stream URLs and cover art |
| Writing | Every route under `/media` is `GET`. The Subsonic client cannot write |

Three consequences matter.

1. The index is a position, not an identity. A playlist created or deleted in
   Subsonic, a radio added to or removed from the configuration, and a short
   Subsonic outage each move playlists to other indices. A client that stored an
   index plays something else afterwards. Writing playlists through the API makes
   this routine.
2. A client has no way to notice. It would have to reread the list on a timer.
3. A search and a management API cannot be added on top of positions.

**In scope.** A stable playlist reference (`playlist_id`) in responses, commands and
paths. The field `index` next to `id`, and the deprecation of `id` (and of
`ZoneChanged.playlist`), with a release guard. A `PlaylistsChanged` event. A cache that does not turn an outage into an
empty list. Paging for tracks. `GET /media/search`. A cover proxy by cover id.
Playing an album. Creating, renaming, reordering and deleting Subsonic playlists
behind an operator switch.

**Non-goals.** Writing the radio list through the API (see 2.6). Artist pages and
a general library browser. Subsonic features beyond playlists and search
(starring, ratings, scrobbling). Removing `id` before SnapDog 1.0, and removing
the index paths and the index bodies at all. Changing how the player queues tracks, except where
2.4 needs a queue that is not a playlist. Work in the Home Assistant integration
and in `pysnapdog`; they follow in their own repositories once the server has
shipped (HAI-0010).

## 2. Design and decisions

### 2.1 A stable reference: `playlist_id`, and `index` instead of `id`

Two things change in the playlist identity, and they are one decision.

**`playlist_id` is new.** Every playlist gets a string `playlist_id`. The rule:
`playlist_id` is the prefix of the provider plus the identifier that provider's
system uses for the playlist, unchanged and unique within that system. SnapDog
assigns no identifiers of its own.

| Playlist | `playlist_id` |
|---|---|
| a Subsonic playlist | `subsonic:<subsonic id>` |
| the radio list | `snapdog:radio` |

The radio list lives in `snapdog.toml`, so SnapDog itself is its provider and the
prefix is `snapdog`. There is only one such list, and its identifier is its fixed
name. The prefix keeps a Subsonic id that happens to be numeric from being taken for
an index, and leaves room for other providers. The name follows `track_id` and
`album_id`, which the search results already use.

`playlist_id` is not a UUID. It is the provider's own id with a prefix, so the server
stores nothing and invents nothing. That rests on one assumption: a Subsonic server
keeps a playlist's id for as long as the playlist exists. The Subsonic API
description does not promise it across a server migration or a library re-import,
and the behaviour of Navidrome and Gonic has not been measured here (open question
below). When the Subsonic server is replaced, every `playlist_id` changes with it,
and a client that stored one treats it as an unknown playlist (404), which is the
correct outcome.

**`index` is new, and `id` is deprecated.** In `PlaylistInfo`, `PlaylistDetails` and
the play body, `id` means the unified *index*. The name is misleading next to a
`playlist_id`. From the release that ships this RFC:

- `PlaylistInfo` and `PlaylistDetails` carry `index` with the same value as `id`.
- The play body (`POST /zones/{z}/play/playlist`) accepts `index` as well as `id`.
  A body with both and different values is a 400. A body with neither is rejected as
  today.
- The WebSocket event `ZoneChanged` carries `playlist_index` next to `playlist`. The
  zone state and the MQTT payload already say `playlist_index`.
- `id` (in the three bodies above) and `ZoneChanged.playlist` are marked deprecated
  in `openapi.json` and `docs/api/asyncapi.json` and in the documentation. The text
  at every one of them says: "Deprecated. Use `index`. Removed with SnapDog 1.0,
  which breaks compatibility of `/api/v1` for clients that still read it."
- The field name `id` is never reused with another meaning, not even after the
  removal. A client from the 0.x time would read the new meaning as an index and
  play the wrong playlist without an error.

New clients (the Home Assistant integration first) use `index` and `playlist_id` and
never `id`. Old clients keep working until 1.0. Renaming at once would break the web
interface, the iOS app (a published app cannot be updated together with the server)
and `pysnapdog` 0.1.0.

**Paths and endpoints.** New paths and resources use the segment `by-id` for
`playlist_id`, in every interface. This is deliberate, even though `id` means the
index until 1.0 and the segment can be misread in that time. The documentation says
at each `by-id` path that it takes the `playlist_id`, and that the index paths are
the ones without it. Paths call the index `{playlist_index}`.

Where `playlist_id` and `index` appear:

- `playlist_id` and `index` in `PlaylistInfo` and `PlaylistDetails`. `playlist_id` in
  the zone's playlist fields of the state and of `ZoneChanged`, in
  `GET /zones/{z}/playlist/info` (`ZonePlaylistInfo`, which already has `index`) and
  in the MQTT zone state payload (next to `playlist_index`).
- Commands that carry a playlist. The index and the `playlist_id` get separate
  endpoints and are never accepted side by side in one body, so there is no "both or
  neither" case for the identifier and every body has one type.
  - `PUT /zones/{z}/playlist` keeps its bare number (the index).
    `PUT /zones/{z}/playlist/by-id` is new and takes a bare JSON string
    (`"subsonic:42"`). `GET /zones/{z}/playlist/by-id` returns the active
    `playlist_id` as a string, as `GET /zones/{z}/playlist` returns the index. A
    string in the body needs no percent-encoding, unlike a `playlist_id` in a path.
  - `POST /zones/{z}/play/playlist` keeps `{"id" | "index": n, "track": n}`.
    `POST /zones/{z}/play/playlist/by-id` is new and takes
    `{"playlist_id": "...", "track": n}`.
  - MQTT `snapdog/zones/{z}/playlist/set` keeps the index.
    `snapdog/zones/{z}/playlist/by-id/set` is new and takes the `playlist_id` as
    text payload.
  - An unknown `playlist_id` is a 404 in every case (MQTT: a warning).
- Reading. `GET /media/playlists/by-id/{playlist_id}` and
  `.../by-id/{playlist_id}/tracks` (percent-encoded), with the same bodies as the
  index paths. The existing paths stay and keep resolving by index.

Where `playlist_id` is deliberately not added:

- `POST /zones/{z}/play/playlist/{playlist_index}/track` keeps its index path.
  `POST /zones/{z}/play/playlist/by-id` covers the same use, so a second path with a
  `playlist_id` would only duplicate it.
- `POST /zones/{z}/play/subsonic/{track_id}` already takes the Subsonic id of the
  track.
- `next/playlist` and `previous/playlist` carry no playlist, and the player moves
  through the list it has.
- KNX keeps the 1-byte playlist number (`Playlist`, `Playlist Status`). A group
  address cannot carry a string. The number still shifts when the list changes, and
  the documentation for KNX says so. A client that needs a stable reference uses
  REST or MQTT.
- MPRIS over D-Bus has no playlist index.

A radio station has no stable identity beyond its position in `snapdog.toml`, in
the same way as a zone. This RFC does not change that and the documentation says
so.

### 2.2 Telling clients when the list changes

A new notification `PlaylistsChanged { total: usize }` on the WebSocket
(`"type": "playlists_changed"`). It means that the list, an index or a name may
have changed, and that a client should read `GET /media/playlists` again.

- A background task reads the Subsonic list every `[subsonic].refresh_secs`
  seconds (default 60, the current cache lifetime) and compares `playlist_id`, name and
  song count with the last good list. A difference sends the event.
- A write from 2.5 invalidates the cache and sends the event at once.
- A failed read keeps the last good list and logs a warning. It never replaces the
  list with an empty one. The first failed read after start leaves the Subsonic
  part empty, as today. The request path already behaves this way after
  SnapDogRocks/snapdog#406; the refresh task has to keep it that way.
- Reads from requests use the same list, so the API and the event agree.

Subsonic has no push channel, so a change made in another Subsonic client shows up
within one refresh interval. The documentation states that figure.

### 2.3 Paging tracks

`GET /media/playlists/{p}/tracks` and the `by-id` variant accept `limit` and
`offset`. Without them the response is what it is today. With them the response
is the same array, cut, and the header `X-Total-Count` carries the number of
tracks. The array form is kept so that existing parsers keep working.

### 2.4 Search

`GET /media/search?q=<text>&kind=track,album,playlist,radio&limit=&offset=`

- `q` has at least two characters, otherwise 400. `kind` defaults to all four.
  `limit` defaults to 20 and is capped at 100.
- `track` and `album` are answered by Subsonic's `search3`. `playlist` and `radio`
  are answered by the server with a case-insensitive substring match on the name,
  over the cached playlists and the configured stations.
- The response is `{ "items": [...], "next_offset": n | null }`. `search3` does not
  report a total, so there is none. `next_offset` is set when a page was full.
- An item has `kind`, `title`, `subtitle` (artist, or album for a track),
  `duration` (seconds, 0 if unknown) and `cover_art` (a path to this section's
  cover proxy, or null). The identifier has a field per kind: a track has `track_id`
  (the raw Subsonic id, as in the existing `play/subsonic/{track_id}`), an album
  `album_id`, a playlist or radio `playlist_id` and `index`. There is no
  generic identifier, because Subsonic numbers playlists, tracks and albums
  separately and a plain id could mean any of the three. `playlist_id` carries the
  provider prefix (2.1) because the radio list is not a Subsonic object, the two ids
  of Subsonic objects stay raw because their endpoints already say `subsonic`.
- `GET /media/cover/{cover_id}` proxies a Subsonic cover id with the same caching
  headers as the existing cover routes. Search results need it because they have
  no playlist index to build the existing cover path from.
- A track from the results plays with the existing
  `POST /zones/{z}/play/subsonic/{track_id}`. An album plays with a new
  `POST /zones/{z}/play/subsonic/album/{album_id}`. A playlist or radio plays as
  before.

An album is not part of the unified playlist list, so playing one needs a queue
that is not backed by a playlist. How the player represents that (state fields
`playlist_index` and `playlist_id` null, `playlist_name` the album) is settled in
M2 with the player code in front of us, see section 4.

### 2.5 Managing Subsonic playlists

Subsonic playlists only, passed through to `createPlaylist`, `updatePlaylist` and
`deletePlaylist`.

| Call | Effect |
|---|---|
| `POST /media/playlists` | `{ "name", "track_ids": [] }`, answers 201 with the new `PlaylistInfo` |
| `PATCH /media/playlists/by-id/{playlist_id}` | `{ "name"?, "add_track_ids"?, "remove_positions"? }`, the three operations of `updatePlaylist` |
| `PUT /media/playlists/by-id/{playlist_id}/tracks` | `{ "track_ids": [] }` replaces the content and the order |
| `DELETE /media/playlists/by-id/{playlist_id}` | deletes |

- **Operator switch.** `[subsonic].playlist_write = false` by default. With it off,
  every call above answers 403 `read_only`. Authentication in SnapDog is optional,
  so an installation without API keys must not let anyone on the network delete a
  music library's playlists by default. The Subsonic user's own rights apply on
  top, and a refusal from Subsonic surfaces as 502 with its message.
- Every successful write clears the cache and sends `PlaylistsChanged`. A new or
  deleted playlist moves the indices behind it, so the answer to `POST` carries the
  `playlist_id` and clients address the new playlist by it.
- Writes to `snapdog:radio` answer 403 `read_only` whatever the switch says.
- A write to a playlist that a zone is playing changes what that zone plays next,
  not the current track. The documentation says so.

### 2.6 The radio list stays read-only

The stations are configuration. No code path of `main` writes `snapdog.toml`
today, and operators edit it by hand and keep it in version control. An API that
rewrites it would be the first writer, and it would have to preserve comments,
ordering and the position-based identity of zones and stations that other
interfaces (MQTT topics, KNX addresses) depend on. Stations are edited in
`snapdog.toml`. If a runtime store for stations is wanted later it is a separate
RFC.

### 2.7 Alternatives considered

- **Make the index stable** (sort Subsonic playlists, always reserve slot 0 for
  radio). Sorting by name still moves playlists when one is added, and a fixed
  slot 0 changes the meaning of every index in existing installations.
- **A server-generated UUID per playlist.** Opaque, and independent of the
  provider's id scheme, which would let the server survive a replaced Subsonic
  server by remapping. It needs a persistent table from Subsonic id to UUID (state
  that today lives nowhere), a second identity to keep in step with the first, and a
  rule for what happens to entries whose playlist vanished. With one provider and
  ids that are already unique, it buys nothing now. If a second provider or a
  migration story arrives, the prefix scheme can be replaced without changing the
  field, because clients treat `playlist_id` as an opaque string.
- **Put the id into the existing `id`** as a string. Breaks every client that
  parses it as a number, including the web interface and `pysnapdog` 0.1.0.
- **Rename `id` to `index` at once.** Clearer, and a break for the web interface
  (`api.ts`, `PlaylistBrowser.tsx`), the iOS app (a published app cannot be updated
  together with the server) and `pysnapdog` 0.1.0, which the Home Assistant
  integration depends on. Instead `index` is added and `id` deprecated (2.1), which
  breaks nothing before 1.0.
- **Keep `id` and document that it is the index.** Considered first. It leaves a
  field named `id` next to a `playlist_id` for good and makes `by-id` permanently
  ambiguous. The deprecation costs two fields and one alias.
- **A long segment such as `by-playlist-id`** to avoid the misreading until 1.0.
  Rejected by the owner in favour of one short `by-id` in every interface. The
  misreading is accepted for the time until 1.0 and handled in the documentation.
- **One endpoint that accepts the index or the `playlist_id`** (a number or an
  object, or two keys in one body). Needs a rule for "both" and "neither", an
  untagged type in the schema and a guess for MQTT. Separate `by-id` endpoints cost
  a route each and keep every body a single type.
- **Accept ids and indices in the same path segment.** Subsonic ids can be
  numeric, so `/media/playlists/12` would be ambiguous. A separate `by-id` segment
  has no such case.
- **Let clients poll for changes.** What the integration would do anyway, at a
  cost on both sides and with a delay chosen by the client. The event costs one
  enum variant.
- **Search on the client side** by loading every playlist's tracks. One request
  per playlist and a stale copy of a large library. Only `search3` scales.
- **Leave management to Subsonic clients.** True for everyday use and the reason
  this is the last milestone and off by default. What it adds is scenes and
  automations that build a playlist, and one place for the web interface.

### 2.8 Compatibility and the 1.0 break

Everything before 1.0 is additive inside `/api/v1`. `id` keeps its value and
meaning, the index paths and the existing bodies keep working, and clients that
ignore the new fields and the new event are unaffected. `GET /system/version`
already tells a client the server version. A client that finds no `playlist_id` or
`index` in a playlist talks to an older server and falls back to `id`.
`openapi.json` and `docs/api/asyncapi.json` are regenerated, and the pinned copies in
`pysnapdog` follow after the release.

**The break.** `/api/v1` is the path version of the API and not the product version.
Removing `id` and `ZoneChanged.playlist` with SnapDog 1.0 changes the contract of
`/api/v1` for clients that still use them. That is acceptable before a stability
promise exists, and only if the documentation says so wherever the fields are
described, in the OpenAPI and AsyncAPI descriptions, in the API pages of the
documentation and in the release notes of the release that introduces `index`. It
is not a statement that `/api/v1` is stable. Index paths such as
`/media/playlists/{playlist_index}` and the index bodies stay.

**The guard.** No release whose version is 1.0.0 or above may exist while a
deprecated entry is left in the API description. That includes `1.0.0-rc.N` and
every other pre-release of 1.x, because a pre-release is still a version a client
can install. Two checks enforce it:

- A test in `tests/ci_guard.rs` reads the workspace version from `Cargo.toml` and
  the release-please manifest, takes the larger one, and fails when its major is 1 or
  more and `openapi.json` or `docs/api/asyncapi.json` contains an entry marked
  `deprecated`. It runs in the required `CI Success`, so the release pull request
  that bumps the version cannot be merged.
- The release workflow runs the same check on the tag before it builds anything, so
  a tag pushed by hand is refused as well.

The guard checks the generated description and not a list of field names, so a
future deprecation in `/api/v1` is covered by the same rule.

## 3. Delivery and acceptance

### M1: Identity, change events, paging

Execution: to be linked on acceptance
Dependencies: none

- MED-M1-A1: every playlist in `GET /media/playlists` and `.../{p}` has a
  `playlist_id` (`snapdog:radio`, `subsonic:<id>`) and an `index` equal to `id`. A
  test with and without configured radios shows the indices shifting while the
  `playlist_id` values stay.
- MED-M1-A2: `by-id` reads work and answer 404 for an unknown `playlist_id`.
  `PUT /zones/{z}/playlist/by-id`, `POST /zones/{z}/play/playlist/by-id` and MQTT
  `playlist/by-id/set` start the same playlist as their index counterparts and
  answer 404 (MQTT: a warning) for an unknown `playlist_id`.
  `GET /zones/{z}/playlist/by-id`, `GET /zones/{z}/playlist/info` and the MQTT state
  payload carry the `playlist_id`. The index endpoints behave as before (positive
  and negative probe each).
- MED-M1-A2a: the play body accepts `index` and `id`. Both with the same value work,
  both with different values are a 400, and the old body without `index` still works.
  `ZoneChanged` carries `playlist_index` and the deprecated `playlist` with equal
  values.
- MED-M1-A3: with the Subsonic server stopped, a running server keeps the last good
  list and logs a warning, also across a refresh task run. The request-path part
  ships in SnapDogRocks/snapdog#406, so the counter-probe is that reverting it
  makes the test fail.
- MED-M1-A4: a playlist created or removed in Subsonic produces `playlists_changed`
  on a connected WebSocket within `refresh_secs` plus one second. An unchanged list
  produces none.
- MED-M1-A5: `limit` and `offset` cut a track list and `X-Total-Count` holds the
  full size. Without them the body is byte-identical to the previous release.
- MED-M1-A6: the OpenAPI and AsyncAPI files carry all of it, `ZoneChanged` includes
  `playlist_id` and `playlist_index`, and `id` (three bodies) and `ZoneChanged.playlist`
  are marked `deprecated` with the text of 2.1. The API pages of the documentation
  and the release notes of that release carry the same notice, including the 1.0 break.
- MED-M1-A7: the release guard of 2.8 fails for the version `1.0.0-rc.1` against the
  current description, and passes for `0.30.0` and for `1.0.0-rc.1` against a
  description without deprecated entries (positive probe and counter-probe, run in
  the test with the version injected). The release workflow refuses a tag `v1.0.0`
  in the same situation.

### M2: Search and album playback

Execution: to be linked on acceptance
Dependencies: M1

- MED-M2-A1: `GET /media/search` returns tracks and albums from a real Subsonic
  server, playlists and radios by name, and pages with `limit`, `offset` and
  `next_offset`. `q` of one character is a 400.
- MED-M2-A2: `GET /media/cover/{cover_id}` serves a result's cover with caching
  headers, and an unknown id is a 404.
- MED-M2-A3: an album from a search result plays on a zone, and the zone's state
  names it (`playlist_name`; `playlist_index`, `playlist_id` and the deprecated `playlist` null).
- MED-M2-A4: the web interface and `pysnapdog` are not broken by the new fields
  (their existing test suites pass against a build of this milestone).

### M3: Playlist management

Execution: to be linked on acceptance
Dependencies: M1

- MED-M3-A1: with `playlist_write = false` every write answers 403 `read_only`.
  With it true, create, rename, add, remove, reorder and delete work against a real
  Subsonic server, each followed by `playlists_changed`.
- MED-M3-A2: a write to `snapdog:radio` answers 403 with the switch on.
- MED-M3-A3: a refusal by Subsonic (a user without rights) answers 502 and carries
  its message.
- MED-M3-A4: the configuration reference and the example configuration describe
  `refresh_secs` and `playlist_write`, with the trade-off of 2.5.

### Completion

The initiative is complete when M1 to M3 meet their criteria, or when M3 is
explicitly waived in a reviewed plan change (section 5). Publication is a normal release
of the repository and is not part of this plan.

## 4. Migration, risks and verification cost

- **Rollout.** Defaults keep today's behaviour. `playlist_id`, `index`,
  `playlist_index` in the event and the event itself are additive, and
  `playlist_write` is off.
- **Rollback.** Remove the new keys. Clients that depend on them fall back to `id`
  and `playlist` on a server without them.
- **Risks.** The refresh task adds a Subsonic request per interval. Mitigation: the
  default equals the current cache lifetime and a failed read is logged once per
  outage. A `playlist_id` built from the Subsonic id changes when a playlist is deleted and
  created again with the same name, which is correct. A client that reads `by-id` as
  the index (2.1) and sends a number gets an error (404 for an unknown path value, 422
  for a number in a string body) and never another playlist.
  The removal of `id` with 1.0 breaks clients that never moved to `index`, which is
  why the guard exists. Album playback needs a queue
  that the player does not have today, and M2 may show that it is larger than
  assumed.
- **Verification cost.** Unit tests run on Linux x86-64. M1 to M3 need a Subsonic
  server, which `tests/integration.rs` already uses for the playlist test. No
  estimate of runner time is given because none has been measured.
- **Open questions.**
  - Whether `search3` behaves the same on Navidrome, Gonic and the reference
    Subsonic server for `songCount`, `albumCount` and paging. It has been read in
    the API description, not measured.
  - Whether the player can queue an album without a unified playlist index, or
    needs a temporary playlist entry (M2).
  - Whether the default of 60 seconds for `refresh_secs` is right for large
    libraries.
  - Whether `playlist_write` should default to true when an API key is
    configured.
  - Whether Subsonic playlist ids stay the same across restarts and library rescans
    on Navidrome, Gonic and the reference server. `playlist_id` depends on it.

## 5. Decision changes

None yet. Changes to a decision after acceptance are listed here with the date and
whether the owner can veto them.
