---
rfc: KZO-0010
title: KNX zone objects for wall switches, single-address navigation and a play/pause switch
status: in-progress      # draft | accepted | in-progress | done | superseded
version: 0.2.0           # v0.2: implemented on the branch feat/knx-object-groups, not merged
created: 2026-11-01
updated: 2026-11-01
target_repo: snapdog
target_branch: main
related: [KEA-0004, HAI-0007]
owners: [metaneutrons]
---

# RFC KZO-0010 — KNX zone objects for wall switches

> **For AI agents:** this RFC collects what the product database gains in one version bump. The
> grouped layout, the four objects and the translations are implemented (section 9). Requirements are `KZO-REQ-*`, decisions `KZO-DEC-*`, tasks `KZO-T*`. Symbol names
> are the anchor, line numbers are approximate. Nothing here is implemented.

## 1. Outcome and boundaries

**Outcome.** A wall switch or a KNX gateway with ordinary parameters can drive a zone without a
translation layer: a play/pause switch, track and playlist navigation either as one address per
direction or as one address for both directions (Step). The product database changes once, while
SnapDog is in beta and few ETS projects depend on the object numbers.

**Why now.** The installation that prompted this replaces a Gira X1 and a Node-RED bridge. Its
group addresses were designed for that bridge: one on/off address for the player, one 1.007 Step
address each for "station next/previous" and "track next/previous". SnapDog's zone objects cannot
take them as they are (section 2).

**Non-goals.** Changing the semantics of existing objects, a second KNX device in one process,
KNX IP Secure, the text objects beyond a playlist name (section 4, candidates).

## 2. Baseline (read from `main` working tree, 2026-11-01)

Anchors: `snapdog/src/knx/group_objects.rs`, `knx/mod.rs`, `player/runner.rs`,
`player/commands.rs`, `config/types.rs`, `xtask/src/main.rs`.

- Zone block: `ZONE_GOS` has 33 objects (`ZGO_PLAY` 0 to `ZGO_PRESENCE_TIMER_ACTIVE` 32), 10 zones,
  30 clients of 11, 5 global. `TOTAL_GO_COUNT` is 665. The ETS object number is the ASAP:
  `zone_asap(z, i) = (z-1) * ZONE_GO_COUNT + i + 1`, clients and globals follow
  (`go_com_object` passes `number` to the authoring `ComObject`). One more object per zone
  therefore moves every zone after the first, all clients and all globals.
- Commands that ignore the telegram value: `play`, `pause`, `stop`, `next`, `previous`,
  `playlist_next`, `playlist_previous` and the `*_toggle` objects (`ZoneCommand::Play` and so on in
  `knx/mod.rs`, around the `match action` in the zone branch). Any write triggers.
- `Next` and `Previous` act on the active source: next or previous station in the radio playlist,
  next or previous track in a Subsonic playlist, and the remote-control command for AirPlay and
  Spotify (`player/runner.rs`, `ZoneCommand::Next`, `ZoneCommand::Previous`).
- `NextPlaylist`, `PreviousPlaylist` and `SetPlaylist(i, track)` move the unified playlist index
  (0 is the radio playlist when at least one `[[radio]]` exists, 1 and up are Subsonic playlists),
  wrap around at both ends, start at track 0 and are skipped while a receiver has priority
  (`may_start_local_playback`).
- `Control Status` (`ZGO_CONTROL_STATUS`) and `Track Playing` carry the playback state as 1 bit. The
  polarity of `Control Status` was not read for this RFC (KZO-T1).
- Client mode reads group addresses from `[zone.knx]` keys (`zone_ga_map` in `knx/mod.rs`); device
  mode reads them from the ETS association table. A new object needs both: an object definition
  and a TOML key.
- The knxprod is locked by a fingerprint of the parameter layout and of each object's DPT, size and
  flags (`EXPECTED_LAYOUT_HASH`, `assert_layout_locked` in `xtask`). A change needs
  `KNXPROD_APP_VERSION` and the hash updated together; `cargo xtask knxprod` and the test
  `ets_memory_layout_is_locked` enforce it.

## 3. Requirements

- **KZO-REQ-1.** A zone can be started and paused from one 1-bit address (1 plays, 0 pauses).
- **KZO-REQ-2.** Track navigation and playlist navigation are each available as two single-trigger
  objects (existing) and as one Step object (new).
