---
rfc: MED-0011
title: Stable playlist identity, search and playlist management in the media API
status: draft            # draft | accepted | in-progress | done | superseded
version: 0.1.0           # v0.1: initial proposal
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
| Identity | The `id` of a playlist in every response is its unified **index** (`PlaylistInfo.id: usize`). The play command takes the same index (`PlaylistPayload { id: usize, track: usize }`) |
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
paths. A `PlaylistsChanged` event. A cache that does not turn an outage into an
empty list. Paging for tracks. `GET /media/search`. A cover proxy by cover id.
Playing an album. Creating, renaming, reordering and deleting Subsonic playlists
behind an operator switch.

**Non-goals.** Writing the radio list through the API (see 2.6). Artist pages and
a general library browser. Subsonic features beyond playlists and search
(starring, ratings, scrobbling). Renaming or removing `id`, the index paths, or
the existing request bodies. Changing how the player queues tracks, except where
2.4 needs a queue that is not a playlist. Work in the Home Assistant integration
and in `pysnapdog`; they follow in their own repositories once the server has
shipped (HAI-0010).

## 2. Design and decisions

### 2.1 A stable reference: `playlist_id`

Every playlist gets a string `playlist_id`, next to the unchanged index `id`. The name
follows `track_id` and `album_id`, which the search results already use. Note that in
`PlaylistInfo`, `PlaylistDetails` and the play body `id` means the *index*, so the
documentation says so beside both fields. `id` stays, because renaming it would break the
web interface, the iOS app and `pysnapdog` 0.1.0 inside `/api/v1`. Since `id` means
the index, no new path says `by-id`: the media paths use `by-playlist-id/{playlist_id}`,
the zone resources are the siblings `playlist-id` of `playlist`, and MQTT uses
`playlist-id/set` next to `playlist/set`. Paths call the index `{playlist_index}`.

The rule: `playlist_id` is the prefix of the provider plus the identifier that provider's
system uses for the playlist, unchanged and unique within that system. SnapDog
assigns no identifiers of its own.

| Playlist | `playlist_id` |
|---|---|
| a Subsonic playlist | `subsonic:<subsonic id>` |
| the radio list | `snapdog:radio` |

The radio list lives in `snapdog.toml`, so SnapDog itself is its provider and the
prefix is `snapdog`. There is only one such list, and its identifier is its fixed
name.

The prefix keeps a Subsonic id that happens to be numeric from being taken for an
index, and leaves room for other providers. The field is new because `id` already
means the index in responses and in the play body, and changing its meaning would
break every client.

`playlist_id` is not a UUID. It is the provider's own id with a prefix, so the server stores
nothing and invents nothing. That rests on one assumption: a Subsonic server keeps a
playlist's id for as long as the playlist exists. The Subsonic API description does
not promise it across a server migration or a library re-import, and the behaviour of
Navidrome and Gonic has not been measured here (open question below). When the
Subsonic server is replaced, every `playlist_id` changes with it, and a client that stored one
treats it as an unknown playlist (404), which is the correct outcome.

Where `playlist_id` appears:

- `playlist_id` in `PlaylistInfo` and `PlaylistDetails`, in the zone's playlist fields
  of the state and of `ZoneChanged` (`playlist_id`, next to `playlist_index`), in
  `GET /zones/{z}/playlist/info` (`ZonePlaylistInfo`) and in the MQTT zone state
  payload (`playlist_id`, next to `playlist_index`).
- Commands that carry a playlist. The index and the `playlist_id` get separate endpoints
  and are never accepted side by side in one body, so there is no "both or neither"
  case and every body has one type.
  - `PUT /zones/{z}/playlist` keeps its bare number (the index). New:
    `PUT /zones/{z}/playlist-id` with a bare JSON string as body
    (`"subsonic:42"`), and `GET /zones/{z}/playlist-id` returning the active
    `playlist_id` as a string, as `GET /zones/{z}/playlist` returns the index. A string in
    the body needs no percent-encoding, unlike a `playlist_id` in a path.
  - `POST /zones/{z}/play/playlist` keeps `{"id": n, "track": n}`. New:
    `POST /zones/{z}/play/playlist-id` with `{"playlist_id": "...", "track": n}`.
  - MQTT `snapdog/zones/{z}/playlist/set` keeps the index. New:
    `snapdog/zones/{z}/playlist-id/set` with the `playlist_id` as text payload.
  - An unknown `playlist_id` is a 404 in every case.
- Reading. `GET /media/playlists/by-playlist-id/{playlist_id}` and `.../by-playlist-id/{playlist_id}/tracks`
  (percent-encoded), with the same bodies as the index paths. The existing paths
  stay and keep resolving by index.

Where `playlist_id` is deliberately not added:

- `POST /zones/{z}/play/playlist/{playlist_index}/track` keeps its index path. The
  `POST /zones/{z}/play/playlist-id` covers the same use, so a
  second path with a `playlist_id` would only duplicate it.
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

`GET /media/playlists/{p}/tracks` and the `by-playlist-id` variant accept `limit` and
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
  `duration` (seconds, 0 if unknown) and `cover_art` (a path to 2.4's cover
  proxy, or null). The identifier has a field per kind: a track has `track_id`
  (the raw Subsonic id, as in the existing `play/subsonic/{track_id}`), an album
  `album_id`, a playlist or radio `playlist_id` and `playlist_index`. There is no
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
| `PATCH /media/playlists/by-playlist-id/{playlist_id}` | `{ "name"?, "add_track_ids"?, "remove_positions"? }`, the three operations of `updatePlaylist` |
| `PUT /media/playlists/by-playlist-id/{playlist_id}/tracks` | `{ "track_ids": [] }` replaces the content and the order |
| `DELETE /media/playlists/by-playlist-id/{playlist_id}` | deletes |

