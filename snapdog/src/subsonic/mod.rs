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

/// How long a read of the playlist list is used.
const PLAYLIST_CACHE_TTL: std::time::Duration = std::time::Duration::from_mins(1);
/// Wait before asking a failing Subsonic server again.
const PLAYLIST_RETRY: std::time::Duration = std::time::Duration::from_secs(10);

/// The Subsonic playlist list, shared by the REST API and the zone players so that both see the
/// same list and the same indexes.
///
/// A failed fetch keeps the last good list instead of replacing it with an empty one, so a
/// Subsonic outage does not make every playlist and the indexes behind them vanish. The next
/// attempt follows after [`PLAYLIST_RETRY`].
#[derive(Default)]
pub struct PlaylistCache {
    inner: tokio::sync::RwLock<Option<(std::time::Instant, Vec<PlaylistEntry>)>>,
    /// The list clients were last told about, see [`PlaylistCache::announce`].
    announced: std::sync::Mutex<Option<Vec<PlaylistEntry>>>,
}

impl PlaylistCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The playlists, read from `client` when the cached list is older than a minute. Without a
    /// client (Subsonic not configured) the list is empty.
    pub async fn get(&self, client: Option<&SubsonicClient>) -> Vec<PlaylistEntry> {
        if let Some((stamp, entries)) = &*self.inner.read().await
            && stamp.elapsed() < PLAYLIST_CACHE_TTL
        {
            return entries.clone();
        }
        self.fetch(client).await
    }

    /// The playlists, read from `client` now, whatever the age of the cached list.
    pub async fn fetch(&self, client: Option<&SubsonicClient>) -> Vec<PlaylistEntry> {
        let fetched = match client {
            Some(client) => client.get_playlists().await.map(Some),
            None => Ok(None),
        };
        let mut cache = self.inner.write().await;
        let (stamp, entries) = refreshed_cache(cache.take(), fetched, std::time::Instant::now());
        *cache = Some((stamp, entries.clone()));
        entries
    }

    /// Whether `entries` differ from the list clients were last told about, and remember them
    /// as that list. The first list only sets the baseline and needs no announcement. The
    /// comparison is against what was announced, not against the previous fetch, so a list that
    /// a zone player refreshed first is still announced once.
    pub fn announce(&self, entries: &[PlaylistEntry]) -> bool {
        let mut announced = self
            .announced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = announced
            .as_deref()
            .is_some_and(|before| playlists_differ(before, entries));
        if changed || announced.is_none() {
            *announced = Some(entries.to_vec());
        }
        changed
    }
}

/// Whether two lists differ in what a client shows: ids and their order, names, song counts.
fn playlists_differ(a: &[PlaylistEntry], b: &[PlaylistEntry]) -> bool {
    a.len() != b.len()
        || a.iter()
            .zip(b)
            .any(|(x, y)| x.id != y.id || x.name != y.name || x.song_count != y.song_count)
}

/// Next cache content after a fetch. `Ok(None)` means Subsonic is not configured.
fn refreshed_cache(
    previous: Option<(std::time::Instant, Vec<PlaylistEntry>)>,
    fetched: Result<Option<Vec<PlaylistEntry>>>,
    now: std::time::Instant,
) -> (std::time::Instant, Vec<PlaylistEntry>) {
    match fetched {
        Ok(entries) => (now, entries.unwrap_or_default()),
        Err(e) => {
            tracing::warn!("Subsonic playlists unavailable, keeping the last list: {e:#}");
            // Back-dated so that the entry expires after PLAYLIST_RETRY, not after the full TTL.
            let stamp = PLAYLIST_CACHE_TTL
                .checked_sub(PLAYLIST_RETRY)
                .and_then(|back| now.checked_sub(back))
                .unwrap_or(now);
            (
                stamp,
                previous.map(|(_, entries)| entries).unwrap_or_default(),
            )
        }
    }
}

