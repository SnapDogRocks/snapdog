// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Spinorama speaker EQ profile fetcher.
//!
//! Fetches speaker correction profiles from the spinorama GitHub repository
//! and parses `EqualizerAPO` format into [`EqConfig`].

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::RwLock;

use crate::audio::eq::{EqBand, EqConfig, FilterType};

const GITHUB_RAW_BASE: &str =
    "https://raw.githubusercontent.com/pierreaubert/spinorama/develop/datas/eq";
const INDEX_URL: &str = "https://api.github.com/repos/pierreaubert/spinorama/contents/datas/eq";
const INDEX_CACHE_DURATION: std::time::Duration = std::time::Duration::from_secs(86400);

/// Cached speaker profile database.
#[derive(Clone)]
pub struct SpeakerDb {
    inner: Arc<RwLock<SpeakerDbInner>>,
    client: reqwest::Client,
}

struct SpeakerDbInner {
    index: Vec<String>,
    index_fetched_at: Option<std::time::Instant>,
    profiles: HashMap<String, EqConfig>,
}

impl SpeakerDb {
    /// Create a new speaker database with empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(SpeakerDbInner {
                index: Vec::new(),
                index_fetched_at: None,
                profiles: HashMap::new(),
            })),
            client: reqwest::Client::new(),
        }
    }

    /// Get the list of available speakers. Fetches from GitHub if cache is stale.
    ///
    /// # Errors
    ///
    /// Returns an error if the remote index cannot be fetched.
    pub async fn list_speakers(&self) -> Result<Vec<String>> {
        {
            let inner = self.inner.read().await;
            if let Some(fetched) = inner.index_fetched_at {
                if fetched.elapsed() < INDEX_CACHE_DURATION {
                    return Ok(inner.index.clone());
                }
            }
        }
        self.refresh_index().await
    }

    /// Get the EQ profile for a speaker. Fetches from GitHub if not cached.
    ///
    /// # Errors
    ///
    /// Returns an error if the profile cannot be fetched or parsed.
    pub async fn get_profile(&self, speaker: &str) -> Result<EqConfig> {
        {
            let inner = self.inner.read().await;
            if let Some(config) = inner.profiles.get(speaker) {
                return Ok(config.clone());
            }
        }
        self.fetch_profile(speaker).await
    }

    async fn refresh_index(&self) -> Result<Vec<String>> {
        let resp = self
            .client
            .get(INDEX_URL)
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", concat!("snapdog/", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .context("Failed to fetch speaker index from GitHub")?;
        if !resp.status().is_success() {
            anyhow::bail!("GitHub API returned {} (rate limit?)", resp.status());
        }
        let entries: Vec<GitHubEntry> = resp
            .json()
            .await
            .context("Failed to parse GitHub directory listing")?;
        let names: Vec<String> = entries
            .into_iter()
            .filter(|e| e.entry_type == "dir")
            .map(|e| e.name)
            .collect();
        let mut inner = self.inner.write().await;
        inner.index.clone_from(&names);
        inner.index_fetched_at = Some(std::time::Instant::now());
        drop(inner);
        Ok(names)
    }

    async fn fetch_profile(&self, speaker: &str) -> Result<EqConfig> {
        let url = format!("{GITHUB_RAW_BASE}/{speaker}/iir-autoeq.txt");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch profile for '{speaker}'"))?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Speaker profile not found: '{speaker}' (HTTP {})",
                resp.status()
            );
        }
        let text = resp.text().await?;
        let config = parse_autoeq(&text, speaker);
        self.inner
            .write()
            .await
            .profiles
            .insert(speaker.to_string(), config.clone());
        Ok(config)
    }
}

impl Default for SpeakerDb {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(serde::Deserialize)]
struct GitHubEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
}

/// Parse `EqualizerAPO` format into [`EqConfig`].
///
/// Format: `Filter N: ON PK Fc 1234 Hz Gain +1.23 dB Q 1.50`
fn parse_autoeq(text: &str, speaker: &str) -> EqConfig {
    let mut bands = Vec::new();
    for line in text.lines() {
        if !line.starts_with("Filter") || !line.contains("ON") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let freq = parts
            .iter()
            .position(|&p| p == "Fc")
            .and_then(|i| parts.get(i + 1))
            .and_then(|s| s.parse::<f32>().ok());
        let gain = parts
            .iter()
            .position(|&p| p == "Gain")
            .and_then(|i| parts.get(i + 1))
            .and_then(|s| s.parse::<f32>().ok());
        let q = parts
            .iter()
            .position(|&p| p == "Q")
            .and_then(|i| parts.get(i + 1))
            .and_then(|s| s.parse::<f32>().ok());

        if let (Some(freq), Some(gain), Some(q)) = (freq, gain, q) {
            let filter_type = parts
                .iter()
                .position(|&p| p == "ON")
                .and_then(|i| parts.get(i + 1))
                .map_or(FilterType::Peaking, |&t| match t {
                    "LSC" | "LS" => FilterType::LowShelf,
                    "HSC" | "HS" => FilterType::HighShelf,
                    "LP" => FilterType::LowPass,
                    "HP" => FilterType::HighPass,
                    _ => FilterType::Peaking,
                });

            bands.push(EqBand {
                freq,
                gain,
                q,
                filter_type,
            });
        }
    }
    EqConfig {
        enabled: true,
        bands,
        preset: Some(format!("spinorama:{speaker}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_autoeq_basic() {
        let input = r"EQ for KEF LS50 Meta
Preamp: -4.0 dB
Filter 1: ON PK Fc 42 Hz Gain +2.88 dB Q 1.04
Filter 2: ON PK Fc 69 Hz Gain +2.25 dB Q 0.80
Filter 3: ON LSC Fc 105 Hz Gain -2.21 dB Q 1.21
";
        let config = parse_autoeq(input, "KEF LS50 Meta");
        assert_eq!(config.bands.len(), 3);
        assert_eq!(config.bands[0].freq, 42.0);
        assert_eq!(config.bands[0].gain, 2.88);
        assert_eq!(config.bands[0].q, 1.04);
        assert_eq!(config.bands[0].filter_type, FilterType::Peaking);
        assert_eq!(config.bands[2].filter_type, FilterType::LowShelf);
        assert!(config.enabled);
    }
}
