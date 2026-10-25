// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Inbound commands: topic and payload to zone or Snapcast command.
//!
//! Zone commands go through the per-zone `ZonePlayer` channel, client commands
//! to the Snapcast command channel. Nothing here blocks: a full channel drops
//! the command, so a stalled zone cannot hold up the MQTT event loop (and with
//! it the keep-alive).
//!
//! Every message passes the same gates, cheapest first:
//!
//! 1. a **retained** message is ignored: a retained command would run again on
//!    every restart, long after anyone meant it;
//! 2. the payload must be small ([`MAX_PAYLOAD_BYTES`]) and UTF-8;
//! 3. the topic must be a command below our base topic;
//! 4. the addressed zone or client is **rate limited**;
//! 5. the payload must be understood exactly (see [`super::parse`]).

use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::mpsc;

use super::limits::RateLimiter;
use super::parse::{
    MAX_PAYLOAD_BYTES, PayloadError, Transport, parse_bool, parse_client_volume, parse_count,
    parse_latency_ms, parse_repeat, parse_seconds, parse_seek_ms, parse_transport,
    parse_zone_volume,
};
use super::topics::{Route, TopicError, Topics};
use crate::player::{ClientAction, SnapcastCmd, ZoneCommand, ZoneCommandSender};
use crate::state;

/// What became of a message that was not rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    /// A command was queued.
    Dispatched,
    /// Retained messages are never executed.
    IgnoredRetained,
    /// Too many commands for this zone or client (or in total).
    RateLimited,
    /// Below the base topic, but not one of our command topics.
    NotACommand,
    /// The client exists but is not connected to Snapcast, so there is nobody to tell.
    ClientOffline,
}

/// Why a message was refused. Never carries the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    Topic(TopicError),
    PayloadTooLarge,
    PayloadNotUtf8,
    Payload(PayloadError),
    /// No such zone or client.
    NoSuchTarget,
    /// The command channel is full or closed.
    NotDelivered,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Topic(e) => e.fmt(f),
            Self::PayloadTooLarge => f.write_str("payload is too large"),
            Self::PayloadNotUtf8 => f.write_str("payload is not valid UTF-8"),
            Self::Payload(e) => e.fmt(f),
            Self::NoSuchTarget => f.write_str("no such zone or client"),
            Self::NotDelivered => f.write_str("command channel is full or closed"),
        }
    }
}

impl std::error::Error for Rejected {}

impl From<PayloadError> for Rejected {
    fn from(e: PayloadError) -> Self {
        Self::Payload(e)
    }
}

impl Rejected {
    /// A stable name for log throttling.
    pub const fn category(&self) -> &'static str {
        match self {
            Self::Topic(_) => "rejected: topic",
            Self::PayloadTooLarge | Self::PayloadNotUtf8 | Self::Payload(_) => "rejected: payload",
            Self::NoSuchTarget => "rejected: no such target",
            Self::NotDelivered => "rejected: not delivered",
        }
    }
}

/// Turns incoming MQTT messages into commands.
pub struct CommandHandler {
    topics: Topics,
    zone_commands: HashMap<usize, ZoneCommandSender>,
    state: state::SharedState,
    snap_tx: mpsc::Sender<SnapcastCmd>,
    limiter: RateLimiter,
}

impl CommandHandler {
    /// A handler for the topics below `topics`.
    pub fn new(
        topics: Topics,
        zone_commands: HashMap<usize, ZoneCommandSender>,
        state: state::SharedState,
        snap_tx: mpsc::Sender<SnapcastCmd>,
    ) -> Self {
        Self {
            topics,
            zone_commands,
            state,
            snap_tx,
            limiter: RateLimiter::new(Instant::now()),
        }
    }

    /// Handle one incoming message.
    ///
    /// # Errors
    ///
    /// See [`Rejected`].
    pub async fn handle(
        &mut self,
        topic: &str,
        payload: &[u8],
        retain: bool,
    ) -> Result<Handled, Rejected> {
        self.handle_at(topic, payload, retain, Instant::now()).await
    }

