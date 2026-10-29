// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The one place that binds sockets.
//!
//! Every TCP listener and UDP socket of the server is created here, so that each one
//! is named (service, protocol, address, port), logged, registered for the listener
//! table, and handed over bound: a service never binds the same address a second time
//! (issue 343). A clippy rule (`clippy.toml`) forbids binding anywhere else.
//!
//! Services whose socket is created by a dependency (`AirPlay`, Spotify Connect, the `KNX`
//! device server) cannot go through [`bind_tcp`] or [`bind_udp`]. They are described
//! with [`planned`] instead, so that [`check_unique`] and [`unassigned`] still see them.

use std::fmt::{self, Write as _};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::config::{AppConfig, KnxRole};

/// Transport protocol of a listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// A TCP listener.
    Tcp,
    /// A UDP socket.
    Udp,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        })
    }
}

/// One listener, planned or bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listener {
    /// Service that owns the listener, for example `HTTP API`.
    pub service: String,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Address and port. A wildcard address means every interface of that family.
    pub addr: SocketAddr,
    /// What creates the socket, for example `axum` or `knx-rs`.
    pub backend: String,
}

impl fmt::Display for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}/{} via {})",
            self.service, self.addr, self.protocol, self.backend
        )
    }
}

static REGISTRY: Mutex<Vec<Listener>> = Mutex::new(Vec::new());

/// The listeners bound through this module so far, in the order they were bound.
#[must_use]
pub fn bound() -> Vec<Listener> {
    REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn register(listener: Listener) {
    tracing::info!(
        service = %listener.service,
        protocol = %listener.protocol,
        address = %listener.addr,
        backend = %listener.backend,
        "Listening"
    );
    REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(listener);
}

/// Bind a TCP listener.
///
/// The listener is non-blocking, so it can go straight into
/// `tokio::net::TcpListener::from_std`. The address is read back after the bind, so a
/// port of `0` is registered with the port the system chose.
///
/// # Errors
///
/// Returns the I/O error of the bind. [`bind_failure_message`] explains it.
#[allow(
    clippy::disallowed_methods,
    reason = "this module is the one place that binds"
)]
pub fn bind_tcp(
    service: &str,
    backend: &str,
    addr: SocketAddr,
) -> io::Result<std::net::TcpListener> {
    let listener = std::net::TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    register(Listener {
        service: service.to_owned(),
        protocol: Protocol::Tcp,
        addr: listener.local_addr()?,
        backend: backend.to_owned(),
    });
    Ok(listener)
}

/// Bind a UDP socket. See [`bind_tcp`].
///
/// # Errors
///
/// Returns the I/O error of the bind.
#[allow(
    clippy::disallowed_methods,
    reason = "this module is the one place that binds"
)]
pub fn bind_udp(service: &str, backend: &str, addr: SocketAddr) -> io::Result<std::net::UdpSocket> {
    let socket = std::net::UdpSocket::bind(addr)?;
    socket.set_nonblocking(true)?;
    register(Listener {
        service: service.to_owned(),
        protocol: Protocol::Udp,
        addr: socket.local_addr()?,
        backend: backend.to_owned(),
    });
    Ok(socket)
}

/// Whether `addr` can be bound right now: binds it and lets go at once. Nothing is
/// registered. For a check ahead of the real bind, for example by `--check-config`.
///
/// # Errors
///
/// Returns the I/O error of the bind.
#[allow(
    clippy::disallowed_methods,
    reason = "this module is the one place that binds"
)]
pub fn probe_tcp(addr: SocketAddr) -> io::Result<()> {
    std::net::TcpListener::bind(addr).map(drop)
}

/// Best effort: whether `ip` is certainly not assigned to any network interface of this host.
///
/// Found by binding a TCP socket to it on an ephemeral port. Only the operating system's
/// "cannot assign requested address" counts. Any other outcome, including a successful
/// bind and every other error, says nothing, so this never reports a problem that is not one.
#[must_use]
pub fn address_is_not_local(ip: IpAddr) -> bool {
    matches!(
        probe_tcp(SocketAddr::new(ip, 0)),
        Err(e) if e.kind() == io::ErrorKind::AddrNotAvailable
    )
}

