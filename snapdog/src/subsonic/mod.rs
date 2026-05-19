// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Subsonic API client for music library access.
//!
//! Playlists, track streaming URLs, cover art.
//! Uses token-based auth (md5(password+salt)) for security.

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::SubsonicConfig;

/// HTTP request timeout for Subsonic API calls.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// TCP connect timeout for Subsonic API.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

const API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "snapdog";
const MAX_COVER_ART_BYTES: u64 = 10 * 1024 * 1024;

/// Subsonic API client.
pub struct SubsonicClient {
    base_url: String,
    username: String,
    password: String,
    format: crate::config::SubsonicFormat,
    http: reqwest::Client,
}

impl SubsonicClient {
    /// Create a client from the `[subsonic]` config section.
    pub fn new(config: &SubsonicConfig) -> Self {
        Self {
            base_url: config.url.trim_end_matches('/').to_string(),
            username: config.username.clone(),
            password: config.password.to_string(),
            format: config.format,
            http: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .connect_timeout(CONNECT_TIMEOUT)
                .danger_accept_invalid_certs(config.tls_skip_verify)
                .build()
                .unwrap_or_default(),
        }
    }

    /// Test connection to the Subsonic server.
    pub async fn ping(&self) -> Result<()> {
        let resp: SubsonicResponse<()> = self.get("ping", &[]).await?;
        if resp.subsonic_response.status == "ok" {
            tracing::info!(url = %self.base_url, "Subsonic connection OK");
            Ok(())
        } else {
            anyhow::bail!(
                "Subsonic ping failed: {}",
                resp.subsonic_response
                    .error
                    .map(|e| e.message)
                    .unwrap_or_default()
            )
        }
    }

    /// Get all playlists.
    pub async fn get_playlists(&self) -> Result<Vec<PlaylistEntry>> {
        let resp: SubsonicResponse<PlaylistsWrapper> = self.get("getPlaylists", &[]).await?;
        Ok(resp
            .subsonic_response
            .playlists
            .map(|p| p.playlist)
            .unwrap_or_default())
    }

    /// Get a playlist with its tracks.
    pub async fn get_playlist(&self, id: &str) -> Result<Playlist> {
        let resp: SubsonicResponse<PlaylistWrapper> =
            self.get("getPlaylist", &[("id", id)]).await?;
        resp.subsonic_response
            .playlist
            .context("Playlist not found")
    }

    /// Get the streaming URL for a track (does not fetch — returns the URL).
    pub fn stream_url(&self, track_id: &str) -> String {
        self.stream_url_with_offset(track_id, 0)
    }

    /// Get the streaming URL with a time offset in seconds.
    pub fn stream_url_with_offset(&self, track_id: &str, offset_secs: u64) -> String {
        let mut url = self.rest_url("stream");
        self.add_auth_query(&mut url);
        url.query_pairs_mut()
            .append_pair("id", track_id)
            .append_pair("f", "json")
            .append_pair("format", self.format.as_str());
        if offset_secs > 0 {
            url.query_pairs_mut()
                .append_pair("timeOffset", &offset_secs.to_string());
        }
        url.into()
    }

    /// Get cover art URL for fetching (authenticated).
    pub fn cover_art_fetch_url(&self, cover_id: &str) -> String {
        let mut url = self.rest_url("getCoverArt");
        self.add_auth_query(&mut url);
        url.query_pairs_mut().append_pair("id", cover_id);
        url.into()
    }

    /// Get cover art bytes.
    pub async fn get_cover_art(&self, cover_id: &str) -> Result<Vec<u8>> {
        let url = self.cover_art_fetch_url(cover_id);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        read_response_bytes_limited(resp, MAX_COVER_ART_BYTES, "Subsonic cover art").await
    }

    /// Make an authenticated GET request to the Subsonic API.
    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let mut url = self.rest_url(method);
        self.add_auth_query(&mut url);
        url.query_pairs_mut().append_pair("f", "json");
        {
            let mut query = url.query_pairs_mut();
            for (k, v) in params {
                query.append_pair(k, v);
            }
        }

