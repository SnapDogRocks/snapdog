---
rfc: KZO-0010
title: KNX zone objects for wall switches, single-address navigation and a play/pause switch
status: in-progress      # draft | accepted | in-progress | done | superseded
version: 0.4.0           # v0.4: rocker objects (1.008) and 1.017 for next and previous
created: 2026-11-01
updated: 2026-11-01
target_repo: snapdog
target_branch: main
related: [KEA-0004, HAI-0007]
owners: [metaneutrons]
---

# RFC KZO-0010 — KNX zone objects for wall switches

> **For AI agents:** this RFC collects what the product database gains in one version bump. The
> grouped layout, the six new zone objects, the grouped global objects, the stale-memory guard and
> the translations are implemented (section 9). Requirements are `KZO-REQ-*`, decisions `KZO-DEC-*`,
> tasks `KZO-T*`. Symbol names are the anchor, line numbers are approximate.

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
| | 6 | Track Next | recv | 1.017 | any value (was 1.001) |
| | 7 | Track Previous | recv | 1.017 | any value (was 1.001) |
| | 8 | **Track Step** | recv | 1.007 | new, 1 next, 0 previous |
| | 9 | **Track Step Up/Down** | recv | 1.008 | new, 1 down is next, 0 up is previous (rocker) |
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
| | 2 | Playlist Next | recv | 1.017 | any value (was 1.001) |
| | 3 | Playlist Previous | recv | 1.017 | any value (was 1.001) |
| | 4 | **Playlist Step** | recv | 1.007 | new, 1 next, 0 previous |
| | 5 | **Playlist Name** | send | 16.001 | new, 14 bytes |
| | 6 | **Playlist Step Up/Down** | recv | 1.008 | new, 1 down is next, 0 up is previous (rocker) |
| | 7 to 9 | reserved | | | |
| **4 Track info** | 0 | Track Title | send | 16.001 | |
| | 1 | Track Artist | send | 16.001 | |
| | 2 | Track Album | send | 16.001 | |
| | 3 | Track Progress | send | 5.001 | |
| | 4 | **Track Number** | recv | 5.010 | new, counted from 1, 0 is ignored |
| | 5 | **Track Number Status** | send | 5.010 | new, 0 without a track |
| | 6 to 9 | reserved | | | candidate: longer text objects (4.2) |
| **5 Presence** | 0 | Presence | recv | 1.018 | |
| | 1 | Presence Enable | recv | 1.001 | |
| | 2 | Presence Timer Active | send | 1.001 | |
| | 3 to 9 | reserved | | | |

Counts: 33 objects before this RFC, 39 defined after it (six new), 21 reserved numbers per zone.
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

### 4.1b The device-level objects, grouped as well

Two groups of ten, `GLOBAL_GO_STRIDE` 20, slots in `GLOBAL_GO_SLOTS`:

| Group | k | Name | Dir | DPT |
|---|---|---|---|---|
| **0 System status** (1501 to 1510) | 0 | Server Online | send | 1.011 |
| | 1 | System Fault | send | 1.005 |
| | 2 | KNX Time | recv | 10.001 |
| | 3 to 9 | reserved | | |
| **1 All zones** (1511 to 1520) | 0 | All Stop | recv | 1.017 |
| | 1 | All Mute | bidir | 1.001 |
| | 2 to 9 | reserved | | |

The global objects were one run of five (1501 to 1505) in the order server online, all stop, all
mute, system fault, KNX time. `GGO_*` follow the new order of `GLOBAL_GOS`.

The existing `Play`, `Pause`, `Stop`, `Track Next`, `Track Previous`, `Playlist Next` and
`Playlist Previous` keep reacting to any value. `Track Next`, `Track Previous`, `Playlist Next` and
`Playlist Previous` are declared as DPT 1.017 (`DPT_Trigger`) since version 14, which says what they
do; the behaviour is unchanged. DPT 1.007 (`DPT_Step`) is 0 = decrease,
1 = increase; the Step objects map increase to next and evaluate the value. DPT 1.008 (`DPT_UpDown`)
is 0 = up, 1 = down. `Track Step Up/Down` and `Playlist Step Up/Down` are the same two objects for a
rocker that is parametrised as up/down (a blind-style rocker on a wall switch, for example): down
(1) is next, up (0) is previous, like scrolling a list. They evaluate the value exactly as the Step
objects do. The DPT of an object is fixed in the product database, so the rocker variants are
objects of their own and not a parameter of the 1.007 ones. A malformed or empty
payload is ignored (`decode_bool` returns false for an empty payload, so the Step decoder must not
reuse it unchanged, KZO-T3).

TOML keys under `[zone.knx]`: `play_pause`, `track_step`, `track_step_updown`, `playlist_step`,
`playlist_step_updown`, `playlist_name_status`, `track_number`, `track_number_status`.

### 4.2 Candidates, to decide together before the bump

- **Track select**, decided and implemented as Track Number and Track Number Status (group 4, k 4
  and 5, 5.010): the number of the track, or station, inside the active playlist, counted from 1
  because 0 is the value of an unset object. It maps to `SetTrack(n - 1)`. This is the way for a
  switch to pick a fixed radio station; the status is `playlist_track_index + 1`, 0 without one.
  It sits with the track information because it describes the playing track.
