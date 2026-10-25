# KNX bridge: how values find their group address

`snapdog/src/knx/` runs in one of two modes. They share the publisher and the listener
and differ in who decides which group address a value belongs to.

## Client mode (`role = "client"`)

snapdog is a KNXnet/IP client of a gateway or router. The only link between a group
object and a group address is the configuration: `[[zone]].knx`, `[[client]].knx` and the
global keys in `[knx]`.

- Incoming telegrams are matched by group address against maps built from the
  configuration. The keys are the 3-level form (`1/7/255`), so `1/2047` and `01/7/255`
  match as well. One address maps to one action per kind of object; a later entry
  replaces an earlier one. Only `L_Data.ind` counts, the gateway's `L_Data.con` for our
  own telegrams is ignored.
- Status values are written to the configured address of their object.
- The URL is `udp://`, `tunnel://` or `router://` plus `host:port`. `udp://` selects
  routing for a multicast address and tunnelling otherwise. A host name is resolved on
  every connection attempt (IPv4 preferred), an IPv6 literal is written in brackets.
  KNXnet/IP over TCP and KNX IP Secure are not supported.

## Device mode (`role = "device"`, `--knx-device`)

snapdog is a KNXnet/IP server (UDP 3671, multicast group 224.0.23.12, IPv4) with a
device stack that ETS programs through a tunnel. ETS fills the address table and the
association table of that stack.

- The device stack tells the bridge which group **object** the bus updated, by ASAP
  number (zone `z`, object `i`: `(z-1)*33 + i + 1`, clients and the five device-level
  objects follow, see `group_objects.rs`). `object_command` in `mod.rs` maps the ASAP to
  the command. The configuration is not consulted, so an ETS-derived configuration
  (`--knx-device` without `--config`, no group addresses in it) works.
- Status values are written to the object. The stack sends to the first association of
  an object; further associations are listening addresses. When an object has a command
  and a status address in TOML (`presence_enable`/`presence_enable_status`, `all_mute`/
  `all_mute_status`) the status address is installed first.
- A device ETS never programmed gets its tables from the TOML addresses, installed
  through the same load state machine ETS uses, so that it also sends.
- Startup and every download log the number of group addresses, associations and
  zone/client/global objects with an association, and warn when there is none.
- ETS-assigned individual addresses are kept in `knx-memory.bin` together with the
  programming (see below).

## Status traffic

`StatusSink` (`sink.rs`) passes a status value on only when it differs from the one sent
last for that group object. The cyclic `server_online` heartbeat is the exception.
Playback progress is rounded to whole percent and sent at most every two seconds, a
position that moves back (the next track) goes out at once. Everything is sent again
after a reconnect, after the device server restarted, after a download changed the
associations and after the notification channel lagged.

## Supervision

- **Client:** `run_client` (`supervisor.rs`) connects, runs the bridge, and connects again
  when the connection ends, until `KnxBridge::shutdown`. An unreachable gateway at
  startup is not an error. Delays grow 1 s, 2 s, 4 s, ... to 60 s with jitter (50 to
  100 % of the nominal delay) and start over once a connection lasted 30 s. The first three
  failures and every tenth log at warn level, the others at debug, the recovery at info.
  The loss is detected when the tunnel reports closed, when a write fails at the socket or
  is rejected by the gateway, or when two writes in a row stay unacknowledged. An idle
  tunnel to a gateway that vanished is noticed by the tunnel heartbeat only, after about
  two to three minutes.
- **Device:** a failing bind (port taken, no multicast route yet) or a stopped server is
  retried the same way. The device state stays in memory and the programming-mode requests
  are still answered while there is no server.

## Persistence

`knx-memory.bin` in the working directory (`/var/lib/snapdog` in the container and the
systemd unit) holds what ETS downloaded: tables, application state, parameter memory and,
since format version 2, the individual address with the configured address in force when
it was saved. Only the segments in use are saved (the device stack appends the segments
of every download, so the live memory grows by about 12 KB each time until the restart
that ends the download), laid out as a first download would leave them, and the parameters
are read from the segment the last download wrote. The saved address is used while the configured one is unchanged; changing
`individual_address` or `--knx-address` takes effect. Programming mode is not persisted.
