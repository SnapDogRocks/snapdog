// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Retained state payloads (zone and client JSON).
//!
//! Pure functions, so the wire schema is unit-testable without a broker.
//!
//! Free text from track tags is cut to a fixed length. rumqttc treats a packet
//! over its size limit as a *connection* error: the packet is consumed, the
//! connection is torn down and re-established, and the snapshot after the
//! reconnect sends the same payload again. One track with an absurd title would
//! make the bridge reconnect on every state change. The limits count **escaped**
//! bytes (what the JSON text costs), not raw ones: a control character becomes
//! `\u0001` (6 bytes) and a quote `\"` (2 bytes). [`zone_state_json`] checks the
//! finished text once more, as the last line of defence.

use crate::state;

/// Longest title, artist or album in the retained state, in bytes of JSON text.
const MAX_TEXT_BYTES: usize = 512;
/// Longest cover URL in the retained state, in bytes of JSON text. A longer one is
/// left out: a cut URL points nowhere.
const MAX_URL_BYTES: usize = 1024;
/// Largest retained state message, in bytes. The real maximum is about 3 KB; this
/// is what the text fields are dropped for if it is ever exceeded.
pub const MAX_STATE_PAYLOAD_BYTES: usize = 16 * 1024;

/// What `c` costs inside a JSON string, in bytes (`serde_json`'s escaping).
const fn json_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// `s` cut so that its JSON-escaped form takes at most `max` bytes, on a
/// character boundary.
fn truncate_json(s: &str, max: usize) -> &str {
    let mut used = 0;
    for (index, c) in s.char_indices() {
        used += json_len(c);
        if used > max {
            return &s[..index];
        }
    }
    s
}

/// The JSON-escaped size of `s` in bytes, without the surrounding quotes.
fn json_size(s: &str) -> usize {
    s.chars().map(json_len).sum()
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
        payload["media_title"] = serde_json::json!(truncate_json(&track.title, MAX_TEXT_BYTES));
        payload["media_artist"] = serde_json::json!(truncate_json(&track.artist, MAX_TEXT_BYTES));
        payload["media_album_name"] =
            serde_json::json!(truncate_json(&track.album, MAX_TEXT_BYTES));
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
        if json_size(&absolute) <= MAX_URL_BYTES {
            payload["media_image_url"] = serde_json::json!(absolute);
        }
    }

    payload
}

/// The retained zone-state message: [`zone_state_payload`] as JSON text, at most
/// [`MAX_STATE_PAYLOAD_BYTES`] long.
pub fn zone_state_json(base_url: &str, zone: &state::ZoneState) -> String {
    bounded_json(zone_state_payload(base_url, zone), MAX_STATE_PAYLOAD_BYTES)
}

/// The keys that carry free text, the only unbounded part of a state payload.
const FREE_TEXT_KEYS: [&str; 4] = [
    "media_title",
    "media_artist",
    "media_album_name",
    "media_image_url",
];