- **Operator switch.** `[subsonic].playlist_write = false` by default. With it off,
  every call above answers 403 `read_only`. Authentication in SnapDog is optional,
  so an installation without API keys must not let anyone on the network delete a
  music library's playlists by default. The Subsonic user's own rights apply on
  top, and a refusal from Subsonic surfaces as 502 with its message.
- Every successful write clears the cache and sends `PlaylistsChanged`.
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
- **Rename `id` to `index`.** Clearer, and a break for the web interface
  (`api.ts`, `PlaylistBrowser.tsx`), the iOS app (a published app cannot be updated
  together with the server) and `pysnapdog` 0.1.0, which the Home Assistant
  integration depends on. An additive `index` next to `id` would avoid the break and
  was left out on purpose: the documentation states that `id` is the index.
- **One endpoint that accepts the index or the `playlist_id`** (a number or an object, or two
  keys in one body). Needs a rule for "both" and "neither", an untagged type in the
  schema and a guess for MQTT. Separate `by-playlist-id` endpoints cost a route each and
  keep every body a single type.
- **Accept ids and indices in the same path segment.** Subsonic ids can be
  numeric, so `/media/playlists/12` would be ambiguous. A separate `by-playlist-id` prefix
  has no such case.
- **Let clients poll for changes.** What the integration would do anyway, at a
  cost on both sides and with a delay chosen by the client. The event costs one
  enum variant.
- **Search on the client side** by loading every playlist's tracks. One request
  per playlist and a stale copy of a large library. Only `search3` scales.
- **Leave management to Subsonic clients.** True for everyday use and the reason
  this is the last milestone and off by default. What it adds is scenes and
  automations that build a playlist, and one place for the web interface.

### 2.8 Compatibility

Everything is additive inside `/api/v1`. `id` keeps meaning the index, the index
paths and the existing bodies keep working, and clients that ignore the new
fields and the new event are unaffected. `GET /system/version` already tells a
client the server version. A client that finds no `playlist_id` in a playlist talks to an
older server and falls back to the index. `openapi.json` and `docs/api/asyncapi.json`
are regenerated, and the pinned copies in `pysnapdog` follow after the release.

## 3. Delivery and acceptance

### M1: Identity, change events, paging

Execution: to be linked on acceptance
Dependencies: none

- MED-M1-A1: every playlist in `GET /media/playlists` and `.../{p}` has a `playlist_id`
  (`snapdog:radio`, `subsonic:<id>`), and `id` is unchanged. A test with and without
  configured radios shows the indices shifting while the `playlist_id` values stay.
- MED-M1-A2: `by-playlist-id` reads work and answer 404 for an unknown `playlist_id`.
  `PUT /zones/{z}/playlist-id`, `POST /zones/{z}/play/playlist-id` and MQTT
  `playlist-id/set` start the same playlist as their index counterparts and answer
  404 (MQTT: a warning) for an unknown `playlist_id`. `GET /zones/{z}/playlist-id`,
  `GET /zones/{z}/playlist/info` and the MQTT state payload carry the `playlist_id`. The
  index endpoints behave as before (positive and negative probe each).
- MED-M1-A3: with the Subsonic server stopped, a running server keeps the last good
  list and logs a warning, also across a refresh task run. The request-path part
  ships in SnapDogRocks/snapdog#406, so the counter-probe is that reverting it
  makes the test fail.
- MED-M1-A4: a playlist created or removed in Subsonic produces `playlists_changed`
  on a connected WebSocket within `refresh_secs` plus one second. An unchanged list
  produces none.
- MED-M1-A5: `limit` and `offset` cut a track list and `X-Total-Count` holds the
  full size. Without them the body is byte-identical to the previous release.
- MED-M1-A6: the OpenAPI and AsyncAPI files carry all of it, and `ZoneChanged`
  includes `playlist_id`.

### M2: Search and album playback

Execution: to be linked on acceptance
Dependencies: M1

- MED-M2-A1: `GET /media/search` returns tracks and albums from a real Subsonic
  server, playlists and radios by name, and pages with `limit`, `offset` and
  `next_offset`. `q` of one character is a 400.
- MED-M2-A2: `GET /media/cover/{cover_id}` serves a result's cover with caching
  headers, and an unknown id is a 404.
- MED-M2-A3: an album from a search result plays on a zone, and the zone's state
  names it (`playlist_name`, `playlist_index` and `playlist_id` null).
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
explicitly waived in a reviewed plan change (2.7). Publication is a normal release
of the repository and is not part of this plan.

## 4. Migration, risks and verification cost

- **Rollout.** Defaults keep today's behaviour. `playlist_id` and the event are additive and
  `playlist_write` is off.
- **Rollback.** Remove the new keys. Clients that depend on `playlist_id` fall back to the
  index on a server without it.
- **Risks.** The refresh task adds a Subsonic request per interval. Mitigation: the
  default equals the current cache lifetime and a failed read is logged once per
  outage. A `playlist_id` built from the Subsonic id changes when a playlist is deleted and
  created again with the same name, which is correct. Album playback needs a queue
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
