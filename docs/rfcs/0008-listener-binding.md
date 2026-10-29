---
rfc: LBD-0008
title: One bind model for every network listener
status: in-progress      # draft | accepted | in-progress | done | superseded
version: 0.3.0           # v0.1: initial proposal; v0.2: baseline corrected after #346 and the dependencies; v0.3: delivery, see section 5
created: 2026-10-28
updated: 2026-10-29
target_repo: snapdog
target_branch: main
related: [343, 346]
owners: [metaneutrons]
---

# RFC LBD-0008 — One bind model for every network listener

Epic: https://github.com/SnapDogRocks/snapdog/issues/352
Decision state: accepted on 2026-10-28 by the owner's instruction to carry out the plan; amended in v0.2.0 and v0.3.0, see section 5

> This document owns requirements and acceptance criteria (`LBD-M*-A*`); the
> epic and milestone issues own execution state and evidence. Do not keep a
> second checklist in issues. It was first drafted as LBD-0007; the number was
> taken by the Home Assistant RFC in #349 and this RFC is LBD-0008.

## 1. Outcome and boundaries

**Outcome.** An operator can state once, in one syntax, on which addresses and
ports SnapDog listens, can read the effective result at startup and through the
API, and can derive firewall rules from it. Where a dependency cannot honour a
setting, SnapDog says so instead of ignoring it.

**Baseline (read from the code of `main` after #346 and from the sources of the
dependencies, 2026-10-28).**

