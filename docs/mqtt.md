# MQTT reference

What SnapDog does over MQTT, which limits apply, and what to configure on the broker.
The user-facing overview is in the [README](../README.md#mqtt), every `[mqtt]` key is
described in [`snapdog.example.toml`](../snapdog.example.toml).

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
message_size_limit 262144     # bytes; 0 (the default) means no limit
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
any more. The log says "Delaying MQTT commands" (at most once every 10 seconds).

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
the restart. A section that is not valid TOML (a value of the wrong type, say), or a wrong value elsewhere in the file,
still stops the start as before.

What counts as wrong (the message names the key):

- an unknown key (a misspelt `ca_flie` would otherwise silently fall back to the
  system trust store);
- a password without a username, `password` together with `password_file`;
- a broker address with credentials, a path, an unknown scheme or a WebSocket scheme;
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

At every start SnapDog asks the broker to delete both (an empty retained publish to
each of them; it cannot know which hold anything) and logs that at info level. This is
a migration aid. It writes outside the base topic, which a strict ACL refuses; most
brokers drop that silently, some (EMQX with `deny_action = disconnect`, AWS IoT)
close the connection, once per start. Set

```toml
[mqtt]
clear_legacy_topics = false
```

once the old messages are gone, or if the ACL refuses them. The default is `true`, so
that an upgrade cleans up by itself.