- **KZO-REQ-3.** Existing objects keep their meaning, DPT, flags and size. Their object numbers may
  move once (KZO-DEC-1).
- **KZO-REQ-4.** The product database is bumped once for everything in this RFC.
- **KZO-REQ-5.** Client mode and device mode expose the same objects (TOML keys for each).

## 4. Design

### 4.1 A zone block in six groups

The zone block is rebuilt as six groups of ten object numbers. Related objects sit together in
ETS, a status object follows its command, and every group has free numbers at its end. The number
of zone object `i` in group `g` (0 to 5), position `k` (0 to 9) of zone `z` is

```text
asap = (z - 1) * 60 + g * 10 + k + 1        # ZONE_GO_STRIDE = 60
```

so zone 2, volume (group 1, position 0) is 71 and the position inside a zone reads off the number.
Undefined numbers are reserved: they get no object in the knxprod and no descriptor bits.

| Group | k | Name | Dir | DPT | Note |
|---|---|---|---|---|---|
| **0 Transport** | 0 | Play | recv | 1.001 | any value |
| | 1 | Pause | recv | 1.001 | any value |
| | 2 | Stop | recv | 1.001 | any value |
| | 3 | **Play/Pause** | recv | 1.001 | new, 1 plays, 0 pauses |
| | 4 | Control Status | send | 1.001 | polarity: KZO-T1 |
| | 5 | Track Playing | send | 1.001 | |
| | 6 | Track Next | recv | 1.001 | any value |
| | 7 | Track Previous | recv | 1.001 | any value |
| | 8 | **Track Step** | recv | 1.007 | new, 1 next, 0 previous |
| | 9 | reserved | | | |
| **1 Volume and mute** | 0 | Volume | recv | 5.001 | |
| | 1 | Volume Status | send | 5.001 | |
| | 2 | Volume Dim | recv | 3.007 | |
| | 3 | Mute | recv | 1.001 | |
| | 4 | Mute Status | send | 1.001 | |
| | 5 | Mute Toggle | recv | 1.001 | |
| | 6 to 9 | reserved | | | |
| **2 Play modes** | 0 | Shuffle | recv | 1.001 | |
| | 1 | Shuffle Status | send | 1.001 | |
| | 2 | Shuffle Toggle | recv | 1.001 | |
| | 3 | Repeat | recv | 1.001 | playlist repeat |
| | 4 | Repeat Status | send | 1.001 | |
| | 5 | Repeat Toggle | recv | 1.001 | |
| | 6 | Track Repeat | recv | 1.001 | |
| | 7 | Track Repeat Status | send | 1.001 | |
| | 8 | Track Repeat Toggle | recv | 1.001 | |
| | 9 | reserved | | | |
| **3 Playlist** | 0 | Playlist | recv | 5.010 | unified index, 0 is radio |
| | 1 | Playlist Status | send | 5.010 | |
| | 2 | Playlist Next | recv | 1.001 | any value |
| | 3 | Playlist Previous | recv | 1.001 | any value |
| | 4 | **Playlist Step** | recv | 1.007 | new, 1 next, 0 previous |
| | 5 | **Playlist Name** | send | 16.001 | new, 14 bytes |
| | 6 to 9 | reserved | | | candidates: track select and its status (4.2) |
| **4 Track info** | 0 | Track Title | send | 16.001 | |
| | 1 | Track Artist | send | 16.001 | |
| | 2 | Track Album | send | 16.001 | |
| | 3 | Track Progress | send | 5.001 | |
| | 4 to 9 | reserved | | | candidates: longer text objects (4.2) |
| **5 Presence** | 0 | Presence | recv | 1.018 | |
| | 1 | Presence Enable | recv | 1.001 | |
| | 2 | Presence Timer Active | send | 1.001 | |
| | 3 to 9 | reserved | | | |

Counts: 33 objects today, 37 defined after this RFC (four new), 23 reserved numbers per zone.
`ZONE_GO_COUNT` stays the number of defined objects, `ZONE_GO_STRIDE` (60) is new and used for the
numbering.

### 4.1a The client block, grouped the same way

Three groups of ten, stride 30. The number of client object `k` in group `g` of client `c` (1 to 30)
is

