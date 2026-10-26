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
  replaces an earlier one, and a global address shadows a zone or client entry. The
  start logs a warning for each such address. A zone and a client entry on one address
  both run.
- Only group writes from the bus are commands: `L_Data.ind` with a group value write.
  The gateway's `L_Data.con` for our own telegrams and group value responses (somebody
  else's answer to a read) are ignored.
- Status values are written to the configured address of their object.
- A group read on a status address is answered with the value last sent to it. When
  nothing was sent to that address yet (the first write failed), the current values are
  offered again and the one for that address goes out as the response. Reads on other
  addresses, command addresses included, are not answered.
- The URL is `udp://`, `tunnel://` or `router://` plus `host:port`; the syntax is checked
  when the configuration loads (`--check-config`). `udp://` selects routing for a
  multicast address and tunnelling otherwise. A host name is resolved on every connection
  attempt (IPv4 preferred), an IPv6 literal is written in brackets. KNXnet/IP over TCP and
  KNX IP Secure are not supported.
- There is no limit on zones and clients. State kept per object (what was sent last) is
  keyed by `ObjectKey` (zone or client index plus object kind), not by ASAP number, which
  exists only for 10 zones and 30 clients.

## Device mode (`role = "device"`, `--knx-device`)

snapdog is a KNXnet/IP server (UDP 3671, multicast group 224.0.23.12, IPv4) with a
device stack that ETS programs through a tunnel or a router. ETS fills the address table
and the association table of that stack. The product has 10 zones and 30 clients; TOML
addresses of a zone above 10 or a client above 30 are ignored with a warning.

- The device stack tells the bridge which group **object** the bus updated, by ASAP
  number (zone `z`, object `i`: `(z-1)*33 + i + 1`, clients and the five device-level
  objects follow, see `group_objects.rs`). `object_command` in `mod.rs` maps the ASAP to
  the command. The configuration is not consulted, so an ETS-derived configuration
  (`--knx-device` without `--config`, no group addresses in it) works.
- The object flags are enforced. The group object table of the product (flags and sizes
  from the knxprod, `group_objects::descriptor_table`) is loaded into the stack, so only
  writable objects accept writes and only readable ones answer reads. In front of the
  stack the BAU task drops group value responses (snapdog never reads, a response answers
  somebody else's read), drops all group telegrams while the device is not configured
  (ETS is reloading a table and the address and association tables disagree), and handles
  a write to an address that only status objects send to: the write is not applied and the
  status object sends its actual value again, so the bus does not keep a foreign value.
  An address that a writable object listens to as well is that object's business.
- Status values are written to the object. The stack sends to the first association of
  an object in the association table; further associations are listening addresses. When
  an object has a command and a status address in TOML (`presence_enable`/
  `presence_enable_status`, `all_mute`/`all_mute_status`) the status address is installed
  first. Which association ETS writes first is not verified against a real ETS.
- An object keeps the value it was last given also when it has no address yet, so a read
  after the download is answered with the actual value. The three track strings are
  published empty, and the progress as 0, when there is no track.
- A device ETS never programmed gets its tables from the TOML addresses, installed
  through the same load state machine ETS uses (from an empty memory, after unloading the
  three table objects), so that it also sends.
- Startup and every download log the number of group addresses, associations and
  zone/client/global objects with an association, and warn when there is none.
- A frame counts as programming when it is a point-to-point frame to this device, over a
  tunnel or a router. Programming arms the debounced write of the state; group telegrams
  do not.
- ETS-assigned individual addresses are kept in `knx-memory.bin` together with the
  programming (see below).

## Status traffic

`StatusSink` (`sink.rs`) passes a status value on only when it differs from the one sent
last for that group object. The cyclic `server_online` heartbeat is the exception.
Playback progress is rounded to whole percent and sent at most every two seconds. A
position that moves back (the next track) goes out at once, and a value held back by the
limit is sent when the two seconds are over, so the last position before a pause arrives.
Everything is sent again after a reconnect, after the device server restarted, after a
download changed the associations and after the notification channel lagged.

## Supervision

- **Client:** `run_client` (`supervisor.rs`) connects, runs the bridge, and connects again
  when the connection ends, until `KnxBridge::shutdown`. An unreachable gateway at
  startup is not an error. Delays grow 1 s, 2 s, 4 s, ... to 60 s with jitter (50 to
  100 % of the nominal delay) and start over once a connection lasted 30 s. The first three
  failures and every tenth log at warn level, the others at debug, the recovery at info.
  The loss is detected when the tunnel reports closed, when a write fails at the socket or
  is rejected by the gateway, or when two writes in a row stay unacknowledged.
- **Idle gateway loss (decided, accepted):** an idle tunnel to a gateway that vanished is
  noticed only by the tunnel heartbeat of knx-rs (60 s interval, three misses), after about
  two to three minutes; until then group telegrams do not arrive and nothing is lost that a
  republish after the reconnect does not restore. There is no keepalive write.
- **Device:** a failing bind (port taken, no multicast route yet) or a stopped server is
  retried the same way. The device state stays in memory and the programming-mode requests
  are still answered while there is no server.
- **Shutdown:** `KnxBridge::shutdown` waits for the client loop, and for the device task,
  which writes the state on its way out and first waits for a debounced write that is still
  running.

## Persistence

`knx-memory.bin` in the working directory (`/var/lib/snapdog` in the container and the
systemd unit) holds what ETS downloaded: tables, application state, parameter memory and the
individual address.

Format version 2, all numbers little endian: the magic `SDKM`, the version byte `2`, the
length of the state as `u16`, the state (the device stack's own save format: three table
object records, five program version bytes, the memory length as `u32` and the memory), the
configured individual address as `u16`, the individual address the device had as `u16`, and a
CRC32 over the state and the two addresses. Version 1 (no addresses, the CRC over the state
only) still loads. **An older snapdog cannot read a version 2 file:** it logs that the version is
unsupported and starts the device unprogrammed, and the next write replaces the file, so ETS
has to program the device again. Keep a copy of the file before a downgrade.

- The file is created with mode 0600 (it holds the MQTT, AirPlay and Subsonic passwords, the
  PSK and the API keys), written to a temp file of its own, flushed, renamed over the old file,
  and the directory is flushed. A snapshot is numbered when it is taken; a slower writer with
  an older one does not replace a newer file.
- Only the segments in use are saved (the device stack appends the segments of every download,
  so the live memory grows by about 12 KB each time until the restart that ends the download),
  laid out as a first download would leave them. The stack ignores writes beyond 256 KB, which
  about 20 downloads reach; with `restart_after_ets = false` a restart is needed after that.
- **Change from earlier releases:** the parameters are read from the segment the last download
  wrote. Earlier releases read the start of the memory, which is where the first download put
  them, so a device kept applying the parameters of its first download. After the upgrade a
  device applies the parameters of its last download: check them in ETS.
- The saved individual address is used while the configured one is unchanged; changing
  `individual_address` or `--knx-address` takes effect. A device ETS never programmed saves
  only the address. Programming mode is not persisted.

## Not verified

The ETS download is simulated with the management frames of knx-rs's own test harness over a
loopback tunnel: connectionless services, the application segment allocated first, `Unload`
and `A_Restart` included. Not covered: a connection-oriented ETS session, the real order in
which ETS writes the association table (which address of an object is its sending address),
ETS programming through a router, ETS accepting the compacted segments, and the
`restart_after_ets` re-exec. Transport layer timers of the device stack (retransmissions in a
connection-oriented session) are only driven by incoming events.
