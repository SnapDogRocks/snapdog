// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Notices derived from a resolved configuration: keys that this build ignores
//! and settings that weaken security. They are collected here because logging
//! does not exist yet while the configuration loads; `main` logs them afterwards.
//!
//! Old files must keep loading, so none of these is an error.

#[cfg(feature = "snapcast-embedded")]
use super::SnapcastConfig;
use super::{AppConfig, AudioCodec};

pub(super) fn collect(config: &mut AppConfig) {
    encryption(config);
    #[cfg(feature = "snapcast-embedded")]
    embedded_snapcast(config);
    spotify_name(config);
    #[cfg(not(feature = "ap2"))]
    airplay(config);
}

/// `f32lz4e` without a key uses the built-in default, which is public.
fn encryption(config: &mut AppConfig) {
    let has_key = config
        .snapcast
        .encryption_psk
        .as_ref()
        .is_some_and(|key| !key.is_empty());

    match config.snapcast.codec {
        AudioCodec::F32lz4e if !has_key => config.warn(
            "codec f32lz4e is configured without encryption_psk, so the built-in default key is \
             used. That key is public: the stream is NOT protected against eavesdropping. Set \
             [snapcast].encryption_psk (or SNAPDOG_SNAPCAST_ENCRYPTION_PSK) and pass the same key \
             to every client with --encryption-psk.",
        ),
        AudioCodec::Pcm | AudioCodec::Flac if has_key => config.warn(format!(
            "[snapcast].encryption_psk is set, but codec {} does not encrypt. Use codec \
             f32lz4e to encrypt the stream.",
            config.snapcast.codec
        )),
        _ => {}
    }
}

/// Keys that only control an external `snapserver` (feature `snapcast-process`).
#[cfg(feature = "snapcast-embedded")]
fn embedded_snapcast(config: &mut AppConfig) {
    let default = SnapcastConfig::default();
    let snapcast = config.snapcast.clone();
    let hint = "It only applies to builds with the snapcast-process feature, which start an \
                external snapserver.";

    if !snapcast.managed {
        config.warn(format!(
            "[snapcast].managed = false has no effect: this build runs the embedded Snapcast \
             server, which cannot be switched off. {hint}"
        ));
    }
    if snapcast.verbose {
        config.warn(format!(
            "[snapcast].verbose has no effect: the embedded Snapcast server has no separate \
             console output. {hint}"
        ));
    }
    if snapcast.jsonrpc_tcp_port != default.jsonrpc_tcp_port {
        config.warn(format!(
            "[snapcast].jsonrpc_tcp_port = {} has no effect: the embedded Snapcast server has \
             no JSON-RPC listener. {hint}",
            snapcast.jsonrpc_tcp_port
        ));
    }
    if snapcast.address != default.address {
        config.warn(format!(
            "[snapcast].address = {:?} has no effect: the embedded Snapcast server listens on \
             all interfaces on streaming_port. {hint}",
            snapcast.address
        ));
    }
}

/// `[spotify].name` is superseded by the per-zone name.
fn spotify_name(config: &mut AppConfig) {
    if config.spotify.as_ref().is_some_and(|s| !s.name.is_empty()) {
        config.warn(
            "[spotify].name is ignored. Each zone advertises its own [[zone]].spotify_name \
             (default: the zone name). Remove the key or set spotify_name per zone.",
        );
    }
}

/// `[airplay].bind` needs feature `ap2`, and without it the mode is always `AirPlay` 1.
#[cfg(not(feature = "ap2"))]
fn airplay(config: &mut AppConfig) {
    if config.airplay.bind.is_some() {
        config.warn(
            "[airplay].bind has no effect: this build was compiled without the ap2 feature \
             and listens on all interfaces.",
        );
    }
    config.info(
        "This build has no AirPlay 2 support (feature ap2): AirPlay receivers speak \
         AirPlay 1 and [airplay].mode is not used.",
    );
}

#[cfg(test)]
mod tests {
    use super::super::{self as config, NoticeLevel};

    fn load(extra: &str) -> config::AppConfig {
        config::load_str(&format!("{extra}\n[[zone]]\nname = \"Living\"\n")).unwrap()
    }

    fn warnings(config: &config::AppConfig) -> Vec<&str> {
        config
            .notices
            .iter()
            .filter(|n| n.level == NoticeLevel::Warn)
            .map(|n| n.message.as_str())
            .collect()
    }

    fn mentions(config: &config::AppConfig, needle: &str) -> bool {
        warnings(config).iter().any(|m| m.contains(needle))
    }

