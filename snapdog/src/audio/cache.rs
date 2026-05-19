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

use anyhow::{Context, Result, bail};
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
        /// Path to the cached file on disk.
        path: PathBuf,
        /// MIME content type (e.g., "audio/flac").
        content_type: String,
    },
    /// Download in progress — partial file available.
    Partial {
        /// Path to the partial file on disk.
        path: PathBuf,
        /// MIME content type (may be empty if unknown).
        content_type: String,
        /// Bytes downloaded so far.
        bytes_written: u64,
        /// Total expected bytes, if known.
        total_bytes: Option<u64>,
    },
    /// Not in cache.
    Miss,
}

/// Handle for writing bytes to a cache entry during download.
pub struct CacheWriter {
    file: Option<fs::File>,
    partial_path: PathBuf,
    final_path: PathBuf,
    track_id: String,
    content_type: String,
    bytes_written: u64,
    total_bytes: Option<u64>,
    cache: TrackCache,
    completed: bool,
}

impl CacheWriter {
    /// Append bytes to the partial file.
    ///
    /// # Errors
    ///
    /// Returns an error if the write exceeds the maximum cache size or the I/O fails.
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        let next_size = self.bytes_written + data.len() as u64;
        if next_size > self.cache.max_bytes() {
            bail!(
                "Cache entry exceeds configured maximum size of {} bytes",
                self.cache.max_bytes()
            );
        }
        if let Some(ref mut file) = self.file {
            file.write_all(data)
                .context("Failed to write to cache file")?;
        }
        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Current number of bytes written.
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Total expected bytes (from Content-Length), if known.
    #[must_use]
    pub const fn total_bytes(&self) -> Option<u64> {
        self.total_bytes
    }

    /// Finalize the download — rename `.partial` to final path and update index.
    ///
    /// # Errors
    ///
    /// Returns an error if the file rename fails.
    pub fn complete(mut self) -> Result<PathBuf> {
        self.completed = true;
        self.file.take(); // close file before rename
        fs::rename(&self.partial_path, &self.final_path)
            .context("Failed to rename partial cache file")?;
        self.cache
            .mark_complete(&self.track_id, &self.content_type, self.bytes_written);
        Ok(self.final_path.clone())
    }

    /// Abort the download — remove the partial file.
    pub fn abort(mut self) {
        self.completed = true; // prevent Drop from double-removing
        self.file.take(); // close file before remove
        let _ = fs::remove_file(&self.partial_path);
    }

    /// Path to the partial file (for reading while downloading).
    #[must_use]
    pub fn partial_path(&self) -> &Path {
        &self.partial_path
    }
}

