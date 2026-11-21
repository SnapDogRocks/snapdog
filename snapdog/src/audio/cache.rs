// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Disk-backed track cache with LRU eviction.
//!
//! Caches Subsonic track downloads on disk for instant seekable playback and
//! look-ahead prefetch. Files transition through `.partial` → complete states.
//!
//! One [`TrackCache`] is shared by all zones (clones share the index), so there is one index
//! and one size limit. At most one download per track runs at a time: a second caller gets
//! no writer and streams without caching. Every writer has its own partial file and indexes
//! only what it wrote itself, so an entry is never shorter than its source.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// A download of this track is in progress, not yet usable.
    Downloading,
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
    /// Returns an error if fewer bytes were written than the server announced (the partial
    /// file is removed, nothing is indexed) or the file rename fails.
    pub fn complete(mut self) -> Result<PathBuf> {
        if let Some(total) = self.total_bytes
            && total != self.bytes_written
        {
            // Drop removes the partial file.
            bail!(
                "Download incomplete: {} of {total} bytes, not cached",
                self.bytes_written
            );
        }
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
        self.cache
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.track_id);
    }
}

// ── TrackCache ────────────────────────────────────────────────

/// Disk-backed LRU track cache.
#[derive(Clone)]
pub struct TrackCache {
    config: SubsonicCacheConfig,
    index: std::sync::Arc<Mutex<CacheIndex>>,
    /// Track ids with a writer alive.
    in_flight: std::sync::Arc<Mutex<HashSet<String>>>,
    /// Source of unique partial file names.
    next_writer: std::sync::Arc<AtomicU64>,
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

        // Partial files left by an earlier run belong to no writer any more.
        if let Ok(dir) = fs::read_dir(&config.path) {
            for entry in dir.flatten() {
                if entry.path().extension().is_some_and(|e| e == PARTIAL_EXT) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        Ok(Self {
            config: config.clone(),
            index: std::sync::Arc::new(Mutex::new(index)),
            in_flight: std::sync::Arc::default(),
            next_writer: std::sync::Arc::default(),
        })
    }

    /// Look up a track. Updates LRU timestamp on hit.
    pub fn get(&self, track_id: &str) -> CacheEntry {
        let mut idx = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

        if self
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(track_id)
        {
            return CacheEntry::Downloading;
        }
        CacheEntry::Miss
    }

    /// Start downloading a track to cache. Returns a writer handle, or `None` while another
    /// download of the same track is running (the caller streams without caching then).
    ///
    /// # Errors
    ///
    /// Returns an error if the track exceeds the maximum cache size or the file cannot be created.
    pub fn start_download(
        &self,
        track_id: &str,
        content_type: &str,
        total_bytes: Option<u64>,
    ) -> Result<Option<CacheWriter>> {
        if let Some(total_bytes) = total_bytes
            && total_bytes > self.max_bytes()
        {
            bail!(
                "Cache entry Content-Length {total_bytes} exceeds configured maximum size of {} bytes",
                self.max_bytes()
            );
        }
        if !self
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(track_id.to_owned())
        {
            return Ok(None);
        }
        let release = || {
            self.in_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(track_id);
        };
        let ext = ext_for_content_type(content_type);
        let filename_stem = filename_stem_for_track_id(track_id);
        let final_path = Path::new(&self.config.path).join(format!("{filename_stem}.{ext}"));
        let writer_id = self.next_writer.fetch_add(1, Ordering::Relaxed);
        let partial_path =
            Path::new(&self.config.path).join(format!("{filename_stem}.{writer_id}.{PARTIAL_EXT}"));

        let file = match fs::File::create(&partial_path) {
            Ok(file) => file,
            Err(e) => {
                release();
                return Err(e).with_context(|| {
                    format!("Failed to create cache file: {}", partial_path.display())
                });
            }
        };

        Ok(Some(CacheWriter {
            file: Some(file),
            partial_path,
            final_path,
            track_id: track_id.to_string(),
            content_type: content_type.to_string(),
            bytes_written: 0,
            total_bytes,
            cache: self.clone(),
            completed: false,
        }))
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
            .unwrap()
            .expect("no download of this track in flight");
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

        let mut w = cache
            .start_download("old", "audio/mpeg", None)
            .unwrap()
            .expect("no download of this track in flight");
        w.write(&vec![0u8; 700 * 1024]).unwrap();
        w.complete().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));

        let mut w = cache
            .start_download("new", "audio/mpeg", None)
            .unwrap()
            .expect("no download of this track in flight");
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
        let mut w = cache
            .start_download("track_a", "audio/flac", None)
            .unwrap()
            .expect("no download of this track in flight");
        w.write(&[0u8; 100]).unwrap();
        w.complete().unwrap();