/// Prefix of the queue id the player uses for an album. It cannot be a Subsonic playlist id.
const ALBUM_QUEUE_PREFIX: &str = "\u{1f}album:";

/// The queue id under which the player holds an album, see [`SubsonicClient::get_queue`].
#[must_use]
pub fn album_queue_id(album_id: &str) -> String {
    format!("{ALBUM_QUEUE_PREFIX}{album_id}")
}

/// Whether `queue_id` names an album queue and not a Subsonic playlist.
#[must_use]
pub fn is_album_queue(queue_id: &str) -> bool {
    queue_id.starts_with(ALBUM_QUEUE_PREFIX)
}

/// One page of a `search3` kind.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    /// Number of results asked for, `0` for none.
    pub count: usize,
    /// Results to skip.
    pub offset: usize,
}

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
    #[must_use]
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
    ///
    /// # Errors
    ///
    /// Returns an error if the server is unreachable or responds with a failure.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails.
    pub async fn get_playlists(&self) -> Result<Vec<PlaylistEntry>> {
        let resp: SubsonicResponse<PlaylistsWrapper> = self.get("getPlaylists", &[]).await?;
        Ok(resp
            .subsonic_response
            .playlists
            .map(|p| p.playlist)
            .unwrap_or_default())
    }

    /// Get a playlist with its tracks.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails or the playlist is not found.
    pub async fn get_playlist(&self, id: &str) -> Result<Playlist> {
        let resp: SubsonicResponse<PlaylistWrapper> =
            self.get("getPlaylist", &[("id", id)]).await?;
        resp.subsonic_response
            .playlist
            .context("Playlist not found")
    }

    /// Get an album with its tracks, shaped like a playlist so the player can queue it.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails or the album is not found.
    pub async fn get_album(&self, id: &str) -> Result<Playlist> {
        let resp: SubsonicResponse<PlaylistWrapper> = self.get("getAlbum", &[("id", id)]).await?;
        let album = resp.subsonic_response.album.context("Album not found")?;
        Ok(Playlist {
            id: album.id,
            name: album.name,
            entry: album.song,
        })
    }

    /// Get the tracks the player queues for `queue_id`: a Subsonic playlist id, or an album
    /// (see [`album_queue_id`]). An album is not part of the unified playlist list, so the
    /// player holds it under a reserved id.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails or nothing is found.
    pub async fn get_queue(&self, queue_id: &str) -> Result<Playlist> {
        match queue_id.strip_prefix(ALBUM_QUEUE_PREFIX) {
            Some(album_id) => self.get_album(album_id).await,
            None => self.get_playlist(queue_id).await,
        }
    }

    /// Search tracks and albums with `search3`. A count of 0 asks for none of that kind.
    ///
    /// `search3` reports no total, and what it matches depends on the Subsonic server.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails.
    pub async fn search(&self, query: &str, songs: Page, albums: Page) -> Result<SearchResult> {
        let (sc, so, ac, ao) = (
            songs.count.to_string(),
            songs.offset.to_string(),
            albums.count.to_string(),
            albums.offset.to_string(),
        );
        let resp: SubsonicResponse<PlaylistWrapper> = self
            .get(
                "search3",
                &[
                    ("query", query),
                    ("songCount", &sc),
                    ("songOffset", &so),
                    ("albumCount", &ac),
                    ("albumOffset", &ao),
                    ("artistCount", "0"),
                ],
            )
            .await?;
        Ok(resp.subsonic_response.search_result3.unwrap_or_default())
    }

    /// Get the streaming URL for a track (does not fetch — returns the URL).
    #[must_use]
    pub fn stream_url(&self, track_id: &str) -> String {
        self.stream_url_with_offset(track_id, 0)
    }

    /// Get the streaming URL with a time offset in seconds.
    #[must_use]
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
    #[must_use]
    pub fn cover_art_fetch_url(&self, cover_id: &str) -> String {
        let mut url = self.rest_url("getCoverArt");
        self.add_auth_query(&mut url);
        url.query_pairs_mut().append_pair("id", cover_id);
        url.into()
    }

    /// Get cover art bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response is too large.
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
    use futures_util::StreamExt;

    if let Some(len) = response.content_length()
        && len > limit
    {
        anyhow::bail!("{label} body is too large: {len} bytes > {limit} bytes");
    }
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
    #[serde(default, rename = "searchResult3")]
    search_result3: Option<SearchResult>,
    #[serde(default)]
    album: Option<AlbumWithSongs>,
}

