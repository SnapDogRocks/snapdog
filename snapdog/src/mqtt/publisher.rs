// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Outgoing messages: availability, retained state and the one-time cleanups.
//!
//! Nothing here waits. A publish is queued with `try_publish`, so a caller (the
//! connection task, the state publisher) never blocks on a broker that is
//! unreachable. The request channel is sized for a full announcement plus
//! headroom ([`request_capacity`]); when it is full anyway the message is
//! dropped, which is harmless for retained state: the snapshot taken after the
//! next (re)connect replaces it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow};
use rumqttc::{AsyncClient, QoS};

use super::payloads::{client_state_payload, zone_state_json};
use super::topics::{COMMAND_ROUTES, Topics};
use super::transport::MAX_OUTGOING_PACKET_BYTES;
use crate::config::LegacyCleanup;
use crate::state;

/// Zone indices whose legacy discovery topic is cleared, at least.
const LEGACY_DISCOVERY_ZONES: usize = 16;
/// Requests an announcement queues apart from the per-zone/-client state.
/// (the subscriptions, `online`, the legacy cleanup, the discovery message and the
/// subscription to Home Assistant's birth topic.)
const ANNOUNCE_FIXED_REQUESTS: usize = COMMAND_ROUTES.len() + 1 + LEGACY_DISCOVERY_ZONES + 1 + 2;
/// Headroom for steady-state publishes on top of an announcement.
const REQUEST_HEADROOM: usize = 256;

/// Capacity of the request channel for `zones` zones and `clients` clients.
///
/// Large enough that the announcement after a (re)connect always fits (it is
/// queued while nothing drains the channel), whatever the installation size.
pub const fn request_capacity(zones: usize, clients: usize) -> usize {
    ANNOUNCE_FIXED_REQUESTS + 2 * (zones + clients) + REQUEST_HEADROOM
}

/// Queues outgoing messages. Cheap to clone; all clones share one client.
#[derive(Clone)]
pub struct Publisher {
    client: AsyncClient,
    topics: Topics,
    base_url: String,
    /// Set while the outbox is known to be full, so that the warning is logged once
    /// per episode instead of once per dropped message.
    outbox_full: Arc<AtomicBool>,
}

