// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Disk-backed track cache with LRU eviction.
//!
//! Caches Subsonic track downloads on disk for instant seekable playback and
//! look-ahead prefetch. Files transition through `.partial` → complete states.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::SubsonicCacheConfig;

/// File extension for in-progress downloads.
const PARTIAL_EXT: &str = "partial";
/// Index file name within the cache directory.
const INDEX_FILE: &str = "index.json";

// ── Public types ──────────────────────────────────────────────

/// Result of looking up a track in the cache.
pub enum CacheEntry {
    /// File fully downloaded — ready for instant seekable playback.
    Complete {
        path: PathBuf,
        content_type: String,
    },
    /// Download in progress — partial file available.
    Partial {
        path: PathBuf,
        content_type: String,
        bytes_written: u64,
        total_bytes: Option<u64>,
    },
    /// Not in cache.
    Miss,
}

/// Handle for writing bytes to a cache entry during download.
pub struct CacheWriter {
    file: fs::File,
    partial_path: PathBuf,
    final_path: PathBuf,
    track_id: String,
    content_type: String,
    bytes_written: u64,
    total_bytes: Option<u64>,
    cache: TrackCache,
}

impl CacheWriter {
    /// Append bytes to the partial file.
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.file
            .write_all(data)
            .context("Failed to write to cache file")?;
        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Current number of bytes written.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Total expected bytes (from Content-Length), if known.
    pub fn total_bytes(&self) -> Option<u64> {
        self.total_bytes
    }

    /// Finalize the download — rename `.partial` to final path and update index.
    pub fn complete(self) -> Result<PathBuf> {
        drop(self.file);
        fs::rename(&self.partial_path, &self.final_path)
            .context("Failed to rename partial cache file")?;
        self.cache.mark_complete(
            &self.track_id,
            &self.content_type,
            self.bytes_written,
        );
        Ok(self.final_path)
    }

    /// Abort the download — remove the partial file.
    pub fn abort(self) {
        drop(self.file);
        let _ = fs::remove_file(&self.partial_path);
    }

    /// Path to the partial file (for reading while downloading).
    pub fn partial_path(&self) -> &Path {
        &self.partial_path
    }
}

// ── TrackCache ────────────────────────────────────────────────

/// Disk-backed LRU track cache.
#[derive(Clone)]
pub struct TrackCache {
    config: SubsonicCacheConfig,
    index: std::sync::Arc<Mutex<CacheIndex>>,
}

impl TrackCache {
    /// Create or open a track cache at the configured path.
    pub fn new(config: &SubsonicCacheConfig) -> Result<Self> {
        fs::create_dir_all(&config.path)
            .with_context(|| format!("Failed to create cache dir: {}", config.path))?;

        let index_path = Path::new(&config.path).join(INDEX_FILE);
        let index = if index_path.exists() {
            let data = fs::read_to_string(&index_path).unwrap_or_default();
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            CacheIndex::default()
        };

        Ok(Self {
            config: config.clone(),
            index: std::sync::Arc::new(Mutex::new(index)),
        })
    }

    /// Look up a track. Updates LRU timestamp on hit.
    pub fn get(&self, track_id: &str) -> CacheEntry {
        let mut idx = self.index.lock().unwrap_or_else(|e| e.into_inner());

        // Check for complete file
        if let Some(pos) = idx.entries.iter().position(|e| e.track_id == track_id) {
            let path = Path::new(&self.config.path).join(&idx.entries[pos].filename);
            if path.exists() {
                idx.entries[pos].last_accessed = now_epoch_secs();
                let content_type = idx.entries[pos].content_type.clone();
                self.persist_index(&idx);
                return CacheEntry::Complete { path, content_type };
            }
            // File missing — remove stale index entry
            idx.entries.remove(pos);
            self.persist_index(&idx);
        }

        // Check for partial file
        let partial = Path::new(&self.config.path).join(format!("{track_id}.{PARTIAL_EXT}"));
        if partial.exists() {
            let bytes_written = partial.metadata().map(|m| m.len()).unwrap_or(0);
            return CacheEntry::Partial {
                path: partial,
                content_type: String::new(), // unknown for partial without index
                bytes_written,
                total_bytes: None,
            };
        }

        CacheEntry::Miss
    }

    /// Start downloading a track to cache. Returns a writer handle.
    pub fn start_download(
        &self,
        track_id: &str,
        content_type: &str,
        total_bytes: Option<u64>,
    ) -> Result<CacheWriter> {
        let ext = ext_for_content_type(content_type);
        let final_path = Path::new(&self.config.path).join(format!("{track_id}.{ext}"));
        let partial_path = Path::new(&self.config.path).join(format!("{track_id}.{PARTIAL_EXT}"));

        // Remove any existing partial
        let _ = fs::remove_file(&partial_path);

        let file = fs::File::create(&partial_path)
            .with_context(|| format!("Failed to create cache file: {}", partial_path.display()))?;

        Ok(CacheWriter {
            file,
            partial_path,
            final_path,
            track_id: track_id.to_string(),
            content_type: content_type.to_string(),
            bytes_written: 0,
            total_bytes,
            cache: self.clone(),
        })
    }

