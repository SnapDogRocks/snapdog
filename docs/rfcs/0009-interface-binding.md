---
rfc: LBD-0009
title: Bind by interface name or address, one list, one rule
status: accepted         # draft | accepted | in-progress | done | superseded
version: 0.1.0
created: 2026-10-31
updated: 2026-10-31
target_repo: snapdog
target_branch: main
related: [352, 356, 377]
owners: [metaneutrons]
---

# RFC LBD-0009 — Bind by interface name or address, one list, one rule

Epic: https://github.com/SnapDogRocks/snapdog/issues/377
Decision state: accepted on 2026-10-31 with the four decisions of section 4. It builds on LBD-0008 and takes up its deferred item LBD-M4-A3
(interface names) and the deferred mDNS scoping (LBD-0008, 2.5).

## 1. Outcome and boundaries

**Outcome.** An operator says once, in one list, where SnapDog is reachable. Each entry is an
interface name (`eth0`) or an IP address (`192.168.1.10`). The same list decides where the
services listen and where SnapDog announces itself. At start the log shows what each entry
turned into, and where a dependency cannot follow the setting, SnapDog says so.

**Baseline (`main` at 4742276, read from code and dependency sources, 2026-10-31).**

- `bind` is a list of IP addresses at `[network]`, `[http]`, `[airplay]`, `[knx]`, and one address
  at `[snapcast].streaming_bind`. Interface names are rejected (LBD-0008, 2.6).
- Sockets are created in `net::listener` (HTTP, Snapcast streaming) or by dependencies
  (AirPlay, Spotify, KNX).
- mDNS (`mdns-sd`) announces on every interface of the host. `mdns-sd` 0.21 can limit this:
  `ServiceDaemon::enable_interface` and `disable_interface` take `IfKind::Name`, `IfKind::Addr`,
  `IfKind::IPv4` and others (`service_daemon.rs`, `IfKind`).
- `shairplay` 0.10 binds the AirPlay 2 clock sockets 319 and 320 to `[::]` regardless of
  `[airplay].bind` (`net/ptp.rs`, `spawn_ptp_sink`). The README matrix calls AirPlay `full`
  for builds with `ap2`; for the UDP side that is too generous (see M4).
- `if-addrs` 0.15 is already in the dependency tree through `mdns-sd`.

**Non-goals.** A firewall replacement, a new `[interfaces]` section, following address changes
at run time, patching dependencies in this repository.

## 2. Design

### 2.1 One rule

Every `bind` entry is an address if it parses as an IP address, otherwise an interface name.
The keys stay as they are, so every existing file keeps working.

```toml
[network]
bind = ["eth0"]                  # default for all services: this interface

[http]
bind = ["eth0", "nebula1"]       # this service only, same syntax

[snapcast]
streaming_bind = "eth0"          # one address is needed, so the interface must have exactly one IPv4 address
```

The resolution order of LBD-0008 is unchanged: the service's own key, then `[network].bind`,
then the service's default.

### 2.2 What a name means

A name stands for the addresses the interface has when SnapDog starts:

- every IPv4 address, and every IPv6 address except link-local (`fe80::/10`, which needs a scope
  id and is only reachable on the link);
- services that listen on IPv4 only use the IPv4 ones.

**AirPlay is the open case for link-local.** `shairplay` treats a link-local source address of a
connection as a special case (`bind_addr_for` in `raop/handlers_ap1.rs` binds `::` then), which
suggests that senders may address the receiver by its link-local address. If so, a list of global
addresses would not reach the AirPlay 2 clock sockets (see 4). That is an inference from the
code and has not been measured.

An interface with several addresses gives several listeners. That is the same behaviour as a
list of those addresses, so there is no new case to learn.

**Services that take one address** (Snapcast streaming, KNX). A name is accepted if it resolves to
exactly one IPv4 address. Otherwise start stops with the list of addresses found and the line to
write, for example `streaming_bind = "192.168.1.10"`. From `[network].bind` the rule of
LBD-0008 stays: more than one result means the service keeps its default and the log says why.

**Errors.** An unknown name stops the start and prints the interfaces that exist with their
addresses. An interface without a usable address is an error of the same kind. Nothing falls back
silently.

