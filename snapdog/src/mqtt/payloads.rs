// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Retained state payloads (zone and client JSON).
//!
//! Pure functions, so the wire schema is unit-testable without a broker.
//!
//! Free text from track tags is cut to a fixed length. rumqttc treats a packet
//! over its size limit (10 KiB by default) as a *connection* error: the packet is
//! consumed, the connection is torn down and re-established. One track with an
//! absurd title would make the bridge reconnect on every state change.

use crate::state;

/// Longest title, artist or album in the retained state, in bytes.
const MAX_TEXT_BYTES: usize = 512;
/// Longest cover URL in the retained state, in bytes.
const MAX_URL_BYTES: usize = 1024;

/// `s` cut to at most `max` bytes, on a character boundary.
fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Build the retained zone-state JSON (Home Assistant `media_player` schema).
///
/// Pure: extracted from `publish_zone_state` so the wire schema is unit-testable
/// without a broker. Note `source` uses the `Display` form (e.g. `subsonic_playlist`,
/// with underscore — differs from the serde form), and `repeat` maps to HA's
/// `off`/`one`/`all`.
pub fn zone_state_payload(base_url: &str, zone: &state::ZoneState) -> serde_json::Value {
    let state_str = match zone.playback {
        state::PlaybackState::Playing => "playing",
        state::PlaybackState::Paused => "paused",
        state::PlaybackState::Stopped => {
            if zone.source == state::SourceType::Idle {
                "idle"
            } else {
                "stopped"
            }
        }
    };

    let mut payload = serde_json::json!({
        "state": state_str,
        "volume_level": f64::from(zone.volume) / 100.0,
        "is_volume_muted": zone.muted,
        "shuffle": zone.shuffle,
        "repeat": match zone.repeat { snapdog_common::RepeatMode::Track => "one", snapdog_common::RepeatMode::Playlist => "all", snapdog_common::RepeatMode::Off => "off" },
        "source": zone.source.to_string(),
        "presence": zone.presence,
    });

    if let Some(track) = &zone.track {
        payload["media_title"] = serde_json::json!(truncate_utf8(&track.title, MAX_TEXT_BYTES));
        payload["media_artist"] = serde_json::json!(truncate_utf8(&track.artist, MAX_TEXT_BYTES));
        payload["media_album_name"] =
            serde_json::json!(truncate_utf8(&track.album, MAX_TEXT_BYTES));
        payload["media_duration"] = serde_json::json!(track.duration_ms / 1000);
        payload["media_position"] = serde_json::json!(track.position_ms / 1000);
        payload["media_content_type"] = serde_json::json!("music");
    }

    if let Some(ref cover_url) = zone.cover_url {
        let absolute = if cover_url.starts_with('/') {
            format!("{base_url}{cover_url}")
        } else {
            cover_url.clone()
        };
        payload["media_image_url"] = serde_json::json!(truncate_utf8(&absolute, MAX_URL_BYTES));
    }

    payload
}