impl Publisher {
    pub fn new(client: AsyncClient, topics: Topics, base_url: &str) -> Self {
        Self {
            client,
            topics,
            base_url: base_url.trim_end_matches('/').to_string(),
            outbox_full: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The underlying client (subscriptions, disconnect).
    pub const fn client(&self) -> &AsyncClient {
        &self.client
    }

    /// The topic scheme.
    pub const fn topics(&self) -> &Topics {
        &self.topics
    }

    /// Queue a retained `QoS` 1 message for the full `topic`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request channel is full or the event loop is gone.
    pub fn retained(&self, topic: &str, payload: impl Into<Vec<u8>>) -> Result<()> {
        let payload = payload.into();
        // A packet over the limit does not just fail: it ends the connection, and
        // the snapshot after the reconnect would send it again.
        if !fits_in_a_packet(topic, payload.len()) {
            tracing::warn!(
                topic = ?topic,
                bytes = payload.len(),
                "MQTT message is larger than the packet limit, not published"
            );
            return Err(anyhow!("MQTT message for {topic} is too large"));
        }
        match self
            .client
            .try_publish(topic, QoS::AtLeastOnce, true, payload)
        {
            Ok(()) => {
                self.outbox_full.store(false, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                if !self.outbox_full.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        error = %e,
                        "MQTT outbox full or closed, dropping messages until it drains"
                    );
                }
                Err(anyhow!("MQTT request queue rejected {topic}: {e}"))
            }
        }
    }

    /// Retained `online` on the availability topic (the last will is `offline`).
    pub fn online(&self) -> Result<()> {
        self.retained(&self.topics.availability(), "online")
    }

    /// Retained `offline` on the availability topic (graceful shutdown: a clean
    /// DISCONNECT suppresses the last will, so it has to be sent by hand).
    pub fn offline(&self) -> Result<()> {
        self.retained(&self.topics.availability(), "offline")
    }

    /// Retained zone state.
    pub fn zone_state(&self, index: usize, zone: &state::ZoneState) -> Result<()> {
        self.retained(
            &self.topics.zone_state(index),
            zone_state_json(&self.base_url, zone),
        )
    }

    /// Retained client state.
    pub fn client_state(&self, index: usize, client: &state::ClientState) -> Result<()> {
        self.retained(
            &self.topics.client_state(index),
            client_state_payload(client).to_string(),
        )
    }

    /// Publish the state of every zone and client. Returns how many messages were
    /// queued. Synchronous on purpose: the caller holds the store lock, so no
    /// newer notification can overtake the snapshot.
    pub fn snapshot(&self, store: &state::Store) -> usize {
        let mut queued = 0;
        for (&index, zone) in &store.zones {
            queued += usize::from(self.zone_state(index, zone).is_ok());
        }
        for (&index, client) in &store.clients {
            queued += usize::from(self.client_state(index, client).is_ok());
        }
        queued
    }

    /// Clear retained messages that earlier releases left behind. Idempotent, so
    /// running it once per process start is enough.
    ///
    /// - `{base}/status` (double slash): `online` published by a past topic bug.
    /// - `homeassistant/media_player/snapdog_zone_{n}/config`: discovery messages
    ///   for a platform Home Assistant's MQTT integration does not have.
    ///
    /// Returns the topics it asked the broker to clear (it cannot know which of them
    /// held anything).
    pub fn clear_legacy_retained(&self, zones: usize, cleanup: LegacyCleanup) -> Vec<String> {
        let mut topics = Vec::new();
        if cleanup != LegacyCleanup::Off {
            topics.push(self.topics.legacy_double_slash_status());
        }
        if cleanup == LegacyCleanup::All {
            topics.extend(
                (1..=zones.max(LEGACY_DISCOVERY_ZONES)).map(legacy_media_player_discovery_topic),
            );
        }
        topics.retain(|topic| {
            // A zero-length retained publish deletes a retained message (MQTT-3.3.1-10).
            self.retained(topic, Vec::<u8>::new()).is_ok()
        });
        topics
    }
}

/// Whether a publish of `payload_len` bytes to `topic` stays within the outgoing
/// packet limit (fixed header, topic length and packet id take up to 8 bytes).
const fn fits_in_a_packet(topic: &str, payload_len: usize) -> bool {
    payload_len.saturating_add(topic.len()) <= MAX_OUTGOING_PACKET_BYTES - 8
}

/// The discovery topic old releases published a `media_player` config to.
fn legacy_media_player_discovery_topic(zone: usize) -> String {
    format!("homeassistant/media_player/snapdog_zone_{zone}/config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcement_always_fits_the_request_channel() {
        // What `announce` queues: one subscription per command route, the online
        // message, the legacy cleanup and the state of every zone and client.
        for (zones, clients) in [(0, 0), (1, 1), (10, 30), (64, 512), (1024, 1024)] {
            let announced = COMMAND_ROUTES.len()
                + 1
                + (1 + zones.max(LEGACY_DISCOVERY_ZONES))
                + zones
                + clients;
            assert!(
                announced + REQUEST_HEADROOM / 2 <= request_capacity(zones, clients),
                "{zones} zones + {clients} clients: {announced} requests do not leave headroom in {}",
                request_capacity(zones, clients)
            );
        }
    }

    #[test]
    fn the_old_fixed_capacity_would_have_stalled_a_large_installation() {
        // Documents the defect: 64 slots, 16 subscriptions, 1 + 1 availability and
        // cleanup messages, 10 discovery messages and the initial state of
        // 10 zones + 30 clients is 68 requests, queued before the event loop runs.
        let old_capacity = 64;
        let queued = 16 + 10 + 1 + 1 + 10 + 30;
        assert!(queued > old_capacity);
        assert!(request_capacity(10, 30) > queued);
    }

    #[tokio::test]
    async fn a_message_over_the_packet_limit_is_refused_instead_of_dropping_the_connection() {
        // Room for two requests: if the oversized message were queued, the second
        // small one would find the channel full.
        let (client, _eventloop) =
            AsyncClient::new(rumqttc::MqttOptions::new("t", "localhost", 1883), 2);
        let publisher = Publisher::new(client, Topics::new("snapdog"), "http://h");
        assert!(publisher.retained("snapdog/x", vec![0_u8; 1000]).is_ok());
        assert!(
            publisher
                .retained("snapdog/x", vec![0_u8; MAX_OUTGOING_PACKET_BYTES])
                .is_err()
        );
        assert!(publisher.retained("snapdog/x", vec![0_u8; 1000]).is_ok());
    }

    #[test]
    fn legacy_discovery_topic_matches_what_old_releases_published() {
        assert_eq!(
            legacy_media_player_discovery_topic(3),
            "homeassistant/media_player/snapdog_zone_3/config"
        );
    }
}
