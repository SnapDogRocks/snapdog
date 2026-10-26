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
