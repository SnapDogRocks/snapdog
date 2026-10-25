// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The MQTT topic scheme: one definition of the base topic and of the command
//! routes.
//!
//! Everything that builds or parses a topic goes through [`Topics`]. The
//! subscriptions are generated from [`COMMAND_ROUTES`], and an incoming topic is
//! matched against the same table. A command that is subscribed to but not
//! routed (or the other way round) is therefore not expressible, and the base
//! topic is normalised in exactly one place ([`normalize_base_topic`]).

use crate::config::normalize_base_topic;

/// Highest zone or client index accepted in a command topic.
///
/// Zone and client indices are 1-based list positions (`[[zone]]`, `[[client]]`).
/// The bound keeps the per-topic bookkeeping of the flood guard finite whatever
/// a broker client publishes.
pub const MAX_INDEX: usize = 1024;

/// The object class a command addresses (the first level below the base topic).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Zone,
    Client,
}

impl Scope {
    /// Topic level that selects this scope (`zones` or `clients`).
    pub const fn segment(self) -> &'static str {
        match self {
            Self::Zone => "zones",
            Self::Client => "clients",
        }
    }
}

/// What a command topic does. One variant per command, shared by the
/// subscription list and the dispatcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    ZoneVolume,
    ZoneMute,
    ZoneControl,
    ZoneShuffle,
    ZoneRepeat,
    ZonePosition,
    ZoneTrack,
    ZonePlaylist,
    ZonePresence,
    ZonePresenceEnable,
    ZonePresenceTimeout,
    ClientVolume,
    ClientMute,
    ClientLatency,
    ClientZone,
}

/// Every command topic: `{base}{scope}/{index}/{tail}`.
///
/// `zones/{index}/position/set` and `zones/{index}/track/position/set` are two
/// spellings of one command.
pub const COMMAND_ROUTES: &[(Scope, &str, Route)] = &[
    (Scope::Zone, "volume/set", Route::ZoneVolume),
    (Scope::Zone, "mute/set", Route::ZoneMute),
    (Scope::Zone, "control/set", Route::ZoneControl),
    (Scope::Zone, "shuffle/set", Route::ZoneShuffle),
    (Scope::Zone, "repeat/set", Route::ZoneRepeat),
    (Scope::Zone, "position/set", Route::ZonePosition),
    (Scope::Zone, "track/set", Route::ZoneTrack),
    (Scope::Zone, "track/position/set", Route::ZonePosition),
    (Scope::Zone, "playlist/set", Route::ZonePlaylist),
    (Scope::Zone, "presence/set", Route::ZonePresence),
    (
        Scope::Zone,
        "presence/enable/set",
        Route::ZonePresenceEnable,
    ),
    (
        Scope::Zone,
        "presence/timeout/set",
        Route::ZonePresenceTimeout,
    ),
    (Scope::Client, "volume/set", Route::ClientVolume),
    (Scope::Client, "mute/set", Route::ClientMute),
    (Scope::Client, "latency/set", Route::ClientLatency),
    (Scope::Client, "zone/set", Route::ClientZone),
];

/// A command topic that matched [`COMMAND_ROUTES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedCommand {
    pub scope: Scope,
    pub route: Route,
    /// 1-based zone or client index.
    pub index: usize,
}

/// Why a topic is not a command for this instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicError {
    /// The topic is not below the base topic.
    OutsideBase,
    /// Below the base topic, but not a command we subscribe to.
    UnknownRoute,
    /// The index level is not a canonical decimal in `1..=MAX_INDEX`.
    InvalidIndex,
}

impl std::fmt::Display for TopicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OutsideBase => "topic is outside the base topic",
            Self::UnknownRoute => "not a command topic",
            Self::InvalidIndex => "invalid zone or client index in topic",
        })
    }
}

impl std::error::Error for TopicError {}