    /// [`Self::handle`] with the clock as a parameter.
    ///
    /// # Errors
    ///
    /// See [`Rejected`].
    pub async fn handle_at(
        &mut self,
        topic: &str,
        payload: &[u8],
        retain: bool,
        now: Instant,
    ) -> Result<Handled, Rejected> {
        if retain {
            return Ok(Handled::IgnoredRetained);
        }
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(Rejected::PayloadTooLarge);
        }
        let payload = std::str::from_utf8(payload).map_err(|_| Rejected::PayloadNotUtf8)?;
        let parsed = match self.topics.parse_command(topic) {
            Ok(parsed) => parsed,
            Err(TopicError::UnknownRoute) => return Ok(Handled::NotACommand),
            Err(e) => return Err(Rejected::Topic(e)),
        };
        if !self.limiter.allow(parsed.scope, parsed.index, now) {
            return Ok(Handled::RateLimited);
        }
        self.dispatch(parsed.route, parsed.index, payload).await
    }

    async fn dispatch(
        &self,
        route: Route,
        index: usize,
        payload: &str,
    ) -> Result<Handled, Rejected> {
        match route {
            // Zone commands → routed through ZonePlayer
            Route::ZoneVolume => {
                self.send_zone(index, ZoneCommand::SetVolume(parse_zone_volume(payload)?))
            }
            Route::ZoneMute => self.send_zone(index, ZoneCommand::SetMute(parse_bool(payload)?)),
            Route::ZoneControl => self.send_zone(
                index,
                match parse_transport(payload)? {
                    Transport::Play => ZoneCommand::Play,
                    Transport::Pause => ZoneCommand::Pause,
                    Transport::Stop => ZoneCommand::Stop,
                    Transport::Next => ZoneCommand::Next,
                    Transport::Previous => ZoneCommand::Previous,
                },
            ),
            Route::ZonePlaylist => {
                self.send_zone(index, ZoneCommand::SetPlaylist(parse_count(payload)?, 0))
            }
            Route::ZoneTrack => self.send_zone(index, ZoneCommand::SetTrack(parse_count(payload)?)),
            Route::ZonePosition => {
                self.send_zone(index, ZoneCommand::Seek(parse_seek_ms(payload)?))
            }
            Route::ZoneShuffle => {
                self.send_zone(index, ZoneCommand::SetShuffle(parse_bool(payload)?))
            }
            Route::ZoneRepeat => {
                self.send_zone(index, ZoneCommand::SetRepeat(parse_repeat(payload)?))
            }
            Route::ZonePresence => {
                self.send_zone(index, ZoneCommand::SetPresence(parse_bool(payload)?))
            }
            Route::ZonePresenceEnable => {
                self.send_zone(index, ZoneCommand::SetPresenceEnabled(parse_bool(payload)?))
            }
            Route::ZonePresenceTimeout => {
                self.send_zone(index, ZoneCommand::SetAutoOffDelay(parse_seconds(payload)?))
            }

            // Client commands → Snapcast commands or direct state mutation
            Route::ClientVolume => {
                let volume = parse_client_volume(payload)?;
                self.send_client(index, ClientAction::Volume(volume)).await
            }
            Route::ClientMute => {
                let muted = parse_bool(payload)?;
                self.send_client(index, ClientAction::Mute(muted)).await
            }
            Route::ClientLatency => {
                let latency = parse_latency_ms(payload)?;
                self.send_client(index, ClientAction::Latency(latency))
                    .await
            }
            Route::ClientZone => self.move_client(index, parse_count(payload)?).await,
        }
    }

    /// Queue a command for zone `index`'s player without waiting.
    fn send_zone(&self, index: usize, cmd: ZoneCommand) -> Result<Handled, Rejected> {
        let tx = self
            .zone_commands
            .get(&index)
            .ok_or(Rejected::NoSuchTarget)?;
        tx.try_send(cmd).map_err(|_| Rejected::NotDelivered)?;
        Ok(Handled::Dispatched)
    }

    /// Queue `action` for client `index`.
    async fn send_client(&self, index: usize, action: ClientAction) -> Result<Handled, Rejected> {
        let snap_id = self
            .state
            .read()
            .await
            .clients
            .get(&index)
            .map(|client| client.snapcast_id.clone())
            .ok_or(Rejected::NoSuchTarget)?;
        let Some(client_id) = snap_id else {
            return Ok(Handled::ClientOffline);
        };
        self.snap_tx
            .try_send(SnapcastCmd::Client { client_id, action })
            .map_err(|_| Rejected::NotDelivered)?;
        Ok(Handled::Dispatched)
    }

    /// Move client `index` to zone `zone` and have Snapcast reconcile the groups.
    async fn move_client(&self, index: usize, zone: usize) -> Result<Handled, Rejected> {
        let mut store = self.state.write().await;
        if zone == 0 || !store.zones.contains_key(&zone) {
            return Err(Rejected::NoSuchTarget);
        }
        let client = store
            .clients
            .get_mut(&index)
            .ok_or(Rejected::NoSuchTarget)?;
        client.zone_index = zone;
        store.dirty = true;
        drop(store);
        self.snap_tx
            .try_send(SnapcastCmd::ReconcileZones)
            .map_err(|_| Rejected::NotDelivered)?;
        Ok(Handled::Dispatched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::test_support::{
        snap_channel, test_state_with_client, zone_channels, zone_channels_with,
    };
    use std::time::Duration;

    /// A handler on `base` plus the receivers of its channels.
    fn handler(
        base: &str,
    ) -> (
        CommandHandler,
        mpsc::Receiver<ZoneCommand>,
        mpsc::Receiver<SnapcastCmd>,
        state::SharedState,
    ) {
        let (snap_tx, snap_rx) = snap_channel();
        let (cmds, zone_rx) = zone_channels();
        let state = test_state_with_client();
        (
            CommandHandler::new(Topics::new(base), cmds, state.clone(), snap_tx),
            zone_rx,
            snap_rx,
            state,
        )
    }

    /// Deliver a live (not retained) message.
    async fn send(h: &mut CommandHandler, topic: &str, payload: &str) -> Result<Handled, Rejected> {
        h.handle(topic, payload.as_bytes(), false).await
    }

    #[tokio::test]
    async fn routes_zone_volume() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        let r = send(&mut h, "snapdog/zones/1/volume/set", "75").await;
        assert_eq!(r, Ok(Handled::Dispatched));
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn zone_volume_takes_both_scales() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        for (payload, expected) in [("0.5", 50), ("50", 50), ("1", 100), ("0.01", 1), ("1.5", 1)] {
            send(&mut h, "snapdog/zones/1/volume/set", payload)
                .await
                .unwrap();
            assert!(
                matches!(rx.recv().await, Some(ZoneCommand::SetVolume(v)) if v == expected),
                "{payload}"
            );
        }
    }

    #[tokio::test]
    async fn routes_zone_mute() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        send(&mut h, "snapdog/zones/1/mute/set", "true")
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(true))));
        send(&mut h, "snapdog/zones/1/mute/set", "OFF")
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(false))));
    }

    #[tokio::test]
    async fn on_means_mute_in_any_case() {
        // Home Assistant's switch sends "ON"; it used to be read as "unmute".
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        send(&mut h, "snapdog/zones/1/mute/set", "ON")
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetMute(true))));
    }

    #[tokio::test]
    async fn every_command_family_is_strict_about_booleans() {
        // Zone mute and shuffle were lenient (anything but true/1/on meant off),
        // client mute and presence strict. All of them are strict now.
        let (mut h, mut rx, mut snap_rx, _state) = handler("snapdog/");
        for topic in [
            "snapdog/zones/1/mute/set",
            "snapdog/zones/1/shuffle/set",
            "snapdog/zones/1/presence/set",
            "snapdog/zones/1/presence/enable/set",
            "snapdog/clients/1/mute/set",
        ] {
            for bad in ["", "maybe", "unmute", "2"] {
                assert!(
                    matches!(send(&mut h, topic, bad).await, Err(Rejected::Payload(_))),
                    "{topic} {bad:?}"
                );
            }
            // Understood spellings pass on every one of them. The limiter is not
            // what this test is about, so give each spelling its own time.
            for (i, good) in ["true", "ON", "1", "false", "off", "0"].iter().enumerate() {
                let now = Instant::now() + Duration::from_secs(i as u64 + 1);
                assert_eq!(
                    h.handle_at(topic, good.as_bytes(), false, now).await,
                    Ok(Handled::Dispatched),
                    "{topic} {good}"
                );
            }
            while rx.try_recv().is_ok() {}
            while snap_rx.try_recv().is_ok() {}
        }
    }

    #[tokio::test]
    async fn a_rejected_payload_changes_nothing() {
        let (mut h, mut rx, mut snap_rx, state) = handler("snapdog/");
        for (topic, payload) in [
            ("snapdog/zones/1/volume/set", "NaN"),
            ("snapdog/zones/1/volume/set", "loud"),
            ("snapdog/zones/1/repeat/set", "sideways"),
            ("snapdog/zones/1/control/set", "explode"),
            ("snapdog/zones/1/position/set", "-5"),
            ("snapdog/zones/1/track/set", "-1"),
            ("snapdog/clients/1/latency/set", "99999"),
            ("snapdog/clients/1/volume/set", "0.5"),
            ("snapdog/clients/1/zone/set", "x"),
        ] {
            assert!(
                matches!(
                    send(&mut h, topic, payload).await,
                    Err(Rejected::Payload(_))
                ),
                "{topic} {payload}"
            );
        }
        assert!(rx.try_recv().is_err() && snap_rx.try_recv().is_err());
        assert_eq!(state.read().await.clients[&1].zone_index, 1);
    }

    #[tokio::test]
    async fn routes_zone_control_commands() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        for (payload, expected) in [
            ("play", ZoneCommand::Play),
            ("pause", ZoneCommand::Pause),
            ("STOP", ZoneCommand::Stop),
            ("next\n", ZoneCommand::Next),
            ("previous", ZoneCommand::Previous),
        ] {
            send(&mut h, "snapdog/zones/1/control/set", payload)
                .await
                .unwrap();
            let cmd = rx.recv().await.unwrap();
            assert_eq!(
                std::mem::discriminant(&cmd),
                std::mem::discriminant(&expected)
            );
        }
    }

    #[tokio::test]
    async fn routes_zone_playlist_and_track() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        send(&mut h, "snapdog/zones/1/playlist/set", "3")
            .await
            .unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetPlaylist(3, 0))
        ));
        send(&mut h, "snapdog/zones/1/track/set", "7")
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetTrack(7))));
    }

    #[tokio::test]
    async fn routes_zone_seek_on_both_topics() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        for topic in [
            "snapdog/zones/1/track/position/set",
            "snapdog/zones/1/position/set",
        ] {
            send(&mut h, topic, "30").await.unwrap();
            assert!(matches!(rx.recv().await, Some(ZoneCommand::Seek(30000))));
        }
    }

    #[tokio::test]
    async fn routes_zone_repeat_and_presence() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        send(&mut h, "snapdog/zones/1/repeat/set", "all")
            .await
            .unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetRepeat(snapdog_common::RepeatMode::Playlist))
        ));
        send(&mut h, "snapdog/zones/1/presence/timeout/set", "90")
            .await
            .unwrap();
        assert!(matches!(
            rx.recv().await,
            Some(ZoneCommand::SetAutoOffDelay(90))
        ));
    }

    #[tokio::test]
    async fn routes_client_volume_mute_and_latency() {
        let (mut h, _rx, mut snap_rx, _state) = handler("snapdog/");
        send(&mut h, "snapdog/clients/1/volume/set", "80")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client { client_id, action: ClientAction::Volume(80) }) if client_id == "snap-1"
        ));
        send(&mut h, "snapdog/clients/1/mute/set", "true")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client { client_id, action: ClientAction::Mute(true) }) if client_id == "snap-1"
        ));
        send(&mut h, "snapdog/clients/1/latency/set", "-20")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Latency(-20),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn clamps_client_volume() {
        let (mut h, _rx, mut snap_rx, _state) = handler("snapdog/");
        send(&mut h, "snapdog/clients/1/volume/set", "200")
            .await
            .unwrap();
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::Client {
                action: ClientAction::Volume(100),
                ..
            })
        ));
    }

    #[tokio::test]
    async fn routes_client_zone_change() {
        let (mut h, _rx, mut snap_rx, state) = handler("snapdog/");
        send(&mut h, "snapdog/clients/1/zone/set", "2")
            .await
            .unwrap();
        assert_eq!(state.read().await.clients[&1].zone_index, 2);
        assert!(matches!(
            snap_rx.recv().await,
            Some(SnapcastCmd::ReconcileZones)
        ));
    }

    #[tokio::test]
    async fn client_zone_change_to_an_unknown_zone_or_client_is_refused() {
        let (mut h, _rx, mut snap_rx, state) = handler("snapdog/");
        for (topic, payload) in [
            ("snapdog/clients/1/zone/set", "99"),
            ("snapdog/clients/1/zone/set", "0"),
            ("snapdog/clients/9/zone/set", "2"),
        ] {
            assert_eq!(
                send(&mut h, topic, payload).await,
                Err(Rejected::NoSuchTarget),
                "{topic} {payload}"
            );
        }
        assert_eq!(state.read().await.clients[&1].zone_index, 1);
        assert!(snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_command_for_an_unknown_target_is_an_error_not_a_silent_drop() {
        let (mut h, _rx, _snap, _state) = handler("snapdog/");
        assert_eq!(
            send(&mut h, "snapdog/zones/2/volume/set", "10").await,
            Err(Rejected::NoSuchTarget),
            "zone 2 has no player in this fixture"
        );
        assert_eq!(
            send(&mut h, "snapdog/clients/5/volume/set", "10").await,
            Err(Rejected::NoSuchTarget)
        );
    }

    #[tokio::test]
    async fn a_client_without_snapcast_id_is_offline_not_an_error() {
        let (mut h, _rx, mut snap_rx, state) = handler("snapdog/");
        state.write().await.clients.get_mut(&1).unwrap().snapcast_id = None;
        assert_eq!(
            send(&mut h, "snapdog/clients/1/volume/set", "10").await,
            Ok(Handled::ClientOffline)
        );
        assert!(snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ignores_unknown_topic_below_the_base() {
        let (mut h, mut rx, mut snap_rx, _state) = handler("snapdog/");
        assert_eq!(
            send(&mut h, "snapdog/unknown/topic", "x").await,
            Ok(Handled::NotACommand)
        );
        assert!(rx.try_recv().is_err() && snap_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn rejects_topic_outside_the_base() {
        let (mut h, _rx, _snap, _state) = handler("snapdog/");
        for topic in ["other/zones/1/volume/set", "snapdogs/zones/1/volume/set"] {
            assert_eq!(
                send(&mut h, topic, "50").await,
                Err(Rejected::Topic(TopicError::OutsideBase)),
                "{topic}"
            );
        }
    }

    // ── Gates ─────────────────────────────────────────────────

    #[tokio::test]
    async fn retained_commands_are_never_executed() {
        // A retained message on a `/set` topic is replayed by the broker on every
        // subscribe, i.e. on every restart and every reconnect.
        let (mut h, mut rx, mut snap_rx, _state) = handler("snapdog/");
        for topic in [
            "snapdog/zones/1/volume/set",
            "snapdog/zones/1/control/set",
            "snapdog/clients/1/mute/set",
        ] {
            assert_eq!(
                h.handle(topic, b"1", true).await,
                Ok(Handled::IgnoredRetained),
                "{topic}"
            );
        }
        // Not even a rejected payload or a foreign topic is looked at.
        assert_eq!(
            h.handle("snapdog/zones/1/volume/set", b"\xff", true).await,
            Ok(Handled::IgnoredRetained)
        );
        assert!(rx.try_recv().is_err() && snap_rx.try_recv().is_err());
        // The same message, live, is executed.
        assert_eq!(
            h.handle("snapdog/zones/1/control/set", b"play", false)
                .await,
            Ok(Handled::Dispatched)
        );
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::Play)));
    }

    #[tokio::test]
    async fn oversized_and_non_utf8_payloads_are_refused() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        let topic = "snapdog/zones/1/volume/set";
        let big = vec![b'5'; MAX_PAYLOAD_BYTES + 1];
        assert_eq!(
            h.handle(topic, &big, false).await,
            Err(Rejected::PayloadTooLarge)
        );
        let mut at_limit = vec![b' '; MAX_PAYLOAD_BYTES - 2];
        at_limit.extend_from_slice(b"50");
        assert_eq!(
            h.handle(topic, &at_limit, false).await,
            Ok(Handled::Dispatched),
            "exactly at the limit still works"
        );
        assert_eq!(
            h.handle(topic, b"5\xff0", false).await,
            Err(Rejected::PayloadNotUtf8)
        );
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::SetVolume(50))));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_flood_on_one_zone_is_cut_off_and_other_targets_are_unaffected() {
        let (mut h, mut rx, _snap, _state) = handler("snapdog/");
        let t0 = Instant::now();
        let mut dispatched = 0;
        let mut limited = 0;
        for _ in 0..1000 {
            match h
                .handle_at("snapdog/zones/1/volume/set", b"50", false, t0)
                .await
            {
                // The zone channel holds 16: past that the command is dropped, not queued.
                Ok(Handled::Dispatched) | Err(Rejected::NotDelivered) => dispatched += 1,
                Ok(Handled::RateLimited) => limited += 1,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(dispatched, 30, "one burst gets through the limiter");
        assert_eq!(limited, 970);
        let mut queued = 0;
        while rx.try_recv().is_ok() {
            queued += 1;
        }
        assert_eq!(queued, 16, "the zone channel is full, not flooded");

        // Later the budget has refilled, and a client is not the zone.
        assert_eq!(
            h.handle_at(
                "snapdog/zones/1/volume/set",
                b"50",
                false,
                t0 + Duration::from_secs(1)
            )
            .await,
            Ok(Handled::Dispatched)
        );
        assert_eq!(
            h.handle_at("snapdog/clients/1/mute/set", b"on", false, t0)
                .await,
            Ok(Handled::Dispatched)
        );
    }

    #[tokio::test]
    async fn a_full_zone_channel_drops_the_command_instead_of_blocking() {
        let (snap_tx, _snap_rx) = snap_channel();
        let (cmds, mut rx) = zone_channels_with(1);
        let mut h = CommandHandler::new(
            Topics::new("snapdog/"),
            cmds,
            test_state_with_client(),
            snap_tx,
        );
        send(&mut h, "snapdog/zones/1/volume/set", "10")
            .await
            .unwrap();
        // The channel holds one command and nobody receives: this must return, not hang.
        let second = tokio::time::timeout(
            Duration::from_secs(1),
            send(&mut h, "snapdog/zones/1/volume/set", "20"),
        )
        .await
        .expect("handle must not block on a full channel");
        assert_eq!(second, Err(Rejected::NotDelivered));
        assert!(matches!(rx.try_recv(), Ok(ZoneCommand::SetVolume(10))));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn rejection_messages_never_contain_a_payload() {
        for rejected in [
            Rejected::PayloadTooLarge,
            Rejected::PayloadNotUtf8,
            Rejected::Payload(PayloadError("volume is not a number")),
            Rejected::NoSuchTarget,
            Rejected::NotDelivered,
            Rejected::Topic(TopicError::InvalidIndex),
        ] {
            assert_ne!(rejected.to_string(), "");
            assert_ne!(rejected.category(), "");
        }
    }

    // ── Regression: command topics under the real, normalised base topic ──
    //
    // `config` appends the trailing slash ("snapdog" becomes "snapdog/"). The bridge
    // once re-derived the base by hand when parsing, so every inbound command was
    // dropped in production while the tests (which used "snapdog" without slash)
    // passed. These tests go through the config loader and take the topics from the
    // function that builds the subscriptions.

    /// The `[mqtt]` section as the config loader resolves it.
    fn mqtt_config(base_topic: Option<&str>) -> crate::config::MqttConfig {
        let base_line = base_topic.map_or(String::new(), |b| format!("base_topic = \"{b}\""));
        let toml = format!(
            r#"
            [mqtt]
            broker = "localhost:1883"
            {base_line}

            [[zone]]
            name = "Z1"
            [[zone]]
            name = "Z2"
            [[client]]
            name = "C1"
            mac = "00:00:00:00:00:01"
            zone = "Z1"
            "#
        );
        crate::config::load_raw(toml::from_str(&toml).unwrap())
            .unwrap()
            .mqtt
            .unwrap()
    }

    /// A payload every command accepts.
    fn valid_payload(tail: &str) -> &'static str {
        match tail {
            "volume/set" => "50",
            "mute/set" | "shuffle/set" | "presence/set" | "presence/enable/set" => "true",
            "control/set" => "play",
            "repeat/set" => "all",
            "position/set" | "track/position/set" => "30",
            "track/set" | "zone/set" => "2",
            "playlist/set" => "1",
            "presence/timeout/set" => "60",
            "latency/set" => "20",
            other => panic!("no sample payload for {other}: extend valid_payload"),
        }
    }

    #[tokio::test]
    async fn command_on_default_normalised_base_is_not_dropped() {
        // The exact topic from the bug report, with the base the config really produces.
        let mqtt = mqtt_config(None);
        assert_eq!(mqtt.base_topic, "snapdog/");
        let (mut h, mut rx, _snap, _state) = handler(&mqtt.base_topic);
        send(&mut h, "snapdog/zones/1/volume/set", "75")
            .await
            .unwrap();
        assert!(matches!(rx.recv().await, Some(ZoneCommand::SetVolume(75))));
    }

    #[tokio::test]
    async fn every_subscribed_topic_is_handled_under_the_real_config_base() {
        for (configured, effective) in [
            (None, "snapdog/"),
            (Some("snapdog"), "snapdog/"),
            (Some("snapdog/"), "snapdog/"),
            (Some("home/audio"), "home/audio/"),
            (Some("home/audio/"), "home/audio/"),
        ] {
            let mqtt = mqtt_config(configured);
            assert_eq!(mqtt.base_topic, effective, "config normalisation");
            // The same function the bridge subscribes with.
            let filters: Vec<String> = Topics::new(&mqtt.base_topic).command_filters().collect();
            assert_eq!(
                filters.len(),
                16,
                "documented command set, base {effective}"
            );

            for filter in filters {
                assert!(filter.starts_with(effective), "{filter}");
                // A concrete topic that this very subscription delivers.
                let topic = filter.replace('+', "1");
                let tail = topic
                    .strip_prefix(effective)
                    .and_then(|t| t.split_once("/1/"))
                    .map(|(_, tail)| tail)
                    .expect("scope/1/tail");

                let (mut h, mut zone_rx, mut snap_rx, _state) = handler(&mqtt.base_topic);
                send(&mut h, &topic, valid_payload(tail))
                    .await
                    .unwrap_or_else(|e| panic!("{topic} was rejected: {e}"));
                assert!(
                    zone_rx.try_recv().is_ok() || snap_rx.try_recv().is_ok(),
                    "{topic} (subscribed) produced no command"
                );
            }
        }
    }
}