### 2.3 Loopback

`bind = "loopback"` means `127.0.0.1` and `::1`. It saves the operator from knowing that the
device is `lo` on Linux and `lo0` on macOS, and it is the common case behind a reverse proxy.
The word is reserved; an interface that carries this name is reached by its addresses.

### 2.4 Announcements follow the list

When the `bind` of the HTTP API (its own or from `[network].bind`) is set, the mDNS announcement
of SnapDog (`_snapdog._tcp`, and `_snapcast._tcp` if `[mdns].advertise_snapcast` is on) is
limited to the interfaces that carry those addresses: `disable_interface(IfKind::All)`, then
`enable_interface(IfKind::Addr)` for each address. The announcement follows where the announced
services listen, so it stays unlimited if one of them has no `bind`, a wildcard address, or a
host name that is not an address (`localhost` counts as `127.0.0.1`). `IfKind::Addr` selects the
interface that carries the address, not the single address.

The AirPlay announcement is not part of this. It is made inside `shairplay` 0.10
(`net/mdns.rs`) by a private `ServiceDaemon` on Linux and by Bonjour (`astro-dnssd`) on macOS,
and the caller can reach neither. Limiting it needs a new setting in `shairplay-rust`
(metaneutrons/shairplay-rust#78). Until then AirPlay announces on every interface, which the
matrix shows as `partial` on Linux and `none` on macOS (M4).

This closes the gap that a service listens on `eth0` but announces on `nebula1`. It does not reach
Spotify Connect: `libmdns` announces on the default-route interface only, `avahi` and `dns-sd` on
all (LBD-0008, 2.5). That stays `none` and stays documented.

### 2.5 Why names are resolved to addresses and not bound to the device

`SO_BINDTODEVICE` on Linux, `IP_BOUND_IF` on macOS and `IP_UNICAST_IF` on Windows (named from
memory, not checked here) would tie a socket to the interface itself. This RFC does not use them:

- Most sockets are created by dependencies (AirPlay, Spotify, KNX), which offer an address or
  nothing. A device option would work for two services and would be a second mechanism next to
  the first.
- Privileges and semantics differ per operating system, which makes the setting harder to explain.
- For a listener the address gives the effect that matters: nobody reaches the service through an
  address the operator did not name.

**Limit that stays.** Linux accepts a packet for a local address even if it arrives on another
interface (weak host model), so an address bind is not strict interface isolation. `bind` is not a
security boundary; the firewall is, as in LBD-0008.

### 2.6 Visibility

- The listener table shows how each address came about: `192.168.1.10 (eth0)`. The API field
  `via` in `GET /api/v1/system/listeners` carries the entry from the configuration.
- `--check-config` first prints the interface inventory of the host (name, addresses), then for
  each service the entries and what they resolved to. This is the answer to "what do I write".
- An unknown interface name is an error at load, so also in `--check-config`. The message lists
  the interfaces of the host. A warning in `--check-config` would need a load without name
  resolution, which this RFC does not add. Checking a configuration on another host than the
  one that runs SnapDog is therefore only reliable for addresses, not for names.

### 2.7 Recipes (go into README and `snapdog.example.toml`)

| Situation | Setting |
|---|---|
| One network card | nothing |
| Audio and API on the LAN card only, host has a second card | `[network] bind = ["eth0"]` |
| API behind a reverse proxy on the same host | `[http] bind = "loopback"` |
| API on the LAN and on the Nebula overlay, audio on the LAN only | `[network] bind = ["eth0"]`, `[http] bind = ["eth0", "nebula1"]` |
| VLAN interface | `[network] bind = ["enp1s0.16"]` |
| Docker with a bridge | publish the port on one address (`-p 192.168.10.5:1704:1704`); names inside the container are the container's |
| Docker with host networking | as on the host |

### 2.8 Alternatives considered

- **A new key `interfaces = [...]` next to `bind`.** Two keys for one idea, with an order of
  precedence to explain. Rejected in favour of one list with one rule.
- **A named profile (`[network.lan]`, `bind = "lan"`).** Clear for large installations, too much
  for the typical one. Can be added later on top of this without change.
- **Following address changes at run time.** Would need re-binding listeners and re-announcing;
  not worth the complexity here, see the open questions.

### 2.9 Compatibility

Existing files keep their meaning: an entry that is an IP address behaves as before. A name that
used to be a load error now resolves. Setting `bind` now also limits the mDNS announcement of
SnapDog itself (2.4), not that of AirPlay. This is a visible change for installations that set it; the changelog says so.

## 3. Delivery and acceptance

### M1: Resolver and configuration

Execution: https://github.com/SnapDogRocks/snapdog/issues/378

- LBD-M1-A1: `bind` entries resolve to addresses as in 2.2, tested against a fake interface table
  (several addresses, IPv6 link-local, no address, unknown name).
- LBD-M1-A2: the single-address rule and the error texts of 2.2 are tested, including the line
  to write.
- LBD-M1-A3: `loopback` resolves to both loopback addresses on Linux and macOS.

### M2: Announcements

Execution: https://github.com/SnapDogRocks/snapdog/issues/379
Dependencies: M1

- LBD-M2-A1: with `bind` set, the mDNS daemon of SnapDog announces only on the resolved
  interfaces, measured with a second host or `dns-sd`/`avahi-browse`; without `bind` nothing
  changes. The AirPlay announcement is excluded until metaneutrons/shairplay-rust#78 is done.

### M3: Visibility and documentation

Execution: https://github.com/SnapDogRocks/snapdog/issues/380
Dependencies: M1

- LBD-M3-A1: listener table, API field `via` and `--check-config` output as in 2.6; the OpenAPI
  contract test covers the field.
- LBD-M3-A2: README, `snapdog.example.toml` and the matrix carry the recipes of 2.7.

### M4: Matrix corrections

Execution: https://github.com/SnapDogRocks/snapdog/issues/381

- LBD-M4-A1: AirPlay 2 is `partial` in the matrix (clock sockets 319 and 320 stay on `[::]`),
  with the firewall hint; the support level is checked against the `shairplay` version that is
  in use at the time.
  The AirPlay mDNS announcement is listed as `partial` on Linux and `none` on macOS until
  metaneutrons/shairplay-rust#78 is done.

## 4. Decisions, risks and open questions

Decided at acceptance (2026-10-31): `bind` takes names and addresses in one list and there is no
second key (2.1); `loopback` stays (2.3); no wait for interfaces at boot; `bind` also limits the
mDNS announcement of SnapDog (2.4). Corrected at M2 (2026-11-01): the AirPlay announcement
cannot be limited from SnapDog, see 2.4.

- **Name and address mismatch after start** (DHCP renewal, VPN that comes up later). Start
  stops with the error of 2.2. Decided: no wait for interfaces at boot; the unit is ordered
  after `network-online.target`. A switch such as `[network].wait = "30s"` can be added later
  if VPN interfaces turn out to come up too late in practice.
- **IPv6 link-local** is excluded from names. Open: whether a user ever needs it listed.
- **AirPlay 2 clock sockets (319/320).** `shairplay` 0.10 binds them to `[::]` in
  `spawn_ptp_sink()`, whatever `[airplay].bind` says. They are the reason AirPlay is `partial`
  in the matrix (M4). Making the sink honour `BindConfig` is a change in `shairplay-rust` and
  does not block this RFC. It needs a measurement first: connect time with a real sender, once
  with the sink on `[::]` and once on the configured addresses only, including a case where
  the sender uses the link-local address. If the connect time stays at about 0.5 s, the likely shape is to
  bind the configured addresses plus the link-local address of the same interface (with scope
  id). If it falls back to several seconds, `[::]` stays and the matrix keeps `partial`. Until
  the measurement exists, the matrix and the firewall hint stand.
- **Keyword `loopback`** is a small convenience that reserves a word. Decided: keep.
- **Scope of the mDNS change** (2.4) is a behaviour change for existing `bind` users. Decided: accepted, with a changelog entry.
- **Verification cost.** Unit tests need no network. M2 needs a second host. No other cost
  estimate is given because none has been measured.
