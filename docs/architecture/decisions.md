# Architecture Decision Record

## Context

SnapDog2 (.NET 9.0) orchestrates multiple external processes (Snapcast server, shairplay,
LibVLC) via network protocols and pipes. This loose coupling is fragile — each process
boundary is a failure point, and configuration must be kept in sync across all components.

This document records the key architecture decisions for the Rust rewrite.

---

## ADR-001: Single Rust Binary with Managed Snapserver

**Decision:** One Rust binary handles all logic (AirPlay, audio decoding, API, MQTT, KNX).
Snapserver runs as a managed child process, started and supervised by the Rust binary.

**Rationale:**
- Eliminates process orchestration fragility
- Snapserver has no library API (C++ standalone binary) — FFI is not feasible
- The Rust binary generates `snapserver.conf` from `snapdog.toml`, ensuring consistency
- Snapserver listens on loopback only (127.0.0.1) — only the Rust binary can talk to it

**Consequences:**
- Docker image must include both `snapdog` binary and `snapserver`
- Snapserver updates are decoupled (apt upgrade)

---

## ADR-002: Pure Rust AirPlay via shairplay crate

**Decision:** AirPlay 1 + 2 support via the `shairplay` crate — a pure Rust
reimplementation of the original C library. No FFI, no vendored C source, no build.rs.

**Rationale:**
- Pure Rust: `#![forbid(unsafe_code)]`, no C compiler needed, cross-compiles trivially
- Supports both AirPlay 1 (ALAC/PCM) and AirPlay 2 (AAC, encrypted transport, HAP pairing)
- Same callback-based design: `AudioHandler` / `AudioSession` traits deliver F32LE PCM + metadata
- Unified `RemoteControl` trait for AP1 (DACP) and AP2 (MediaRemote) transport control
- `PairingStore` trait for persistent AP2 device pairing across restarts
- Built-in ALAC/AAC decoding, optional resampling via `resample` feature
- mDNS/Zeroconf service advertisement included

**API surface:**
- `RaopServer::builder()` → `.name()` / `.hwaddr()` / `.port()` / `.bind()` / `.pairing_store()` / `.build(handler)`
- `AudioHandler::audio_init(format) → Box<dyn AudioSession>`
- `AudioSession`: `audio_process`, `audio_flush`, `audio_set_volume`, `audio_set_metadata`,
  `audio_set_coverart`, `audio_set_progress`, `remote_control_available`
- `RemoteControl::send_command(RemoteCommand)` — Play, Pause, NextTrack, PreviousTrack, etc.

**Supersedes:** Original ADR-002 (vendored C libshairplay + FFI via `cc` crate)

---

## ADR-003: Symphonia for Audio Decoding (No LibVLC)

**Decision:** Use `symphonia` (pure Rust) for all audio decoding. LibVLC is eliminated.

**Rationale:**
- Supports all required codecs: AAC, MP3, FLAC, ALAC
- Pure Rust — no FFI, no external process, no system dependency
- Gapless playback for MP3, FLAC, ALAC
- LibVLC was the primary source of fragility in SnapDog2

---

## ADR-004: TOML Configuration with Convention over Configuration

**Decision:** Single `snapdog.toml` file. KNX addresses, sink paths, and snapserver
config are auto-generated from zone/client definitions.

**Rationale:**
- SnapDog2's `.env` file was ~500 lines, mostly repetitive KNX addresses following
  a clear pattern (zone N → `N/x/y`, client N → `3/N/x`)
- Auto-generation reduces config from ~500 lines to ~70 lines
- Overrides are possible for non-standard setups
- TOML is the Rust ecosystem standard for configuration

**Convention rules:**
- Zone N: sink = `/snapsinks/zoneN`, stream = `ZoneN`, KNX = `N/{1-4}/x`
- Client N: KNX = `3/N/x`
- Snapserver TCP source ports: `4952 + zone_index`

---

## ADR-005: JSON File for State Persistence (No Redis)

**Decision:** Persist application state to a single JSON file instead of Redis.

**Rationale:**
- SnapDog2 stored ~6 keys in Redis (2 zone states, 3 client states, 1 config fingerprint)
- A full Redis server for 6 keys is massive overhead (container, connection management,
  resilience policies, health checks)
- Single-writer model (only the Rust binary writes) — no concurrency concerns
- Atomic write (write to temp file, then rename) is crash-safe
- Eliminates one container from the deployment