/// An album as `getAlbum` returns it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumWithSongs {
    id: String,
    name: String,
    #[serde(default)]
    song: Vec<Track>,
}

/// Result of `search3`: matching songs and albums.
#[derive(Debug, Default, Deserialize)]
pub struct SearchResult {
    /// Matching tracks.
    #[serde(default)]
    pub song: Vec<Track>,
    /// Matching albums.
    #[serde(default)]
    pub album: Vec<AlbumEntry>,
}

/// An album in a search result.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AlbumEntry {
    /// Subsonic album ID.
    pub id: String,
    /// Album name (`name`, older servers send `title`).
    #[serde(alias = "title")]
    pub name: String,
    /// Album artist.
    pub artist: Option<String>,
    /// Cover art ID.
    pub cover_art: Option<String>,
    /// Total duration in seconds.
    #[serde(default)]
    pub duration: u64,
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

#[cfg(test)]
mod playlist_cache_tests {
    use super::*;
    use std::time::Instant;

    fn entry(id: &str) -> PlaylistEntry {
        PlaylistEntry {
            id: id.into(),
            name: id.into(),
            song_count: 0,
            duration: 0,
            cover_art: None,
        }
    }

    #[test]
    fn the_first_list_only_sets_the_baseline() {
        let cache = PlaylistCache::new();
        assert!(!cache.announce(&[entry("a")]));
        assert!(!cache.announce(&[entry("a")]));
    }

    #[test]
    fn a_change_is_announced_once() {
        let cache = PlaylistCache::new();
        cache.announce(&[entry("a")]);
        assert!(cache.announce(&[entry("a"), entry("b")]));
        assert!(!cache.announce(&[entry("a"), entry("b")]));
        let mut renamed = entry("a");
        renamed.name = "other".into();
        assert!(cache.announce(&[renamed, entry("b")]));
    }

    #[test]
    fn successful_fetch_replaces_the_list() {
        let now = Instant::now();
        let previous = Some((now, vec![entry("old")]));
        let (stamp, entries) = refreshed_cache(previous, Ok(Some(vec![entry("new")])), now);
        assert_eq!(stamp, now);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "new");
    }

    #[test]
    fn unconfigured_subsonic_yields_an_empty_list() {
        let now = Instant::now();
        let (_, entries) = refreshed_cache(Some((now, vec![entry("old")])), Ok(None), now);
        assert!(entries.is_empty());
    }

    #[test]
    fn failed_fetch_keeps_the_last_good_list() {
        let now = Instant::now();
        let previous = Some((now, vec![entry("a"), entry("b")]));
        let (_, entries) = refreshed_cache(previous, Err(anyhow::anyhow!("down")), now);
        assert_eq!(
            entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn failed_first_fetch_yields_an_empty_list() {
        let (_, entries) = refreshed_cache(None, Err(anyhow::anyhow!("down")), Instant::now());
        assert!(entries.is_empty());
    }

    #[test]
    fn failed_fetch_expires_after_the_retry_delay() {
        let now = Instant::now();
        let (stamp, _) = refreshed_cache(None, Err(anyhow::anyhow!("down")), now);
        let age = now.duration_since(stamp);
        assert_eq!(age, PLAYLIST_CACHE_TTL.checked_sub(PLAYLIST_RETRY).unwrap());
    }
}