/// What a failed bind means, by kind of error.
///
/// A port that is taken is probably a second instance, an address that is not ours is a
/// wrong setting (`setting` names it), and anything else is reported as it is.
#[must_use]
pub fn bind_failure_message(
    service: &str,
    addr: SocketAddr,
    setting: &str,
    error: &io::Error,
) -> String {
    match error.kind() {
        io::ErrorKind::AddrInUse => format!(
            "{service} port {addr} is already in use - is snapdog already running? ({error})"
        ),
        io::ErrorKind::AddrNotAvailable => format!(
            "{service} port {addr} cannot be bound: {} is not an address of this host. \
             Check {setting} ({error})",
            addr.ip()
        ),
        io::ErrorKind::PermissionDenied => format!(
            "{service} port {addr} cannot be bound: permission denied. Ports below 1024 \
             need extra privileges ({error})"
        ),
        _ => format!("{service} port {addr} cannot be bound: {error}"),
    }
}

/// Do two listeners compete for the same port? The same protocol and port, and addresses
/// that can both be reached through one socket: equal, or one is a wildcard that covers the
/// other. `::` covers IPv4 as well (dual stack), `0.0.0.0` covers IPv4 only.
fn overlaps(a: &Listener, b: &Listener) -> bool {
    if a.protocol != b.protocol || a.addr.port() != b.addr.port() || a.addr.port() == 0 {
        return false;
    }
    let covers = |wide: IpAddr, other: IpAddr| {
        wide == other || (wide.is_unspecified() && (wide.is_ipv6() || other.is_ipv4()))
    };
    covers(a.addr.ip(), b.addr.ip()) || covers(b.addr.ip(), a.addr.ip())
}

/// Reject two listeners on the same port and protocol with overlapping addresses.
///
/// # Errors
///
/// Returns an error naming both services.
pub fn check_unique(listeners: &[Listener]) -> Result<()> {
    for (i, a) in listeners.iter().enumerate() {
        for b in &listeners[i + 1..] {
            if overlaps(a, b) {
                bail!(
                    "{} and {} would listen on the same {} port {}. Change one of the ports \
                     or give them different bind addresses.",
                    a,
                    b,
                    a.protocol,
                    a.addr.port()
                );
            }
        }
    }
    Ok(())
}

/// The listeners whose address is certainly not an address of this host.
/// Wildcards are always fine.
#[must_use]
pub fn unassigned(listeners: &[Listener]) -> Vec<&Listener> {
    listeners
        .iter()
        .filter(|l| !l.addr.ip().is_unspecified() && address_is_not_local(l.addr.ip()))
        .collect()
}

/// Port of the first `AirPlay` zone minus one: zone `n` listens on `AIRPLAY_BASE_PORT + n`.
pub const AIRPLAY_BASE_PORT: u16 = 7000;

/// The UDP port of the KNXnet/IP device server.
pub const KNX_PORT: u16 = 3671;

/// Port of the Spotify Connect Zeroconf HTTP server for a zone: zone indices are 1-based,
/// so the first zone gets `base` itself.
///
/// # Errors
///
/// Returns an error if `base` plus the zone number does not fit into a port.
pub fn spotify_zone_port(base: u16, zone_index: usize) -> Result<u16> {
    u16::try_from(zone_index.saturating_sub(1))
        .ok()
        .and_then(|n| base.checked_add(n))
        .ok_or_else(|| anyhow::anyhow!("Spotify port {base} + zone {zone_index} out of range"))
}

