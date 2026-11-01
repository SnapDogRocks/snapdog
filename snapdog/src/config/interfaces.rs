// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Interface names in `bind` settings, see RFC LBD-0009.
//!
//! A `bind` entry is an IP address if it parses as one, the keyword `loopback`, or the name
//! of a network interface. A name stands for the addresses the interface has when the
//! configuration is read: every IPv4 address and every IPv6 address except link-local
//! (`fe80::/10`, which needs a scope id and is only reachable on the link).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// One network interface and the addresses it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    /// Name as the operating system gives it, for example `eth0` or `en0`.
    pub name: String,
    /// Every address of the interface, link-local ones included.
    pub addrs: Vec<IpAddr>,
}

/// The interfaces of a host, grouped by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterfaceTable(Vec<Interface>);

impl InterfaceTable {
    /// A table from a list of interfaces, for tests and for hosts that are not this one.
    #[must_use]
    pub const fn new(interfaces: Vec<Interface>) -> Self {
        Self(interfaces)
    }

    /// The interfaces of this host. An interface the system cannot list is simply missing.
    #[must_use]
    pub fn system() -> Self {
        let mut table: Vec<Interface> = Vec::new();
        for entry in if_addrs::get_if_addrs().unwrap_or_default() {
            let ip = entry.ip();
            match table.iter_mut().find(|i| i.name == entry.name) {
                Some(existing) => existing.addrs.push(ip),
                None => table.push(Interface {
                    name: entry.name,
                    addrs: vec![ip],
                }),
            }
        }
        Self(table)
    }