| Listener | Port (default) | How the address is chosen today |
|---|---|---|
| HTTP API / WebUI (+TLS) | 5555 | `http.bind`, one string, default `::` (dual stack, falls back to `0.0.0.0` where IPv6 is missing), also `--bind`. With TLS the pre-bound listener is dropped and the address is bound a second time by `axum_server::bind_rustls` (#343) |
| Snapcast streaming (embedded) | 1704 | `[snapcast].streaming_bind`, one address, default all IPv4 interfaces (added by #346). `check_streaming_port` probes the port, then the server binds it again |
| Snapcast streaming (managed `snapserver` process) | 1704 | the same key, written to the generated `bind_to_address` |
| Snapcast JSON-RPC (managed `snapserver` process) | 1705 | not configurable, snapserver's default |
| AirPlay (RAOP) | 7001 + zone | `airplay.bind`, list of IPs, handed to shairplay |
| Spotify Connect (zeroconf HTTP) | 7002 + zone − 1 (#360), random before | not configurable; `librespot-discovery` 0.8 binds `0.0.0.0` and `::` itself (`server.rs`) |
| KNX device server | 3671/udp | `DeviceServer::start(Ipv4Addr::UNSPECIFIED)`. In `knx-rs-ip` 0.11 that argument only selects the multicast interface: the socket is always bound to `0.0.0.0:3671`. `DeviceServer::start_at(SocketAddr)` binds a given address but joins no multicast group |
| MQTT | none | client only, no listener |
| mDNS announcements | 5353/udp | `mdns-sd` for SnapDog and AirPlay; `libmdns` (librespot) announces on the default-route interface only; the `avahi` and `dns-sd` backends announce on all interfaces |

Five different mechanisms, two hard-coded wildcards, no overview of what is
open. A pre-flight bind exists for HTTP and Snapcast (fail-fast, good) but is
not shared. Issue 343 (TLS listener bound twice) is a symptom of listeners being
created in several places.

**In scope.** Listener configuration (`[network]` and per-service `bind`),
a single listener module, start-up validation, the listener table (log and API),
a port and bind matrix in the documentation, mDNS interface scoping where the
library allows it.

**Non-goals.** Replacing the firewall (bind is not a security boundary),
authentication, IPv6 transition policy, per-interface bind by name in the first
milestones (see M4), changing the wire protocols, patching librespot or knx-rs
inside this repository (an upstream change is tracked separately, see 2.5).

## 2. Design and decisions

### 2.1 Configuration

```toml
[network]
bind = ["192.168.1.10"]       # default for every listener; list of IP addresses; unset = each service keeps its own default

[http]
bind = ["192.168.1.10"]       # overrides [network].bind for this service
[airplay]
bind = ["192.168.1.10"]
[snapcast]
streaming_bind = "192.168.1.10"   # exists since #346; one address (see 2.2)
[spotify]
bind = ["192.168.1.10"]       # rejected: the backend cannot honour it (2.5)
[knx]
bind = ["192.168.1.10"]       # one address, selects the interface of the device server (partial, 2.5)
```

Resolution order: the service's own key, then `[network].bind`, then the
service's built-in default (HTTP `::`, every other service the IPv4 wildcard).
There is no global default address, so a configuration without `[network]`
behaves exactly as before. The type is a list of IP addresses.

**Single-address services.** A backend that takes one address (the embedded
Snapcast streaming listener, the KNX device server) accepts one address. A
service key with several addresses is a start-up error naming the service. When
the address comes from `[network].bind` and that list has several entries, the
service uses its own default and logs which `[network].bind` it did not apply
and why; it never silently takes the first entry.

`http.bind` keeps accepting a single string, as it always did; a list is the new
form. `[snapcast].streaming_bind` keeps its name and its single address: it was
added by #346, is unreleased and documented, and the name says which of the
Snapcast listeners it moves.

### 2.2 One listener module

A module `net::listener` is the only place that creates sockets
(`bind_tcp`, `bind_udp`). Each call names the service, protocol, addresses and
port. The module validates, binds, logs and registers the result, and hands the
bound listener to the service, so a service never binds again (the cause of
#343). A clippy `disallowed_methods` rule forbids direct `TcpListener::bind`
and `UdpSocket::bind` elsewhere in the shipped code, so the rule cannot erode.
Tests may bind ephemeral ports directly.

### 2.3 Validation at start

- Every configured address exists on a local interface (wildcards excepted).
- Duplicate `address:port` across services is rejected before any bind, with
  both service names in the error. A wildcard overlaps every address of the same
  family and port.
- A bind failure names the service, address, port and the likely cause; no silent
  fallback to another port or address. The one existing fallback (`::` to
  `0.0.0.0` where IPv6 is not available) stays and is logged.

### 2.4 Visibility

At start SnapDog logs one table (service, protocol, address, port, backend) and
exposes it as `GET /api/v1/system/listeners`. The default ports are documented in
one matrix next to the configuration reference.

### 2.5 Honesty about dependency limits

Each service is marked `full`, `partial` or `none` for bind support in the
matrix. If `bind` is set for a service whose backend cannot honour it, start-up
fails with an explanation, or logs a warning where a partial effect is useful.
A setting must never silently do nothing.

- **Spotify: `none`.** The zeroconf HTTP server binds the wildcard inside
  `librespot-discovery` 0.8 (`none`). `[spotify].bind` is rejected at start.
  `libmdns` announces on the default-route interface only (`partial`), whereas
  the `avahi` and `dns-sd` backends announce on all interfaces. Real support
  needs an upstream change (listener injection or a bind address); that is an
  open question, not an assumption.
- **KNX: `partial`.** `DeviceServer::start(addr)` selects the interface for the
  multicast group; the UDP socket stays on `0.0.0.0:3671`. `[knx].bind` therefore
  names the interface of the device server and the matrix says that 3671/udp
  stays open on all interfaces. `start_at` would bind the address precisely but
  gives up routing multicast, which is a behaviour change and is not chosen here.
- **mDNS:** `mdns-sd` can be limited to interfaces; `libmdns` cannot. Limiting `mdns-sd`
  is not part of the first delivery: the matrix lists mDNS as `none`.
- **Upstream question for `librespot-discovery` (LBD-M4-A2), answered.** Not feasible
  inside this repository. In 0.8 `DiscoveryServer::new` binds `0.0.0.0:<port>` and
  `[::]:<port>` itself, and the builder offers `port()` but neither an address nor a
  listener. A change needs a `bind_address` on the builder or a pre-bound listener to be
  accepted. The other Zeroconf backends (`avahi`, `dns-sd`) change only the announcement.
  This is the proposal; it has not been filed upstream, because that is an external action
  left to the owner.

### 2.6 Alternatives considered

- **Only per-service `bind`, no `[network]`.** Simpler, but every new service
  repeats the setting and the common case (one NIC for everything) needs many
  lines.
- **Bind by interface name from the start** (`interfaces = ["enp1s0.16"]`).
  Robust against DHCP changes, but needs re-resolution when addresses change and
  `SO_BINDTODEVICE` needs privileges. Deferred to M4 as an additive option.
- **Leave binding to the container runtime.** Does not apply with
  `network_mode: host`, which is how the Docks deployment runs.
- **Rename `streaming_bind` to `snapcast.bind`** (v0.1 of this RFC). Rejected in
  v0.2: the key exists, is documented and is more exact, since Snapcast has more
  than one listener.

### 2.7 Compatibility

Defaults keep today's behaviour. The behaviour change of v0.1 (the Snapcast
streaming listener following its bind) was delivered by #346 and is listed
there. What this plan adds is opt-in per installation, with one exception: a
`[knx]` or `[spotify]` configuration that sets `bind` is new and therefore cannot
break an existing file.

## 3. Delivery and acceptance

### M1: Listener module and validation

Execution: https://github.com/SnapDogRocks/snapdog/issues/353
Dependencies: none

- LBD-M1-A1: `net::listener` is the only shipped code that binds sockets; a
  clippy rule fails the build on a direct bind elsewhere (counter-probe: a test
  branch with a direct bind fails CI).
- LBD-M1-A2: duplicate `address:port` and non-existent bind addresses are
  rejected before any socket is opened; unit tests cover both and a valid
  configuration (positive and negative probes).
- LBD-M1-A3: HTTP (including the TLS path, closing issue 343) and the Snapcast
  streaming listener go through the module; with TLS enabled the server starts
  and listens once.

### M2: `[network]` and per-service `bind`

Execution: https://github.com/SnapDogRocks/snapdog/issues/354
Dependencies: M1

- LBD-M2-A1: resolution order service, `[network]`, built-in default is
  implemented and tested for HTTP, Snapcast streaming, AirPlay and KNX, including
  the single-address rule of 2.1.
- LBD-M2-A2: `http.bind` as a plain string still works and a list works; a bad
  value fails the load with the key named.
- LBD-M2-A3: `[snapcast].streaming_bind = "127.0.0.1"` leaves 1704 closed on
  other interfaces, measured with `ss` or `lsof` against a running instance
  (delivered by #346, re-measured on the build that contains M1 and M2).
- LBD-M2-A4: the example configuration and the configuration reference describe
  every key.

### M3: Visibility and documentation

Execution: https://github.com/SnapDogRocks/snapdog/issues/355
Dependencies: M1

- LBD-M3-A1: the start-up log contains the listener table; the endpoint returns
  the same data and appears in the OpenAPI contract test.
- LBD-M3-A2: the port and bind matrix covers every service with its default
  port, protocol, bind support level and the firewall rule it implies.
- LBD-M3-A3: README and configuration reference are current before the release
  that contains M1 to M3 (README check from the release standard).

### M4: Dependency-limited services and optional interface names

Execution: https://github.com/SnapDogRocks/snapdog/issues/356
Dependencies: M2

- LBD-M4-A1: Spotify, KNX and mDNS behave as in 2.5: a `bind` the backend cannot
  honour fails or warns as specified, with tests for each backend.
- LBD-M4-A2: the upstream question for librespot-discovery is answered in the
  plan (change proposed upstream, or recorded as not feasible with the reason).
- LBD-M4-A3 (optional, only if accepted at the M2 review; not delivered in v0.3.0, waiting for the owner's decision): `interfaces = [...]`
  resolves names to addresses at start; a name that does not exist is a start-up
  error.

### Completion

The initiative is complete when M1 to M3 meet their criteria and M4 is either
accepted or explicitly waived in a reviewed plan change. Publication is a normal
release of the repository and is not part of this plan.

## 4. Migration, risks and verification cost

- **Rollout.** Defaults preserve each service's current bind, so adoption is
  opt-in per installation.
- **Rollback.** Remove the new keys; `http.bind` as a string and
  `streaming_bind` keep working.
- **Risks.** A stricter validation rejects configurations that used to start
  (for example a bind address that exists only after a VPN comes up). Mitigation:
  clear error text; whether an opt-out is needed is decided at the M2 review.
  Docker deployments with a published port are unaffected, but the matrix must
  say that bind inside a bridged container is not the exposure boundary.
- **Verification cost.** Unit and integration tests run on Linux x86-64; no
  native-platform matrix is needed except one macOS run for the listener module
  because address semantics differ (`::` dual stack). No cost estimate is given
  because none has been measured.
- **Open questions.** Upstream path for Spotify bind (M4); whether interface
  names are wanted at all (decided at the M2 review); behaviour on an address that
  appears later.

## 5. Decision changes

- **v0.2.0 (2026-10-28), made while carrying the plan out; the owner can veto
  each of them.**
  1. Renumbered from LBD-0007 to LBD-0008 (taken by the Home Assistant RFC in
     #349). Criterion identifiers do not carry the number and stay.
  2. Baseline corrected: HTTP defaults to `::`, not `0.0.0.0`; the Snapcast
     streaming bind exists since #346 as `[snapcast].streaming_bind`, one
     address; there is no global `0.0.0.0` default, so `[network].bind` is unset
     by default and each service keeps its own.
  3. The key name `streaming_bind` stays instead of `snapcast.bind`; services
     whose backend takes one address accept one (2.1).
  4. KNX is `partial`, not a plain bind: the library socket stays on the
     wildcard (2.5). `[knx].bind` selects the interface.
  5. Spotify `bind` is rejected rather than accepted and ignored (2.5, M4-A1).
  6. LBD-M2-A2 no longer asks for a deprecation warning: `http.bind` as a string
     is not deprecated, and a list is added next to it. LBD-M2-A3 is already
     met by #346 and is re-measured on the new build.
- **v0.3.0 (2026-10-29), made while delivering; the owner can veto each of them.**
  1. The four milestones are delivered in one pull request, at the owner's request,
     instead of one pull request per milestone. The criteria and their evidence stay
     per milestone in the milestone issues.
  2. The default Spotify port is 7100, not 7002 (#360). `AirPlay` zone *n* listens on
     7000 + *n*, so Spotify zone 1 on 7002 collided with `AirPlay` zone 2 on every
     installation with two or more zones. The start-up check of this plan found it.
  3. Spotify Connect is part of the plan, so a port shared with another service is found
     before anything starts.
  4. LBD-M4-A3 (interface names) is not delivered; it needs the owner's decision at
     the M2 review. Limiting `mdns-sd` to interfaces is not delivered either; both are
     recorded in the matrix and in section 2.5.
  5. LBD-M4-A2 is answered in section 2.5; no upstream issue has been filed.