---

## ADR-006: Plain WebSocket Instead of SignalR

**Decision:** Use axum's built-in WebSocket support with JSON messages instead of SignalR.

**Rationale:**
- SignalR is a Microsoft-specific protocol with complex negotiation
- No Rust SignalR implementation exists
- Plain WebSocket with typed JSON messages is simpler, universal, and sufficient
- The WebUI (React) replaces `@microsoft/signalr` with native `WebSocket` — less code

**Message format:**
```json
{"type": "zone.playback_changed", "zone": 1, "state": "playing", "track": {...}}
```

---

## ADR-007: tracing for Logging (No OpenTelemetry at Launch)

**Decision:** Use `tracing` + `tracing-subscriber` for structured logging.
OpenTelemetry deferred to a later phase.

**Rationale:**
- `tracing` is the Rust ecosystem standard, used by tokio/axum/hyper natively
- Structured logging with spans provides context propagation through async calls
- File rotation via `tracing-appender`
- OpenTelemetry Rust SDK is less mature than .NET's, and the overhead of OTEL Collector +
  Grafana is not justified for a home automation system
- `tracing` is forward-compatible: adding `tracing-opentelemetry` later requires zero
  changes to existing instrumentation

---

## ADR-008: Snapcast Control via `snapcast-control` Crate

**Decision:** Use the `snapcast-control` crate for all Snapcast JSON-RPC operations.

**Rationale:**
- Covers 100% of the Snapcast JSON-RPC API (v0.28+)
- Tokio-based async client with auto-reconnect
- Replaces SnapDog2's custom `SnapcastJsonRpcClient` (339 lines of C#)
- Actively maintained (v0.4.0, Jan 2026)

---

## ADR-009: KNX via knxkit Crate

**Decision:** Use `knxkit` for KNX/IP communication. If routing (multicast) support
is missing, extend or supplement with direct UDP implementation.

**Rationale:**
- Supports tunneling (confirmed) and likely routing
- High-level API for group value read/write
- DPT (Datapoint Type) support included
- Fallback: KNX/IP routing is simple UDP multicast — ~200 lines to add if needed

---

## ADR-010: Development Test Rig

**Decision:** Provide a `docker-compose.dev.yml` with simulated Snapcast clients,
MQTT broker, KNX simulator, and Navidrome — but no Redis, Grafana, or OTEL Collector.

**Rationale:**
- SnapDog2's dev environment had 13 containers. The Rust version needs ~6
- Simulated Snapcast clients with fixed MAC addresses for realistic testing
- Mosquitto for MQTT, knxd for KNX simulation, Navidrome for Subsonic API
- No observability stack needed — `tracing` logs to console/file

---

## ADR-011: ZonePlayer — Per-Zone Audio Pipeline with Command Channel

**Decision:** Each zone runs an independent ZonePlayer task that owns its audio pipeline,
AirPlay receiver, and Snapcast TCP source connection.

**Architecture:**
```
ZonePlayer (one per zone, runs as tokio task)
├── Command Channel: mpsc::Receiver<ZoneCommand>
├── State: Idle | Playing(Source) | AirPlayActive
├── AirPlay Receiver (own libshairplay instance, own mDNS name)
├── Decode Task (abortable): Radio/Subsonic/URL → PCM
├── PCM Channel: Decode or AirPlay → TCP Writer
├── TCP Writer: PCM → Snapcast Source (connection stays open)
└── Volume: controlled via Snapcast Group (never PCM amplitude)
```

**Complete command set:**
```rust
enum ZoneCommand {
    // Source selection
    PlaySubsonicPlaylist(String, usize),     // Playlist ID, start track index
    PlaySubsonicTrack(String),               // Single track ID
    PlayUrl(String),                         // Arbitrary HTTP stream URL
    PlayPlaylist(usize, usize),              // Unified playlist index, start track index
    SetTrack(usize),                         // Jump to track N in current playlist

    // Transport
    Play,                                    // Resume or restart current source
    Pause,
    Stop,
    Next,                                    // Next track (playlist) or next station (radio)
    Previous,                                // Previous track/station

    // Playlist navigation
    NextPlaylist,
    PreviousPlaylist,
    SetPlaylist(usize, usize),               // Select/switch unified playlist by index and track

    // Seek
    Seek(i64),                               // Absolute position in ms
    SeekProgress(f64),                        // Relative 0.0..1.0

    // Zone settings
    SetVolume(i32),                          // → Snapcast Group Volume
    SetMute(bool),
    ToggleMute,
    SetShuffle(bool),
    ToggleShuffle,
    SetRepeat(bool),                         // Playlist repeat
    ToggleRepeat,
    SetTrackRepeat(bool),                    // Single track repeat
    ToggleTrackRepeat,
}
```

