<div align="center">

<!-- Logo placeholder — replace with actual logo -->
<img src="https://raw.githubusercontent.com/SnapDogRocks/snapdog/main/assets/snapdog-logo.svg" alt="SnapDog" width="200">

**Multi-room audio system with smart home integration**

One binary. AirPlay + Spotify + Subsonic + MQTT + KNX. Snapcast-based audio delivery.

[![CI](https://github.com/SnapDogRocks/snapdog/actions/workflows/ci.yml/badge.svg)](https://github.com/SnapDogRocks/snapdog/actions/workflows/ci.yml)
[![Release](https://github.com/SnapDogRocks/snapdog/actions/workflows/release.yml/badge.svg)](https://github.com/SnapDogRocks/snapdog/actions/workflows/release.yml)
[![GitHub Release](https://img.shields.io/github/v/release/SnapDogRocks/snapdog)](https://github.com/SnapDogRocks/snapdog/releases/latest)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)
[![Docker](https://img.shields.io/badge/ghcr.io-snapdog-blue?logo=docker)](https://ghcr.io/snapdogrocks/snapdog)

</div>

---

SnapDog turns a Linux box (or a Mac) into a synchronized multi-room audio system with deep smart home integration. It embeds a [Snapcast](https://github.com/badaix/snapcast) compatible server completly reimplemented in pure Rust (see [snapcast-rs](https://github.com/metaneutrons/snapcast-rs)), runs AirPlay and Spotify Connect receivers per zone, streams from subsonic-compatible media servers like [Navidrome](https://www.navidrome.org), plays internet radio — and bridges everything tightly to MQTT and KNX.



## Features

| | |
|---|---|
| 🔊 **Snapcast** | Synchronized playback, embedded server [snapcast-rs](https://github.com/metaneutrons/snapcast-rs) or external snapcast process |
| 🎵 **AirPlay 1 + 2** | Per-zone receivers, stream from iPhone/Mac |
| 🎧 **Spotify Connect** | Per-zone receivers via librespot |
| 📻 **Internet Radio** | Station list with live ICY metadata (artist/title parsing, dynamic cover art) |
| 📚 **Subsonic/Navidrome** | Personal music library with playlist navigation and seek |
| 💾 **Track Cache** | Disk-backed LRU cache for Subsonic tracks — instant seek, replay, and look-ahead prefetch |
| ⚡ **Source Conflict** | Configurable priority: `last_wins` or `receiver_wins` (AirPlay/Spotify vs local) |
| 🎨 **Cover Art** | Content-addressed caching, ICY StreamUrl fallback, unified per-zone endpoint |
| 🎛️ **Multiband Parametric EQ** | Per-zone and per-client, genre presets, real-time via custom protocol |
| 🔊 **Speaker Correction** | Per-client Spinorama profiles (1000+ speakers from (https://spinorama.org)) |
| 🔀 **Audio Fade** | Smooth transitions: zone switch (client-side) and source switch (server-side) |
| 🏠 **MQTT** | Bidirectional smart home integration, Home Assistant auto-discovery |
| 🏢 **KNX** | Building automation — client mode (tunnel/router) or device mode (ETS-programmable, 35 group objects per zone, 11 group objects per client, presence detection mode) |
| 🌐 **REST API** | ~90 endpoints, full zone/client/media control |
| 📡 **WebSocket** | Real-time state push notifications |
| 🖥️ **WebUI** | Responsive SPA, drag-and-drop, tabbed EQ overlay, i18n (5 languages) |

## Quick Start

### Docker

```bash
docker run -d --name snapdog \
  --restart unless-stopped \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --stop-timeout 15 \
  -v snapdog-data:/var/lib/snapdog \
  -v ./snapdog.toml:/etc/snapdog/snapdog.toml:ro \
  -p 5555:5555 -p 1704:1704 -p 3671:3671/udp \
  ghcr.io/snapdogrocks/snapdog:latest
```

<details>
<summary><strong>Docker Compose (Production)</strong></summary>

```yaml
services:
  snapdog:
    image: ghcr.io/snapdogrocks/snapdog:latest
    restart: unless-stopped
    read_only: true
    tmpfs:
      - /tmp:rw,noexec,nosuid,size=64m
    cap_drop: [ALL]
    security_opt:
      - no-new-privileges:true
    stop_grace_period: 15s
    volumes:
      - snapdog-data:/var/lib/snapdog
      - ./snapdog.toml:/etc/snapdog/snapdog.toml:ro
    ports:
      - "5555:5555"      # WebUI + REST API
      - "1704:1704"      # Snapcast streaming
      - "3671:3671/udp"  # KNX/IP device
    healthcheck:
      test: ["CMD", "curl", "--fail", "--silent", "--show-error", "--max-time", "4", "http://127.0.0.1:5555/health/live"]
      interval: 30s
      timeout: 5s
      retries: 3

  snapdog-client:
    image: ghcr.io/snapdogrocks/snapdog-client:latest
    restart: unless-stopped
    read_only: true
    tmpfs:
      - /tmp:rw,noexec,nosuid,size=64m
    cap_drop: [ALL]
    security_opt:
      - no-new-privileges:true
    stop_grace_period: 15s
    devices:
      - /dev/snd
    command: ["tcp://snapdog:1704"]
    depends_on:
      snapdog:
        condition: service_healthy

volumes:
  snapdog-data:  # Persists KNX programming, state, EQ config
```

</details>

Published images are multi-architecture (`linux/amd64`, `linux/arm64`) and include
SBOM and SLSA provenance attestations. Release manifests are signed keylessly with
Sigstore. Verify an immutable release before deployment:

```bash
cosign verify ghcr.io/snapdogrocks/snapdog:v0.24.1 \
  --certificate-identity-regexp '^https://github\.com/SnapDogRocks/snapdog/\.github/workflows/release\.yml@refs/tags/v[0-9].*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

For reproducible deployments, pin a version or digest instead of `latest`. On Linux,
mDNS discovery and KNX/IP routing use multicast and generally require host networking
or a multicast-capable macvlan/ipvlan network; published ports alone cover unicast
traffic.

### KNX Device Mode (no config file needed)

```bash
# Start as KNX/IP device — ETS programs zones, clients, group addresses and parameters
snapdog --knx-device --knx-address 1.1.100

# Programming mode can be enabled via CLI flag, About dialog in WebUI, or REST API
snapdog --knx-device --knx-address 1.1.100 --knx-prog-mode

# Dual-stack IPv4+IPv6
snapdog --knx-device --knx-address 1.1.100 --bind ::
```

Started this way (device mode, no `--config`), SnapDog **configures itself entirely
from ETS**: the parameters ETS downloads — zones, clients, radios, Subsonic and
MQTT endpoints, HTTP port, log level — are persisted and applied as the running
configuration on start. When ETS finishes a programming session (sending
`A_Restart`), SnapDog persists the new programming and **restarts itself** so the
changes take effect immediately (disable with `restart_after_ets = false`). Until a
device has been programmed, built-in defaults are used.

The `.knxprod` file for ETS import is available from [Releases](https://github.com/SnapDogRocks/snapdog/releases/latest).

### Binary

Download from [Releases](https://github.com/SnapDogRocks/snapdog/releases/latest), then:

```bash
snapdog --config snapdog.toml
```

On Windows, SnapDog can run as a native service:

```cmd
sc create SnapDog binPath= "\"C:\Program Files\SnapDog\snapdog.exe\" --service --config \"C:\ProgramData\snapdog\snapdog.toml\""
sc start SnapDog
```

### Debian/Ubuntu (APT)

```bash
echo "deb [trusted=yes] https://metaneutrons.github.io/snapdog/debian stable main" \
  | sudo tee /etc/apt/sources.list.d/snapdog.list
sudo apt update
sudo apt install snapdog snapdog-client
```

### Homebrew (macOS)

```bash
brew tap snapdogrocks/tap
brew install snapdog
brew install snapdog-client
```

### macOS App

Download `SnapDog-Server-*.dmg` from [Releases](https://github.com/SnapDogRocks/snapdog/releases/latest). The menu bar app embeds the server binary, manages start/stop, and includes Sparkle auto-update.

### From Source

```bash
cargo build --release
./target/release/snapdog --config snapdog.toml
```

**Access:**

| | |
|---|---|
| WebUI | http://localhost:5555 |
| API | http://localhost:5555/api/v1/zones |
| Health | http://localhost:5555/health |
| WebSocket | ws://localhost:5555/ws |

## Configuration

Single file: [`snapdog.example.toml`](snapdog.example.toml)

```toml
# Server display name — used for mDNS, MQTT, browser title
# name = "SnapDog"

[http]
port = 5555
bind = "0.0.0.0"                       # :: for dual-stack IPv4+IPv6
base_url = "http://192.168.1.10:5555"  # Required for MQTT cover art URLs; REST API uses relative URLs
# tls_cert = "/etc/snapdog/tls/fullchain.pem"  # Enables HTTPS
# tls_key = "/etc/snapdog/tls/privkey.pem"

[audio]
sample_rate = 48000
bit_depth = 16                       # FLAC: max 24; f32lz4: always 32 (setting ignored)
channels = 2
source_conflict = "last_wins"        # last_wins | receiver_wins
zone_switch_fade_ms = 300            # Client zone switch fade (0 to disable, max 1000)
source_switch_fade_ms = 300          # Source change fade within a zone (0 to disable, max 1000)

[snapcast]
streaming_port = 1704
unknown_clients = "accept"           # accept | ignore | reject
default_zone = "Living Room"         # Zone for unknown clients (accept only)

[mdns]
# enabled = true                     # Advertises _snapdog._tcp via OS daemon
# advertise_snapcast = false         # Additionally advertise _snapcast._tcp

[dbus]
# enabled = true                     # MPRIS2 per zone (Linux only)

[airplay]
# mode = "airplay2"                  # airplay1 | airplay2
# password = "1234"

[mqtt]
broker = "192.168.1.10:1883"
# client_id = "snapdog"              # Must be unique per broker
# username = "user"
# password = "pass"
base_topic = "snapdog/"

[subsonic]
url = "https://music.example.com"
username = "user"
password = "pass"
# format = "raw"                      # raw | flac | mp3 | opus
# [subsonic.cache]
# path = "~/.cache/snapdog/tracks"    # Cache directory
# max_size_mb = 2048                  # LRU eviction

[knx]
# role = "client"                     # Connect to a KNX/IP gateway
url = "udp://192.168.1.50:3671"
# role = "device"                     # Run as ETS-programmable KNX/IP device
# individual_address = "1.1.100"
# persist_ets_config = true           # Persist ETS programming across restarts (device role)
# restart_after_ets = true            # Restart to apply new parameters after an ETS reprogram

[[zone]]
# name = "SnapDog"

[[client]]
name = "Kitchen Speaker"
mac = "02:42:ac:11:00:10"
zone = "Living Room"

[[radio]]
name = "Deutschlandfunk"
url = "https://st01.sslstream.dlf.de/dlf/01/high/aac/stream.aac"
```

Snapcast sink paths, stream names, and AirPlay names are auto-generated from zone/client definitions. KNX addresses are explicit in client mode (fits into existing installations). In device mode, ETS assigns group addresses via the `.knxprod` product database.

<details>
<summary><strong>Home Assistant Integration</strong></summary>

SnapDog publishes [MQTT Discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery) messages automatically. Zones appear as `media_player` entities in Home Assistant with zero configuration — just point both at the same MQTT broker.

Supported features: play, pause, stop, next/previous, volume, mute, shuffle, repeat (off/one/all), seek, cover art, track metadata, and availability.

</details>

<details>
<summary><strong>API Authentication</strong></summary>

If `api_keys` is set in `[http]`, all `/api/v1/*` and `/ws` endpoints require authentication:
- REST: `Authorization: Bearer <key>` header
- WebSocket: `ws://host:port/ws?token=<key>` query parameter; query tokens are accepted only on `/ws`
- Health endpoints and the WebUI are always accessible

</details>

<details>
<summary><strong>Subsonic Server Notes</strong></summary>

When using Navidrome, ensure transcoding is configured for the format specified in `format`. Without transcoding, files in non-streamable containers (ALAC/AAC in MP4) will be downloaded fully before playback, causing significant latency.

</details>

## mDNS / Bonjour Discovery

SnapDog advertises `_snapdog._tcp` via the OS DNS-SD daemon (Avahi on Linux, mDNSResponder on macOS). Clients discover the server automatically on the local network.

**Advertised service:**

| Field | Value |
|-------|-------|
| Service type | `_snapdog._tcp` |
| Port | REST API port (default 5555) |
| TXT `api_version` | `1` |
| TXT `server_id` | Persistent UUID v4 — identifies the same server across interfaces/restarts |
| TXT `snapcast_port` | Binary audio streaming port (default `1704`) |
| TXT `auth` | `true` if API keys are configured |
| TXT `tls` | `true` if HTTPS is enabled (only present when active) |
| TXT `docker` | `true` if running in a container (only present when detected) |
| TXT `base_url` | Canonical server URL (see below) |

**`base_url` resolution:** The `base_url` TXT record is included when the server is not reachable on all interfaces. Clients should prefer `base_url` over the mDNS hostname when present:

- `base_url` explicitly configured → use configured value (reverse proxy, custom domain)
- `[http].bind` set to a specific IP (not `::` or `0.0.0.0`) → auto-derived as `http://{bind}:{port}`
- Otherwise → `base_url` omitted; clients use the mDNS hostname + port directly

Optionally, `_snapcast._tcp` can be advertised on the streaming port for standard Snapcast client compatibility:

```toml
[mdns]
advertise_snapcast = true
```

## D-Bus / MPRIS2

On Linux, SnapDog registers an [MPRIS2](https://specifications.freedesktop.org/mpris-spec/latest/) D-Bus interface per zone. This enables desktop media widgets, `playerctl`, and Bluetooth AVRCP control.

```
$ playerctl -l
snapdog.zone1
snapdog.zone2

$ playerctl -p snapdog.zone1 metadata
$ playerctl -p snapdog.zone1 play
$ playerctl -p snapdog.zone1 volume 0.8
```

Each zone appears as `org.mpris.MediaPlayer2.snapdog.zone{N}` on the session bus (or system bus when running as root). Disable with:

```toml
[dbus]
enabled = false
```

## Custom Protocol (snapdog-client ↔ server)

SnapDog extends the Snapcast binary protocol with custom messages for metadata and playback control:

| Type | Direction | Purpose |
|------|-----------|---------|
| 10 | Server → Client | EQ configuration |
| 11 | Server → Client | Speaker correction profile |
| 12 | Server → Client | Fade-out trigger |
| 13 | Client → Server | Playback controls (play/pause/next/seek/shuffle/repeat) |
| 14 | Server → Client | Full zone metadata (track, playback state, volume) |
| 15 | Server → Client | Cover art binary (JPEG/PNG) |

Standard Snapcast clients ignore these messages. Only `snapdog-client` sends/receives them.

## Ecosystem

SnapDog builds on a family of Rust crates:

| Crate | Description |
|-------|-------------|
| [snapcast-server](https://github.com/metaneutrons/snapcast-rs) | Embeddable Snapcast server with per-stream codecs, custom protocol, encryption |
| [shairplay-rust](https://github.com/metaneutrons/shairplay-rust) | AirPlay 1 + 2 receiver library (RAOP/AirTunes) |
| [knx-rs](https://github.com/metaneutrons/knx-rs) | KNX protocol stack — core types, KNXnet/IP, device stack, TP-UART, .knxprod generator |
| snapdog-common | Shared types and constants between server and client (EQ, protocol IDs, volume curve) |

### snapdog-client

A specialized Snapcast client that understands SnapDog's custom protocol extensions:

- **F32+LZ4 codec** — lossless 32-bit float audio with LZ4 compression (not supported by stock snapclients)
- **Per-client parametric EQ** — receives EQ curves via custom protocol, applies biquad filters before output
- **Speaker correction** — second EQ stage for Spinorama-based speaker profiles
- **Audio fade** — smooth fade-out/fade-in on zone switch (triggered by server)
- **Hardware volume** — native ALSA mixer control with perceptual (quadratic) curve
- **MIDI CC volume** — send volume as MIDI Control Change (e.g., for professional mixing consoles)

  ```bash
  # Send volume on MIDI channel 1, CC7 (default) to a USB MIDI interface
  snapdog-client --mixer midi:hw:1:0
  # Send volume on MIDI channel 3, CC11 (expression) to a named port
  snapdog-client --mixer midi:"Scarlett 18i8":2:11
  ```
- **Encryption** — PSK-based chunk encryption matching the embedded server

Available as binary and Docker image (`ghcr.io/snapdogrocks/snapdog-client`).

## Architecture

``` plain
┌─────────────────────────────────────────────────────┐
│                     SnapDog                         │
│                                                     │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐        │
│  │ ZonePlayer│  │ ZonePlayer│  │ ZonePlayer│  ...   │
│  │ (tokio)   │  │ (tokio)   │  │ (tokio)   │        │
│  └────┬──────┘  └────┬──────┘  └────┬──────┘        │
│       │              │              │               │
│  ┌────┴──────────────┴──────────────┴─────┐         │
│  │        Embedded Snapcast Server        │         │
│  │      (per-zone streams + encoders)     │         │
│  └───────────────────┬────────────────────┘         │
│                      │                              │
│  ┌─────────┐  ┌──────┴────┐  ┌──────────┐           │
│  │ AirPlay │  │  REST API │  │   MQTT   │           │
│  │receivers│  │  + WebUI  │  │  bridge  │           │
│  └─────────┘  └───────────┘  └──────────┘           │
│                                    ┌──────────┐     │
│                                    │   KNX    │     │
│                                    │  bridge  │     │
│                                    └──────────┘     │
└─────────────────────────────────────────────────────┘
         │                   
    ┌────┴──────┐        
    │snapclients│        
    │(per room) │        
    └───────────┘        
```

- **ZonePlayer** — per-zone tokio task, owns audio pipeline (decode → resample → encode → Snapcast)
- **Dual Snapcast backend** — embedded server (default) or external process via JSON-RPC
- **Volume via Snapcast** — never PCM amplitude scaling, full dynamic range preserved
- **MAC-based client matching** — clients auto-assigned to zones from config

### Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `snapcast-embedded` | ✅ | In-process Snapcast server ([snapcast-server](https://crates.io/crates/snapcast-server)) |
| `snapcast-process` | — | External snapserver binary + JSON-RPC |
| `ap2` | — | AirPlay 2 (encrypted transport, HAP pairing) |
| `dbus` | ✅ | MPRIS2 D-Bus interface per zone (Linux only) |
| `spotify` | ✅ | Spotify Connect receiver ([librespot](https://github.com/librespot-org/librespot)) |

See [Architecture Decision Records](docs/architecture/decisions.md) for design rationale.

## Development

```bash
make setup                                    # Git hooks
docker compose -f docker-compose.dev.yml up -d  # Dev infrastructure
cargo run -- --config snapdog.dev.toml        # Run
cargo xtask ci                                # Run all CI checks locally
cargo test                                    # Test
cargo clippy -- -D warnings                   # Lint
```

<details>
<summary><strong>Dev Infrastructure (Docker Compose)</strong></summary>

| Service | Purpose |
|---------|---------|
| 3× snapclient | Simulated rooms (Living Room, Kitchen, Bedroom) |
| mqtt | Mosquitto MQTT broker |
| knxd | KNX gateway simulator |
| knx-monitor | Visual KNX bus debugging |
| navidrome | Subsonic-compatible music server |

</details>

## License

[GPL-3.0-only](LICENSE)

---

<p align="center">
If SnapDog is useful to you, consider <a href="https://www.paypal.com/donate/?hosted_button_id=DQ77WMXPGY3XJ">buying me a coffee</a> ☕
</p>