- **Longer text objects** (group 4), open. Title, artist, album and playlist name are 14 bytes
  (DPT 16.001, ISO-8859-1). A long variant is possible with DPT 28.001 (`DPT_UTF-8`, variable
  length, UTF-8, terminated by a zero octet; KNX Standard 03_07_02 v02.02.01, section 3.27.1).
  The device stack accepts object sizes up to 252 bytes (descriptor codes 21 to 255).
  `knx-rs-core` 0.13 already has a DPT 28 codec (`dpt::encode`/`dpt::decode` with `Dpt::new(28, 1)`,
  `DptValue::Text`), so the status sink needs no raw-bytes path. What is left:
  the length of the object (the encoded value, including the terminating zero, has to fit it, so a
  longer text is cut at a UTF-8 character boundary before encoding; `encode_dpt28` does not cut);
  the `size_str` and DPT string of the knxprod entry; and whether common displays and visualisations
  (Gira, xknx and others) read DPT 28.001 or objects longer than 14 bytes, which is not verified.
  Decide the length first (KZO-DEC-4), then it is three objects with the pattern of Track Number.
  An earlier version of this RFC said that `knx-rs-core` has no DPT 28 codec. That was wrong; the
  search for it had missed `dpt/convert/string.rs`.

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
- The group object table grows: 10 zones x 60 + 30 clients x 30 + 20 = 1520 descriptors instead of
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
- **KZO-DEC-5.** Rocker objects: DPT 1.008 is added as separate objects (`Track Step Up/Down`,
  `Playlist Step Up/Down`) next to the 1.007 ones, because the DPT of an object is fixed in the
  product database; a parameter cannot switch it. Down (1) is next, up (0) is previous. The numbers of
  the existing objects stay: `Track Step Up/Down` takes the free tenth number of group 0,
  `Playlist Step Up/Down` the first free number of group 3. `Track Next`, `Track Previous`,
  `Playlist Next` and `Playlist Previous` are declared 1.017 (trigger).
- **KZO-DEC-4.** Longer text objects: DPT (28.001 UTF-8 or a longer 16.xxx variant), length, and who
  reads them. Open (4.2).

## 6. Tasks

- **KZO-T1.** Read the polarity of `Control Status` and `Track Playing` and write it down.
- **KZO-T2.** Restructure `ZONE_GOS`, `CLIENT_GOS` and `GLOBAL_GOS` into groups with gaps, add the six
  objects, constants (`ZGO_PLAY_PAUSE`, `ZGO_TRACK_STEP`, `ZGO_PLAYLIST_STEP`, `ZGO_PLAYLIST_NAME`,
  `ZGO_TRACK_NUMBER`, `ZGO_TRACK_NUMBER_STATUS`), `ZONE_ACTIONS` entries and the publishers for the
  name and the track number.
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

- **KZO-DEC-1:** six zone groups, three client groups and two groups of device-level objects,
  strides 60, 30 and 20, reserved numbers undefined.
- **KZO-DEC-2:** Play/Pause is 1.001. **KZO-DEC-3:** as proposed.
- Done: T2 (objects, constants, `ZONE_GO_SLOTS`, `CLIENT_GO_SLOTS`, `GLOBAL_GO_SLOTS`, `locate`), T3
  (`decode_switch` rejects an empty payload), T4 (strides, descriptor table with empty descriptors
  for reserved numbers; the device-mode integration tests run with 1520 table entries), T5
  (`KNXPROD_APP_VERSION` 14, layout hash covers the slots and strides, knxprod and XML regenerated,
  unsigned like the committed one), T6 (TOML keys, example, macOS settings field list,
  `docs/knx-bridge.md`, `docs/upgrading.md`).
- Track Number and Track Number Status (4.2) and the grouping of the global objects (4.1b) are done.
- KZO-DEC-5 is done: `Track Step Up/Down` and `Playlist Step Up/Down` (1.008, `ZGO_TRACK_STEP_UPDOWN`,
  `ZGO_PLAYLIST_STEP_UPDOWN`, keys `track_step_updown` and `playlist_step_updown`), the four next and
  previous objects are 1.017, a zone has 41 objects, `KNXPROD_APP_VERSION` 14. The numbers of the other
  objects did not move.
- Added beyond the RFC: every text of the product database is translated into English, Dutch,
  Spanish and French (`xtask/src/i18n.rs`, injected into the generated XML, the golden file covers
  it). A new text without a translation stops the generation. The version-mismatch message also has
  a German text, its base text is English.
- Stale device memory: the memory file (`knx-memory.bin`, format version 3) carries the product
  database version of the build that wrote it. A file with another version, or without one (format
  versions 1 and 2), is not restored; SnapDog logs a warning and keeps the individual address ETS
  assigned, so ETS reaches the device and can download. The boot-time ETS parameter read ignores such
  a file too. Not covered: ETS downloading an older product to a device that runs the new build
  stamps the file with the new version, because the stamp is the build's, not the program version
  ETS wrote.
- Not done: T7 (ETS test with a project that uses the device) and the longer text objects
  (KZO-DEC-4, 4.2).