/// `payload` as JSON text of at most `max` bytes: if it is longer, the free-text
/// keys are dropped (the rest of the state is a few hundred bytes).
fn bounded_json(mut payload: serde_json::Value, max: usize) -> String {
    let text = payload.to_string();
    if text.len() <= max {
        return text;
    }
    if let Some(map) = payload.as_object_mut() {
        for key in FREE_TEXT_KEYS {
            map.remove(key);
        }
    }
    payload.to_string()
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
        assert_eq!(truncate_json("abc", 5), "abc");
        assert_eq!(truncate_json("abcdef", 3), "abc");
        // "ä" is two bytes: a cut in the middle backs off to the boundary before it.
        assert_eq!(truncate_json("aä", 2), "a");
        assert_eq!(truncate_json("aä", 3), "aä");
        assert_eq!(truncate_json("日本語", 4), "日");
        assert_eq!(truncate_json("", 0), "");
    }

    #[test]
    fn truncation_counts_the_escaped_size() {
        // A control character is 6 bytes in JSON (`\u0001`), a quote and a
        // backslash 2, the common whitespace escapes 2.
        assert_eq!(json_size("\u{1}"), 6);
        assert_eq!(json_size("\""), 2);
        assert_eq!(json_size("\\"), 2);
        assert_eq!(json_size("\n\t"), 4);
        assert_eq!(truncate_json("\u{1}\u{1}\u{1}", 12), "\u{1}\u{1}");
        assert_eq!(truncate_json("\"\"\"", 5), "\"\"");
        // What the size function says is what serde_json writes.
        for s in ["plain", "q\"b\\", "\u{1}\u{1f}\u{8}\u{c}", "日本ä\u{7f}"] {
            let quoted = serde_json::Value::from(s).to_string();
            assert_eq!(quoted.len(), json_size(s) + 2, "{s:?}");
        }
    }

    async fn zone_with_text(
        title: &str,
        artist: &str,
        album: &str,
        cover: &str,
    ) -> state::ZoneState {
        let mut z = test_state_with_client().read().await.zones[&1].clone();
        z.cover_url = Some(cover.to_string());
        z.track = Some(
            serde_json::from_value(serde_json::json!({
                "title": title, "artist": artist, "album": album,
                "album_artist": null, "genre": null, "year": null,
                "track_number": null, "disc_number": null,
                "duration_ms": 200_000, "position_ms": 5_000, "seekable": true,
                "source": "subsonicplaylist", "bitrate_kbps": null,
                "content_type": null, "sample_rate": null
            }))
            .unwrap(),
        );
        z
    }

    #[tokio::test]
    async fn an_absurd_track_tag_cannot_exceed_the_packet_size_limit() {
        let z = zone_with_text(
            &"ä".repeat(500_000),
            &"A".repeat(200_000),
            &"日".repeat(100_000),
            &format!("/cover/{}", "x".repeat(100_000)),
        )
        .await;
        let json = zone_state_json("http://host:8000", &z);
        // rumqttc's default limit was 10 KiB, topic and header included.
        assert!(json.len() < 4096, "{} bytes", json.len());
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(back["media_title"].as_str().unwrap().starts_with("ää"));
        assert!(back["media_title"].as_str().unwrap().len() <= MAX_TEXT_BYTES);
        // A cover URL that does not fit is left out, not cut into a broken one.
        assert!(back.get("media_image_url").is_none());
    }

    /// Review finding F2: control characters and quotes grow six- and twofold in
    /// JSON, so cutting by raw bytes let the message through at 11.5 KB.
    #[tokio::test]
    async fn escaping_cannot_blow_up_a_truncated_tag() {
        let z = zone_with_text(
            &"\u{1}".repeat(5000),
            &"\u{1}".repeat(5000),
            &"\u{1}".repeat(5000),
            &format!("http://x/{}", "\"".repeat(500)),
        )
        .await;
        let json = zone_state_json("http://host:8000", &z);
        assert!(json.len() < 4096, "{} bytes", json.len());
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        let title = back["media_title"].as_str().unwrap();
        assert!(!title.is_empty(), "the tag is cut, not dropped");
        assert!(json_size(title) <= MAX_TEXT_BYTES);
        // 500 quotes cost 1000 bytes of JSON: still within the URL limit, kept whole.
        assert_eq!(
            back["media_image_url"]
                .as_str()
                .unwrap()
                .matches('"')
                .count(),
            500
        );
    }

    #[test]
    fn the_last_line_of_defence_drops_free_text_when_the_message_is_still_too_long() {
        let payload = serde_json::json!({
            "state": "playing",
            "media_title": "x".repeat(40_000),
            "media_image_url": "y",
        });
        let text = bounded_json(payload, MAX_STATE_PAYLOAD_BYTES);
        assert!(text.len() <= MAX_STATE_PAYLOAD_BYTES);
        let back: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back["state"], "playing");
        assert!(back.get("media_title").is_none());
        // A message within the limit is left alone.
        let small = serde_json::json!({"state": "idle", "media_title": "t"});
        assert_eq!(bounded_json(small.clone(), 1000), small.to_string());
    }
}
