# MQTT reference

What SnapDog does over MQTT, which limits apply, and what to configure on the broker.
The user-facing overview is in the [README](../README.md#mqtt), every `[mqtt]` key is
described in [`snapdog.example.toml`](../snapdog.example.toml).

## Topics

All topics are below `base_topic` (default `snapdog/`; a missing trailing slash is
added). `<n>` is the 1-based position of the `[[zone]]` or `[[client]]` in the config
file. A client that Snapcast accepts at runtime (`snapcast.unknown_clients = "accept"`)
gets the next free number after those in the file.

| Topic | Direction | Content |
|---|---|---|
| `status` | published, retained | `online` / `offline`; `offline` is also the last will |
| `zones/<n>/state`, `clients/<n>/state` | published, retained | JSON, see `snapdog/src/mqtt/payloads.rs` |
| `zones/<n>/volume/set` | command | `0.0..1.0` or `0..100` (`1` is 100 %, `0.01` is 1 %) |
| `zones/<n>/mute/set`, `shuffle/set`, `presence/set`, `presence/enable/set` | command | `true`/`false`, `on`/`off`, `1`/`0`, any case |
| `zones/<n>/control/set` | command | `play`, `pause`, `stop`, `next`, `previous` |
| `zones/<n>/repeat/set` | command | `off`, `one`/`track`, `all`/`playlist` |
| `zones/<n>/position/set`, `track/position/set` | command | seconds, `0` to one week, fractions allowed |
| `zones/<n>/track/set`, `playlist/set` | command | a non-negative integer |
| `zones/<n>/presence/timeout/set` | command | seconds, `0..65535` |
| `clients/<n>/volume/set` | command | integer `0..100` |
| `clients/<n>/mute/set` | command | boolean, as above |
| `clients/<n>/latency/set` | command | milliseconds, `-10000..10000` |
| `clients/<n>/zone/set` | command | zone number |

A command is understood exactly or rejected: a payload that is not understood changes
nothing. A **retained** command is never executed (the broker would replay it at every
start). Commands arrive at QoS 1. Command topics have two and three levels below the
zone or client, so an ACL needs both `.../+/+/set` and `.../+/+/+/set` (see
`snapdog.example.toml`, which a test checks against a real broker).

## Home Assistant

Home Assistant's MQTT integration has no `media_player` platform, so a zone cannot be
announced as a media player. SnapDog announces what MQTT discovery can express (stage 1
of [RFC HAI-0007](rfcs/0007-home-assistant-integration.md)):

```toml
[mqtt.homeassistant]
discovery = true                        # announce
# discovery_prefix = "homeassistant"    # Home Assistant's discovery prefix
# status_topic = "homeassistant/status" # Home Assistant's birth message
```

`discovery` has three states: `true` announces, `false` announces nothing and
**deletes** the device message of an earlier run (that is how the entities leave Home
Assistant again), and not set means off with nothing written to the discovery topic at
all, so that a broker ACL without `homeassistant/#` is not asked for anything. The
default is off: the entity set is new, entity ids follow the list positions of
`[[zone]]` and `[[client]]` (RFC HAI-DEC-6 is open), and a discovery message puts
dozens of entities into every connected Home Assistant.

**One device, one message.** `homeassistant/device/snapdog_<server id>/config`
(retained, QoS 1) holds the whole server as one Home Assistant device; the server id is
the one in `state_dir/server_id` and in the mDNS record. It lists, per zone: playback
state, volume (a slider), mute, shuffle, repeat (`off`/`one`/`all`), five transport
buttons, title, artist, album, duration, source, playlist number, presence (a binary
sensor), presence control (a switch, category `config`) and presence timeout (category
`config`); per client: volume, mute, latency (`config`), zone (a select of the zone
names) and connected (`diagnostic`). The entities read the existing state topics
(`zones/<n>/state`, `clients/<n>/state`) through templates and send to the existing
command topics, so nothing about the topic scheme changes. Names are `<zone or client
name> <what>`, entity ids `<domain>.snapdog_<name>_<what>`.

- **Availability.** All entities follow `{base}status` (`online`, `offline`: the last
  will, and `offline` is also sent at a clean shutdown, so Home Assistant greys the
  entities out at once). A client's volume, mute and latency also need the client to be
  connected to Snapcast.
- **Sent again** after every connect, when Snapcast accepts a client at runtime (it gets
  entities), and when Home Assistant announces `online` on its birth topic, after a random
  delay of 1 to 5 seconds (a Home Assistant that restarted forgets what it discovered).
  The message always lists every component; SnapDog never relies on removing single
  components.
- **Volume.** The zone slider goes from 0 to 100 and sends a fraction (`0.33`), because a
  zone takes a fraction up to `1.0` and a percentage above, so a plain `1` would mean
  100 %. Everything else that is a number is sent as an integer.
- **What is not there:** no cover art (the `image` entity logs an error for each message
  that has no valid URL on its topic, which an idle zone's state is, and it cannot send an
  API key), no position (a per-second sensor would fill the database; Home Assistant
  wants `media_position` at jumps), no `media_player`, no `tts.speak` target, no browsing,
  no EQ presets. Those need the custom integration of stages 2 and 3 of the RFC.
- **Size.** The message grows with zones x clients, because the zone select of every
  client lists all zone names (three times: its options and two templates). Measured
  with short names: 2 zones and 2 clients 17 KB, 4 and 12 60 KB, 8 and 24 125 KB, 16 and
  32 211 KB, 16 and 64 331 KB. That is below SnapDog's own 1 MiB packet limit, but
  **above a broker's `message_size_limit` of 256 KiB** as recommended for the commands
  below: with discovery on, allow at least 512 KiB (`message_size_limit 524288`), and more
  for a larger installation. Managed brokers may have a lower per-message limit than that: check yours. A message that does not fit, for SnapDog
  or for the broker, is not announced and the log says so (`Home Assistant discovery
  message could not be published`; a broker that drops it says nothing).
- **Server id.** The entities carry the server id (`state_dir/server_id`). If it cannot
  be stored (a missing or read-only `system.state_dir`), every start would make up a new
  id and announce a second device next to the old one, so SnapDog announces nothing then
  and logs an error. Make the state directory writable.
- **Names.** Entity ids are built by Home Assistant from the device name (SnapDog's
  `name`, "SnapDog" by default), so `number.snapdog_living_room_volume` assumes that
  name. In KNX device mode ETS sets the whole `[mqtt]` section, so `[mqtt.homeassistant]`
  cannot be configured there.
- **ACL.** SnapDog needs `write homeassistant/device/+/config` and `read
  homeassistant/status`; `snapdog.example.toml` lists both.

To remove the entities: set `discovery = false`, restart SnapDog once, and leave it or
remove the section. To remove them by hand: `mosquitto_pub -r -n -t
'homeassistant/device/snapdog_<server id>/config'`.

**How this was checked.** The message is pinned by a golden file
(`snapdog/src/mqtt/golden/discovery_device.json`) and schema tests written from the
Home Assistant documentation (device discovery, the `number`, `switch`, `select`,
`sensor`, `binary_sensor` and `button` pages as of Home Assistant 2026.10), and the
Tier-2 tests send it through mosquitto. In addition, a Home Assistant 2026.10.4
container was run against SnapDog and a broker (by hand, not part of the test suite):
it created all 48 entities of two zones and two clients with the names, categories and
device classes above and logged no warning or error; every value and command template
was rendered with Home Assistant's own template engine and the rendered commands go
through SnapDog's parsers (a unit test pins those strings); a restart of Home Assistant
made SnapDog announce again; `discovery = false` removed all entities. Not checked
against a real Home Assistant: entity states in the UI, availability switching, and
commands sent from the UI (that needs a logged-in user, which was not created).

## Limits

SnapDog speaks MQTT 3.1.1 (rumqttc). The sizes below are fixed in the code.

| What | Limit | Why |
|---|---|---|
| Packet from the broker | 1 MiB | A larger packet ends the connection. See below. |
| Packet to the broker | 1 MiB | Backstop for what SnapDog publishes. |
| Retained zone or client state | 16 KiB | Normally about 3 KB. Title, artist and album are cut at 512 bytes each (counted as JSON text: a control character costs 6, a quote 2), a cover URL that needs more than 1024 is left out. |
| Command payload | 256 bytes | Anything longer is rejected (and logged, throttled). |

### One oversized retained message can keep MQTT down

An MQTT client cannot skip a packet it does not accept: it has to close the
connection. A **retained** message is delivered again at every subscribe, so a
retained message that is larger than the packet limit puts the client into a loop:
connect, subscribe, the broker replays the message, the connection is closed, backoff,
repeat. No command gets through while it lasts.

Anybody who may publish below the base topic can leave such a message behind, usually
by accident (a retained publish of a file to a `.../set` topic). SnapDog raises its
limit to 1 MiB, so a message of up to that size arrives and is ignored (a retained
command is never executed). A message above 1 MiB still breaks the connection. Then
the log says so:

```text
ERROR MQTT broker sent a packet of 2000000 bytes, above the limit of 1048576 bytes,
and the connection was dropped. This is usually a retained message that somebody
published below snapdog/ ...
```

To recover, find the message and clear it with an empty retained publish:

```sh
mosquitto_sub -v -t 'snapdog/#'              # shows the retained messages first
mosquitto_pub -r -n -t 'snapdog/zones/1/volume/set'
```

To keep it from happening, limit the size on the broker. In Mosquitto:

```conf
message_size_limit 262144     # bytes; 0 (the default) means no limit.
                              # 524288 or more if Home Assistant discovery is on
```

and restrict who may publish to the `.../set` topics with an ACL (see
`snapdog.example.toml`). Other brokers have an equivalent setting under another name.

## Command flood

Every zone and every client may take 20 commands per second (a burst of 30), and all
together 100 per second (a burst of 100). A slider drag sends about ten.

Beyond that nothing is lost but the intermediate values. A command that sets a value
(volume, mute, shuffle, repeat, position, track, playlist, presence, latency, client
zone) and cannot be delivered right now, because its budget is used up or the zone
player's channel is full, waits in a backlog that keeps the **latest** value per zone
or client and setting. It is delivered as soon as there is room (checked every 50 ms),
so the last position of a dragged slider is the one that arrives. A value that waited
longer than 5 seconds is dropped as stale: it is not what anybody wants applied
any more. A value is never delivered after a newer one for the same setting, however
many arrive at once. Values of different settings (say `playlist` and `track`) are
independent: if only one of them has to wait, the other may arrive first. The log says "Delaying MQTT commands" (at most once every 10 seconds).

Transport commands (`play`, `pause`, `stop`, `next`, `previous` on `control/set`) are
events: two `next` are not one. Beyond the budget they are dropped, as before.

## Configuration errors

A wrong `[mqtt]` section switches MQTT off. It does not stop the server: audio and
everything else keep running, because the smart-home link matters less than the
music. At start-up the log says

```text
ERROR MQTT is switched off: the [mqtt] section is invalid. The server keeps running
without MQTT; fix the section and restart (snapdog --check-config reports it) problem=...
```

with the reason. `snapdog --config snapdog.toml --check-config` reports the same
problem as an error (`Invalid [mqtt] section, the server would start without MQTT: ...`)
and exits with a non-zero status, so a deployment script or a CI job catches it before
the restart. A value of the wrong type inside `[mqtt]` (`broker = 5`, `discovery = "yes"`)
counts too (the message names the key and the line, never the value). A file that is not
valid TOML, or a wrong value outside `[mqtt]`, still stops the start as before.

What counts as wrong (the message names the key):

- a value of the wrong type;
- an unknown key (a misspelt `ca_flie` would otherwise silently fall back to the
  system trust store);
- a password without a username, `password` together with `password_file`;
- a broker address with credentials, a path, an unknown scheme or a WebSocket scheme,
  or a host name that is not ASCII (use the punycode form of an international name,
  for example `xn--mller-kva.lan` for `müller.lan`; the check is stricter than the
  releases before it, which passed such a name on);
- a base topic with `+`, `#`, NUL, control characters or a leading `$`;
- `ca_file`, `client_cert` or `client_key` with a plaintext address, `client_cert`
  without `client_key`, or TLS material the TLS library refuses: a CA file without a
  usable certificate, a client key that does not belong to the certificate.

There is no fallback to a weaker setting: no MQTT at all until the section is fixed.

## Credentials

Order of precedence: the `SNAPDOG_MQTT_PASSWORD` variable, then `password_file`,
then `password`. `SNAPDOG_MQTT_USERNAME`, `SNAPDOG_MQTT_CA_FILE`,
`SNAPDOG_MQTT_CLIENT_CERT` and `SNAPDOG_MQTT_CLIENT_KEY` override their keys.

- **An empty variable counts as unset.** A `SNAPDOG_MQTT_USERNAME=` left behind in a
  compose file does not wipe out the configured username. The price: you cannot
  override a configured password with an empty one.
- **`password_file` holds the password alone.** One trailing line break is dropped; a
  second line is an error. At most 4 KiB are read. A file that other users can read
  (mode `0644`, `0666`) gives a warning at start-up and in `--check-config`; use
  `chmod 600`, or `640` with a group. (Docker and Kubernetes secrets are often
  `0444` or `0644` inside the container: the warning is harmless there if nothing
  else runs in it.)
- Certificates and credentials are read once, at start. Rotation needs a restart.

## What happens when something breaks

- **The broker is away or refuses us.** The connection is retried with a jittered
  backoff (1 to 60 seconds; it starts over only after a connection held for 30
  seconds). After every reconnect SnapDog subscribes again, announces `online` and
  republishes the state of every zone and client.
- **Two clients with one client id.** The broker drops the older connection whenever
  a new one with the same client id arrives, so two such clients take turns,
  reconnecting and flipping `status` between `online` and `offline`. Backoff does
  not stop that (each connection is new). After the third connection in a row that
  closes within 30 seconds of connecting, the log says so (`MQTT connection was
  closed shortly after it was made, again and again ...`, with the client id), and
  repeats it every 20th time. The default `client_id` stays `snapdog`, because
  deriving it from the host name would change what a broker ACL or a persistent
  session sees; set `client_id` to something unique per instance.
- **A bug panics one of the MQTT tasks.** The panic is logged (`MQTT task panicked,
  restarting it`, with its message), the task starts again after the same backoff on
  the connection it had, and `is_connected` is false in between. Audio and the rest of
  the server are not affected either way.
- **Shutdown.** SnapDog publishes `offline` itself and sends DISCONNECT, because a
  clean DISCONNECT suppresses the last will. It waits at most one second for that.

## Retained messages of earlier releases

Earlier releases left two kinds of retained messages behind: `{base}/status` with a
double slash (`snapdog//status`, `online`), and `media_player` discovery messages
under `homeassistant/media_player/snapdog_zone_<n>/config` (Home Assistant's MQTT
integration has no such platform, so they never did anything).

At every start SnapDog asks the broker to delete `snapdog//status` (an empty retained
publish; it cannot know whether it holds anything) and logs that at info level. That
topic is inside the base topic, so an ACL that confines SnapDog to it is not asked for
more. The `media_player` topics are outside the base topic, which a strict ACL refuses
(most brokers drop that silently, some, such as EMQX with `deny_action = disconnect` or
AWS IoT, close the connection, once per start), so they are only cleared on request:

```toml
[mqtt]
clear_legacy_topics = true     # set for one start, then remove it
```

`clear_legacy_topics = false` switches the cleanup off completely. Unset is the
default: only `snapdog//status`. By default SnapDog therefore writes nothing outside
the base topic (a test pins this), and a retained `media_player` message of an earlier
release stays on the broker until you ask for it. It is harmless: Home Assistant never
did anything with it.

## Migration notes

For users of earlier releases. Inbound MQTT commands did not work at all in releases
from v0.16.0 on (the bridge rejected every message), so only automations written
for v0.15 or older, or written since against the documented topics, can notice what
follows.

**Commands (stricter, now working)**

| Command | Before | Now |
|---|---|---|
| `mute`, `shuffle` of a zone | `true`/`1`/`on` was true, anything else false (so Home Assistant's `ON` was false) | `true`/`on`/`1` and `false`/`off`/`0`, any case, whitespace ignored; anything else is rejected and changes nothing |
| `repeat` | `one`/`track` and `all`/`playlist`, anything else was `off` | `off` must be written; `false`, `none`, `0`, an empty payload are rejected |
| zone `volume` | `NaN` was 0, `inf` was 100 | rejected |
| `position` | negative and `NaN` accepted | finite, `0` to one week |
| client `latency` | any integer | within +-10000 ms |
| counts, zone and client numbers | `+3`, `01` accepted | plain digits; topic numbers `1..1024` |
| retained command | executed at every start | ignored (a retained publish that is delivered live to a connected bridge still runs) |
| flood | each command took the channel | 20 per second per zone or client, backlog of the latest value, see above |

**Discovery.** The `media_player` discovery messages are no longer published (Home
Assistant has no such MQTT platform, they never produced an entity) and are deleted
from the broker when `clear_legacy_topics = true` is set (and `snapdog//status` at every
start). Discovery for the entities MQTT can
express is new and off by default (see [Home Assistant](#home-assistant)). The zone
state gained three keys for it (`presence_enabled`, `auto_off_delay`, `playlist_index`);
no key was removed or changed. A cover URL that needs more than 1024 bytes as JSON
text is now left out of the state; before it was cut, which gave a URL that points
nowhere.

**Configuration.** A wrong `[mqtt]` section switches MQTT off and logs why, where an
earlier build of this branch stopped the server and releases before it disabled MQTT
without a clear message. New keys: `password_file`, `ca_file`, `client_cert`,
`client_key`, `clear_legacy_topics`; `broker` takes `mqtts://`. Unknown keys, a password
without a username, wildcards or a leading `$` in `base_topic`, and TLS options on a
plaintext address are now errors (MQTT off). `SNAPDOG_MQTT_USERNAME` and the TLS path
variables are new; an empty `SNAPDOG_MQTT_PASSWORD` or `SNAPDOG_MQTT_USERNAME` is ignored.

**Reconnects.** The client id, keep-alive (60 s), last will (`offline`, retained, QoS 1)
and the topics are unchanged. After a reconnect SnapDog now subscribes again and
republishes (before, a broker restart silently ended command handling and
availability). Reconnects use a jittered backoff of 1 to 60 seconds instead of a flat 5.

**Library users** (code that links the `snapdog` crate): `MqttBridge::connect`,
`subscribe_commands`, `publish_ha_discovery`, `poll_once` and `run` are gone;
`MqttBridge::start` takes the configuration and what the bridge needs from the server
(`MqttRuntime`) and runs its own tasks; `publish`, `publish_zone_state` and
`publish_client_state` are synchronous and queue the message.
