// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Cover art cache — in-memory store for zone cover images.
//! Served via GET /api/v1/zones/{id}/cover with correct MIME type.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

const MAX_COVER_ART_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FAVICON_HTML_BYTES: u64 = 1024 * 1024;
/// The cover URL of a zone as the store keeps it: relative to the HTTP server, with the image
/// hash so that clients reload a changed cover. MQTT makes it absolute with `base_url`.
#[must_use]
pub fn zone_cover_url(zone_index: usize, hash: &str) -> String {
    format!("/api/v1/zones/{zone_index}/cover?h={hash}")
}

/// Upper bound for fetching one cover image, body included.
const COVER_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Thread-safe cover cache handle.
pub type SharedCoverCache = Arc<RwLock<CoverCache>>;

/// Create a new empty shared cover cache.
#[must_use]
pub fn new_cache() -> SharedCoverCache {
    Arc::new(RwLock::new(CoverCache::default()))
}

/// In-memory store mapping zone indices or static keys to their cover art.
#[derive(Default)]
pub struct CoverCache {
    entries: HashMap<usize, CoverEntry>,
    static_entries: HashMap<String, CoverEntry>,
}

/// A single cached cover image with its MIME type and content hash.
pub struct CoverEntry {
    /// Raw image bytes.
    pub bytes: Vec<u8>,
    /// MIME type (e.g. `image/jpeg`).
    pub mime: String,
    /// CRC32 hex hash for `ETag` / cache validation.
    pub hash: String,
}

impl CoverEntry {
    fn new(bytes: Vec<u8>, mime: String) -> Self {
        let hash = format!("{:08x}", crc32fast::hash(&bytes));
        Self { bytes, mime, hash }
    }
}

impl CoverCache {
    /// Store cover art for a zone.
    pub fn set(&mut self, zone_index: usize, bytes: Vec<u8>, mime: String) {
        self.entries
            .insert(zone_index, CoverEntry::new(bytes, mime));
    }

    /// Store cover art with a static key.
    pub fn set_static(&mut self, key: &str, bytes: Vec<u8>, mime: String) {
        self.static_entries
            .insert(key.to_string(), CoverEntry::new(bytes, mime));
    }

    /// Store cover art with auto-detected MIME from magic bytes.
    pub fn set_auto_mime(&mut self, zone_index: usize, bytes: Vec<u8>) {
        let mime = detect_mime(&bytes).to_string();
        self.entries
            .insert(zone_index, CoverEntry::new(bytes, mime));
    }

    /// Get cover art for a zone.
    #[must_use]
    pub fn get(&self, zone_index: usize) -> Option<&CoverEntry> {
        self.entries.get(&zone_index)
    }

    /// Get cover art by a static key.
    #[must_use]
    pub fn get_static(&self, key: &str) -> Option<&CoverEntry> {
        self.static_entries.get(key)
    }

    /// Clear cover art for a zone.
    pub fn clear(&mut self, zone_index: usize) {
        self.entries.remove(&zone_index);
    }
}

/// Simple percent-decoding for data: URI payloads.
fn percent_decode_bytes(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().and_then(|c| (c as char).to_digit(16));
            let lo = chars.next().and_then(|c| (c as char).to_digit(16));
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
            }
        } else {
            out.push(b);
        }
    }
    out
}
/// Detect image MIME type from magic bytes. Falls back to `application/octet-stream`.
#[must_use]
pub fn detect_mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [0x89, 0x50, 0x4E, 0x47, ..] => "image/png",
        [0x00, 0x00, 0x01 | 0x02, 0x00, ..] => "image/x-icon",
        [
            0x52,
            0x49,
            0x46,
            0x46,
            _,
            _,
            _,
            _,
            0x57,
            0x45,
            0x42,
            0x50,
            ..,
        ] => "image/webp",
        _ if bytes.starts_with(b"<svg")
            || bytes.starts_with(b"<?xml") && bytes.windows(4).any(|w| w == b"<svg") =>
        {
            "image/svg+xml"
        }
        _ => "application/octet-stream",
    }
}