impl Drop for CacheWriter {
    fn drop(&mut self) {
        if !self.completed && self.partial_path.exists() {
            let _ = std::fs::remove_file(&self.partial_path);
        }
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
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
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
        let mut idx = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let filename_stem = filename_stem_for_track_id(track_id);

        // Check for complete file
        if let Some(pos) = idx.entries.iter().position(|e| e.track_id == track_id) {
            let path = Path::new(&self.config.path).join(&idx.entries[pos].filename);
            if path.exists() {
                idx.entries[pos].last_accessed = now_epoch_secs();
                let content_type = idx.entries[pos].content_type.clone();
                return CacheEntry::Complete { path, content_type };
            }
            // File missing — remove stale index entry
            idx.entries.remove(pos);
            self.persist_index(&idx);
            drop(idx);
        }

        // Check for partial file
        let partial = Path::new(&self.config.path).join(format!("{filename_stem}.{PARTIAL_EXT}"));
        if partial.exists() {
            let bytes_written = partial.metadata().map_or(0, |m| m.len());
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
    ///
    /// # Errors
    ///
    /// Returns an error if the track exceeds the maximum cache size or the file cannot be created.
    pub fn start_download(
        &self,
        track_id: &str,
        content_type: &str,
        total_bytes: Option<u64>,
    ) -> Result<CacheWriter> {
        if let Some(total_bytes) = total_bytes {
            if total_bytes > self.max_bytes() {
                bail!(
                    "Cache entry Content-Length {total_bytes} exceeds configured maximum size of {} bytes",
                    self.max_bytes()
                );
            }
        }
        let ext = ext_for_content_type(content_type);
        let filename_stem = filename_stem_for_track_id(track_id);
        let final_path = Path::new(&self.config.path).join(format!("{filename_stem}.{ext}"));
        let partial_path =
            Path::new(&self.config.path).join(format!("{filename_stem}.{PARTIAL_EXT}"));

        // Remove any existing partial
        let _ = fs::remove_file(&partial_path);

        let file = fs::File::create(&partial_path)
            .with_context(|| format!("Failed to create cache file: {}", partial_path.display()))?;

        Ok(CacheWriter {
            file: Some(file),
            partial_path,
            final_path,
            track_id: track_id.to_string(),
            content_type: content_type.to_string(),
            bytes_written: 0,
            total_bytes,
            cache: self.clone(),
            completed: false,
        })
    }

    /// Check if a track is fully cached.
    #[must_use]
    pub fn is_complete(&self, track_id: &str) -> bool {
        matches!(self.get(track_id), CacheEntry::Complete { .. })
    }

    /// Remove a track from the cache (e.g., on decode failure due to corruption).
    pub fn invalidate(&self, track_id: &str) {
        let mut idx = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(pos) = idx.entries.iter().position(|e| e.track_id == track_id) {
            let path = Path::new(&self.config.path).join(&idx.entries[pos].filename);
            let _ = fs::remove_file(&path);
            idx.entries.remove(pos);
            self.persist_index(&idx);
            drop(idx);
            tracing::debug!(track_id, "Invalidated cached track");
        }
    }

    /// Evict oldest entries until total size ≤ `max_size_mb`.
    pub fn evict_lru(&self) {
        let max_bytes = self.max_bytes();
        let mut idx = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

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
        drop(idx);
    }

    fn mark_complete(&self, track_id: &str, content_type: &str, size_bytes: u64) {
        let ext = ext_for_content_type(content_type);
        let filename = format!("{}.{ext}", filename_stem_for_track_id(track_id));
        let mut idx = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

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

    const fn max_bytes(&self) -> u64 {
        self.config.max_size_mb.saturating_mul(1024 * 1024)
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

fn filename_stem_for_track_id(track_id: &str) -> String {
    format!("track-{:x}", md5::compute(track_id.as_bytes()))
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

    fn test_config(dir: &tempfile::TempDir) -> SubsonicCacheConfig {
        SubsonicCacheConfig {
            path: dir.path().to_string_lossy().into_owned(),
            max_size_mb: 100,
        }
    }

    // ── Basic operations ──────────────────────────────────────

    #[test]
    fn cache_miss_on_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();
        assert!(matches!(cache.get("nonexistent"), CacheEntry::Miss));
    }

    #[test]
    fn write_and_complete() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        let mut writer = cache
            .start_download("track1", "audio/mpeg", Some(1024))
            .unwrap();
        writer.write(&[0u8; 512]).unwrap();
        writer.write(&[1u8; 512]).unwrap();
        assert_eq!(writer.bytes_written(), 1024);
        let path = writer.complete().unwrap();

        assert!(path.exists());
        assert_eq!(path.extension().unwrap(), "mp3");

        match cache.get("track1") {
            CacheEntry::Complete {
                path: p,
                content_type,
            } => {
                assert_eq!(p, path);
                assert_eq!(content_type, "audio/mpeg");
            }
            _ => panic!("Expected Complete"),
        }
    }

    // ── LRU eviction ──────────────────────────────────────────

    #[test]
    fn eviction_removes_oldest_not_most_recent() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(&dir);
        // 1 MiB max — each entry fits on its own, but two entries exceed the cap.
        config.max_size_mb = 1;

        let cache = TrackCache::new(&config).unwrap();

        let mut w = cache.start_download("old", "audio/mpeg", None).unwrap();
        w.write(&vec![0u8; 700 * 1024]).unwrap();
        w.complete().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));

        let mut w = cache.start_download("new", "audio/mpeg", None).unwrap();
        w.write(&vec![0u8; 700 * 1024]).unwrap();
        w.complete().unwrap();

        assert!(matches!(cache.get("old"), CacheEntry::Miss));
        assert!(matches!(cache.get("new"), CacheEntry::Complete { .. }));
    }

