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
///
/// The HTTP chunks do not follow the metadata interval, so a length byte or a metadata block
/// can end in the middle of a chunk. The processor keeps its position across `process` calls.
pub struct IcyProcessor {
    metaint: usize,
    /// Audio bytes since the last metadata block.
    audio_count: usize,
    /// Metadata bytes still to come when a block is in progress.
    meta_remaining: Option<usize>,
    /// The part of the current metadata block received so far.
    meta_buf: Vec<u8>,
    meta_tx: mpsc::Sender<IcyMetadata>,
}

impl IcyProcessor {
    /// Create from the `icy-metaint` response header value.
    pub fn new(metaint: usize, meta_tx: mpsc::Sender<IcyMetadata>) -> Self {
        tracing::debug!(metaint, "ICY metadata enabled");
        Self {
            metaint,
            audio_count: 0,
            meta_remaining: None,
            meta_buf: Vec::new(),
            meta_tx,
        }
    }

    /// Process a chunk from the HTTP stream. Returns only audio bytes.
    /// Metadata blocks are stripped and parsed asynchronously.
    pub fn process(&mut self, mut data: Bytes) -> Vec<u8> {
        let mut audio = Vec::with_capacity(data.len());

        while !data.is_empty() {
            if let Some(remaining) = self.meta_remaining {
                // Inside a metadata block, possibly continued from the previous chunk.
                let take = remaining.min(data.len());
                self.meta_buf.extend_from_slice(&data[..take]);
                data = data.slice(take..);
                if take == remaining {
                    if let Some(meta) = parse_icy_metadata(&self.meta_buf) {
                        let _ = self.meta_tx.try_send(meta);
                    }
                    self.meta_buf.clear();
                    self.meta_remaining = None;
                    self.audio_count = 0;
                } else {
                    self.meta_remaining = Some(remaining - take);
                }
            } else if self.audio_count < self.metaint {
                let take = (self.metaint - self.audio_count).min(data.len());
                audio.extend_from_slice(&data[..take]);
                data = data.slice(take..);
                self.audio_count += take;
            } else {
                // The length byte: the block that follows is 16 times as long.
                let meta_len = usize::from(data[0]) * 16;
                data = data.slice(1..);
                if meta_len == 0 {
                    self.audio_count = 0;
                } else {
                    self.meta_remaining = Some(meta_len);
                }
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

/// Build a reqwest client that requests ICY metadata, with the connect and read timeouts of
/// [`super::streaming_client_builder`]: a stream that stalls fails instead of hanging.
///
/// # Errors
///
/// Returns an error if the TLS backend cannot be initialised.
pub fn icy_client() -> anyhow::Result<reqwest::Client> {
    use anyhow::Context;
    super::streaming_client_builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                "Icy-MetaData",
                reqwest::header::HeaderValue::from_static("1"),
            );
            h
        })
        .build()
        .context("Failed to build HTTP client")
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

    /// A stream of `blocks` intervals of `metaint` audio bytes, each followed by a metadata
    /// block with a title (the second one empty: a length byte of 0).
    fn stream(metaint: usize, blocks: usize) -> (Vec<u8>, Vec<u8>) {
        let mut wire = Vec::new();
        let mut audio = Vec::new();
        let mut counter = 0u8;
        for i in 0..blocks {
            for _ in 0..metaint {
                counter = counter.wrapping_add(1);
                wire.push(counter);
                audio.push(counter);
            }
            if i % 2 == 1 {
                wire.push(0);
            } else {
                let mut meta = format!("StreamTitle='Title {i}';").into_bytes();
                meta.resize(meta.len().div_ceil(16) * 16, 0);
                wire.push(u8::try_from(meta.len() / 16).unwrap());
                wire.extend_from_slice(&meta);
            }
        }
        (wire, audio)
    }

    fn titles(rx: &mut mpsc::Receiver<IcyMetadata>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.extend(m.title);
        }
        out
    }

    #[test]
    fn a_stream_split_at_every_position_gives_the_same_audio_and_titles() {
        let metaint = 40;
        let (wire, expected_audio) = stream(metaint, 3);
        for split in 1..wire.len() {
            let (tx, mut rx) = mpsc::channel(8);
            let mut proc = IcyProcessor::new(metaint, tx);
            let mut audio = proc.process(Bytes::copy_from_slice(&wire[..split]));
            audio.extend(proc.process(Bytes::copy_from_slice(&wire[split..])));
            assert_eq!(audio, expected_audio, "split at {split}");
            assert_eq!(titles(&mut rx), ["Title 0", "Title 2"], "split at {split}");
        }
    }

    #[test]
    fn a_stream_fed_one_byte_at_a_time_gives_the_same_audio_and_titles() {
        let metaint = 40;
        let (wire, expected_audio) = stream(metaint, 4);
        let (tx, mut rx) = mpsc::channel(8);
        let mut proc = IcyProcessor::new(metaint, tx);
        let mut audio = Vec::new();
        for byte in &wire {
            audio.extend(proc.process(Bytes::copy_from_slice(std::slice::from_ref(byte))));
        }
        assert_eq!(audio, expected_audio);
        assert_eq!(titles(&mut rx), ["Title 0", "Title 2"]);
    }
}
