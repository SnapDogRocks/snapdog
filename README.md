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

SnapDog turns a Linux box (or a Mac) into a synchronized multi-room audio system with deep smart home integration. It embeds a [Snapcast](https://github.com/badaix/snapcast) compatible server completely reimplemented in pure Rust (see [snapcast-rs](https://github.com/metaneutrons/snapcast-rs)), runs AirPlay and Spotify Connect receivers per zone, streams from subsonic-compatible media servers like [Navidrome](https://www.navidrome.org), plays internet radio — and bridges everything tightly to MQTT and KNX.



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
| 🏠 **MQTT** | Bidirectional smart home integration over MQTT 3.1.1: retained state, commands, availability; TLS and mutual TLS |
| 🏢 **KNX** | Building automation — client mode (tunnel/router) or device mode (ETS-programmable, 33 group objects per zone, 11 per client, plus 5 device-level global objects) |
| 🌐 **REST API** | ~100 endpoints, full zone/client/media control |
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
cosign verify ghcr.io/snapdogrocks/snapdog:v0.25.1 \
  --certificate-identity-regexp '^https://github\.com/SnapDogRocks/snapdog/\.github/workflows/release\.yml@refs/tags/v[0-9].*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

For unattended, signature-verified updates with health-gated rollback, use the
[production Compose updater](docs/docker-updates.md). For reproducible deployments,
pin a version or digest instead of `latest`. On Linux,
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
Running servers also expose the exact embedded product database at
`GET /api/v1/knx/knxprod` and its application/hardware identity at
`GET /api/v1/knx/product-info`, so integrators can verify that ETS and firmware
use the same product version.

### Binary

Download from [Releases](https://github.com/SnapDogRocks/snapdog/releases/latest),
verify the measured digest, then run it:

```bash
sha256sum --check SHA256SUMS --ignore-missing
snapdog --config snapdog.toml
```

On Windows, SnapDog can run as a native service:

```cmd
sc create SnapDog binPath= "\"C:\Program Files\SnapDog\snapdog.exe\" --service --config \"C:\ProgramData\snapdog\snapdog.toml\""
sc start SnapDog
```

### Debian/Ubuntu (APT)

Packages come from `deb.snapdog.cc`. Check the key before you trust it:

```bash
curl -fsSL https://deb.snapdog.cc/snapdog-archive-keyring.pgp -o /tmp/snapdog-archive-keyring.pgp
gpg --show-keys --with-colons /tmp/snapdog-archive-keyring.pgp \
  | awk -F: '$1 == "fpr" { print $10; exit }'
# Expected: 1B7B79417383648BBFBE282E01AB8296EF0FCD76
```

That is the certifying primary key. It signs nothing itself; the packages for
this domain are signed by its subkey
`CFE7BACA652DD0D72EB7156FC7252FAD41F8F7DA`. Only install once the primary
fingerprint above matches:

```bash
sudo install -m 0644 /tmp/snapdog-archive-keyring.pgp \
  /usr/share/keyrings/snapdog-archive-keyring.pgp
echo "deb [signed-by=/usr/share/keyrings/snapdog-archive-keyring.pgp] https://deb.snapdog.cc rolling main" \
  | sudo tee /etc/apt/sources.list.d/snapdog.list
sudo apt update
sudo apt install snapdog snapdog-client
```

Do not add `trusted=yes`. It disables exactly the check the fingerprint above is
for.

#### What the packages do

The packages install the systemd units and manage them on install, upgrade and removal.

- **Fresh install:** `snapdog-client` is enabled and started. It finds its server through mDNS. `snapdog` is installed disabled, because it needs a configuration first. Edit `/etc/snapdog/snapdog.toml`, then run `sudo systemctl enable --now snapdog`.
- **Upgrade:** a service that runs is restarted, which interrupts the audio for a moment. Whether a service is enabled stays as you left it. A stopped `snapdog` stays stopped, and a disabled or masked `snapdog-client` stays that way. A `snapdog-client` that is enabled but stopped is started again, because the package starts it on every upgrade.
- **Removal:** both services are stopped. If `snapdog` was enabled and running when you removed it, installing it again starts it again, also after a reboot, as long as you did not purge it in between.
- **Purge:** also deletes what `snapdog` keeps in `/var/lib/snapdog`, such as pairing keys and the ETS and EQ settings. If `snapdog` still runs, because a `policy-rc.d` on the host keeps packages from stopping services, the directory is kept and the purge says so. Stop the service and delete `/var/lib/private/snapdog` and `/var/lib/snapdog` yourself then.

**Upgrading from 0.28.2 or older.** Those packages shipped the unit files but no maintainer scripts, so nothing recorded whether you ever enabled `snapdog-client`. The upgrade therefore leaves it as it is: if you enabled it, it stays enabled and is restarted, and otherwise it stays disabled and stopped. Only a fresh install enables it. The same holds if you removed the old package without purging it and install the new one. `snapdog` is not enabled either, and one you enabled yourself is restarted like any running service, so expect a short gap in the audio.

#### Moving from the old repository

Until September 2026 the packages were served from a branch of this repository
at `https://snapdogrocks.github.io/snapdog/debian`, signed with the key
`E4AAC210C8C21377554DBDE40623E5F3B4379FC7`. That location is frozen at 0.27.4
and receives nothing further. It is not redirected, because GitHub Pages cannot
redirect and `apt` would ignore an HTML one.

```bash
sudo rm -f /etc/apt/sources.list.d/snapdog.list \
           /usr/share/keyrings/snapdog-archive-keyring.asc
```

Then follow the steps above. The package names do not change, so an installed
system upgrades in place.

### Homebrew (macOS)

```bash
brew tap snapdogrocks/tap
brew install snapdog
brew install snapdog-client
```

### Arch Linux (AUR)

```bash
yay -S snapdog          # or: yay -S snapdog-client
```

#### Default configuration of the packages

The Debian and AUR packages install a small starter configuration as `/etc/snapdog/snapdog.toml`: one zone named "Living Room" (the name AirPlay and
Spotify show), Spotify and AirPlay with their defaults, the web UI on port 5555. Earlier packages installed the
full example instead (two zones, three speakers, an MQTT broker, a Subsonic server), which started services that
were not yours. The full reference stays in `snapdog.example.toml` and at https://snapdog.cc/docs/configuration.

The file is a package configuration file. If you edited it, `dpkg` asks before replacing it and `pacman` leaves your
file and writes the new one next to it as `snapdog.toml.pacnew`. A copy you never edited is replaced by the starter
configuration on upgrade, so the zone names, and with them the AirPlay and Spotify names, the MQTT entities and the
KNX bindings of the old example, change. Copy the sections you want from the example before you upgrade, or restore
them afterwards.

The AUR recipes install the same measured Linux binaries used by the release
tarballs, Debian packages and container images; they do not rebuild from source.

### macOS App

Download `SnapDog-Server-*.dmg` from [Releases](https://github.com/SnapDogRocks/snapdog/releases/latest). The menu bar app embeds the server binary, manages start/stop, and includes Sparkle auto-update.

### From Source

Requires Rust 1.94 or newer. Use the committed lockfile; see
[dependency maintenance](docs/dependencies.md) for upstream compatibility holds.

```bash
cargo build --release --locked
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
bind = "::"                            # default; 0.0.0.0 for IPv4 only
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
# streaming_bind = "192.168.10.5"    # One address for the audio port (default: all interfaces)
unknown_clients = "accept"           # accept | ignore | reject
default_zone = "Living Room"         # Zone for unknown clients (accept only)

[mdns]
# enabled = true                     # Advertises _snapdog._tcp via OS daemon
# advertise_snapcast = false         # Additionally advertise _snapcast._tcp

[dbus]
# enabled = true                     # MPRIS2 per zone (Linux only)

[airplay]
# mode = "airplay2"                  # airplay1 | airplay2 (airplay2 needs a build with feature ap2)
# password = "1234"

[spotify]
# name = "Home"                      # Spotify device name: exactly this with one zone, "<name> <zone>" with several

[mqtt]
broker = "192.168.1.10:1883"         # or "mqtts://broker.example.org:8883" for TLS
# client_id = "snapdog"              # Must be unique per broker
# username = "user"
# password_file = "/run/secrets/mqtt_password"   # or password = "pass", or SNAPDOG_MQTT_PASSWORD
# ca_file = "/etc/snapdog/mqtt-ca.pem"           # private CA (TLS only)
# client_cert = "/etc/snapdog/mqtt-client.pem"   # mutual TLS (TLS only)
# client_key = "/etc/snapdog/mqtt-client.key"
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

### Upgrading: changes that can stop a start or move data

These changes came with the configuration and command-line fixes. Each one is a deliberate choice; the
alternative is named.

- **Command-line overrides are validated like file values.** `--sample-rate` accepts 44100, 48000, 88200,
  96000, 176400 and 192000, `--bit-depth` 16, 24 and 32, and the ports must differ, exactly as the same keys in the
  file always did. Before, a value only given on the command line (`--sample-rate 32000`, `--sample-rate 8000`)
  was not checked. It now stops the start with the message that names the value. A systemd unit with
  `SNAPDOG_ARGS="--sample-rate 32000"` restarts in a loop until the argument is changed. The other choice,
  accepting more rates on the command line than in the file, would have made the same setting valid in one place
  and invalid in the other.
- **A leading `~` in a path means your home directory.** `state_dir`, `log_file`, the Subsonic cache path and the
  TLS paths used to take `~` literally, as a directory named `~` under the working directory. State written there
  (`zones.json`, `eq.json`, `server_id`, `airplay-pairing`) is no longer read. If such a directory still exists,
  SnapDog says so at startup and in `--check-config`; move its contents to the real path. Writing the old
  literal path was never what anybody meant, so the literal reading was not kept.
- **`[spotify].name` names the Spotify Connect device.** It used to be ignored (with a warning); the name in the
  Spotify app was always the zone's `spotify_name`, by default the zone name. Now a zone without its own
  `spotify_name` is called `[spotify].name` when there is exactly one zone, and `"<name> <zone name>"` when there are
  several, because every zone runs its own device and Spotify lists devices by name. A zone's own `spotify_name`
  always wins, and without a name (or with a blank one) the zone name is used as before. This is visible for every
  configuration that already has the key, among them those the macOS Settings app wrote: the device names in the
  Spotify app change from the zone names to the new ones, and the Spotify app may list them as new devices (the
  Zeroconf id is derived from the name). Remove the key, or set `spotify_name` per zone, to keep the old names.
- **An empty encryption key is never a key.** `SNAPDOG_SNAPCAST_ENCRYPTION_PSK` set to nothing (for example
  `${PSK:-}` in a compose file whose secret did not arrive) used to be used as the key. It is now ignored with a
  warning, a key from the file stays, and with `codec = "f32lz4e"` and no other key SnapDog refuses to start
  instead of falling back to the public built-in key. An empty `encryption_psk` in the file is treated the same.
  Clients that were configured with an empty key need the real key, or the setting has to be removed.
- **A WebSocket client that falls behind is closed with code 1013 (try again later)**, not 1001 (going away),
  which is for a server that shuts down. The web UI reconnects on any close and is not affected. A client that
  treated 1001 as "stop reconnecting" will now reconnect, which is what it should do for a lagging connection;
  one that expects 1001 for lag has to accept 1013.
- **The interactive API console at `/docs` is off by default.** It used to be on, and its page embedded the
  whole OpenAPI document without any API key. Set `[http].api_console = true` to get it back (see
  "API documentation (OpenAPI)" below for what that costs). The OpenAPI document is served on its own at
  `GET /api/v1/openapi.json`, and that route needs the key when `api_keys` is set.

<details>
<summary><strong>MQTT</strong></summary>

Topics, all below `base_topic` (`{n}` is the 1-based position of the `[[zone]]` or `[[client]]`):

- `status`: `online` / `offline`, retained. `offline` is the last will, and is also published on a clean shutdown.
- `zones/{n}/state`, `clients/{n}/state`: retained JSON.
- `zones/{n}/{volume,mute,control,shuffle,repeat,position,track,playlist}/set`, `zones/{n}/presence/{,enable/,timeout/}set`, `clients/{n}/{volume,mute,latency,zone}/set`: commands. `volume` takes `0.0..1.0` or `0..100` (so `1` is 100 %); booleans take `true`/`false`, `on`/`off`, `1`/`0`. A retained command is ignored.

After a reconnect SnapDog subscribes again and republishes availability and state. Commands are rate limited per zone and client.

`broker` takes `host:port` (plaintext) or `mqtts://host:8883` (TLS 1.2 or newer, certificate and host name always verified, optional `ca_file` and mutual TLS). A wrong `[mqtt]` section switches MQTT off and is logged as an error, the rest of the server keeps running (`snapdog --check-config` reports it). Limits, error handling, broker settings and what changed from earlier releases: [docs/mqtt.md](docs/mqtt.md). `snapdog.example.toml` describes every key and how to harden the broker (TLS-only listener, per-client credentials, ACL).

**Home Assistant.** Home Assistant's MQTT integration has no `media_player` platform, so a zone cannot appear as one (the discovery messages earlier releases published never produced an entity, and SnapDog now deletes them). What MQTT can express is available as opt-in discovery: set `[mqtt.homeassistant] discovery = true` and SnapDog announces one device with, per zone, volume, mute, shuffle, repeat, transport buttons, playback state, title, artist, album, duration, source, playlist and presence, and per client volume, mute, latency, zone and connection (48 entities for two zones and two clients). It is sent after every connect, when a client is added, and after Home Assistant's birth message, and `discovery = false` takes the entities out again. No cover art, no position bar and no `tts.speak` target: those need a real integration, see [RFC 0007](docs/rfcs/0007-home-assistant-integration.md) and [docs/mqtt.md](docs/mqtt.md#home-assistant).

</details>

<details>
<summary><strong>API Authentication</strong></summary>

If `api_keys` is set in `[http]`, all `/api/v1/*` and `/ws` endpoints require authentication:
- REST: `Authorization: Bearer <key>` header
- WebSocket: `ws://host:port/ws?token=<key>` query parameter; query tokens are accepted only on `/ws`
- Health endpoints and the WebUI are always accessible

Without `api_keys` every endpoint is open, and SnapDog warns at startup when it listens on all interfaces.

</details>

<details>
<summary><strong>API documentation (OpenAPI)</strong></summary>

Builds with the `api-docs` feature, which the released binaries have, serve the OpenAPI document at
`GET /api/v1/openapi.json`. It is part of `/api/v1`: with `api_keys` set it needs the Bearer key, without keys
it is open like the rest of the API. Set `[http].api_docs = false` to switch it off. Builds without the
feature serve no API documentation at all.

The interactive console (Scalar) at `/docs` is off by default. `[http].api_console = true` turns it on. A
browser cannot send the `Authorization` header when it opens a page, so the console is not behind the API key,
and the page contains the whole OpenAPI document. The page and its script are served by SnapDog itself: a pinned,
vendored copy of Scalar (`@scalar/api-reference` 1.74.0, MIT, about 1.3 MB gzip-compressed in the binary, licence at
`/docs/assets/NOTICE.md`) at `/docs/assets/scalar.js`. No other host is contacted, so the console also works
without internet access, and a Content-Security-Policy keeps the page to its own origin. Scalar's fonts, telemetry,
AI agent and MCP features are switched off. The page runs on the same origin as the web UI, and SnapDog logs one
warning at startup when the console is on. Enable it only on a network you trust. The script is sent compressed
(`Content-Encoding: gzip`); a client that does not accept gzip gets 406, which no browser does.

```bash
curl -H "Authorization: Bearer $SNAPDOG_KEY" http://localhost:5555/api/v1/openapi.json
```

</details>

<details>
<summary><strong>Network ports and what they expose</strong></summary>

| Port | Protocol | Carries | Protection |
|------|----------|---------|------------|
| 5555 (`[http].port`) | TCP | WebUI, REST API, WebSocket | `[http].api_keys`, optional TLS; `[http].bind` limits the address |
| 1704 (`[snapcast].streaming_port`) | TCP | The audio stream for Snapcast clients | None. Whoever can reach the port can connect as a client and receive the audio |
| 1705 (`[snapcast].jsonrpc_tcp_port`) | TCP | JSON-RPC control of an external `snapserver`, only in builds with the `snapcast-process` feature (the released binaries use the embedded server and do not open it) | None. Anyone who can reach it controls the server, and SnapDog's generated `snapserver.conf` does not restrict its address, so block it in the firewall |
| 3671 | UDP | KNX/IP device mode | KNX/IP has no authentication of its own |

Port 1704 has no authentication, in SnapDog as in Snapcast. To limit it:

- `[snapcast].streaming_bind = "192.168.10.5"` makes SnapDog listen on that one address instead of all
  interfaces (one address, IPv4 or IPv6; `127.0.0.1` for clients on the same host only). It is also written
  to the generated `snapserver.conf` in builds that drive an external `snapserver`.
- Block the port in the firewall for every network that is not yours. In Docker, publish it on one
  address (`-p 192.168.10.5:1704:1704`) and leave `streaming_bind` unset, because inside the container
  the address is the container's.
- If the stream must not be readable on the network, use `codec = "f32lz4e"` with your own `encryption_psk`
  and pass the same key to every client (`--encryption-psk`). SnapDog warns when `f32lz4e` runs with
  the built-in default key, which is public.
- Clients that find the server with mDNS connect to the address mDNS resolves, so the bound address must be
  reachable for them.

`snapdog --config snapdog.toml --check-config` prints a note while the port listens on all interfaces, refuses a
`streaming_bind` that is not an address, and warns when the address belongs to no network interface of the machine
it runs on (a best-effort check: it only reports the operating system's "cannot assign requested address", so it is
a warning, not an error, because the file may be checked on another host). It does not bind the port itself. At
start, an unassignable address stops SnapDog with a message that says so; a port that is already in use says that
instead.

</details>

<details>
<summary><strong>Log file</strong></summary>

`[system].log_file` writes the log to a file in addition to stdout, one file per day (`snapdog.log` becomes
`snapdog.2026-10-26.log`). SnapDog keeps 14 daily files, the current one and 13 more, and deletes older ones when
it starts and then hourly. It deletes only files named exactly `snapdog.<yyyy-mm-dd>.log`, the oldest date first;
everything else in the directory, such as `snapdog-audit.log`, is left alone. Days change at midnight UTC,
whatever the local time zone. An error while writing (disk full) is not reported; stdout keeps working.
The packaged systemd unit creates `/var/log/snapdog` for this (`LogsDirectory=`).

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

Each zone appears as `org.mpris.MediaPlayer2.snapdog.zone{N}`. SnapDog uses the session bus when `DBUS_SESSION_BUS_ADDRESS` is set and the system bus otherwise, which is the case for the systemd service. The system bus only lets a process own a name that a policy allows, so the `.deb`, AUR and SnapDog OS packages install one to `/usr/share/dbus-1/system.d/` (`snapdog.conf` for the server, `snapdog-client.conf` for the client). If you install the binaries by hand, copy the files from `packaging/` in the release archive there as well.

The policy lets every local user read the players and control them, which is what `playerctl` and media keys do. It allows sending to each bus name one by one instead of by prefix, because `dbus-broker` ignores prefix rules for sending and would deny every caller. The server's file covers zones 1 to 16 (the plain name and `.instance2` to `.instance10` for each). For a zone above 16 the server logs a warning, and nobody can call its player on the system bus until you add its names to `snapdog.conf`.

The same rule also lets every local user own those names, not only the service, because the service runs as a systemd `DynamicUser` and its uid is not known to the bus when it reads the policy. A local user who takes a name before the service asks for it (at boot, or right after a restart) makes the service fall back to the next `.instanceN` name, and calls to the plain name then reach that user. They can misdirect or swallow media key commands and show wrong metadata, but they only receive MPRIS calls and nothing secret. If you cannot accept that, run the service with a fixed `User=` instead of `DynamicUser=yes` and move the `own_prefix` rule in the policy file into a `<policy user="...">` element for that user. Disable with:

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
- **Behringer X32 / Midas M32 volume** — move channel faders over OSC on the network, no MIDI interface needed. The volume is the fader position over the whole travel, the same scale `--mixer midi` gives on the X32: 100 % is the top (+10 dB), 75 % is 0 dB. To stop at unity, set the client's `max_volume` to 75. Mute switches the channel off, but only when SnapDog itself mutes or unmutes: a plain volume change never re-opens a channel someone muted at the desk. Every write is read back and repeated if the desk lost it, and a desk that boots later still receives the volume.

  Changes made at the desk flow back to SnapDog: the client reads the faders twice a second, and a moved fader or a channel switched off becomes the client's volume or mute in SnapDog, the app and every integration. The other named channels follow the moved one. SnapDog still sets the desk when the client starts, and the fader cannot stay above the client's maximum volume: SnapDog moves it back. In absolute group volume mode a client has no volume of its own, so the desk fader moves the zone. `--mixer-sync out` turns the reverse channel off; changes at the desk then stay until SnapDog sends the next volume.

  ```bash
  # Stereo pair on input channels 31 and 32 (both are written, whatever the desk's link covers)
  snapdog-client --mixer x32:192.168.1.20:31,32
  # One channel, OSC on a non-default port (the X32 itself always uses 10023)
  snapdog-client --mixer x32:x32.local:5:10023
  # SnapDog sets the faders, the desk does not report back
  snapdog-client --mixer x32:192.168.1.20:31,32 --mixer-sync out
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
cargo run --package xtask -- gen-api-spec openapi.json  # Refresh API contract
```

`openapi.json` is the committed REST contract. The OpenAPI contract test rejects
Rust API changes until this snapshot is regenerated; after merge, automation
opens an auto-merge PR with the updated reference in
[`snapdog-web`](https://github.com/SnapDogRocks/snapdog-web).

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