    #[test]
    fn plain_config_has_no_warnings() {
        assert_eq!(warnings(&load("")), Vec::<&str>::new());
    }

    #[test]
    fn f32lz4e_without_key_warns_about_the_default_key() {
        let config = load("[snapcast]\ncodec = \"f32lz4e\"");
        assert!(mentions(&config, "built-in default key"));
        assert!(mentions(&config, "NOT protected"));
    }

    #[test]
    fn f32lz4e_with_empty_key_still_warns() {
        let config = load("[snapcast]\ncodec = \"f32lz4e\"\nencryption_psk = \"\"");
        assert!(mentions(&config, "built-in default key"));
    }

    #[test]
    fn f32lz4e_with_key_is_silent() {
        let config = load("[snapcast]\ncodec = \"f32lz4e\"\nencryption_psk = \"s3cret\"");
        assert!(!mentions(&config, "encryption_psk"));
    }

    #[test]
    fn key_with_unencrypted_codec_warns() {
        let config = load("[snapcast]\ncodec = \"flac\"\nencryption_psk = \"s3cret\"");
        assert!(mentions(&config, "does not encrypt"));
        let config = load("[snapcast]\ncodec = \"f32lz4\"\nencryption_psk = \"s3cret\"");
        assert!(!mentions(&config, "does not encrypt"));
    }

    #[test]
    fn psk_from_the_environment_counts_as_a_key() {
        let mut config = load("[snapcast]\ncodec = \"f32lz4e\"");
        config.notices.clear();
        config::override_psk(&mut config, Some("from-env".into()));
        super::encryption(&mut config);
        assert_eq!(config.notices, []);
    }

    #[test]
    fn empty_psk_variable_is_not_a_key() {
        let mut config = load("");
        config::override_psk(&mut config, Some(String::new()));
        assert!(config.snapcast.encryption_psk.is_none());
    }

    #[test]
    fn spotify_name_is_reported_as_ignored() {
        let config = load("[spotify]\nname = \"SnapDog\"");
        assert!(mentions(&config, "[spotify].name is ignored"));
        let config = load("[spotify]\nbitrate = 160");
        assert!(!mentions(&config, "[spotify].name"));
    }

    #[test]
    fn unknown_keys_are_reported_with_their_path() {
        let config = load("[snapcast]\nstreaming_prot = 1704");
        assert!(mentions(&config, "`snapcast.streaming_prot`"));

        let config =
            config::load_str("[[zone]]\nname = \"A\"\n[zone.knx]\npresence_timeout = \"1/1/53\"\n")
                .unwrap();
        assert!(mentions(&config, "`zone[0].knx.presence_timeout`"));
    }

    #[test]
    fn known_aliases_are_not_unknown() {
        let config = load(
            "[snapcast]\njsonrpc_port = 1705\n[knx]\nmode = \"device\"\nindividual_address = \"1.1.1\"",
        );
        assert!(!mentions(&config, "Unknown configuration key"));
    }

    #[test]
    fn notices_are_not_duplicated() {
        let mut config = load("[snapcast]\ncodec = \"f32lz4e\"");
        let before = config.notices.len();
        let first_warning = warnings(&config)[0].to_string();
        config.warn(first_warning);
        assert_eq!(config.notices.len(), before);
    }

    #[cfg(feature = "snapcast-embedded")]
    mod embedded {
        use super::*;

        #[test]
        fn defaults_are_silent() {
            let config = load(
                "[snapcast]\naddress = \"127.0.0.1\"\njsonrpc_tcp_port = 1705\nmanaged = true",
            );
            assert!(!mentions(&config, "has no effect"));
        }

        #[test]
        fn inert_keys_warn_once_each() {
            let config = load(
                "[snapcast]\nmanaged = false\nverbose = true\njsonrpc_port = 1800\naddress = \"10.0.0.5\"",
            );
            for key in ["managed", "verbose", "jsonrpc_tcp_port", "address"] {
                let hits = warnings(&config)
                    .into_iter()
                    .filter(|m| m.contains(&format!("[snapcast].{key}")))
                    .count();
                assert_eq!(hits, 1, "{key}");
            }
        }
    }

    #[cfg(not(feature = "ap2"))]
    #[test]
    fn airplay_bind_without_ap2_warns() {
        let config = load("[airplay]\nbind = [\"192.168.1.10\"]");
        assert!(mentions(&config, "[airplay].bind has no effect"));
        assert!(!mentions(&load(""), "[airplay].bind"));
    }
}