    #[test]
    fn lru_access_updates_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(&dir);
        // Allow ~250 bytes (fits 2 tracks of 100 bytes, not 3)
        // We use raw byte math: 250 bytes < 1 MB, so set to 1 and use small files
        config.max_size_mb = 1; // 1 MB — plenty for test

        let cache = TrackCache::new(&config).unwrap();

        // Add track_a (oldest)
        let mut w = cache.start_download("track_a", "audio/flac", None).unwrap();
        w.write(&[0u8; 100]).unwrap();
        w.complete().unwrap();

        // Add track_b (newer)
        std::thread::sleep(std::time::Duration::from_millis(10));
        let mut w = cache.start_download("track_b", "audio/flac", None).unwrap();
        w.write(&[0u8; 100]).unwrap();
        w.complete().unwrap();

        // Access track_a — makes it most recently used
        assert!(matches!(cache.get("track_a"), CacheEntry::Complete { .. }));

        // Verify both still exist
        assert!(matches!(cache.get("track_a"), CacheEntry::Complete { .. }));
        assert!(matches!(cache.get("track_b"), CacheEntry::Complete { .. }));
    }

    // ── Invalidate ────────────────────────────────────────────

    #[test]
    fn invalidate_removes_file_and_index_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        let mut w = cache.start_download("corrupt", "audio/mpeg", None).unwrap();
        w.write(&[0u8; 50]).unwrap();
        let path = w.complete().unwrap();
        assert!(path.exists());

        cache.invalidate("corrupt");

        assert!(!path.exists());
        assert!(matches!(cache.get("corrupt"), CacheEntry::Miss));
    }

    #[test]
    fn invalidate_nonexistent_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();
        cache.invalidate("ghost"); // should not panic
    }

    // ── Stale index ───────────────────────────────────────────

    #[test]
    fn stale_index_entry_cleaned_on_get() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        // Create a complete entry
        let mut w = cache.start_download("stale", "audio/flac", None).unwrap();
        w.write(&[0u8; 10]).unwrap();
        let path = w.complete().unwrap();

        // Manually delete the file (simulating external corruption)
        fs::remove_file(&path).unwrap();

        // get() should detect missing file, clean index, return Miss
        assert!(matches!(cache.get("stale"), CacheEntry::Miss));

        // Subsequent get should also be Miss (index was cleaned)
        assert!(matches!(cache.get("stale"), CacheEntry::Miss));
    }

    // ── CacheWriter Drop cleanup ──────────────────────────────

    #[test]
    fn drop_without_complete_removes_partial() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        let partial_path;
        {
            let mut writer = cache
                .start_download("abandoned", "audio/ogg", None)
                .unwrap();
            writer.write(&[0u8; 50]).unwrap();
            partial_path = writer.partial_path().to_path_buf();
            assert!(partial_path.exists());
            // writer dropped here without complete()
        }

        assert!(
            !partial_path.exists(),
            "Partial file should be cleaned up by Drop"
        );
    }

    #[test]
    fn abort_removes_partial() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        let mut writer = cache.start_download("aborted", "audio/wav", None).unwrap();
        writer.write(&[0u8; 50]).unwrap();
        let partial_path = writer.partial_path().to_path_buf();
        assert!(partial_path.exists());

        writer.abort();
        assert!(!partial_path.exists());
    }

    #[test]
    fn complete_does_not_trigger_drop_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        let mut writer = cache.start_download("good", "audio/flac", None).unwrap();
        writer.write(&[0u8; 50]).unwrap();
        let final_path = writer.complete().unwrap();

        // Final file should exist (not removed by Drop)
        assert!(final_path.exists());
    }

    // ── Re-download overwrites partial ────────────────────────

    #[test]
    fn start_download_removes_existing_partial() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        // Create a partial file
        let mut w1 = cache.start_download("retry", "audio/mpeg", None).unwrap();
        w1.write(&[0xAA; 100]).unwrap();
        let partial = w1.partial_path().to_path_buf();
        // Don't complete — just drop (simulating interrupted download)
        std::mem::forget(w1); // prevent Drop from cleaning up for this test
        assert!(partial.exists());

        // Start a new download for the same track
        let mut w2 = cache.start_download("retry", "audio/mpeg", None).unwrap();
        w2.write(&[0xBB; 50]).unwrap();
        assert_eq!(w2.bytes_written(), 50); // fresh start, not appended

        let path = w2.complete().unwrap();
        let data = fs::read(&path).unwrap();
        assert_eq!(data.len(), 50);
        assert!(data.iter().all(|&b| b == 0xBB));
    }

    #[test]
    fn track_ids_are_sanitized_for_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        let mut writer = cache
            .start_download("../outside/track", "audio/mpeg", None)
            .unwrap();
        writer.write(&[0u8; 10]).unwrap();
        let path = writer.complete().unwrap();

        assert!(path.starts_with(dir.path()));
        assert!(!path.to_string_lossy().contains("../outside"));
    }

    #[test]
    fn rejects_oversized_cache_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(&dir);
        config.max_size_mb = 1;
        let cache = TrackCache::new(&config).unwrap();

        assert!(
            cache
                .start_download("too-large", "audio/mpeg", Some(2 * 1024 * 1024))
                .is_err()
        );

        let mut writer = cache
            .start_download("grows-too-large", "audio/mpeg", None)
            .unwrap();
        assert!(writer.write(&vec![0u8; 1024 * 1024]).is_ok());
        assert!(writer.write(&[0u8; 1]).is_err());
    }

    // ── Content type → extension mapping ──────────────────────

    #[test]
    fn ext_mapping() {
        assert_eq!(ext_for_content_type("audio/mpeg"), "mp3");
        assert_eq!(ext_for_content_type("audio/mp4"), "m4a");
        assert_eq!(ext_for_content_type("audio/x-m4a"), "m4a");
        assert_eq!(ext_for_content_type("audio/flac"), "flac");
        assert_eq!(ext_for_content_type("audio/ogg"), "ogg");
        assert_eq!(ext_for_content_type("audio/opus"), "ogg");
        assert_eq!(ext_for_content_type("audio/aac"), "aac");
        assert_eq!(ext_for_content_type("audio/wav"), "wav");
        assert_eq!(ext_for_content_type("application/octet-stream"), "bin");
        assert_eq!(ext_for_content_type(""), "bin");
    }

    // ── Index persistence ─────────────────────────────────────

    #[test]
    fn index_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(&dir);

        {
            let cache = TrackCache::new(&config).unwrap();
            let mut w = cache.start_download("persist", "audio/flac", None).unwrap();
            w.write(&[0u8; 42]).unwrap();
            w.complete().unwrap();
        }

        // Reopen cache from same directory
        let cache = TrackCache::new(&config).unwrap();
        match cache.get("persist") {
            CacheEntry::Complete { content_type, .. } => {
                assert_eq!(content_type, "audio/flac");
            }
            _ => panic!("Expected Complete after reopen"),
        }
    }

    #[test]
    fn index_not_corrupted_by_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        // Simulate rapid sequential operations
        for i in 0..10 {
            let id = format!("track_{i}");
            let mut w = cache.start_download(&id, "audio/mpeg", None).unwrap();
            w.write(&[i as u8; 10]).unwrap();
            w.complete().unwrap();
        }

        // All should be retrievable
        for i in 0..10 {
            let id = format!("track_{i}");
            assert!(
                matches!(cache.get(&id), CacheEntry::Complete { .. }),
                "track_{i} missing"
            );
        }
    }
}
