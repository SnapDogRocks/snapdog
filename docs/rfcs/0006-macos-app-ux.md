---
rfc: MAC-0006
title: SnapDog macOS app — Apple-native UX, enterprise hardening, WebUI config parity
status: in-progress    # draft | accepted | in-progress | done | superseded
version: 0.1.0         # v0.1: review + roadmap; Phase 1 in progress on fix/macos-app-phase1
created: 2026-07-10
updated: 2026-07-10
target_repo: snapdog   # macos-helper/SnapDogServer
target_branch: main
related: []
owners: [metaneutrons]
---

# RFC MAC-0006 — macOS app: Apple-native UX, enterprise hardening, WebUI config parity

> **For AI agents:** scoping + partial-implementation RFC for the SwiftUI menu-bar app
> under `macos-helper/SnapDogServer/SnapDog Server/` (App.swift, ConfigView.swift,
> ServerManager.swift, TOMLConfigParser.swift, LogView.swift, AboutView.swift).
> Requirements `MAC-REQ-*`, decisions `MAC-DEC-*`, tasks `MAC-T*`. **Line numbers are
> approximate; symbol names are the anchor.** Deployment target is **macOS 15.0**
> (so `@Environment(\.openSettings)`, `SMAppService`, `MenuBarExtra` are all available).
> Build/verify: `xcodebuild -project "SnapDog Server.xcodeproj" -scheme "SnapDog Server"
> -configuration Debug -destination 'platform=macOS' CODE_SIGNING_ALLOWED=NO build`.

## 1. Summary & verdict

The app is a correct macOS **menu-bar agent** (`LSUIElement` + `MenuBarExtra`, real
`Settings` scene, separate Log/About `Window`s) that supervises the `snapdog` server
binary and edits its TOML config. It has a genuinely good foundation — but it is **not
yet "perfect Apple UX and enterprise-grade"**, and its config UX is a ~30% subset of the
snapdog-os WebUI.

| Axis | Grade | Biggest gap |
|---|---|---|
| **Apple UX** | B (right archetype) | zero accessibility labels + zero localization (WebUI has 5 locales) |
| **Enterprise** | Distribution B+, Operation C | no login-item/auto-start; config edits never reach the running server (UI lies about state) |
| **Config parity** | ~30% subset + a data bug | Audio sample-rate/bit-depth pickers are dead (never persisted/loaded); whole surfaces missing (KNX, API-keys, Spotify, half of Audio) |

**Genuine strengths (do not regress):** native grouped `Form`s with `Section`/`Picker`/
`Toggle`/`SecureField` (sectioning is already at parity), template menu-bar icon, semantic
colors → automatic Dark Mode, modern `@Observable`/`@MainActor` architecture, a solid
crash supervisor (exp-backoff 1→30 s, ceiling 5, 60 s stability reset), and enterprise-grade
distribution (Developer-ID + Hardened Runtime, `notarytool`+`stapler`, EdDSA-signed Sparkle
appcast over R2, universal build).

## 2. The three "the app is wrong / lies" defects (fix first)

1. **Audio round-trip bug** — `TOMLConfigParser.load` never reads `[audio]`, and `.save`
   only writes hardcoded `sample_rate=48000`/`bit_depth=16` *if the table is absent*
   (`TOMLConfigParser.swift:93-102`); the model's `sampleRate`/`bitDepth` are discarded.
   The Audio-tab pickers therefore silently do nothing.
2. **Config edits never reach the running server** — `start()` reads config once; auto-save
   writes TOML with no reload/restart (`ServerManager.swift:50`, `ConfigView.swift:286`), so
   the user believes a change is live when it is not.
3. **Spawn failure is invisible** — the `proc.run()` catch logs but sets no `lastError`
   (`ServerManager.swift:87-90`), so a failed start shows nothing in the menu.

## 3. Apple-UX gaps (prioritized)

| Gap | Current (file) | HIG / fix | Effort |
|---|---|---|---|
| Settings open path | `SettingsLink` + `.onTapGesture` (App.swift:37) | `@Environment(\.openSettings)` + `Button("Settings…").keyboardShortcut(",")` | S |
| No accessibility labels | icon-only `Button("", systemImage:)`, empty emoji fields (ConfigView.swift:129/195/213/234) | `.accessibilityLabel(...)` on every icon-only control | S |
| No localization | `knownRegions=(Base,en)`, hardcoded strings | `Localizable.xcstrings`; `String(localized:)` for error strings | L |
| Destructive quit/stop unconfirmed | quit stops a live server with no confirm (App.swift:58) | `confirmationDialog`/`NSAlert` with verb-labeled destructive button | M |
| Logs window bare | no Copy/Clear/Reveal, no empty state (LogView.swift) | `NavigationStack`+toolbar; `ContentUnavailableView` on empty | M |
| No menus/shortcuts | only ⌘Q | `.keyboardShortcut`s + `.commands { TextEditingCommands() }` | M |
| Client "Zone" free text | `TextField` (ConfigView.swift:223) | `Picker` over `config.zones` + "Unassigned" | S |
| Dead `.onMove` | reorder with no EditButton (ConfigView.swift:124/192/231) | add `EditButton()` or drop `.onMove` | S |

## 4. Enterprise gaps (prioritized)