    /// Check if a track is fully cached.
    pub fn is_complete(&self, track_id: &str) -> bool {
        matches!(self.get(track_id), CacheEntry::Complete { .. })
    }

    /// Evict oldest entries until total size ≤ max_size_mb.
    pub fn evict_lru(&self) {
        let max_bytes = self.config.max_size_mb * 1024 * 1024;
        let mut idx = self.index.lock().unwrap_or_else(|e| e.into_inner());

        let total: u64 = idx.entries.iter().map(|e| e.size_bytes).sum();
        if total <= max_bytes {
            return;
        }

        // Sort by last_accessed ascending (oldest first)
        idx.entries.sort_by_key(|e| e.last_accessed);

        let mut current = total;
        while current > max_bytes {
            let Some(entry) = idx.entries.first() else {
                break;
            };
            let path = Path::new(&self.config.path).join(&entry.filename);
            let size = entry.size_bytes;
            tracing::debug!(track = %entry.track_id, size, "Evicting cached track");
            let _ = fs::remove_file(&path);
            idx.entries.remove(0);
            current -= size;
        }

        self.persist_index(&idx);
    }

    fn mark_complete(&self, track_id: &str, content_type: &str, size_bytes: u64) {
        let ext = ext_for_content_type(content_type);
        let filename = format!("{track_id}.{ext}");
        let mut idx = self.index.lock().unwrap_or_else(|e| e.into_inner());

        // Remove any existing entry for this track
        idx.entries.retain(|e| e.track_id != track_id);

        idx.entries.push(CacheIndexEntry {
            track_id: track_id.to_string(),
            filename,
            content_type: content_type.to_string(),
            size_bytes,
            last_accessed: now_epoch_secs(),
        });

        self.persist_index(&idx);
        drop(idx);

        // Evict if over limit
        self.evict_lru();
    }

    fn persist_index(&self, idx: &CacheIndex) {
        let path = Path::new(&self.config.path).join(INDEX_FILE);
        if let Ok(data) = serde_json::to_string_pretty(idx) {
            let _ = fs::write(path, data);
        }
    }
}

// ── Index ─────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheIndex {
    entries: Vec<CacheIndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheIndexEntry {
    track_id: String,
    filename: String,
    content_type: String,
    size_bytes: u64,
    last_accessed: u64,
}

// ── Helpers ───────────────────────────────────────────────────

fn ext_for_content_type(ct: &str) -> &'static str {
    match ct {
        t if t.contains("mp4") || t.contains("m4a") => "m4a",
        t if t.contains("mp3") || t.contains("mpeg") => "mp3",
        t if t.contains("flac") => "flac",
        t if t.contains("ogg") || t.contains("opus") => "ogg",
        t if t.contains("aac") => "aac",
        t if t.contains("wav") => "wav",
        _ => "bin",
    }
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_miss_on_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = SubsonicCacheConfig {
            enabled: true,
            path: dir.path().to_string_lossy().into_owned(),
            max_size_mb: 100,
            lookahead: 2,
        };
        let cache = TrackCache::new(&config).unwrap();
        assert!(matches!(cache.get("nonexistent"), CacheEntry::Miss));
    }

    #[test]
    fn write_and_complete() {
        let dir = tempfile::tempdir().unwrap();
        let config = SubsonicCacheConfig {
            enabled: true,
            path: dir.path().to_string_lossy().into_owned(),
            max_size_mb: 100,
            lookahead: 2,
        };
        let cache = TrackCache::new(&config).unwrap();

        let mut writer = cache.start_download("track1", "audio/mpeg", Some(1024)).unwrap();
        writer.write(&[0u8; 512]).unwrap();
        writer.write(&[1u8; 512]).unwrap();
        assert_eq!(writer.bytes_written(), 1024);
        let path = writer.complete().unwrap();

        assert!(path.exists());
        assert_eq!(path.extension().unwrap(), "mp3");

        match cache.get("track1") {
            CacheEntry::Complete { path: p, content_type } => {
                assert_eq!(p, path);
                assert_eq!(content_type, "audio/mpeg");
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[test]
    fn eviction_removes_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let config = SubsonicCacheConfig {
            enabled: true,
            path: dir.path().to_string_lossy().into_owned(),
            max_size_mb: 0, // force eviction
            lookahead: 2,
        };
        let cache = TrackCache::new(&config).unwrap();

        let mut w = cache.start_download("old", "audio/mpeg", None).unwrap();
        w.write(&[0u8; 100]).unwrap();
        w.complete().unwrap();

        // Eviction should have removed it since max_size_mb = 0
        assert!(matches!(cache.get("old"), CacheEntry::Miss));
    }
}
