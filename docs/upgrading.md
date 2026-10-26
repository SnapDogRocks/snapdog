# Upgrade notes

Changes that alter what an existing installation does after an update. Newest first.
The changelog lists every change, this page lists the ones you have to act on or check.

## Release builds include AirPlay 2 and use it by default (`ap2`)

First release after 0.28.2. This is a breaking change.

### What changed

Up to and including 0.28.2 the release binaries were built without the `ap2` feature, so
every zone ran AirPlay 1. The `[airplay]` keys `mode` and `bind` were compiled out and did
nothing, although `snapdog.example.toml` documents `airplay2` as the default.

Release binaries (archives, `.deb`, AUR, Homebrew) and the published container images are
now built with the `full` feature, which includes `ap2`. The default `mode = "airplay2"`
is therefore live: after the update each zone is an AirPlay 2 receiver. Source builds with
plain `cargo build` keep the default feature set and stay AirPlay 1 only.

### Check your senders

Phones, Macs and most players speak AirPlay 2. Senders that only use the classic RAOP
protocol (some software senders on Linux, for example) may not connect to an AirPlay 2
receiver. We have not tested every sender. If one stops working after the update, use the
way back below.

### Keep AirPlay 1

Add this to `snapdog.toml` and restart. The setting is shared by all zones:

```toml
[airplay]
mode = "airplay1"
```

The start-up log shows what runs. Each zone logs `AirPlay started` with `mode=Airplay2` or
`mode=Airplay1`.

### Network and firewall

AirPlay 1 needs nothing beyond what you already allow. AirPlay 2 adds:

| What | Detail |
|------|--------|
| mDNS | A `_airplay._tcp` record per zone next to the `_raop._tcp` record. |
| UDP 319 and UDP 320 | PTP clock traffic from the sender. IPv6 wildcard, one listener for the whole process. |
| Ports chosen by the system | Per stream, event (TCP) plus data and control (UDP). AirPlay 1 already used such ports. Allow your sender's subnet rather than fixed numbers. |

Unchanged: the receiver of zone N listens on TCP 7000 + N (zone 1 on 7001).

UDP 319 and 320 are privileged ports. SnapDog treats the PTP listener as optional. If it
cannot bind, AirPlay 2 still works, but each connect starts slower (the shairplay
documentation gives about 2.6 s without the listener and 0.5 s with it). The listener
logs under its own `shairplay` target, which the default log filter hides. To see it, add
`RUST_LOG=shairplay=info` to the service environment. You then find `PTP sink active on
319/320` when it works and `PTP sink: cannot bind` when it does not. The hint in that
message (ports below 1024 may need privileges) is only right for `Permission denied`.
Every zone after the first logs the same line with `Address already in use`, because the
first zone's listener already serves the process. That is expected. You also get it on
every zone if another PTP daemon such as `ptp4l` owns the ports.

### Allowing UDP 319 and 320

The packaged systemd unit (`.deb`, AUR, release archive) grants the service
`CAP_NET_BIND_SERVICE` and nothing else, so the listener binds without further steps.
This needs the unit file of the new release: after the package update, run
`systemctl daemon-reload` and restart the service. If you replaced the unit or use a
drop-in that resets it, add these lines to the `[Service]` section:

```ini
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
```

Containers: the image runs as a non-root user. Docker does not pass `cap_add` on to
such a user as an ambient capability, so `cap_add: [NET_BIND_SERVICE]` does not help (tested
with Docker 29 and the user the image uses). Under host networking, which AirPlay needs for
discovery, lower the host's limit for unprivileged ports instead. The `sysctl` option of
the container is refused there with `not allowed in host network namespace`:

```sh
echo 'net.ipv4.ip_unprivileged_port_start=319' | sudo tee /etc/sysctl.d/50-snapdog-ptp.conf
sudo sysctl --system
```

This lets every unprivileged process on that host bind the ports from 319 upwards. If you
do not want that, leave it. AirPlay 2 then works with the slower connect described above.
With bridge networking the container has its own limit (0 by default on Docker 29,
tested), so the listener binds, but discovery does not reach your network that way.

### Pairing data