/// Build the retained client-state JSON. Pure (see [`zone_state_payload`]).
pub fn client_state_payload(client: &state::ClientState) -> serde_json::Value {
    serde_json::json!({
        "volume": client.base_volume,
        "muted": client.muted,
        "connected": client.connected,
        "zone": client.zone_index,
        "latency": client.latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::test_support::test_state_with_client;

    // ── IT-T31: retained-state JSON schema ────────────────────────

    #[tokio::test]
    async fn zone_state_payload_idle() {
        let state = test_state_with_client();
        let z = state.read().await.zones[&1].clone();
        let p = zone_state_payload("http://host:8000", &z);
        assert_eq!(p["state"], "idle"); // Stopped + Idle source => "idle"
        assert_eq!(p["volume_level"], 0.5); // 50 / 100
        assert_eq!(p["is_volume_muted"], false);
        assert_eq!(p["shuffle"], false);
        assert_eq!(p["repeat"], "off");
        assert_eq!(p["source"], "idle");
        assert_eq!(p["presence"], false);
        assert!(p.get("media_title").is_none(), "no track fields when idle");
        assert!(p.get("media_image_url").is_none(), "no cover when none");
    }

    #[tokio::test]
    async fn zone_state_payload_stopped_non_idle() {
        let state = test_state_with_client();
        let mut z = state.read().await.zones[&1].clone();
        z.source = state::SourceType::Radio; // Stopped + non-Idle source => "stopped"
        let p = zone_state_payload("http://h", &z);
        assert_eq!(p["state"], "stopped");
        assert_eq!(p["source"], "radio");
    }

    #[tokio::test]
    async fn zone_state_payload_playing_with_track_and_cover() {
        let state = test_state_with_client();
        let mut z = state.read().await.zones[&1].clone();
        z.volume = 80;
        z.muted = true;
        z.shuffle = true;
        z.playback = state::PlaybackState::Playing;
        z.repeat = snapdog_common::RepeatMode::Playlist;
        z.source = state::SourceType::SubsonicPlaylist;
        z.cover_url = Some("/cover/1.jpg".into());
        z.track = Some(
            serde_json::from_value(serde_json::json!({
                "title": "T", "artist": "A", "album": "Al",
                "album_artist": null, "genre": null, "year": null,
                "track_number": null, "disc_number": null,
                "duration_ms": 200_000, "position_ms": 5_000, "seekable": true,
                "source": "subsonicplaylist", "bitrate_kbps": null,
                "content_type": null, "sample_rate": null
            }))
            .unwrap(),
        );

        let p = zone_state_payload("http://host:8000", &z);
        assert_eq!(p["state"], "playing");
        assert_eq!(p["volume_level"], 0.8);
        assert_eq!(p["is_volume_muted"], true);
        assert_eq!(p["shuffle"], true);
        assert_eq!(p["repeat"], "all"); // RepeatMode::Playlist => HA "all"
        // `source` is the Display form (underscore), NOT the serde form "subsonicplaylist".
        assert_eq!(p["source"], "subsonic_playlist");
        assert_eq!(p["media_title"], "T");
        assert_eq!(p["media_duration"], 200); // ms -> s
        assert_eq!(p["media_position"], 5);
        assert_eq!(p["media_content_type"], "music");
        // Relative cover URL is absolutized against base_url.
        assert_eq!(p["media_image_url"], "http://host:8000/cover/1.jpg");
    }

    #[tokio::test]
    async fn client_state_payload_schema() {
        let state = test_state_with_client();
        let c = state.read().await.clients[&1].clone();
        let p = client_state_payload(&c);
        assert_eq!(p["volume"], 50); // base_volume
        assert_eq!(p["muted"], false);
        assert_eq!(p["connected"], true);
        assert_eq!(p["zone"], 1);
        assert_eq!(p["latency"], 0);
    }

    #[test]
    fn truncation_respects_character_boundaries() {
        assert_eq!(truncate_utf8("abc", 5), "abc");
        assert_eq!(truncate_utf8("abcdef", 3), "abc");
        // "ä" is two bytes: a cut in the middle backs off to the boundary before it.
        assert_eq!(truncate_utf8("aä", 2), "a");
        assert_eq!(truncate_utf8("aä", 3), "aä");
        assert_eq!(truncate_utf8("日本語", 4), "日");
        assert_eq!(truncate_utf8("", 0), "");
    }

    #[tokio::test]
    async fn an_absurd_track_tag_cannot_exceed_the_packet_size_limit() {
        let state = test_state_with_client();
        let mut z = state.read().await.zones[&1].clone();
        z.cover_url = Some(format!("/cover/{}", "x".repeat(100_000)));
        z.track = Some(
            serde_json::from_value(serde_json::json!({
                "title": "ä".repeat(500_000), "artist": "A".repeat(200_000), "album": "日".repeat(100_000),
                "album_artist": null, "genre": null, "year": null,
                "track_number": null, "disc_number": null,
                "duration_ms": 200_000, "position_ms": 5_000, "seekable": true,
                "source": "subsonicplaylist", "bitrate_kbps": null,
                "content_type": null, "sample_rate": null
            }))
            .unwrap(),
        );
        let json = zone_state_payload("http://host:8000", &z).to_string();
        // rumqttc's default limit is 10 KiB, topic and header included.
        assert!(json.len() < 4096, "{} bytes", json.len());
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(back["media_title"].as_str().unwrap().starts_with("ää"));
        assert!(back["media_title"].as_str().unwrap().len() <= MAX_TEXT_BYTES);
    }
}