**Volume routing decision:** Volume/Mute commands go through the ZonePlayer even though
they only call Snapcast Group API. Rationale: single entry point for all zone operations,
consistent state updates, and future-proof (e.g. volume fade effects).

**Source types and behavior:**

| Source | Pause | Seek | Next/Previous | Auto-advance | Metadata source |
|--------|-------|------|---------------|-------------|-----------------|
| Radio | No (Stop+Restart) | No | Next/prev station | No | ICY-metadata + config |
| Subsonic Playlist | Yes | Yes | Next/prev track | Yes | Subsonic API (before play) |
| Subsonic Track | Yes | Yes | No | No | Subsonic API (before play) |
| URL | No | No | No | No | ICY-metadata if available |
| AirPlay | External | External | External | External | DMAP callbacks |

**Metadata architecture:**
- Subsonic: full metadata (title, artist, album, cover URL, duration) set in state
  *before* decode starts. Position updated from symphonia packet timestamps during playback.
- Radio: station name from config set initially. ICY-metadata parsed from HTTP stream
  updates title live (e.g. "Artist - Song"). Cover URL optional from config.
- AirPlay: `audio_set_metadata` callback delivers DMAP (title/artist/album),
  `audio_set_coverart` delivers JPEG/PNG bytes, `audio_set_progress` delivers position.
- All metadata written to shared state store. API/MQTT/WebSocket read from there.

**Track completion and auto-advance:**
- Decode task ends → PCM channel closes → `pcm_rx.recv()` returns `None`
- ZonePlayer detects this and applies repeat/advance logic:
  - TrackRepeat=true → restart same track
  - More tracks in playlist → start next track
  - Last track + PlaylistRepeat=true → restart from track 1
  - Last track + PlaylistRepeat=false → zone goes Idle
  - Shuffle=true → random next track (no immediate repeats)

**Preemption rules:**
- AirPlay preempts everything — stops current decode, takes over PCM channel
- When AirPlay session ends, zone returns to Idle (previous source NOT resumed)
- Play(new source) stops current source, starts new one
- Second AirPlay connection to same zone: libshairplay handles this (max_clients=1,
  new connection replaces old)

**AirPlay naming convention:**
- Default: `{airplay.name} {zone.name}` → e.g. "SnapDog Ground Floor"
- Override: `zone.airplay_name` in config

**Error handling:**
- Stream connection failure → log error, zone goes to Idle, state updated
- Decode error mid-stream → skip packet, continue (radio) or advance track (playlist)
- Snapcast TCP write failure → reconnect TCP, resume writing

**Volume architecture:**
- Zone volume = Snapcast Group volume (digital mixing in client, full dynamic range)
- Client volume = Snapcast Client volume (per-speaker within a zone)
- PCM stream is always full-scale — never attenuated in the pipeline

**AirPlay disconnect detection:**
- `audio_destroy` callback is called reliably on disconnect (TEARDOWN, connection drop,
  or cleanup). This is the ZonePlayer's signal to transition from AirPlayActive → Idle.

**AirPlay metadata and cover art (TODO — not yet wired):**
- `audio_set_metadata`: DMAP-encoded metadata (title, artist, album) — must be parsed
- `audio_set_coverart`: JPEG/PNG cover art bytes — store or serve via API
- `audio_set_progress`: start/current/end timestamps — map to position/duration
- `audio_remote_control_id`: DACP ID for remote control — not needed for MVP
- Currently all four callbacks are set to `None` — must be implemented

**AirPlay resampling:**
- libshairplay delivers PCM at 44100 Hz / 16-bit (Apple standard)
- Snapcast TCP source expects the configured sample rate (default 48000 Hz)
- If input rate ≠ output rate, resample before writing to TCP source
- Resampling also needed for any other source that doesn't match the target format