/// What the configuration says the server will open.
///
/// HTTP and Snapcast streaming are bound by this crate through [`bind_tcp`]; the others
/// are created by their dependencies and are listed so that conflicts between all of them
/// are found before anything starts.
///
/// An HTTP bind that is not an IP address (a host name) is left out, because it is only
/// resolved when the socket is bound.
#[must_use]
pub fn planned(config: &AppConfig) -> Vec<Listener> {
    let wildcard = IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
    let mut out = Vec::new();

    for bind in config.http.bind_addresses() {
        if let Ok(ip) = bind.parse::<IpAddr>() {
            out.push(Listener {
                service: "HTTP API".into(),
                protocol: Protocol::Tcp,
                addr: SocketAddr::new(ip, config.http.port),
                backend: "axum".into(),
            });
        }
    }

    out.push(Listener {
        service: "Snapcast streaming".into(),
        protocol: Protocol::Tcp,
        addr: config.snapcast.streaming_socket_addr(),
        backend: if cfg!(feature = "snapcast-embedded") {
            "snapcast-server".into()
        } else {
            "snapserver".into()
        },
    });

    let airplay_addrs = config
        .airplay
        .bind
        .clone()
        .filter(|addrs| !addrs.is_empty())
        .unwrap_or_else(|| vec![wildcard]);
    for zone in &config.zones {
        let Some(port) = u16::try_from(zone.index)
            .ok()
            .and_then(|n| AIRPLAY_BASE_PORT.checked_add(n))
        else {
            continue;
        };
        for ip in &airplay_addrs {
            out.push(Listener {
                service: format!("AirPlay zone {}", zone.index),
                protocol: Protocol::Tcp,
                addr: SocketAddr::new(*ip, port),
                backend: "shairplay".into(),
            });
        }
    }

    #[cfg(feature = "spotify")]
    if let Some(spotify) = &config.spotify
        && spotify.port != 0
    {
        for zone in &config.zones {
            if let Ok(port) = spotify_zone_port(spotify.port, zone.index) {
                out.push(Listener {
                    service: format!("Spotify Connect zone {}", zone.index),
                    protocol: Protocol::Tcp,
                    addr: SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), port),
                    backend: "librespot-discovery".into(),
                });
            }
        }
    }

    if config
        .knx
        .as_ref()
        .is_some_and(|knx| knx.role == KnxRole::Device)
    {
        out.push(Listener {
            service: "KNX device".into(),
            protocol: Protocol::Udp,
            addr: SocketAddr::new(wildcard, KNX_PORT),
            backend: "knx-rs".into(),
        });
    }

    out
}

/// How far a service honours a `bind` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BindSupport {
    /// The service listens on exactly the configured addresses.
    Full,
    /// The setting has an effect, but not on every socket of the service.
    Partial,
    /// The service cannot be bound to an address.
    None,
}

impl fmt::Display for BindSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::None => "none",
        })
    }
}

/// One line of the listener table: a listener, and what an operator needs to know about
/// binding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct Row {
    /// Service that owns the listener.
    pub service: String,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Address, `0.0.0.0` or `::` for every interface of that family.
    pub address: String,
    /// Port. `0` means a random port at every start.
    pub port: u16,
    /// What creates the socket.
    pub backend: String,
    /// Whether a `bind` setting reaches this socket.
    pub bind_support: BindSupport,
    /// What the bind support level means for this service, in one sentence.
    pub note: String,
}

fn support_of(listener: &Listener) -> (BindSupport, &'static str) {
    match listener.service.as_str() {
        "HTTP API" => (BindSupport::Full, "[http].bind, one listener per address"),
        "Snapcast streaming" => (BindSupport::Full, "[snapcast].streaming_bind, one address"),
        s if s.starts_with("AirPlay") && cfg!(feature = "ap2") => {
            (BindSupport::Full, "[airplay].bind")
        }
        s if s.starts_with("AirPlay") => (
            BindSupport::None,
            "this build has no AirPlay 2 (feature ap2): AirPlay listens on all interfaces",
        ),
        s if s.starts_with("Spotify") => (
            BindSupport::None,
            "the discovery server listens on all interfaces; announcements follow zeroconf_backend",
        ),
        "KNX device" => (
            BindSupport::Partial,
            "[knx].bind selects the multicast interface; the UDP socket stays on all interfaces",
        ),
        _ => (BindSupport::None, ""),
    }
}