/// Topic builder and parser rooted at one normalised base topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topics {
    /// Always ends with `/`.
    base: String,
}

impl Topics {
    /// Build the scheme for `base_topic` (with or without trailing slash).
    pub fn new(base_topic: &str) -> Self {
        Self {
            base: normalize_base_topic(base_topic),
        }
    }

    /// The normalised base topic (ends with `/`).
    #[cfg(test)]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `{base}{suffix}`.
    pub fn join(&self, suffix: &str) -> String {
        format!("{}{suffix}", self.base)
    }

    /// Availability topic: the last will and the retained `online` live here.
    pub fn availability(&self) -> String {
        self.join("status")
    }

    /// Retained zone state (JSON).
    pub fn zone_state(&self, index: usize) -> String {
        self.join(&format!("zones/{index}/state"))
    }

    /// Retained client state (JSON).
    pub fn client_state(&self, index: usize) -> String {
        self.join(&format!("clients/{index}/state"))
    }

    /// The `{base}/status` topic (double slash) that an earlier release published
    /// `online` to by mistake. Only used to clear the stale retained message.
    pub fn legacy_double_slash_status(&self) -> String {
        format!("{}/status", self.base)
    }

    /// Subscription filters for all commands, in [`COMMAND_ROUTES`] order.
    pub fn command_filters(&self) -> impl Iterator<Item = String> + '_ {
        COMMAND_ROUTES
            .iter()
            .map(|(scope, tail, _)| self.join(&format!("{}/+/{tail}", scope.segment())))
    }

    /// A concrete command topic for `index`, built the way
    /// [`Self::command_filters`] builds the filter (`+` replaced by the index).
    #[cfg(test)]
    pub fn command_topic(&self, scope: Scope, tail: &str, index: usize) -> String {
        self.join(&format!("{}/{index}/{tail}", scope.segment()))
    }

    /// Match an incoming topic against [`COMMAND_ROUTES`].
    ///
    /// # Errors
    ///
    /// See [`TopicError`].
    pub fn parse_command(&self, topic: &str) -> Result<ParsedCommand, TopicError> {
        let rest = topic
            .strip_prefix(self.base.as_str())
            .ok_or(TopicError::OutsideBase)?;
        let mut levels = rest.splitn(3, '/');
        let (Some(scope), Some(index), Some(tail)) = (levels.next(), levels.next(), levels.next())
        else {
            return Err(TopicError::UnknownRoute);
        };
        let scope = match scope {
            "zones" => Scope::Zone,
            "clients" => Scope::Client,
            _ => return Err(TopicError::UnknownRoute),
        };
        let route = COMMAND_ROUTES
            .iter()
            .find(|(s, t, _)| *s == scope && *t == tail)
            .map(|(_, _, route)| *route)
            .ok_or(TopicError::UnknownRoute)?;
        let index = parse_index(index).ok_or(TopicError::InvalidIndex)?;
        Ok(ParsedCommand {
            scope,
            route,
            index,
        })
    }
}