**Extensibility:**
- The PCM channel between decoder and TCP writer serves as the insertion point for
  future audio processing (EQ, crossfade, normalization). No architectural changes needed.

---

## ADR-012: Unified Cover Art via API Proxy

**Decision:** All cover art is served through a single API endpoint per zone.
No external URLs are exposed to clients.

**Endpoint:**
```
GET /api/v1/zones/{id}/cover → image bytes with correct Content-Type
                             → 204 No Content if no cover available
```

**Cover cache (in-memory, not persisted):**
```rust
struct CoverEntry {
    bytes: Vec<u8>,
    mime: String,   // From source, not guessed
}
// HashMap<zone_index, CoverEntry>
```

**MIME-Type sourcing:**
- Subsonic: `Content-Type` header from `getCoverArt` HTTP response
- Radio: `Content-Type` header from cover URL fetch
- AirPlay: Magic bytes fallback (FF D8 FF → image/jpeg, 89 50 4E 47 → image/png)
- Unknown: `image/octet-stream` (browser handles it)

**Supports all formats:** JPEG, PNG, WebP, AVIF, SVG — whatever the source delivers.
No conversion, just proxying.

**Why not expose external URLs directly:**
- AirPlay has no URL, only bytes
- Subsonic URLs contain auth tokens
- CORS issues with external URLs
- Inconsistent behavior per source

**Cache lifecycle:**
- Source change → ZonePlayer fetches/receives new cover → replaces cache entry
- Zone stops → cache entry cleared
- Not persisted to state.json (transient, 50-500KB per image)

---

## ADR-013: Extended TrackInfo and SourceType

**Decision:** TrackInfo extended with genre, track number, and source type.
Cover art removed from TrackInfo (served via ADR-012 instead).

```rust
pub enum SourceType {
    Radio,
    SubsonicPlaylist,
    SubsonicTrack,
    Url,
    AirPlay,
    Idle,
}

pub struct TrackInfo {
    // Metadata
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,

    // Playback
    pub duration_ms: i64,
    pub position_ms: i64,
    pub source: SourceType,

    // Technical
    pub bitrate_kbps: Option<u32>,       // 128, 320, etc.
    pub content_type: Option<String>,    // "audio/flac", "audio/aac"
    pub sample_rate: Option<u32>,        // 44100, 48000
}
```

**Rationale:** `source` tells the WebUI what controls to show (seek for Subsonic,
next station for Radio, nothing for AirPlay). Cover art is decoupled because it's
binary data with its own caching and serving strategy (ADR-012).

---

## ADR-014: Embedded WebUI — Next.js Static Export in Rust Binary

**Decision:** Build a reactive, mobile-first WebUI using Next.js (latest) with static
export (`output: 'export'`). The built assets are embedded into the Rust binary at
compile time via `rust-embed` and served by axum.

**Rationale:**
- SnapDog's "single binary" philosophy (ADR-001) extends to the frontend — no separate
  web server, no external files, no CDN
- Next.js static export produces plain HTML/CSS/JS — no Node.js runtime needed
- The WebUI is a pure client-side SPA that talks to the existing REST API + WebSocket
- `rust-embed` bakes the `webui/out/` directory into the binary at compile time
- axum serves embedded assets with a catch-all fallback route for SPA client-side routing

**Tech stack:**
- **Next.js** (latest stable) with App Router, `output: 'export'`
- **React 19** — transitions, optimistic updates, `use` hook
- **Tailwind CSS v4** — CSS-first configuration, no `tailwind.config.js`
- **shadcn/ui** — accessible component primitives built on Radix UI
- **Framer Motion** — swipe gestures, page transitions, spring animations
- **Zustand** — lightweight reactive state management
- **Native WebSocket** — connects to existing `/ws` endpoint (replaces SignalR from ADR-006)

**Project structure:**
```
SnapDogRust/
├── snapdog/          # Rust binary crate
├── webui/            # Next.js app (independent, not a Rust crate)
│   ├── package.json
│   ├── next.config.ts
│   ├── src/
│   │   ├── app/          # App Router pages
│   │   ├── components/   # shadcn/ui + custom components
│   │   ├── hooks/        # useWebSocket, useZoneStore, etc.
│   │   ├── lib/          # API client, utils
│   │   └── stores/       # Zustand stores
│   └── out/              # Static export output (gitignored)
└── Cargo.toml
```

