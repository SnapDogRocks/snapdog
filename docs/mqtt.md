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