| Gap | Current (file) | Fix | Effort |
|---|---|---|---|
| Edits don't reach running server | `start()` reads once (ServerManager.swift:50) | `configDirty` flag → "Restart to apply" banner + `restart()` | M |
| Spawn failure silent | catch sets no `lastError` (ServerManager.swift:87) | set `lastError` in catch | S |
| No login-item / auto-start | none (Info.plist) | `SMAppService.mainApp` toggle (default off) + start-on-launch pref | M |
| Save swallows errors, no validation | `try?` (ConfigView.swift:287) | throwing save + `saveState`; MAC/URL/port validators | M |
| Secrets plaintext in TOML | (TOMLConfigParser.swift) | Keychain, inject at spawn | L |
| Sparkle no channel parity | single appcast | Stable/Beta channels matching the server's release channels | M |

## 5. Config-UX parity with the WebUI (core goal)

| WebUI section / pattern | Missing / weak in Swift | SwiftUI approach | Effort |
|---|---|---|---|
| `[audio]` round-trip | **BLOCKER** (see §2.1) | read+write `[audio]` from model | M |
| Explicit Save + "Saved"/error | `try?` swallows | `saveState` enum → status indicator | M |
| Per-field validation | none (MAC/URL/port raw) | validators + inline `.caption` error, gate save | M |
| Restart/reboot requester | edits never applied | "Restart to apply" banner + `restart()` | M |
| Prefilled Zone picker | free TextField | `Picker` over `config.zones` | S (biggest quick win) |
| Rest of Server>Audio (streaming port, fades, source-conflict, unknown-clients, default-zone, log-level, advertise) | write-only defaults | model+parser+controls | L |
| KNX matrix + API keys | absent (only MQTT) | API-keys `ForEach`; KNX `DisclosureGroup` per zone/client, phased | L |
| Sources (Spotify, AirPlay mode, Subsonic format) | Spotify absent, format hardcoded `raw` | sections + pickers | M |
| Client max-volume slider · curated emoji picker | absent | `Slider(1...100)` · popover grid | S |
| i18n (~325 keys, 5 locales) | English-only | string catalog | L |

## 6. Roadmap

- **Phase 1 — native-UX & config-parity quick wins (this RFC, in progress).** Fix the
  "app lies" defects and the highest-value native/config gaps; almost all S/M.
- **Phase 2 — enterprise hardening.** Login-item/auto-start (`SMAppService`), single-instance
  + clean quit, secrets → Keychain, Sparkle Stable/Beta channel parity, signing hygiene.
- **Phase 3 — full WebUI parity.** Rest of Server>Audio, source integrations, KNX matrix +
  API keys, live file reconciliation, string-catalog i18n (de first).

## 7. Requirements / Decisions / Tasks

### Requirements
- **MAC-REQ-1** The config UI must never silently discard a setting (round-trip integrity).
- **MAC-REQ-2** The UI must not imply a change is live when it is not.
- **MAC-REQ-3** Every start/save failure must be visible to the operator.
- **MAC-REQ-4** Icon-only controls must carry VoiceOver labels.
- **MAC-REQ-5** Config UX should converge on the snapdog-os WebUI's quality patterns
  (prefilled pickers, validation, save feedback, restart requester).

### Decisions
- **MAC-DEC-1** Menu-bar-agent archetype stays; native `Form`/`Settings` foundation is kept
  (already at parity) — build on it, do not rewrite.
- **MAC-DEC-2** Deployment target macOS 15 → use modern APIs (`openSettings`, `SMAppService`).
- **MAC-DEC-3** Phase the KNX matrix (Role+Gateway+subset first); it is the largest surface.

### Tasks — Phase 1 (fix/macos-app-phase1)
- **MAC-T1** Fix the `[audio]` round-trip (load + save sample_rate/bit_depth) + stale-PSK
  cleanup (write `encryption_psk` only for `f32lz4e`).
- **MAC-T2** Set `lastError` in the `start()` spawn-failure catch.
- **MAC-T3** Add `ServerManager.restart()` + a `configDirty` flag; ConfigView shows a
  "Restart to apply" banner after saving while running.
- **MAC-T4** Client "Zone" `TextField` → `Picker` over `config.zones` (+ "Unassigned").
- **MAC-T5** `SettingsLink` → `@Environment(\.openSettings)` button with ⌘, shortcut.
- **MAC-T6** Accessibility labels on all icon-only controls (+/− buttons, emoji fields).
- **MAC-T7** Save feedback: `saveState` enum (idle/saving/saved/failed) + status indicator.
- **MAC-T8** Basic validation (MAC address, MQTT `host:port`, Subsonic URL) with inline errors.

### Tasks — Phase 2 / 3 (later branches)
- **MAC-T20** `SMAppService` login-item + start-on-launch. **MAC-T21** single-instance +
  clean quit (real termination await). **MAC-T22** secrets → Keychain. **MAC-T23** Sparkle
  Stable/Beta channels. **MAC-T24** signing hygiene (drop `--deep`, staple `.app`).
- **MAC-T30** rest of Server>Audio. **MAC-T31** source integrations. **MAC-T32** KNX matrix +
  API keys (phased). **MAC-T33** live file reconciliation. **MAC-T34** string-catalog i18n.