/// Fetch cover art from a URL, returning (bytes, mime).
pub async fn fetch_cover(url: &str) -> Option<(Vec<u8>, String)> {
    // Handle data: URIs (e.g. data:image/svg+xml;charset=US-ASCII,%3Csvg...)
    if let Some(rest) = url.strip_prefix("data:") {
        let (header, data) = rest.split_once(',')?;
        let mime = header
            .split(';')
            .next()
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = percent_decode_bytes(data);
        if bytes.len() as u64 > MAX_COVER_ART_BYTES {
            tracing::debug!(url, size = bytes.len(), "Data URI cover art too large");
            return None;
        }
        return Some((bytes, mime));
    }

    if !is_http_url(url) {
        tracing::debug!(url, "Ignoring non-HTTP cover art URL");
        return None;
    }

    let client = reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .connect_timeout(crate::audio::CONNECT_TIMEOUT)
        .timeout(COVER_FETCH_TIMEOUT)
        .build()
        .inspect_err(
            |e| tracing::debug!(error = %e, url, "Failed to build HTTP client for cover fetch"),
        )
        .ok()?;
    let resp = client
        .get(url)
        .send()
        .await
        .inspect_err(|e| tracing::debug!(error = %e, url, "Cover art request failed"))
        .ok()?
        .error_for_status()
        .inspect_err(|e| tracing::debug!(error = %e, url, "Cover art HTTP error"))
        .ok()?;
    let mime = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = response_bytes_limited(resp, MAX_COVER_ART_BYTES, url, "cover art").await?;
    Some((bytes, mime))
}

/// Fetch cover art with favicon fallback.
/// Tries `cover_url` first; if None or fetch fails, extracts the largest favicon from the stream URL's domain.
pub async fn fetch_cover_with_favicon_fallback(
    cover_url: Option<&str>,
    stream_url: &str,
) -> Option<(Vec<u8>, String)> {
    if let Some(url) = cover_url
        && let Some((bytes, _)) = fetch_cover(url).await
    {
        let mime = detect_mime(&bytes);
        if mime.contains("icon")
            && let Some(converted) = ico_to_png(&bytes)
        {
            return Some(converted);
        }
        if mime.starts_with("image/") {
            tracing::debug!(url, %mime, "Found cover via config URL");
            return Some((bytes, mime.to_string()));
        }
        tracing::debug!(url, %mime, "Config cover URL returned non-image content");
    }
    let base = url::Url::parse(stream_url).ok().and_then(|u| {
        let scheme = u.scheme();
        let host = u.host_str()?;
        let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
        Some(format!("{scheme}://{host}{port}"))
    })?;

    tracing::debug!(%base, "Falling back to favicon search");
    fetch_best_favicon(&base).await
}

/// Fetch the best (largest) favicon from a website.
async fn fetch_best_favicon(base_url: &str) -> Option<(Vec<u8>, String)> {
    if !is_http_url(base_url) {
        return None;
    }
    let client = reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;

    let resp = client
        .get(base_url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let html_bytes =
        response_bytes_limited(resp, MAX_FAVICON_HTML_BYTES, base_url, "favicon HTML").await?;
    let html = String::from_utf8(html_bytes).ok()?;
    let icon_url =
        parse_best_icon_url(&html, base_url).unwrap_or_else(|| format!("{base_url}/favicon.ico"));

    let resp = client
        .get(&icon_url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let mime = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/x-icon")
        .to_string();
    let bytes = response_bytes_limited(resp, MAX_COVER_ART_BYTES, &icon_url, "favicon").await?;

    // Skip tiny responses (tracking pixels, empty files)
    if bytes.len() < 100 {
        return None;
    }

    // Detect actual format from bytes (servers often lie about content-type)
    let detected = detect_mime(&bytes);

    if detected == "image/svg+xml" || mime.contains("svg") {
        return Some((bytes, "image/svg+xml".into()));
    }

    if detected == "image/x-icon"
        || mime.contains("icon")
        || mime.contains("octet-stream")
        || std::path::Path::new(&icon_url)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ico"))
    {
        return ico_to_png(&bytes);
    }

    if detected.starts_with("image/") {
        return Some((bytes, detected.into()));
    }

    None
}

/// Parse HTML for the largest icon link.
fn parse_best_icon_url(html: &str, origin: &str) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for line in html.lines() {
        let lower = line.to_lowercase();
        if !lower.contains("rel=") || !lower.contains("icon") {
            continue;
        }
        let Some(href) = extract_attr(line, "href") else {
            continue;
        };
        if href.is_empty() {
            continue;
        }
        let size = extract_attr(line, "sizes")
            .and_then(|s| s.split('x').next()?.parse::<u32>().ok())
            .unwrap_or(0);
        let url = if href.starts_with("http") {
            href
        } else if href.starts_with("//") {
            format!("https:{href}")
        } else if href.starts_with('/') {
            format!("{origin}{href}")
        } else {
            format!("{origin}/{href}")
        };
        if best.as_ref().is_none_or(|(s, _)| size > *s) {
            best = Some((size, url));
        }
    }
    best.map(|(_, url)| url)
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    // Search the original string, ASCII case-insensitively: lowercasing first can change byte
    // lengths (`İ` becomes three bytes), so an offset into the lowercased copy may not even be
    // a char boundary of the original. An ASCII match always starts on a char boundary.
    let needle = format!("{attr}=");
    let pos = tag
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))?
        + needle.len();
    let rest = &tag[pos..];
    let (quote, rest) = if let Some(stripped) = rest.strip_prefix('"') {
        ('"', stripped)
    } else if let Some(stripped) = rest.strip_prefix('\'') {
        ('\'', stripped)
    } else {
        return rest.split_whitespace().next().map(String::from);
    };
    rest.split(quote).next().map(String::from)
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
}

