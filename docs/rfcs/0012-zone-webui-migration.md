# Zone WebUI migration

Status: implemented and locally qualified on 2026-11-15. Scope authorized on 2026-11-14.

## Outcome and boundaries

Replace the default embedded WebUI with the reviewed zone-based interface while
retaining the corrected existing interface as a selectable legacy variant.
A zone is a logical playback destination, not necessarily a physical room.
The existing REST API and WebSocket remain authoritative. No deployment,
release, or external issue-tracker mutation is part of this change.

## Design

One Next.js static export contains `index.html` (current) and `legacy.html`
(legacy), with shared API, connection, playback, library, and EQ components.
The server selects its root entry using `--webui current|legacy` (current by
default); configuration can persist the same choice. The `/legacy` route is
also retained for direct access and local development. Existing API and asset
URLs stay unchanged. Switching back to legacy requires no data migration.

The current interface has persistent zone navigation, one selected-zone player,
a shared music browser, and a speaker panel with selection, drag/drop and an
explicit destination picker. Both pointer and keyboard users can move speakers.
Moves use existing per-client assignment calls: the backend has no atomic batch
transaction. A batch records only confirmed changes, reports partial failure,
and allows undo of those changes. Undo must not silently overwrite a later
assignment made by another controller. Speaker volume and playback settings
are not reset when moving or undoing.

## Delivery and acceptance

### A. Correct the existing UI

- A1: A 401 opens a stable key prompt without page reload loops. Successful
  authentication resumes HTTP and WebSocket operation.
- A2: Bootstrap failures and successful empty configuration are distinct;
  reconnection preserves the loaded UI and newer live state.
- A3: Keyboard commands target the selected zone and do not intercept widgets,
  text entry, dialogs, or modified shortcuts.
- A4: Phone navigation does not collapse; content remains scrollable. Desktop
  carousel selection follows the visible zone. Hidden duplicate views do not
  run data loaders.
- A5: EQ is a viewport-level dialog with working focus, close and error states;
  playlist actions are separate named controls; sliders have accessible names.
- A6: Failed commands and search requests give visible feedback. Stale playlist
  responses cannot replace a later selection. Optimistic values recover.

### B. Preserve compatibility

- B1: Both entry points are exported, packaged and served. The current entry is
  the default; CLI selection and configuration round-trip are tested.
- B2: API/docs routes retain their behavior; missing static assets return 404.
- B3: Shared existing features remain reachable: source-aware transport, seek,
  volume/mute, shuffle/repeat, media search, playlists, speaker assignment,
  zone/client EQ and speaker profiles, language/theme, connection sharing,
  system information and KNX programming/download controls.

### C. New default interface

- C1: The default uses zones throughout, handles arbitrary server zone/client
  lists, and presents one active player with an explicit destination.
- C2: Single and multi-speaker drag/drop, destination picker, cancel, feedback,
  partial failure and undo work against the real API contract. Offline clients
  remain discoverable; a zone can have zero speakers.
- C3: Layout fits 320px through desktop, light/dark themes, and long labels;
  controls work with keyboard and pointer. New messages cover every locale.
- C4: Frontend lint, type checks, translations, tests and production export pass;
  focused Rust CLI/config/serving tests pass. Browser regressions exercise both
  production entries with the repository demo backend and injected failures.

## Qualification record

Completed against the production static export and the repository demo backend:

- `pnpm run lint`, `pnpm run typecheck`, `pnpm run test` (49 tests),
  `pnpm run i18n:check` (159 keys in all eight locales), and `pnpm run build` pass.
  Coverage floors were preserved. Both `out/index.html` and `out/legacy.html` exist.
- `cargo fmt --all -- --check`, `cargo clippy -p snapdog --all-targets -- -D warnings`,
  nine focused library tests for WebUI serving/configuration and three CLI tests pass.
  The Rust checks use the actual exported frontend assets. `cargo build -p snapdog`
  also passes; the built executable advertises both CLI choices.
- Chromium browser regressions pass for single/multi-selection drag, touch drag,
  destination-picker/keyboard moves, Escape and pointer cancellation, same-zone
  no-ops, partial failures, undo, failed-undo retry, later external assignments,
  and retained volume. Secondary-pointer cancellation preserves a primary drag.