```text
asap = MAX_ZONES * ZONE_GO_STRIDE + (c - 1) * CLIENT_GO_STRIDE + g * 10 + k + 1     # CLIENT_GO_STRIDE = 30
```

so client 1 starts at 601 and client 2 at 631.

| Group | k | Name | Dir | DPT | Note |
|---|---|---|---|---|---|
| **0 Volume and mute** | 0 | Volume | recv | 5.001 | |
| | 1 | Volume Status | send | 5.001 | |
| | 2 | Volume Dim | recv | 3.007 | |
| | 3 | Mute | recv | 1.001 | |
| | 4 | Mute Status | send | 1.001 | |
| | 5 | Mute Toggle | recv | 1.001 | |
| | 6 to 9 | reserved | | | |
| **1 Latency and zone** | 0 | Latency | recv | 5.010 | |
| | 1 | Latency Status | send | 5.010 | |
| | 2 | Zone | bidir | 5.010 | zone assignment |
| | 3 | Zone Status | send | 5.010 | |
| | 4 to 9 | reserved | | | |
| **2 Connection** | 0 | Connected | send | 1.001 | |
| | 1 to 9 | reserved | | | |

The 11 objects keep their meaning, DPT, flags and size. `CLIENT_GO_COUNT` stays 11,
`CLIENT_GO_STRIDE` (30) is new. The global block follows the clients: `global_asap` starts after
`MAX_ZONES * ZONE_GO_STRIDE + MAX_CLIENTS * CLIENT_GO_STRIDE`, that is at 1501.

The existing `Play`, `Pause`, `Stop`, `Track Next`, `Track Previous`, `Playlist Next` and
`Playlist Previous` keep reacting to any value. DPT 1.007 (`DPT_Step`) is 0 = decrease,
1 = increase; the Step objects map increase to next and evaluate the value. A malformed or empty
payload is ignored (`decode_bool` returns false for an empty payload, so the Step decoder must not
reuse it unchanged, KZO-T3).

TOML keys under `[zone.knx]`: `play_pause`, `track_step`, `playlist_step`, `playlist_name_status`.

### 4.2 Candidates, to decide together before the bump

- **Track select** (recv 5.010 with a status object, group 3, k 6 and 7): the index of the track,
  or station, inside the active playlist. `SetPlaylist(index, track)` already carries a track
  argument; the KNX `Playlist` object always sends 0. This is the only way for a switch to pick a
  fixed radio station.
- **Text objects** (group 4). Track title, artist and album are 14 bytes (DPT 16.001). Open:
  truncate at a word end, ISO-8859-1 for umlauts (the DPT allows them), or split a long value over
  several objects in the reserved numbers of group 4.

### 4.3 What the restructuring costs

- Every object number in the product changes, zone 1 included. An ETS project that uses the device
  has to be checked after the product update (KZO-T7).
- `ZONE_GOS` is a plain slice that is indexed by position. With gaps it becomes a list of
  `(group, position, definition)` entries, and `zone_asap`, `definition_of`, the `ZGO_*` constants
  and `descriptor_table` derive the number from group and position. The constants keep their names,
  their values become the new numbers inside a zone.
- `descriptor_table` writes a descriptor for every ASAP. Reserved numbers get zero bits, and the test
  `descriptor_table_declares_flags_and_sizes_of_every_object` asserts communication enabled on all
  numbers, so it has to skip the reserved ones.
- The group object table grows: 10 zones x 60 + 30 clients x 30 + 5 = 1505 descriptors instead of
  665 (2 bytes each, about 3 KB). Most of the growth is reserved numbers in the client block, which
  has 19 spare numbers per client and no candidate objects yet. Whether the device stack and the
  memory reserved for it take that is not verified (KZO-T4).
- The client block with groups of ten is a consequence of "the same grouping". A narrower client
  stride (groups of 8, stride 24, 720 numbers) would fit the 11 objects as well and save 180
  descriptors, at the price of a scheme that differs between zones and clients.
- The stride is a product decision: 60 leaves 23 numbers per zone and keeps the numbers readable
  (inside a zone each group starts at a multiple of ten plus one). A stride of 40 would save 200
  descriptors but needs groups of different lengths and gives up that regularity.

## 5. Decisions