async fn response_bytes_limited(
    response: reqwest::Response,
    limit: u64,
    url: &str,
    label: &str,
) -> Option<Vec<u8>> {
    crate::audio::read_response_bytes_limited(response, limit, label)
        .await
        .inspect_err(
            |e| tracing::debug!(error = %format!("{e:#}"), url, label, "Remote body not read"),
        )
        .ok()
        .map(|body| body.to_vec())
}

/// Convert ICO to PNG using the `image` crate.
fn ico_to_png(data: &[u8]) -> Option<(Vec<u8>, String)> {
    use image::ImageReader;
    use std::io::Cursor;
    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    // Limit to 4MB total pixels (e.g. 2048x2048) and 10MB input to prevent OOM/DoS
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(10 * 1024 * 1024);
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    reader.limits(limits);

    let img = reader.decode().ok()?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some((buf.into_inner(), "image/png".into()))
}

/// A cover as an absolute URL for MQTT and MPRIS.
///
/// A value that already has a scheme (`https://…`, `data:…`) stays unchanged, anything else
/// becomes a path below `base_url`, with exactly one slash between them
/// (`/api/v1/zones/1/cover` and `covers/x.png` alike).
#[must_use]
pub fn absolute_cover_url(base_url: &str, url: &str) -> String {
    if url::Url::parse(url).is_ok() {
        return url.to_owned();
    }
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        url.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cover_becomes_one_absolute_url() {
        let base = "https://audio.example";
        assert_eq!(
            absolute_cover_url(base, "https://cdn.example/a.png"),
            "https://cdn.example/a.png"
        );
        assert_eq!(
            absolute_cover_url(base, "data:image/png;base64,AAAA"),
            "data:image/png;base64,AAAA"
        );
        assert_eq!(
            absolute_cover_url(base, "/api/v1/zones/1/cover?h=1"),
            "https://audio.example/api/v1/zones/1/cover?h=1"
        );
        assert_eq!(
            absolute_cover_url("http://host:5555/", "covers/x.png"),
            "http://host:5555/covers/x.png",
            "a path without a leading slash still gets one"
        );
    }

    #[test]
    fn detects_jpeg() {
        assert_eq!(detect_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
    }

    #[test]
    fn detects_png() {
        assert_eq!(detect_mime(&[0x89, 0x50, 0x4E, 0x47]), "image/png");
    }

    #[test]
    fn detects_webp() {
        let webp = [0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50];
        assert_eq!(detect_mime(&webp), "image/webp");
    }

    #[test]
    fn extract_attr_survives_characters_that_grow_when_lowercased() {
        // `İ` (2 bytes) lowercases to `i̇` (3 bytes): an offset into a lowercased copy would
        // point into the middle of a character of the original and panic.
        let tag = r#"<link title="İİİİ" REL="icon" HREF="/fav.png" sizes="32x32">"#;
        assert_eq!(extract_attr(tag, "href").as_deref(), Some("/fav.png"));
        assert_eq!(extract_attr(tag, "sizes").as_deref(), Some("32x32"));
        assert_eq!(extract_attr(tag, "rel").as_deref(), Some("icon"));
        assert_eq!(
            extract_attr("<link İ href=x.ico >", "href").as_deref(),
            Some("x.ico")
        );
        assert_eq!(extract_attr("<link title=\"İ\">", "href"), None);
    }

    #[test]
    fn unknown_fallback() {
        assert_eq!(detect_mime(&[0x00, 0x01]), "application/octet-stream");
    }

    #[test]
    fn cache_set_get_clear() {
        let mut cache = CoverCache::default();
        cache.set(1, vec![0xFF, 0xD8], "image/jpeg".into());
        assert!(cache.get(1).is_some());
        assert_eq!(cache.get(1).unwrap().mime, "image/jpeg");
        cache.clear(1);
        assert!(cache.get(1).is_none());
    }
}