AirPlay 2 can keep the keys of paired senders in `airplay-pairing` in the state directory
(`/var/lib/snapdog` by default). SnapDog configures no PIN, and the shairplay source shows
that senders then pair transiently and nothing is stored. We have not tested this with a
real sender. The file is only written when a sender runs a persistent pair-setup. Two
limits apply to that case (read from the source of SnapDog and shairplay 0.10.0):

- Every zone loads its own copy of the file at start and rewrites the whole file on each
  change. A key stored through zone B is therefore overwritten by the next pairing through
  zone A, at any time before the next restart, because zone A's copy does not contain it.
  A restart does not bring the key back. Until this is fixed, pair through one zone only,
  or restart SnapDog after each pairing so that all zones reload the file before the next
  sender pairs.
- SnapDog does not store the server's own identity key. shairplay then generates a new
  identity at every start, and its documentation says that senders paired persistently
  have to pair again in that case.

## API documentation and console (`api-docs`)

First release after 0.28.2. This is a breaking change.

Release builds now contain the API documentation, where earlier release builds did not.
The OpenAPI document is served at `/api/v1/openapi.json` and needs the API key like the
rest of `/api/v1` (it is open when no keys are configured). The interactive console
(Scalar) at `/docs` is off by default. `[http] api_console = true` turns it on. A browser
cannot send the key to the console page, so the console is not behind the key, and
startup warns about that when keys are set. Unknown paths under `/api` and `/docs` answer
404 instead of the web UI page. Nothing changes for clients that only use the REST or
WebSocket API.

## Also changed in this release

Other changes of the same release that can need action, taken from the commit messages of
the configuration and CLI fixes:

- **Debian and AUR configuration.** `/etc/snapdog/snapdog.toml` is now a minimal
  one-zone configuration instead of the example file. A copy that was never edited is
  replaced on upgrade, so the zones, speakers, MQTT broker and Subsonic server of the old
  example disappear. An edited copy is kept (dpkg asks, pacman writes
  `snapdog.toml.pacnew`).
- **snapdog-client `--soundcard`.** It selects the playback device now. The value is a
  device index, or a part of the name or id, ignoring case, as `snapdog-client --list`
  prints them. If nothing matches, the client stops at startup. A unit that passes a
  Snapclient-style value such as `hw:1` no longer starts and, with `Restart=on-failure`,
  restarts in a loop. Check `SNAPDOG_CLIENT_ARGS`.
- **snapdog-client `--user`, `ws://` and `wss://`.** They are refused at startup. They
  never worked: `--user` carried on as the invoking user, and the websocket transport is
  not supported by the Snapcast client library.
- **Command-line values are validated.** The check runs again after the overrides, so
  for example `--sample-rate` outside 44100, 48000, 88200, 96000, 176400 and 192000 now
  stops startup. A unit with such an argument fails to start.
- **`~` in paths.** A leading `~` or `~/` in `state_dir`, `log_file`, `subsonic.cache.path`,
  `http.tls_cert` and `http.tls_key` is replaced by the home directory. Before, SnapDog
  created a directory named `~` in its working directory, and state kept there is not
  found any more.
- **`[system].log_file`** is implemented. It writes a daily file such as
  `snapdog.2026-10-25.log` and keeps 14 of them. The retention deletes older files in that
  directory with the same prefix and suffix, so give SnapDog a directory of its own. The
  packaged unit has `LogsDirectory=snapdog`.
- **Empty encryption key.** An empty `SNAPDOG_SNAPCAST_ENCRYPTION_PSK` counts as not set.
  With codec `f32lz4e` the built-in public key is used then, and startup warns. Set a real
  key on the server and the clients if you relied on an empty one.
- **WebSocket.** A client that reads too slowly is closed with code 1013 (try again later)
  instead of 1001, which the web UI showed as a stopping server.
- **Startup notices.** Unknown keys, keys a build ignores and the default encryption key
  are reported once at startup, and `--check-config` prints them. Existing files still load.
- **New.** `[snapcast].streaming_bind` binds the audio port 1704 to one address. The README
  lists the ports and what they expose.

## Features that did not change

`snapcast-embedded`, `dbus` and `spotify` are default features and were part of every
release already. `snapcast-process` is an alternative engine and is not part of a release.