- **KZO-DEC-1 (proposed).** Zones: six groups of ten, stride 60. Clients: three groups of ten,
  stride 30. Reserved numbers undefined (4.1, 4.1a).
  Alternative: no groups and no stride, append and let the numbers move at every addition. That is
  only acceptable while the product is in beta.
- **KZO-DEC-2.** DPT of Play/Pause: 1.001 (proposal, same as `Control Status` and `Mute`) or 1.010
  (`DPT_Start`: 1 start, 0 stop). 1.010 would suggest Stop, which this object does not do.
- **KZO-DEC-3.** Step objects evaluate the value, single-trigger objects keep ignoring it.

## 6. Tasks

- **KZO-T1.** Read the polarity of `Control Status` and `Track Playing` and write it down.
- **KZO-T2.** Restructure `ZONE_GOS` and `CLIENT_GOS` into groups with gaps, add the four objects, constants
  (`ZGO_PLAY_PAUSE`, `ZGO_TRACK_STEP`, `ZGO_PLAYLIST_STEP`, `ZGO_PLAYLIST_NAME`), `ZONE_ACTIONS`
  entries and the publisher for the name.
- **KZO-T3.** Step decoder that rejects empty and malformed payloads. Tests for 0, 1, empty.
- **KZO-T4.** `ZONE_GO_STRIDE` and `CLIENT_GO_STRIDE`: `zone_asap`, `client_asap`, `global_asap`,
  `TOTAL_GO_COUNT`,
  `definition_of`, the descriptor table and its test. Check the size of the group object table in the
  device stack.
- **KZO-T5.** `xtask`: the objects with their new numbers, `EXPECTED_LAYOUT_HASH` and
  `KNXPROD_APP_VERSION` together, regenerate `knx/snapdog.knxprod` and `knx/snapdog.xml`.
- **KZO-T6.** TOML keys, convention defaults, `snapdog.example.toml`, `docs/knx-bridge.md` (it
  quotes the old formula).
- **KZO-T7.** ETS test with a project copy that already uses the device: update the product and
  check whether the associations survive the new numbers. Document the result in
  `docs/upgrading.md`.

## 7. Risks

- ETS projects of beta users lose associations if the update does not map by object name or
  reference. Unverified; KZO-T7 decides how strong the upgrade note has to be.
- A Step write that arrives as 0 by mistake goes backwards. The single-trigger objects do not
  have this property, which is why both are offered.

## 8. Open points

- KZO-DEC-1 and KZO-DEC-2.
- Candidates in 4.2.
- Whether ETS keeps associations when object numbers move (KZO-T7).
- Polarity of `Control Status` (KZO-T1).
- Which zone a wall switch should control when its ETS location and its function differ (a bathroom
  switch that controls the kitchen zone): not a product question, noted here because it shaped the
  question.

## 9. Implementation (2026-11-01, branch `feat/knx-object-groups`)

Decided with the maintainer: the knxprod is not in production use, so the numbers move once.

- **KZO-DEC-1:** six zone groups and three client groups, strides 60 and 30, reserved numbers
  undefined. The global block follows at 1501 to 1505 and keeps its single run of five numbers.
- **KZO-DEC-2:** Play/Pause is 1.001. **KZO-DEC-3:** as proposed.
- Done: T2 (objects, constants, `ZONE_GO_SLOTS`, `CLIENT_GO_SLOTS`, `locate`), T3 (`decode_switch`
  rejects an empty payload), T4 (strides, descriptor table with empty descriptors for reserved
  numbers; the device-mode integration tests run with 1505 table entries), T5 (`KNXPROD_APP_VERSION`
  12, layout hash now also covers the slots and strides, knxprod and XML regenerated, unsigned like
  the committed one), T6 (TOML keys, example, macOS settings field list, `docs/knx-bridge.md`,
  `docs/upgrading.md`).
- Added beyond the RFC: every text of the product database is translated into English, Dutch,
  Spanish and French (`xtask/src/i18n.rs`, injected into the generated XML, the golden file covers
  it). A new text without a translation stops the generation. The version-mismatch message also has
  a German text, its base text is English.
- Not done: T7 (ETS test with a project that uses the device), the candidates of 4.2 (track select,
  longer text objects), grouping the five global objects.
- Open: a device that holds the tables of an older product version keeps them (SnapDog does not
  compare the stored program version with `KNXPROD_APP_VERSION`), see `docs/upgrading.md`.
