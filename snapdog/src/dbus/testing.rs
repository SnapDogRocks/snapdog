// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Test support for the MPRIS2 interfaces: the Player interface served over a
//! peer-to-peer connection, so no bus daemon is needed.
//!
//! Peer-to-peer connections run over a Unix socket pair, hence the `unix` gate on the
//! module. Daemon policy and name ownership are not covered here, only what the interface
//! does with the calls it gets.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc};
use zbus::{Guid, connection};

use super::player::{PlayerInterface, PlayerState};
use crate::player::ZoneCommand;

pub const PATH: &str = "/org/mpris/MediaPlayer2";
pub const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// A reply that never arrives is the failure these tests exist to catch, so every wait
/// has a deadline.
pub const DEADLINE: Duration = Duration::from_secs(5);

/// Zone the fixture's player belongs to.
pub const ZONE: usize = 1;

/// The Player interface served over a socketpair, and the calling end of it.
pub struct Fixture {
    pub caller: zbus::Connection,
    /// The serving end. Keeps the connection open and hands out the interface reference.
    pub server: zbus::Connection,
    pub state: Arc<Mutex<PlayerState>>,
    pub commands: mpsc::Receiver<ZoneCommand>,
}

pub async fn fixture() -> Fixture {
    let (server_end, caller_end) = UnixStream::pair().expect("socketpair");
    let (cmd_tx, commands) = mpsc::channel(8);
    let state = Arc::new(Mutex::new(PlayerState::default()));
    let player = PlayerInterface::new(
        ZONE,
        "http://localhost:5555".into(),
        Arc::clone(&state),
        cmd_tx,
    );
    let (server, caller) = tokio::try_join!(
        async {
            connection::Builder::unix_stream(server_end)
                .server(Guid::generate())
                .p2p()
                .serve_at(PATH, player)
                .build()
                .await
        },
        connection::Builder::unix_stream(caller_end).p2p().build(),
    )
    .expect("peer-to-peer handshake");
    Fixture {
        caller,
        server,
        state,
        commands,
    }
}

pub async fn call<B>(caller: &zbus::Connection, method: &str, body: &B)
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
{
    tokio::time::timeout(
        DEADLINE,
        caller.call_method(None::<&str>, PATH, Some(PLAYER), method, body),
    )
    .await
    .unwrap_or_else(|_| {
        panic!("{method} got no reply within {DEADLINE:?}: the method most likely panicked")
    })
    .unwrap_or_else(|e| panic!("{method} failed: {e}"));
}

pub async fn next_command(commands: &mut mpsc::Receiver<ZoneCommand>) -> ZoneCommand {
    tokio::time::timeout(DEADLINE, commands.recv())
        .await
        .expect("no command was forwarded")
        .expect("command channel closed")
}

/// `Properties.Get` on the Player interface, with a deadline.
pub async fn get_property(caller: &zbus::Connection, name: &str) -> zbus::OwnedVariant {
    let reply = tokio::time::timeout(
        DEADLINE,
        caller.call_method(None::<&str>, PATH, Some(PROPERTIES), "Get", &(PLAYER, name)),
    )
    .await
    .unwrap_or_else(|_| panic!("Get {name} got no reply within {DEADLINE:?}"))
    .unwrap_or_else(|e| panic!("Get {name} failed: {e}"));
    reply.body().deserialize().expect("variant body")
}

pub async fn get_string(caller: &zbus::Connection, name: &str) -> String {
    String::try_from(get_property(caller, name).await).expect("string property")
}

pub async fn get_bool(caller: &zbus::Connection, name: &str) -> bool {
    bool::try_from(get_property(caller, name).await).expect("bool property")
}

pub async fn get_f64(caller: &zbus::Connection, name: &str) -> f64 {
    f64::try_from(get_property(caller, name).await).expect("double property")
}

pub async fn get_i64(caller: &zbus::Connection, name: &str) -> i64 {
    i64::try_from(get_property(caller, name).await).expect("int64 property")
}

/// `Properties.Set` on the Player interface, with a deadline. The setters run on the
/// runtime thread like the methods do, so a blocking call in one of them shows up as a
/// missing reply.
pub async fn set_property(caller: &zbus::Connection, name: &str, value: zbus::Variant<'_>) {
    tokio::time::timeout(
        DEADLINE,
        caller.call_method(
            None::<&str>,
            PATH,
            Some(PROPERTIES),
            "Set",
            &(PLAYER, name, value),
        ),
    )
    .await
    .unwrap_or_else(|_| {
        panic!("Set {name} got no reply within {DEADLINE:?}: the setter most likely panicked")
    })
    .unwrap_or_else(|e| panic!("Set {name} failed: {e}"));
}