    /// The table as one line for an error message: `eth0 (192.168.1.10), lo (127.0.0.1, ::1)`.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.0.is_empty() {
            return "none found".to_owned();
        }
        self.0
            .iter()
            .map(|i| {
                let addrs: Vec<String> = i.addrs.iter().map(ToString::to_string).collect();
                format!("{} ({})", i.name, addrs.join(", "))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The table as lines for `--check-config`: one per interface, with its addresses.
    #[must_use]
    pub fn inventory(&self) -> String {
        if self.0.is_empty() {
            return "  none found".to_owned();
        }
        self.0
            .iter()
            .map(|i| {
                let addrs: Vec<String> = i.addrs.iter().map(ToString::to_string).collect();
                format!("  {}: {}", i.name, addrs.join(", "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn find(&self, name: &str) -> Option<&Interface> {
        self.0.iter().find(|i| i.name == name)
    }
}

const fn is_link_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// The addresses of an interface that a name stands for: IPv4, and IPv6 without link-local.
fn usable(interface: &Interface) -> Vec<IpAddr> {
    interface
        .addrs
        .iter()
        .copied()
        .filter(|ip| match ip {
            IpAddr::V4(_) => true,
            IpAddr::V6(v6) => !is_link_local_v6(*v6),
        })
        .collect()
}

fn lookup<'a>(entry: &str, table: &'a InterfaceTable) -> Result<&'a Interface, String> {
    table.find(entry).ok_or_else(|| {
        format!(
            "\"{entry}\" is neither an IP address nor an interface of this host. \
             Interfaces found: {}",
            table.describe()
        )
    })
}

/// What one `bind` entry stands for: every address a listener of a service that takes a
/// list is to use.
///
/// # Errors
///
/// Returns a message that names the entry and the interfaces that exist when the name is
/// unknown, or the interface has no usable address.
pub fn resolve(entry: &str, table: &InterfaceTable) -> Result<Vec<IpAddr>, String> {
    let entry = entry.trim();
    if let Ok(ip) = entry.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    if entry == "loopback" {
        return Ok(vec![
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ]);
    }
    let interface = lookup(entry, table)?;
    let addrs = usable(interface);
    if addrs.is_empty() {
        return Err(format!(
            "interface \"{entry}\" has no usable address (IPv6 link-local addresses are not \
             used). Its addresses: {}",
            interface
                .addrs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(addrs)
}

/// What one `bind` entry stands for in a service that listens on exactly one address
/// (Snapcast streaming, the KNX device server).
///
/// An address is itself. `loopback` is `127.0.0.1`. An interface name is accepted if it has
/// exactly one IPv4 address, otherwise the message lists them and says what to write.
///
/// # Errors
///
/// Returns a message as for [`resolve`], or when the interface has no IPv4 address or
/// more than one.
pub fn resolve_single(entry: &str, table: &InterfaceTable) -> Result<IpAddr, String> {
    let entry = entry.trim();
    if let Ok(ip) = entry.parse::<IpAddr>() {
        return Ok(ip);
    }
    if entry == "loopback" {
        return Ok(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let interface = lookup(entry, table)?;
    let v4: Vec<IpAddr> = interface
        .addrs
        .iter()
        .copied()
        .filter(IpAddr::is_ipv4)
        .collect();
    match v4.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!(
            "interface \"{entry}\" has no IPv4 address, and this service listens on exactly one \
             IPv4 address"
        )),
        many => Err(format!(
            "this service listens on exactly one address, but interface \"{entry}\" has {}: {}. \
             Write one of them, for example \"{}\"",
            many.len(),
            many.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            many[0]
        )),
    }
}

/// Whether an entry is an address, `loopback` or the name of an interface of this host. What
/// is none of these is left to the service, which fails on it with the entry in the message.
#[must_use]
pub fn is_address_or_known_name(entry: &str) -> bool {
    !needs_table(entry) || host_table().find(entry.trim()).is_some()
}

/// An entry that is not an address or `loopback` and so needs the interface table.
fn needs_table(entry: &str) -> bool {
    let entry = entry.trim();
    entry != "loopback" && entry.parse::<IpAddr>().is_err()
}

#[cfg(test)]
thread_local! {
    static TEST_TABLE: std::cell::RefCell<Option<InterfaceTable>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    static ORIGINS: std::cell::RefCell<Vec<(IpAddr, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Remember that `entry` (an interface name or `loopback`) stood for `addrs` while the
/// configuration was read, so the listener table can say how an address came about.
fn note_origin(entry: &str, addrs: &[IpAddr]) {
    let entry = entry.trim();
    if entry.parse::<IpAddr>().is_ok() {
        return;
    }
    ORIGINS.with(|o| {
        let mut origins = o.borrow_mut();
        for ip in addrs {
            if !origins.iter().any(|(a, _)| a == ip) {
                origins.push((*ip, entry.to_owned()));
            }
        }
    });
}

/// The names that stood for addresses since the last call, as `(address, entry)`. The
/// configuration loader takes them into [`crate::config::AppConfig::bind_origins`].
#[must_use]
pub fn take_origins() -> Vec<(IpAddr, String)> {
    ORIGINS.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

/// Run `f` with `table` as the interfaces of the host, on this thread only.
#[cfg(test)]
pub fn with_table<T>(table: InterfaceTable, f: impl FnOnce() -> T) -> T {
    TEST_TABLE.with(|t| *t.borrow_mut() = Some(table));
    let result = f();
    TEST_TABLE.with(|t| *t.borrow_mut() = None);
    result
}

fn host_table() -> InterfaceTable {
    #[cfg(test)]
    if let Some(table) = TEST_TABLE.with(|t| t.borrow().clone()) {
        return table;
    }
    InterfaceTable::system()
}

/// [`resolve`] against the interfaces of this host. The table is read only for an entry
/// that is a name.
///
/// # Errors
///
/// As [`resolve`].
pub fn resolve_here(entry: &str) -> Result<Vec<IpAddr>, String> {
    let addrs = if needs_table(entry) {
        resolve(entry, &host_table())?
    } else {
        resolve(entry, &InterfaceTable::default())?
    };
    note_origin(entry, &addrs);
    Ok(addrs)
}

/// [`resolve_single`] against the interfaces of this host.
///
/// # Errors
///
/// As [`resolve_single`].
pub fn resolve_single_here(entry: &str) -> Result<IpAddr, String> {
    let addr = if needs_table(entry) {
        resolve_single(entry, &host_table())?
    } else {
        resolve_single(entry, &InterfaceTable::default())?
    };
    note_origin(entry, &[addr]);
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn table() -> InterfaceTable {
        InterfaceTable::new(vec![
            Interface {
                name: "lo".into(),
                addrs: vec![ip("127.0.0.1"), ip("::1")],
            },
            Interface {
                name: "eth0".into(),
                addrs: vec![ip("192.168.1.10"), ip("fd00::10"), ip("fe80::1")],
            },
            Interface {
                name: "eth1".into(),
                addrs: vec![ip("10.0.0.5"), ip("10.0.0.6")],
            },
            Interface {
                name: "wg0".into(),
                addrs: vec![ip("fe80::2")],
            },
        ])
    }

    #[test]
    fn an_address_is_itself_and_needs_no_table() {
        let empty = InterfaceTable::default();
        assert_eq!(
            resolve("192.168.1.10", &empty),
            Ok(vec![ip("192.168.1.10")])
        );
        assert_eq!(resolve("::", &empty), Ok(vec![ip("::")]));
        assert_eq!(resolve(" 0.0.0.0 ", &empty), Ok(vec![ip("0.0.0.0")]));
    }

    #[test]
    fn loopback_is_both_loopback_addresses() {
        assert_eq!(
            resolve("loopback", &InterfaceTable::default()),
            Ok(vec![ip("127.0.0.1"), ip("::1")])
        );
    }

    #[test]
    fn a_name_is_its_ipv4_and_non_link_local_ipv6_addresses() {
        assert_eq!(
            resolve("eth0", &table()),
            Ok(vec![ip("192.168.1.10"), ip("fd00::10")])
        );
        assert_eq!(
            resolve("eth1", &table()),
            Ok(vec![ip("10.0.0.5"), ip("10.0.0.6")])
        );
        assert_eq!(
            resolve("lo", &table()),
            Ok(vec![ip("127.0.0.1"), ip("::1")])
        );
    }

    #[test]
    fn an_unknown_name_lists_the_interfaces_that_exist() {
        let error = resolve("eht0", &table()).unwrap_err();
        assert!(error.contains("\"eht0\""), "{error}");
        assert!(
            error.contains("eth0 (192.168.1.10, fd00::10, fe80::1)"),
            "{error}"
        );
        assert!(error.contains("lo (127.0.0.1, ::1)"), "{error}");
    }

    #[test]
    fn an_interface_with_only_link_local_addresses_is_an_error() {
        let error = resolve("wg0", &table()).unwrap_err();
        assert!(error.contains("no usable address"), "{error}");
        assert!(error.contains("fe80::2"), "{error}");
    }

    #[test]
    fn a_host_without_interfaces_says_so() {
        let error = resolve("eth0", &InterfaceTable::default()).unwrap_err();
        assert!(error.contains("none found"), "{error}");
    }

    #[test]
    fn a_single_address_service_takes_an_interface_with_one_ipv4_address() {
        assert_eq!(resolve_single("eth0", &table()), Ok(ip("192.168.1.10")));
        assert_eq!(resolve_single("lo", &table()), Ok(ip("127.0.0.1")));
        assert_eq!(
            resolve_single("loopback", &InterfaceTable::default()),
            Ok(ip("127.0.0.1"))
        );
        assert_eq!(
            resolve_single("::1", &InterfaceTable::default()),
            Ok(ip("::1"))
        );
    }

    #[test]
    fn a_single_address_service_says_which_address_to_write() {
        let error = resolve_single("eth1", &table()).unwrap_err();
        assert!(error.contains("exactly one address"), "{error}");
        assert!(error.contains("10.0.0.5, 10.0.0.6"), "{error}");
        assert!(error.contains("for example \"10.0.0.5\""), "{error}");
    }

    #[test]
    fn a_single_address_service_refuses_an_interface_without_ipv4() {
        let error = resolve_single("wg0", &table()).unwrap_err();
        assert!(error.contains("no IPv4 address"), "{error}");
    }

    #[test]
    fn here_reads_the_table_only_for_a_name() {
        // An address resolves without touching the host.
        assert_eq!(resolve_here("127.0.0.1"), Ok(vec![ip("127.0.0.1")]));
        with_table(table(), || {
            assert_eq!(
                resolve_here("eth0"),
                Ok(vec![ip("192.168.1.10"), ip("fd00::10")])
            );
            assert_eq!(resolve_single_here("eth0"), Ok(ip("192.168.1.10")));
            assert!(resolve_here("nope").is_err());
        });
    }
}