        let resp = self
            .http
            .get(url.clone())
            .send()
            .await
            .with_context(|| format!("GET {method}"))?;
        resp.error_for_status_ref()
            .with_context(|| format!("GET {method}"))?;
        resp.json()
            .await
            .with_context(|| format!("Parse {method} response"))
    }

    fn rest_url(&self, method: &str) -> url::Url {
        let base = format!("{}/", self.base_url);
        let mut url = url::Url::parse(&base).expect("validated Subsonic base URL");
        url.path_segments_mut()
            .expect("Subsonic base URL cannot be a base")
            .extend(["rest", method]);
        url
    }

    fn add_auth_query(&self, url: &mut url::Url) {
        let (token, salt) = self.auth_token();
        url.query_pairs_mut()
            .append_pair("u", &self.username)
            .append_pair("t", &token)
            .append_pair("s", &salt)
            .append_pair("v", API_VERSION)
            .append_pair("c", CLIENT_NAME);
    }

    /// Generate auth token: token = md5(password + salt), returns (token, salt).
    fn auth_token(&self) -> (String, String) {
        let salt: String = (0..8).map(|_| fastrand::alphanumeric()).collect();
        let token = format!("{:x}", md5::compute(format!("{}{salt}", self.password)));
        (token, salt)
    }
}

async fn read_response_bytes_limited(
    response: reqwest::Response,
    limit: u64,
    label: &str,
) -> Result<Vec<u8>> {
    if let Some(len) = response.content_length() {
        if len > limit {
            anyhow::bail!("{label} body is too large: {len} bytes > {limit} bytes");
        }
    }

    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut body = bytes::BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("Failed to read {label} body"))?;
        let next_len = body.len() as u64 + chunk.len() as u64;
        if next_len > limit {
            anyhow::bail!("{label} body exceeded {limit} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.to_vec())
}

// ── Subsonic API response types ───────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SubsonicResponse<T> {
    subsonic_response: SubsonicInner<T>,
}

#[derive(Deserialize)]
struct SubsonicInner<T> {
    status: String,
    #[serde(flatten)]
    _data: Option<T>,
    error: Option<SubsonicError>,
    // Re-expose specific fields for typed access
    #[serde(default)]
    playlists: Option<PlaylistsContainer>,
    #[serde(default)]
    playlist: Option<Playlist>,
}

#[derive(Deserialize)]
struct SubsonicError {
    message: String,
}

#[derive(Deserialize)]
struct PlaylistsWrapper {}

#[derive(Deserialize)]
struct PlaylistWrapper {}

#[derive(Deserialize)]
struct PlaylistsContainer {
    #[serde(default)]
    playlist: Vec<PlaylistEntry>,
}

/// Summary of a Subsonic playlist (without tracks).
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    /// Subsonic playlist ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Number of songs in the playlist.
    #[serde(default)]
    pub song_count: u32,
    /// Total duration in seconds.
    #[serde(default)]
    pub duration: u64,
    /// Cover art ID for [`SubsonicClient::cover_art_fetch_url`].
    pub cover_art: Option<String>,
}

/// Full Subsonic playlist including its tracks.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    /// Subsonic playlist ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Tracks in playlist order.
    #[serde(default)]
    pub entry: Vec<Track>,
}

/// A single track from a Subsonic playlist.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)] // Subsonic API field name
pub struct Track {
    /// Subsonic track ID.
    pub id: String,
    /// Track title.
    pub title: String,
    /// Artist name.
    pub artist: Option<String>,
    /// Album name.
    pub album: Option<String>,
    /// Duration in seconds.
    #[serde(default)]
    pub duration: u64,
    /// Cover art ID for [`SubsonicClient::cover_art_fetch_url`].
    pub cover_art: Option<String>,
    /// Track number within the album.
    pub track: Option<u32>,
}