/// Everything the server listens on or is configured to listen on.
///
/// Each row carries the bind support of its service. Listeners bound through this module are
/// reported as bound (so a random port is the real one); the rest is what the configuration
/// plans.
#[must_use]
pub fn table(config: &AppConfig) -> Vec<Row> {
    let bound = bound();
    let mut rows = Vec::new();
    let mut seen_bound = Vec::new();
    for listener in planned(config) {
        let actual = bound
            .iter()
            .find(|b| b.service == listener.service && b.protocol == listener.protocol);
        if actual.is_some() {
            if seen_bound.contains(&listener.service) {
                continue;
            }
            seen_bound.push(listener.service.clone());
        }
        let shown = actual.unwrap_or(&listener);
        let (bind_support, note) = support_of(&listener);
        rows.push(Row {
            service: shown.service.clone(),
            protocol: shown.protocol,
            address: shown.addr.ip().to_string(),
            port: shown.addr.port(),
            backend: shown.backend.clone(),
            bind_support,
            note: note.to_owned(),
        });
    }
    if config
        .http
        .bind_addresses()
        .iter()
        .any(|b| b.parse::<IpAddr>().is_err())
    {
        rows.retain(|r| r.service != "HTTP API");
        for listener in bound.iter().filter(|b| b.service == "HTTP API") {
            let (bind_support, note) = support_of(listener);
            rows.insert(
                0,
                Row {
                    service: listener.service.clone(),
                    protocol: listener.protocol,
                    address: listener.addr.ip().to_string(),
                    port: listener.addr.port(),
                    backend: listener.backend.clone(),
                    bind_support,
                    note: note.to_owned(),
                },
            );
        }
    }
    rows
}

