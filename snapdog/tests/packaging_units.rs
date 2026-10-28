// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Contents of the shipped systemd unit. The `.deb`, the AUR package and the release
//! archive all install `snapdog/systemd/snapdog.service`, so what it grants is what
//! every package user gets.
//!
//! The server runs as an unprivileged `DynamicUser`. `AirPlay` 2 accepts the sender's
//! PTP clock on UDP 319 and 320 (shairplay, `net::ptp`), which are privileged ports.
//! Without `CAP_NET_BIND_SERVICE` the bind fails with `Permission denied`, and every
//! `AirPlay` 2 connect starts slower. Verified on Debian 12 (systemd 252) with the
//! release binary: no listener without the capability, UDP 319 and 320 with it.

/// The capability the unit has to grant, and the only one.
const PTP_CAPABILITY: &str = "CAP_NET_BIND_SERVICE";

fn workspace_file(relative: &str) -> String {
    let path = format!("{}/../{relative}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

/// `key=value` lines of the `[Service]` section, comments and blank lines dropped.
fn service_settings(unit: &str) -> Vec<(String, String)> {
    let mut in_service = false;
    let mut settings = Vec::new();
    for line in unit.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_service = line == "[Service]";
            continue;
        }
        if in_service && let Some((key, value)) = line.split_once('=') {
            settings.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    settings
}

/// All values of a key, in file order (systemd accumulates repeated list settings).
fn values<'a>(settings: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    settings
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect()
}

#[test]
fn service_settings_parser_reads_only_the_service_section() {
    let unit = "[Unit]\nAmbientCapabilities=CAP_SYS_ADMIN\n\n[Service]\n# a comment\n\
                AmbientCapabilities = CAP_NET_BIND_SERVICE \n; another\nRestart=on-failure\n\
                [Install]\nWantedBy=multi-user.target\n";
    let settings = service_settings(unit);
    assert_eq!(
        settings,
        vec![
            ("AmbientCapabilities".to_owned(), PTP_CAPABILITY.to_owned()),
            ("Restart".to_owned(), "on-failure".to_owned()),
        ]
    );
}

#[test]
fn server_unit_grants_exactly_the_ptp_port_capability() {
    let unit = workspace_file("snapdog/systemd/snapdog.service");
    let settings = service_settings(&unit);
    assert_eq!(
        values(&settings, "AmbientCapabilities"),
        [PTP_CAPABILITY],
        "the server needs CAP_NET_BIND_SERVICE as an ambient capability to bind UDP 319 and \
         320 for the AirPlay 2 PTP sink, and nothing else"
    );
    assert_eq!(
        values(&settings, "CapabilityBoundingSet"),
        [PTP_CAPABILITY],
        "the bounding set must not allow more than the ambient set"
    );
}

/// Ambient capabilities are only a small step because the rest of the sandbox stays
/// shut: a dynamic non-root user that cannot gain privileges through `exec`.
#[test]
fn server_unit_keeps_the_unprivileged_sandbox() {
    let unit = workspace_file("snapdog/systemd/snapdog.service");
    let settings = service_settings(&unit);
    for (key, want) in [
        ("DynamicUser", "yes"),
        ("NoNewPrivileges", "yes"),
        ("ProtectSystem", "strict"),
    ] {
        assert_eq!(
            values(&settings, key),
            [want],
            "snapdog.service must keep {key}={want}"
        );
    }
    assert!(
        values(&settings, "User").is_empty(),
        "the unit must not run as a fixed user"
    );
}

/// The client opens no privileged port and gets no capability.
#[test]
fn client_unit_stays_capability_free() {
    let unit = workspace_file("snapdog-client/systemd/snapdog-client.service");
    let settings = service_settings(&unit);
    let none: [&str; 0] = [];
    assert_eq!(values(&settings, "AmbientCapabilities"), none);
    assert_eq!(values(&settings, "CapabilityBoundingSet"), none);
}

/// Packages other than the `.deb` and the AUR package (own units, containers) have to
/// be told how to give the process the same ability.
#[test]
fn upgrade_notes_explain_how_to_bind_the_ptp_ports() {
    let notes = workspace_file("docs/upgrading.md");
    for needle in [
        "AmbientCapabilities=CAP_NET_BIND_SERVICE",
        "net.ipv4.ip_unprivileged_port_start",
        "cap_add",
        "not allowed in host network namespace",
    ] {
        assert!(
            notes.contains(needle),
            "docs/upgrading.md must explain `{needle}`"
        );
    }
}
