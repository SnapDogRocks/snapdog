// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! ICY metadata parsing for Icecast/Shoutcast streams.
//!
//! Strips inline metadata from the audio byte stream and extracts
//! `StreamTitle` updates (e.g. "Artist - Song").

use bytes::Bytes;
use tokio::sync::mpsc;

/// Parsed ICY metadata update.
#[derive(Debug, Clone)]
pub struct IcyMetadata {
    /// Current stream title (e.g. "Artist - Song").
    pub title: Option<String>,
    /// Stream URL, if provided by the server.
    pub url: Option<String>,
}

/// ICY stream processor. Strips metadata from audio bytes.
pub struct IcyProcessor {
    metaint: usize,
    audio_count: usize,
    meta_tx: mpsc::Sender<IcyMetadata>,
}

impl IcyProcessor {
    /// Create from the `icy-metaint` response header value.
    pub fn new(metaint: usize, meta_tx: mpsc::Sender<IcyMetadata>) -> Self {
        tracing::debug!(metaint, "ICY metadata enabled");
        Self {
            metaint,
            audio_count: 0,
            meta_tx,
        }
    }

    /// Process a chunk from the HTTP stream. Returns only audio bytes.
    /// Metadata blocks are stripped and parsed asynchronously.
    pub fn process(&mut self, mut data: Bytes) -> Vec<u8> {
        let mut audio = Vec::with_capacity(data.len());

        while !data.is_empty() {
            let remaining_audio = self.metaint - self.audio_count;

            if remaining_audio > 0 {
                // Audio bytes
                let take = remaining_audio.min(data.len());
                audio.extend_from_slice(&data[..take]);
                data = data.slice(take..);
                self.audio_count += take;
            } else {
                // Metadata block
                if data.is_empty() {
                    break;
                }
                let meta_len = data[0] as usize * 16;
                data = data.slice(1..);

                if meta_len > 0 && data.len() >= meta_len {
                    let meta_bytes = &data[..meta_len];
                    if let Some(meta) = parse_icy_metadata(meta_bytes) {
                        let _ = self.meta_tx.try_send(meta);
                    }
                    data = data.slice(meta_len..);
                } else if meta_len > 0 {
                    // Incomplete metadata block — skip (rare edge case)
                    tracing::warn!(
                        expected = meta_len,
                        got = data.len(),
                        "Incomplete ICY metadata block"
                    );
                    data = Bytes::new();
                }

                self.audio_count = 0;
            }
        }

        audio
    }
}

/// Parse ICY metadata string like `StreamTitle='Artist - Song';StreamUrl='http://...';`
fn parse_icy_metadata(bytes: &[u8]) -> Option<IcyMetadata> {
    let s = std::str::from_utf8(bytes).ok()?.trim_end_matches('\0');
    if s.is_empty() {
        return None;
    }

    let title = extract_field(s, "StreamTitle");
    let url = extract_field(s, "StreamUrl");

    if title.is_none() && url.is_none() {
        return None;
    }

    Some(IcyMetadata { title, url })
}

fn extract_field(s: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}='");
    let start = s.find(&prefix)? + prefix.len();
    let end = s[start..].find("';")?;
    let value = &s[start..start + end];
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parse the `icy-metaint` header from an HTTP response.
#[must_use]
pub fn parse_metaint(response: &reqwest::Response) -> Option<usize> {
    response
        .headers()
        .get("icy-metaint")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// Build a reqwest client that requests ICY metadata.
#[must_use]
pub fn icy_client() -> reqwest::Client {
    reqwest::Client::builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                "Icy-MetaData",
                reqwest::header::HeaderValue::from_static("1"),
            );
            h
        })
        .build()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stream_title() {
        let meta = b"StreamTitle='Beethoven - Moonlight Sonata';StreamUrl='';";
        let result = parse_icy_metadata(meta).unwrap();
        assert_eq!(result.title.unwrap(), "Beethoven - Moonlight Sonata");
        assert!(result.url.is_none());
    }

    #[test]
    fn parses_title_and_url() {
        let meta = b"StreamTitle='News';StreamUrl='http://example.com';";
        let result = parse_icy_metadata(meta).unwrap();
        assert_eq!(result.title.unwrap(), "News");
        assert_eq!(result.url.unwrap(), "http://example.com");
    }

    #[test]
    fn handles_empty_metadata() {
        let meta = b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        assert!(parse_icy_metadata(meta).is_none());
    }

    #[test]
    fn strips_metadata_from_stream() {
        let (tx, _rx) = mpsc::channel(4);
        let mut proc = IcyProcessor::new(8, tx);

        // 8 bytes audio + 1 byte meta_len(1) + 16 bytes metadata + 4 bytes audio
        let mut data = Vec::new();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]); // 8 audio bytes
        data.push(1); // meta_len = 1 * 16 = 16 bytes
        data.extend_from_slice(b"StreamTitle='X';"); // exactly 16 bytes
        data.extend_from_slice(&[9, 10, 11, 12]); // 4 more audio bytes

        let audio = proc.process(Bytes::from(data));
        assert_eq!(audio, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }
}