- Authentication 401 does not navigate/reload. A valid key resumes HTTP and
  WebSocket operation. Initial 503 and valid empty configuration remain distinct.
  A failed reconnect snapshot keeps the player visible and offers retry.
- Keyboard slider input sends one volume write and subsequently accepts external
  updates. Selected-zone shortcuts target that zone. EQ is viewport-contained,
  closable after a failed fetch, and restores focus. Library 503 has an explicit
  unavailable/retry state.
- Both exported routes fit 320, 390, 768, 1024, and 1440px without document overflow.
  An unbroken long zone name fits at 320px. The legacy export also hydrates correctly
  when served as `/`, as required by CLI mode selection.
- Axe reports no violations on the tested current/legacy main screens in light and
  dark themes, and on the tested current EQ dialog. These automated checks do not
  establish complete WCAG conformance or replace assistive-technology testing.

The browser harness and screenshots are local artifacts under
`/tmp/snapdog-ux-audit/`; they exercise the repository demo API and injected failures,
not real audio hardware. No production deployment was performed.

### Remaining protocol limitations

Speaker batches use sequential per-client calls, not an atomic server transaction.
Undo reads the current assignment and skips speakers whose destination has changed.
The API has no compare-and-swap operation: an external assignment between that read
and the restore write cannot be ruled out. A lost HTTP response can also make a
successful server-side change appear failed; feedback asks users to check the
assignment before retrying. Undo history lasts for the current page session and
retains the latest 20 batches.


## Density revision — 2026-11-15

User review rejected excessive whitespace and decorative copy. The current UI now
orders its content as player, library, speakers. The separate toolbar row, slogan
footer, promotional headings and sidebar explanation were removed, together with
11 unused translation keys in every locale. Speaker drag instructions remain
available to assistive technology through the handle description.

Artwork, headings and spacing are smaller; phone artwork sits beside track metadata.
Transport hit targets remain unchanged. Production export, lint, type checking and
translation validation pass. Browser verification covers section order and no
horizontal overflow at 320, 390, 768, 1024 and 1440px.


## Speaker move interaction — 2026-11-15

Removed the persistent selected-speaker status and the destination-select/submit
bar. A contextual move menu appears beside the speaker heading only when speakers
are selected. Choosing a destination executes the move directly; the existing undo
and partial-failure handling remain. The grip also opens the menu on click, tap or
keyboard activation, while dragging it still moves the selection to a zone.

The menu is non-modal and belongs to the speaker section, preserving page landmarks.
Escape restores focus to the initiating grip; a completed menu action focuses the
stable speaker heading. Failed speakers stay selected for retry. Selecting no
speakers leaves only the existing Select all action.


## Transient success feedback — 2026-11-15

Successful move and restore notifications now expire after five seconds. A new
result restarts the timer; pending operations, failures and skipped restores do not
auto-dismiss. Notification expiry does not alter the undo history. Undo lives as a
labelled icon button in the speaker heading, remains available across zone changes,
and disappears when history is exhausted. Browser checks exercise expiry, successive
moves, keyboard undo after expiry, persistent partial failures and a 320px layout.


## Brand palette alignment — 2026-11-15

Compared the live https://snapdog.cc/ page with the website's source palette. The
current UI now uses its amber brand color (`oklch(0.769 0.188 70.08)`), near-black
`#0a0a0a` background and neutral gray surfaces. Light mode uses white and neutral
off-white surfaces. Inter and Outfit already match the website. Compact spacing
and content order are unchanged.

Filled actions use amber with dark foregrounds. Small accent text uses a separate
`primary-ink` utility, including hover states: dark amber in light mode and lighter
amber in dark mode. Light-mode slider ranges, focus and selection indicators also
use darker amber for contrast. Popovers inherit the current palette through root
tokens. Shared components fall back to the legacy primary color on the legacy route.

Production export, TypeScript, translation validation and lint pass. Chromium axe
checks report no violations on the tested light/dark main screens, mobile views,
move menus and EQ dialogs. Layout checks cover 320, 390, 768, 1024 and 1440px without
document overflow. Desktop and phone screenshots were visually inspected.