**Responsive layout — Apple-style adaptive design:**

| Breakpoint | Layout | Interaction |
|------------|--------|-------------|
| Mobile (< 768px) | Full-viewport zone cards, swipe left/right | Thumb-reachable controls in lower 2/3, swipe up for clients/playlists |
| Tablet (768–1280px) | Left rail with zone previews, right detail pane | Tap zone in rail → crossfade transition |
| Desktop (> 1280px) | Persistent sidebar + full zone detail | Keyboard shortcuts (Cmd+1/2/3) |

**Theme:**
- System-default dark/light via `prefers-color-scheme` (Tailwind `darkMode: 'media'`)
- System font stack: `-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI', Roboto, sans-serif`
- Warm neutral palette (Tailwind stone scale): light `stone-50` (#FAFAF9), dark `stone-950` (#0C0A09)
- Accent: amber-500 (#F59E0B) — warm, energetic, fits "SnapDog" brand
- Connected/active: teal accent
- Muted/disconnected: soft red indicator

**Embedding in Rust:**
```rust
#[derive(rust_embed::Embed)]
#[folder = "../webui/out"]
struct WebAssets;
```
- Axum catch-all route serves embedded files with correct MIME types
- API routes (`/api/v1/...`, `/ws`, `/health`) take precedence over static assets
- SPA fallback: any non-API, non-file request returns `index.html`

**Build integration:**
- `make build-webui` runs `npm run build` in `webui/`
- `cargo build --release` optionally triggers webui build via `build.rs`
- CI builds webui first, then Rust binary
- Dev workflow: `cd webui && npm run dev` with proxy to running Rust backend

**API consumption:**
- REST: all endpoints under `/api/v1/zones`, `/api/v1/clients`, `/api/v1/media`, `/api/v1/system`
- WebSocket: `/ws` for real-time state updates (zone_state_changed, zone_track_changed,
  zone_progress, client_state_changed) and bidirectional commands
- Cover art: `/api/v1/zones/{id}/cover` — unified proxy (ADR-012)

**What the WebUI does NOT do:**
- No server-side rendering (static export only)
- No API routes inside Next.js (all API is in Rust)
- No server actions or middleware
- No direct Snapcast/MQTT/KNX access — everything goes through the REST API

**Consequences:**
- Binary size increases by ~1-3 MB (compressed static assets)
- `rust-embed` adds a build dependency on the webui output directory
- Development requires Node.js toolchain in addition to Rust
- Two build steps: `npm run build` → `cargo build --release`

---

## ADR-015: Spotify Integration via librespot

**Decision:** Integrate Spotify support using the `librespot` crate in two phases:
- Phase 1: Spotify Connect receiver (passive, no credentials)
- Phase 2: Active Spotify client (credentials, playlist playback)

A potential Phase 3 (full media browser with search, albums, artists, library) is
deferred as a future optional path.

**Rationale:**
- librespot is a mature, pure-Rust open-source Spotify client library (6.7k stars)
- Spotify Connect is the #1 requested streaming protocol alongside AirPlay
- Phase 1 requires zero user configuration (like AirPlay — just appears on the network)
- Phase 2 integrates Spotify playlists into the unified playlist model alongside Radio and Subsonic

**Phase 1 — Spotify Connect Receiver (passive):**
- New module `snapdog/src/spotify/mod.rs` wrapping librespot
- One Spotify Connect device advertised per zone via Zeroconf/mDNS
- Device names: `"{prefix} {zone_name}"` (e.g., "SnapDog Ground Floor")
- No credentials needed — the Spotify app on the user's device handles authentication
- Audio: librespot decodes Vorbis/AAC → PCM → resampler → snapcast (same pipeline as all sources)
- Metadata: librespot events → zone state → WebSocket notifications
- Preemption: Spotify preempts current source (same as AirPlay)
- Config:
  ```toml
  [spotify]
  enabled = true
  device_name_prefix = "SnapDog"
  bitrate = 320  # 96, 160, 320
  ```

**Phase 2 — Active Spotify Client (credentials):**
- When optional credentials are configured, SnapDog can actively browse and play Spotify playlists
- Spotify playlists appear in the unified playlist index alongside Radio and Subsonic
- Same unified playlist command model (`SetPlaylist(index, track)` for selection,
  `PlayPlaylist(index, track)` for immediate playback), same cover endpoint pattern
- Config adds optional credentials:
  ```toml
  [spotify]
  enabled = true
  device_name_prefix = "SnapDog"
  bitrate = 320
  username = "user@example.com"  # optional: enables active playback
  password = "..."               # or token cache for OAuth
  ```

**Integration pattern (same as AirPlay):**

| Aspect | AirPlay | Spotify Connect | Spotify Active | Radio/Subsonic/URL |
|--------|---------|-----------------|----------------|-------------------|
| Control | External app | External app | SnapDog | SnapDog |
| Audio | Push (RAOP) | Pull (librespot) | Pull (librespot) | Pull (HTTP) |
| Metadata | DMAP push | librespot events | librespot events | ICY/Subsonic API |
| Cover art | Raw bytes | URL in metadata | URL in metadata | Config URL/Subsonic |
| Preempts | Yes | Yes | No (explicit) | No (explicit) |
| Per-zone | Yes | Yes | N/A (shared) | N/A (shared) |

**Consequences:**
- New dependency: `librespot` crate (~adds to binary size)
- Spotify Connect requires Spotify Premium on the controlling device
- Active client mode requires Spotify Premium credentials in config
- Cover art for Spotify tracks uses external URLs (i.scdn.co) — no proxy needed,
  `cover_url` field in track metadata already supports arbitrary URLs

---

## ADR-016: Configurable Playlist Provider Ordering

**Decision:** The order of playlist providers in the unified index is user-configurable
via a `playlist_providers` list in the config. Radio is always index 0 (when configured).

**Rationale:**
- Different users have different primary music sources
- KNX/MQTT use numeric playlist indices — the order matters for automation
- Fixed index ranges (e.g., Spotify=1-99, Subsonic=100+) are fragile and wasteful
- Dynamic ordering is extensible — new providers slot in without changing the scheme

**Config:**
```toml
# Radio is always index 0 when [[radio]] entries exist.
# Remaining providers are assigned indices in this order.
playlist_providers = ["spotify", "subsonic"]
```

**Index assignment:**
- Index 0 = Radio playlist (if `[[radio]]` entries exist)
- Index 1..N = First provider's playlists
- Index N+1..M = Second provider's playlists
- etc.

**Example with `playlist_providers = ["spotify", "subsonic"]` and 3 Spotify + 2 Subsonic playlists:**
- 0 = Radio (12 stations)
- 1 = Spotify: Discover Weekly
- 2 = Spotify: Liked Songs
- 3 = Spotify: Road Trip
- 4 = Subsonic: Bulldogs Hits
- 5 = Subsonic: SnapDog Hits

**Swap order with `playlist_providers = ["subsonic", "spotify"]`:**
- 0 = Radio
- 1 = Subsonic: Bulldogs Hits
- 2 = Subsonic: SnapDog Hits
- 3 = Spotify: Discover Weekly
- 4 = Spotify: Liked Songs
- 5 = Spotify: Road Trip

**Implementation:**
- `AppConfig` gains `playlist_providers: Vec<String>` field (default: `["subsonic"]`)
- `resolve_playlist_index()` iterates providers in order, accumulating offsets
- `get_playlists` API assembles the unified list by querying each provider in order
- `ResolvedPlaylist` enum gains a `Spotify(usize)` variant
- Existing `Radio` and `Subsonic(usize)` variants unchanged

**Consequences:**
- Adding a new provider requires only: enum variant + provider query function + config name
- KNX/MQTT automation scripts may need updating if the user changes provider order
- Default behavior (no `playlist_providers` in config) is backward-compatible: `["subsonic"]`

---

## ADR-017: Built-in Per-Zone DSP (Parametric EQ)

**Status:** Implemented

**Decision:** Per-zone parametric EQ using the `biquad` crate. Compressor/limiter deferred.
Per-client EQ not possible due to Snapcast architecture (one stream per group).

**Implementation:**
- `audio/eq.rs` — `ZoneEq` struct: 1-10 biquad bands per zone, in-place f32 processing
- Filter types: `low_shelf`, `high_shelf`, `peaking`, `low_pass`, `high_pass`
- 5 built-in presets: flat, bass_boost, treble_boost, vocal, loudness
- Persistence: `eq.json` (separate from state.json, keyed by zone index)
- Pipeline: `f32 → Resampler → EQ → f32_to_pcm → TCP → Snapserver`

**API:**
- `GET/PUT /api/v1/zones/{id}/eq` — full config
- `PUT /api/v1/zones/{id}/eq/{band}` — single band update
- `POST /api/v1/zones/{id}/eq/preset` — apply named preset
- WebSocket: `zone_eq_changed` notification

**WebUI:**
- Blurred overlay (reusable component for future client EQ)
- Enable toggle, A/B comparison, preset dropdown
- Per-band: type, frequency (log scale), gain (±12dB), Q sliders
- SVG frequency response curve (client-side biquad transfer function)

**Why not per-client EQ:**
Snapcast syncs audio within a group via a single stream. Per-client EQ would require
separate streams per client = separate groups = no sync. This is a fundamental Snapcast
limitation. Per-zone EQ is the only option that preserves synchronized playback.

**Future:** Compressor/limiter (~50 lines, envelope follower + gain reduction). No external
dependency needed.
- WebUI "save" writes current settings back to TOML for persistence across restarts
- On startup, DSP chain is initialized from TOML config

**Consequences:**
- New `dsp` module with `Equalizer` (N-band biquad) and `Compressor` structs
- Internal pipeline switches to F32 (all sources convert at decode boundary)
- Per-zone DSP chain runs in the ZonePlayer select loop, zero additional latency
- CPU cost is negligible for biquad filters + envelope follower at audio rates

---

## ADR-018: Replace snapcast-control Crate with Own JSON-RPC Client

**Status:** Accepted

**Decision:** Replace the `snapcast-control` crate with a custom Snapcast JSON-RPC
client built on `tokio::net::TcpStream` and `serde_json`. The client will own no
state — all messages (results and notifications) flow through channels to the
consumer, which manages state.

**Rationale:**
- The `snapcast-control` crate (v0.4) has a fundamental design flaw: it silently
  updates internal state from both Result and Notification messages, but provides
  no mechanism to observe state changes (no watch channel, no callback, no event)
- For commands initiated by SnapDog, Snapcast sends a Result (not a Notification),
  so the crate updates its state but SnapDog has no way to know what changed
- This forces a workaround: diff the crate's internal state after every command
- The crate also has an `init()` race condition: `server_get_status()` sends the
  request but doesn't wait for the response, so state is empty when read immediately

**Architecture:**
```
┌─────────────────────────────────────────────────────────────┐
│ snapdog/src/snapcast/mod.rs                                  │
│                                                              │
│  SnapcastClient                                              │
│    ├── connect(addr) → TcpStream (line-delimited JSON)      │
│    ├── send(request) → oneshot::Receiver<Result>             │
│    ├── notification_rx → broadcast::Receiver<Notification>   │
│    └── reconnect logic (auto-reconnect on disconnect)       │
│                                                              │
│  Internal task (spawned):                                    │
│    ├── reads lines from TcpStream                           │
│    ├── Result messages → matched to pending request via id  │
│    ├── Notification messages → broadcast to all listeners   │
│    └── disconnect → trigger reconnect                       │
└─────────────────────────────────────────────────────────────┘
```

**Key design principles:**
- **No internal state**: the client is a pure message transport
- **Request/response correlation**: each `send()` returns a `oneshot::Receiver`
  that resolves when the matching Result arrives (matched by JSON-RPC `id`)
- **Notifications via broadcast**: all Notification messages are sent through a
  `broadcast::Sender`, any number of consumers can subscribe
- **Auto-reconnect**: on disconnect, automatically reconnect and re-fetch state
- **Typed messages**: use serde for Snapcast-specific request/response/notification
  types (can reuse type definitions from `snapcast-control` or define our own)

**State management (in main.rs):**
- Subscribe to notification channel
- On any notification: update SnapDog state + send WebSocket notification
- On command execution: await the oneshot Result, then handle success/failure
- Single source of truth: Snapcast server → notifications → SnapDog state → WebUI

**Dependencies:**
- `tokio` (already have) — TcpStream, channels, spawn
- `serde_json` (already have) — JSON serialization
- `uuid` (already have) — JSON-RPC request IDs
- Remove: `snapcast-control`

**Consequences:**
- Full control over message flow — no hidden state mutations
- Proper request/response correlation — API handlers can await results
- Clean notification pipeline — all state changes flow through one path
- More code to maintain, but simpler and more predictable than the crate