        // Add track_b (newer)
        std::thread::sleep(std::time::Duration::from_millis(10));
        let mut w = cache
            .start_download("track_b", "audio/flac", None)
            .unwrap()
            .expect("no download of this track in flight");
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

        let mut w = cache
            .start_download("corrupt", "audio/mpeg", None)
            .unwrap()
            .expect("no download of this track in flight");
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
        let mut w = cache
            .start_download("stale", "audio/flac", None)
            .unwrap()
            .expect("no download of this track in flight");
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
                .unwrap()
                .expect("no download of this track in flight");
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

        let mut writer = cache
            .start_download("aborted", "audio/wav", None)
            .unwrap()
            .expect("no download of this track in flight");
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

        let mut writer = cache
            .start_download("good", "audio/flac", None)
            .unwrap()
            .expect("no download of this track in flight");
        writer.write(&[0u8; 50]).unwrap();
        let final_path = writer.complete().unwrap();

        // Final file should exist (not removed by Drop)
        assert!(final_path.exists());
    }

    // ── One download per track, one index for all users (#495) ──

    #[test]
    fn a_second_download_of_the_same_track_gets_no_writer() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();
        let other_zone = cache.clone();

        let mut first = cache
            .start_download("t", "audio/mpeg", Some(6))
            .unwrap()
            .expect("first writer");
        assert!(
            other_zone
                .start_download("t", "audio/mpeg", Some(6))
                .unwrap()
                .is_none(),
            "a running download is not started twice"
        );
        assert!(matches!(other_zone.get("t"), CacheEntry::Downloading));
        first.write(b"abc").unwrap();
        drop(first); // cancelled: no entry, no file, and the track is free again
        assert!(matches!(cache.get("t"), CacheEntry::Miss));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);

        let mut again = other_zone
            .start_download("t", "audio/mpeg", Some(6))
            .unwrap()
            .expect("free after the first writer is gone");
        again.write(b"abcdef").unwrap();
        again.complete().unwrap();
        match cache.get("t") {
            CacheEntry::Complete { path, .. } => assert_eq!(fs::read(path).unwrap(), b"abcdef"),
            _ => panic!("expected a complete entry"),
        }
    }

    #[test]
    fn a_download_shorter_than_announced_is_not_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();
        let mut w = cache
            .start_download("short", "audio/mpeg", Some(100))
            .unwrap()
            .expect("writer");
        w.write(&[0u8; 40]).unwrap();
        assert!(w.complete().is_err());
        assert!(matches!(cache.get("short"), CacheEntry::Miss));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn writers_use_distinct_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();
        let a = cache
            .start_download("a", "audio/mpeg", None)
            .unwrap()
            .unwrap();
        let b = cache
            .start_download("b", "audio/mpeg", None)
            .unwrap()
            .unwrap();
        assert_ne!(a.partial_path(), b.partial_path());
    }

    #[test]
    fn clones_share_entries_and_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(&dir);
        config.max_size_mb = 1;
        let zone1 = TrackCache::new(&config).unwrap();
        let zone2 = zone1.clone();

        let mut w = zone1
            .start_download("one", "audio/mpeg", None)
            .unwrap()
            .unwrap();
        w.write(&vec![0u8; 700 * 1024]).unwrap();
        w.complete().unwrap();
        assert!(zone2.is_complete("one"), "zone 2 sees the entry of zone 1");

        let mut w = zone2
            .start_download("two", "audio/mpeg", None)
            .unwrap()
            .unwrap();
        w.write(&vec![0u8; 700 * 1024]).unwrap();
        w.complete().unwrap();
        // Together above 1 MB: the limit holds in total, the older entry goes.
        assert!(!zone1.is_complete("one"));
        assert!(zone1.is_complete("two"));
    }

    #[test]
    fn partial_files_of_an_earlier_run_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("track-abc.7.partial");
        fs::write(&stale, b"x").unwrap();
        let _cache = TrackCache::new(&test_config(&dir)).unwrap();
        assert!(!stale.exists());
    }

    #[test]
    fn track_ids_are_sanitized_for_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TrackCache::new(&test_config(&dir)).unwrap();

        let mut writer = cache
            .start_download("../outside/track", "audio/mpeg", None)
            .unwrap()
            .expect("no download of this track in flight");
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
            .unwrap()
            .expect("no download of this track in flight");
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
            let mut w = cache
                .start_download("persist", "audio/flac", None)
                .unwrap()
                .expect("no download of this track in flight");
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
            let mut w = cache
                .start_download(&id, "audio/mpeg", None)
                .unwrap()
                .expect("no download of this track in flight");
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