/// The table as the start-up log prints it: one line per listener, aligned in columns.
#[must_use]
pub fn format_table(rows: &[Row]) -> String {
    let width = |f: fn(&Row) -> usize| rows.iter().map(f).max().unwrap_or(0);
    let (service, address, backend) = (
        width(|r| r.service.len()),
        width(|r| r.address.len()),
        width(|r| r.backend.len()),
    );
    let mut out = String::new();
    for row in rows {
        let _ = write!(
            out,
            "\n  {:service$}  {}  {:>address$}:{:<5}  {:backend$}  bind: {}",
            row.service, row.protocol, row.address, row.port, row.backend, row.bind_support,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp(service: &str, addr: &str) -> Listener {
        Listener {
            service: service.into(),
            protocol: Protocol::Tcp,
            addr: addr.parse().unwrap(),
            backend: "test".into(),
        }
    }

    fn udp(service: &str, addr: &str) -> Listener {
        Listener {
            protocol: Protocol::Udp,
            ..tcp(service, addr)
        }
    }

    #[test]
    fn distinct_ports_and_protocols_are_fine() {
        let listeners = [
            tcp("a", "0.0.0.0:5555"),
            tcp("b", "0.0.0.0:1704"),
            udp("c", "0.0.0.0:5555"),
            tcp("d", "127.0.0.1:0"),
            tcp("e", "127.0.0.1:0"),
        ];
        check_unique(&listeners).expect("no conflict");
    }

    #[test]
    fn the_same_address_and_port_is_a_conflict_that_names_both_services() {
        let error = check_unique(&[
            tcp("HTTP API", "10.0.0.1:7001"),
            tcp("AirPlay", "10.0.0.1:7001"),
        ])
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("HTTP API") && error.contains("AirPlay"),
            "{error}"
        );
        assert!(error.contains("7001"), "{error}");
    }

    #[test]
    fn a_wildcard_covers_the_addresses_of_its_family() {
        // 0.0.0.0 covers every IPv4 address, not IPv6.
        assert!(check_unique(&[tcp("a", "0.0.0.0:80"), tcp("b", "10.0.0.1:80")]).is_err());
        assert!(check_unique(&[tcp("a", "0.0.0.0:80"), tcp("b", "[::1]:80")]).is_ok());
        // :: is dual stack: it covers IPv4 as well.
        assert!(check_unique(&[tcp("a", "[::]:80"), tcp("b", "10.0.0.1:80")]).is_err());
        assert!(check_unique(&[tcp("a", "[::]:80"), tcp("b", "0.0.0.0:80")]).is_err());
        // Different specific addresses on the same port are fine.
        assert!(check_unique(&[tcp("a", "10.0.0.1:80"), tcp("b", "10.0.0.2:80")]).is_ok());
    }

    #[test]
    fn another_protocol_on_the_same_port_is_not_a_conflict() {
        assert!(check_unique(&[tcp("a", "0.0.0.0:3671"), udp("b", "0.0.0.0:3671")]).is_ok());
    }

    #[test]
    fn a_documentation_address_is_reported_as_unassigned_and_loopback_is_not() {
        let listeners = [
            tcp("here", "127.0.0.1:1"),
            tcp("everywhere", "0.0.0.0:2"),
            tcp("elsewhere", "192.0.2.1:3"),
        ];
        let unassigned = unassigned(&listeners);
        assert_eq!(unassigned.len(), 1, "{unassigned:?}");
        assert_eq!(unassigned[0].service, "elsewhere");
    }

    #[test]
    fn binding_registers_the_real_port_and_a_second_bind_fails() {
        let first = bind_tcp("test service", "std", "127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = first.local_addr().unwrap();
        assert_ne!(addr.port(), 0);
        assert!(
            bound().iter().any(|l| l.service == "test service"
                && l.addr == addr
                && l.protocol == Protocol::Tcp)
        );

        let error = bind_tcp("test service", "std", addr).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        let message = bind_failure_message("Test", addr, "[test].bind", &error);
        assert!(
            message.contains("already in use") && message.contains("already running"),
            "{message}"
        );

        let socket = bind_udp("test udp", "std", "127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(bound().iter().any(|l| l.service == "test udp"
            && l.addr == socket.local_addr().unwrap()
            && l.protocol == Protocol::Udp));
    }

    #[test]
    fn failures_are_explained_by_their_kind() {
        use io::{Error, ErrorKind};
        let addr: SocketAddr = "192.0.2.1:1704".parse().unwrap();
        let message = |kind| {
            bind_failure_message(
                "Snapcast streaming",
                addr,
                "[snapcast].streaming_bind",
                &Error::from(kind),
            )
        };

        let in_use = message(ErrorKind::AddrInUse);
        assert!(
            in_use.starts_with("Snapcast streaming port 192.0.2.1:1704 is already in use"),
            "{in_use}"
        );
        assert!(in_use.contains("already running"), "{in_use}");

        let unassigned = message(ErrorKind::AddrNotAvailable);
        assert!(
            unassigned.contains("192.0.2.1 is not an address of this host"),
            "{unassigned}"
        );
        assert!(
            unassigned.contains("[snapcast].streaming_bind"),
            "{unassigned}"
        );
        assert!(!unassigned.contains("already running"), "{unassigned}");

        let denied = message(ErrorKind::PermissionDenied);
        assert!(denied.contains("permission denied"), "{denied}");

        let other = message(ErrorKind::TimedOut);
        assert!(!other.contains("already running"), "{other}");
    }

    fn config(toml: &str) -> AppConfig {
        crate::config::load_str(&format!(
            "{toml}\n[[zone]]\nname = \"A\"\n[[zone]]\nname = \"B\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn the_default_plan_lists_http_snapcast_and_airplay_per_zone() {
        let plan = planned(&config(""));
        let ports = |service: &str| -> Vec<u16> {
            plan.iter()
                .filter(|l| l.service.starts_with(service))
                .map(|l| l.addr.port())
                .collect()
        };
        assert_eq!(ports("HTTP API"), [5555]);
        assert_eq!(ports("Snapcast streaming"), [1704]);
        assert_eq!(ports("AirPlay"), [7001, 7002]);
        assert!(!plan.iter().any(|l| l.service == "KNX device"));
        check_unique(&plan).expect("the defaults do not collide");
    }

    #[test]
    fn the_plan_follows_the_configured_addresses() {
        let plan = planned(&config(
            "[http]\nbind = \"127.0.0.1\"\nport = 8080\n[snapcast]\nstreaming_bind = \"10.0.0.5\"\n[airplay]\nbind = [\"10.0.0.5\", \"10.0.0.6\"]\n[knx]\nrole = \"device\"\nindividual_address = \"1.1.100\"\n",
        ));
        let find = |service: &str| {
            plan.iter()
                .filter(|l| l.service.starts_with(service))
                .map(|l| l.addr.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(find("HTTP API"), ["127.0.0.1:8080"]);
        assert_eq!(find("Snapcast streaming"), ["10.0.0.5:1704"]);
        assert_eq!(find("AirPlay zone 1"), ["10.0.0.5:7001", "10.0.0.6:7001"]);
        assert_eq!(find("KNX device"), ["0.0.0.0:3671"]);
    }

    #[test]
    fn a_port_shared_by_two_services_is_found_in_the_plan() {
        // The HTTP port on AirPlay's first port: a collision nobody sees until the second bind fails.
        let plan = planned(&config("[http]\nport = 7001\n"));
        let error = check_unique(&plan).unwrap_err().to_string();
        assert!(
            error.contains("HTTP API") && error.contains("AirPlay zone 1"),
            "{error}"
        );
    }

    #[test]
    fn the_table_says_how_far_each_service_honours_bind() {
        let rows = table(&config(
            "[knx]\nrole = \"device\"\nindividual_address = \"1.1.100\"\n",
        ));
        let support = |service: &str| {
            rows.iter()
                .find(|r| r.service.starts_with(service))
                .unwrap_or_else(|| panic!("{service} is missing from {rows:?}"))
                .bind_support
        };
        assert_eq!(support("HTTP API"), BindSupport::Full);
        assert_eq!(support("Snapcast streaming"), BindSupport::Full);
        assert_eq!(support("KNX device"), BindSupport::Partial);
        assert_eq!(
            support("AirPlay zone 1"),
            if cfg!(feature = "ap2") {
                BindSupport::Full
            } else {
                BindSupport::None
            }
        );
        let knx = rows.iter().find(|r| r.service == "KNX device").unwrap();
        assert_eq!((knx.protocol, knx.port), (Protocol::Udp, 3671));
        assert!(knx.note.contains("stays on all interfaces"), "{}", knx.note);
    }

    #[cfg(feature = "spotify")]
    #[test]
    fn spotify_is_in_the_table_with_no_bind_support_and_its_zone_ports() {
        let rows = table(&config("[spotify]\nname = \"Home\"\n"));
        let ports: Vec<_> = rows
            .iter()
            .filter(|r| r.service.starts_with("Spotify"))
            .map(|r| (r.port, r.bind_support))
            .collect();
        assert_eq!(
            ports,
            [(7100, BindSupport::None), (7101, BindSupport::None)]
        );
        // port = 0 is a random port, which no firewall rule can name: it is not planned.
        let rows = table(&config("[spotify]\nport = 0\n"));
        assert!(!rows.iter().any(|r| r.service.starts_with("Spotify")));
    }

    #[cfg(feature = "spotify")]
    #[test]
    fn spotify_and_http_on_the_same_port_are_a_conflict() {
        let plan = planned(&config("[http]\nport = 7100\n[spotify]\nname = \"Home\"\n"));
        let error = check_unique(&plan).unwrap_err().to_string();
        assert!(
            error.contains("HTTP API") && error.contains("Spotify Connect zone 1"),
            "{error}"
        );
    }

    #[test]
    fn spotify_zone_ports_count_up_and_reject_overflow() {
        assert_eq!(spotify_zone_port(7002, 1).unwrap(), 7002);
        assert_eq!(spotify_zone_port(7002, 3).unwrap(), 7004);
        assert!(spotify_zone_port(u16::MAX, 2).is_err());
        assert!(spotify_zone_port(7002, 70_000).is_err());
    }

    #[test]
    fn the_log_table_has_one_aligned_line_per_listener() {
        let rows = table(&config(""));
        let text = format_table(&rows);
        assert_eq!(text.lines().filter(|l| !l.is_empty()).count(), rows.len());
        assert!(
            text.contains("HTTP API") && text.contains("tcp") && text.contains("bind: full"),
            "{text}"
        );
    }

    #[test]
    fn the_defaults_of_every_service_do_not_collide_with_many_zones() {
        // AirPlay takes 7001 and up, Spotify 7100 and up: eleven zones still fit each.
        let zones: String = (1..=11).fold(String::new(), |mut out, n| {
            let _ = writeln!(out, "[[zone]]\nname = \"Z{n}\"");
            out
        });
        let config = crate::config::load_str(&format!(
            "[spotify]\nname = \"Home\"\n[knx]\nrole = \"device\"\nindividual_address = \"1.1.100\"\n{zones}"
        ))
        .unwrap();
        check_unique(&planned(&config)).expect("defaults must not collide");
    }

    #[test]
    fn a_host_name_as_http_bind_is_not_planned() {
        let plan = planned(&config("[http]\nbind = \"localhost\"\n"));
        assert!(!plan.iter().any(|l| l.service == "HTTP API"));
    }
}