/// A canonical decimal index in `1..=MAX_INDEX`: ASCII digits only, no sign and
/// no leading zero, so that one object has exactly one topic.
fn parse_index(level: &str) -> Option<usize> {
    if level.is_empty() || level.len() > 4 || level.starts_with('0') {
        return None;
    }
    if !level.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    level.parse().ok().filter(|i| (1..=MAX_INDEX).contains(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_is_normalised_once_whatever_the_input() {
        assert_eq!(Topics::new("snapdog").base(), "snapdog/");
        assert_eq!(Topics::new("snapdog/").base(), "snapdog/");
        assert_eq!(Topics::new("home/audio").base(), "home/audio/");
        assert_eq!(Topics::new("home/audio/").base(), "home/audio/");
    }

    #[test]
    fn state_and_availability_topics_have_a_single_slash() {
        let t = Topics::new("snapdog/");
        assert_eq!(t.availability(), "snapdog/status");
        assert_eq!(t.zone_state(2), "snapdog/zones/2/state");
        assert_eq!(t.client_state(3), "snapdog/clients/3/state");
        assert_eq!(t.legacy_double_slash_status(), "snapdog//status");
    }

    #[test]
    fn subscription_list_is_the_documented_set_in_order() {
        let filters: Vec<String> = Topics::new("snapdog/").command_filters().collect();
        assert_eq!(
            filters,
            [
                "snapdog/zones/+/volume/set",
                "snapdog/zones/+/mute/set",
                "snapdog/zones/+/control/set",
                "snapdog/zones/+/shuffle/set",
                "snapdog/zones/+/repeat/set",
                "snapdog/zones/+/position/set",
                "snapdog/zones/+/track/set",
                "snapdog/zones/+/track/position/set",
                "snapdog/zones/+/playlist/set",
                "snapdog/zones/+/presence/set",
                "snapdog/zones/+/presence/enable/set",
                "snapdog/zones/+/presence/timeout/set",
                "snapdog/clients/+/volume/set",
                "snapdog/clients/+/mute/set",
                "snapdog/clients/+/latency/set",
                "snapdog/clients/+/zone/set",
            ]
        );
    }

    #[test]
    fn every_subscribed_filter_parses_back_for_every_base_spelling() {
        for base in ["snapdog", "snapdog/", "home/audio", "home/audio/", "a/b/c"] {
            let topics = Topics::new(base);
            for (scope, tail, route) in COMMAND_ROUTES {
                // Built by the same function that builds the filter, with `+` = 7.
                let topic = topics.command_topic(*scope, tail, 7);
                let filter = topics
                    .command_filters()
                    .find(|f| f.replace('+', "7") == topic)
                    .unwrap_or_else(|| panic!("{topic} is not covered by a subscription"));
                assert!(filter.contains("/+/"));
                assert_eq!(
                    topics.parse_command(&topic),
                    Ok(ParsedCommand {
                        scope: *scope,
                        route: *route,
                        index: 7
                    }),
                    "{topic} (base {base:?})"
                );
            }
        }
    }

    #[test]
    fn rejects_foreign_unknown_and_malformed_topics() {
        let t = Topics::new("snapdog/");
        assert_eq!(
            t.parse_command("other/zones/1/volume/set"),
            Err(TopicError::OutsideBase)
        );
        // Without the slash the base is a prefix of a different topic tree.
        assert_eq!(
            t.parse_command("snapdogs/zones/1/volume/set"),
            Err(TopicError::OutsideBase)
        );
        assert_eq!(
            t.parse_command("snapdog/unknown/topic"),
            Err(TopicError::UnknownRoute)
        );
        assert_eq!(
            t.parse_command("snapdog/zones/1/volume"),
            Err(TopicError::UnknownRoute),
            "the state-less spelling is not a command"
        );
        assert_eq!(
            t.parse_command("snapdog/zones/1/volume/set/extra"),
            Err(TopicError::UnknownRoute)
        );
        assert_eq!(
            t.parse_command("snapdog/clients/1/presence/set"),
            Err(TopicError::UnknownRoute),
            "presence is a zone command only"
        );
        for bad in ["0", "01", "+1", "-1", "1.0", "x", "", "1025", "99999999999"] {
            assert_eq!(
                t.parse_command(&format!("snapdog/zones/{bad}/volume/set")),
                Err(TopicError::InvalidIndex),
                "index {bad:?}"
            );
        }
        assert!(t.parse_command("snapdog/zones/1024/volume/set").is_ok());
    }

    #[test]
    fn empty_base_keeps_its_historic_leading_slash() {
        // `base_topic = ""` has always normalised to "/", which gives topics with an
        // empty first level. Kept as it was.
        let t = Topics::new("");
        assert_eq!(t.base(), "/");
        assert_eq!(t.availability(), "/status");
    }
}
