// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Build a `FileConfig` from ETS-programmed parameters (RFC KEA-0004).
//!
//! When snapdog runs as a KNX device that ETS has programmed, the downloaded
//! parameter memory is parsed into an [`EtsParams`]. This module maps those
//! parameters onto a base `FileConfig` — the TOML config or defaults — so the
//! normal [`config::load_raw_no_validate`](crate::config::load_raw_no_validate)
//! pipeline resolves them into an `AppConfig`. ETS-provided values override the
//! base; empty/inactive ETS fields fall back to it.

use std::str::FromStr as _;

use knx_rs_core::address::IndividualAddress;

use super::device::{EtsParams, load_persisted_ets_params};
use super::group_objects::mem::MAX_RADIOS;
use super::group_objects::{MAX_CLIENTS, MAX_ZONES};
use crate::config::{
    FileConfig, LogLevel, MqttConfig, PresenceConfig, RawClientConfig, RawClientKnxConfig,
    RawRadioConfig, RawZoneConfig, RawZoneKnxConfig, SecretString, SubsonicCacheConfig,
    SubsonicConfig, SubsonicFormat,
};

/// Icon for ETS-derived zones/clients (ETS programs no icon).
const ETS_ICON: &str = "🎵";
/// Default KNX individual address when none is given (15.15.255).
const DEFAULT_IA: &str = "15.15.255";

/// Build the effective `FileConfig` for KNX **device mode** started without a
/// TOML config.
///
/// If the device has been programmed by ETS (persisted memory exists and reports
/// configured), its parameters provide the config; otherwise built-in defaults
/// are used. `addr` is the CLI-provided individual address. This is the boot-time
/// half of RFC KEA-0004 — the returned `FileConfig` is resolved by the normal
/// `config::load_raw_no_validate` pipeline.
pub fn ets_device_config(addr: Option<&str>) -> FileConfig {
    let base = FileConfig::default();
    let ia = addr
        .and_then(|s| IndividualAddress::from_str(s).ok())
        .or_else(|| IndividualAddress::from_str(DEFAULT_IA).ok());
    let Some(ia) = ia else {
        return base;
    };
    let Some(ets) = load_persisted_ets_params(ia) else {
        return base;
    };
    let cfg = ets_params_to_file_config(&ets, base);
    tracing::info!(
        zones = cfg.zone.len(),
        clients = cfg.client.len(),
        radios = cfg.radio.len(),
        "Applied ETS-programmed parameters as configuration"
    );
    cfg
}

/// Map the raw ETS `log_level` byte to a [`LogLevel`] (Trace=0 … Error=4).
const fn log_level_from_u8(b: u8) -> LogLevel {
    match b {
        0 => LogLevel::Trace,
        1 => LogLevel::Debug,
        3 => LogLevel::Warn,
        4 => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// A trimmed ETS string, or `fallback()` when it is empty.
fn or_else(s: &str, fallback: impl FnOnce() -> String) -> String {
    let t = s.trim();
    if t.is_empty() {
        fallback()
    } else {
        t.to_string()
    }
}

/// Resolve a client's default-zone **index** to a zone **name** that exists in
/// the ETS zone set (falling back to the first active zone), since
/// `RawClientConfig.zone` is a name the convention layer looks up.
fn client_zone_name(ets: &EtsParams, client: usize) -> String {
    let idx = ets.client_default_zone[client] as usize;
    if idx < MAX_ZONES && ets.zone_active[idx] {
        return or_else(&ets.zone_names[idx], || format!("Zone {}", idx + 1));
    }
    (0..MAX_ZONES).find(|&z| ets.zone_active[z]).map_or_else(
        || "Zone 1".to_string(),
        |z| or_else(&ets.zone_names[z], || format!("Zone {}", z + 1)),
    )
}

/// Merge ETS-programmed parameters onto `base`.
fn ets_params_to_file_config(ets: &EtsParams, mut base: FileConfig) -> FileConfig {
    base.system.log_level = log_level_from_u8(ets.log_level);
    if ets.http_port != 0 {
        base.http.port = ets.http_port;
    }

    if !ets.subsonic_url.trim().is_empty() {
        base.subsonic = Some(SubsonicConfig {
            url: ets.subsonic_url.trim().to_string(),
            username: ets.subsonic_user.clone(),
            password: SecretString::new(ets.subsonic_pass.clone()),
            format: SubsonicFormat::default(),
            tls_skip_verify: false,
            cache: SubsonicCacheConfig::default(),
        });
    }

    if !ets.mqtt_broker.trim().is_empty() {
        let mut base_topic = or_else(&ets.mqtt_topic, || "snapdog".to_string());
        if !base_topic.ends_with('/') {
            base_topic.push('/');
        }
        base.mqtt = Some(MqttConfig {
            broker: ets.mqtt_broker.trim().to_string(),
            client_id: "snapdog".to_string(),
            username: String::new(),
            password: SecretString::new(String::new()),
            base_topic,
        });
    }

    let zones: Vec<RawZoneConfig> = (0..MAX_ZONES)
        .filter(|&i| ets.zone_active[i])
        .map(|i| RawZoneConfig {
            name: or_else(&ets.zone_names[i], || format!("Zone {}", i + 1)),
            icon: ETS_ICON.to_string(),
            sink: None,
            airplay_name: None,
            knx: RawZoneKnxConfig::default(),
            group_volume_mode: None,
            presence: ets.zone_presence_enabled[i].then(|| PresenceConfig {
                auto_off_delay: ets.zone_presence_timeout[i],
                default_source: None,
                schedule: Vec::new(),
            }),
        })
        .collect();
    if !zones.is_empty() {
        base.zone = zones;
    }

    let clients: Vec<RawClientConfig> = (0..MAX_CLIENTS)
        .filter(|&i| ets.client_active[i])
        .map(|i| RawClientConfig {
            name: or_else(&ets.client_names[i], || format!("Client {}", i + 1)),
            mac: ets.client_macs[i].clone(),
            zone: client_zone_name(ets, i),
            icon: ETS_ICON.to_string(),
            max_volume: i32::from(ets.client_max_volume[i]),
            knx: RawClientKnxConfig::default(),
        })
        .collect();
    if !clients.is_empty() {
        base.client = clients;
    }

    let radios: Vec<RawRadioConfig> = (0..MAX_RADIOS)
        .filter(|&i| ets.radio_active[i])
        .map(|i| RawRadioConfig {
            name: or_else(&ets.radio_names[i], || format!("Radio {}", i + 1)),
            url: ets.radio_urls[i].trim().to_string(),
            cover: None,
        })
        .collect();
    if !radios.is_empty() {
        base.radio = radios;
    }

    base
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn maps_active_zones_clients_radios_and_scalars() {
        let mut ets = EtsParams::default();
        ets.zone_active[0] = true;
        ets.zone_names[0] = "Kitchen".into();
        ets.zone_active[2] = true;
        ets.zone_names[2] = "Bath".into();
        ets.client_active[0] = true;
        ets.client_names[0] = "Speaker".into();
        ets.client_macs[0] = "aa:bb:cc:dd:ee:ff".into();
        ets.client_default_zone[0] = 2; // → "Bath"
        ets.client_max_volume[0] = 80;
        ets.http_port = 8080;
        ets.log_level = 1; // Debug
        ets.subsonic_url = "https://music.example.com".into();
        ets.mqtt_broker = "mqtt.local:1883".into();
        ets.mqtt_topic = "home/audio".into();
        ets.radio_active[0] = true;
        ets.radio_names[0] = "SomaFM".into();
        ets.radio_urls[0] = "http://ice.somafm.com/groove".into();

        let cfg = ets_params_to_file_config(&ets, FileConfig::default());

        assert_eq!(cfg.zone.len(), 2);
        assert_eq!(cfg.zone[0].name, "Kitchen");
        assert_eq!(cfg.zone[1].name, "Bath");
        assert_eq!(cfg.client.len(), 1);
        assert_eq!(cfg.client[0].name, "Speaker");
        assert_eq!(cfg.client[0].zone, "Bath", "index 2 resolves to Bath");
        assert_eq!(cfg.client[0].max_volume, 80);
        assert_eq!(cfg.http.port, 8080);
        assert!(matches!(cfg.system.log_level, LogLevel::Debug));
        assert!(cfg.subsonic.is_some());
        let mqtt = cfg.mqtt.expect("mqtt");
        assert_eq!(mqtt.broker, "mqtt.local:1883");
        assert_eq!(mqtt.base_topic, "home/audio/", "trailing slash appended");
        assert_eq!(cfg.radio.len(), 1);
        assert_eq!(cfg.radio[0].name, "SomaFM");
    }

    #[test]
    fn empty_ets_falls_back_to_base() {
        let ets = EtsParams::default(); // nothing active
        let mut base = FileConfig::default();
        base.name = "Base".into();
        let cfg = ets_params_to_file_config(&ets, base);
        assert!(cfg.zone.is_empty());
        assert!(cfg.client.is_empty());
        assert!(cfg.subsonic.is_none());
        assert_eq!(cfg.name, "Base", "base fields preserved");
    }

    #[test]
    fn out_of_range_client_zone_falls_back_to_first_active_zone() {
        let mut ets = EtsParams::default();
        ets.zone_active[3] = true;
        ets.zone_names[3] = "Only".into();
        ets.client_active[0] = true;
        ets.client_names[0] = "C".into();
        ets.client_macs[0] = "aa:bb:cc:dd:ee:ff".into();
        ets.client_default_zone[0] = 9; // inactive → fall back to zone 3 "Only"
        let cfg = ets_params_to_file_config(&ets, FileConfig::default());
        assert_eq!(cfg.client[0].zone, "Only");
    }

    #[test]
    fn derived_config_resolves_through_the_load_pipeline() {
        // The ETS-derived FileConfig must pass the real resolution pipeline
        // (convention + skip-validate), including client→zone name resolution.
        let mut ets = EtsParams::default();
        ets.zone_active[0] = true;
        ets.zone_names[0] = "Living".into();
        ets.client_active[0] = true;
        ets.client_names[0] = "Sofa".into();
        ets.client_macs[0] = "aa:bb:cc:dd:ee:ff".into();
        ets.client_default_zone[0] = 0;

        let cfg = ets_params_to_file_config(&ets, FileConfig::default());
        let app = crate::config::load_raw_no_validate(cfg).expect("ETS config resolves");
        assert_eq!(app.zones.len(), 1);
        assert_eq!(app.clients.len(), 1);
        assert_eq!(app.clients[0].zone_index, 1, "client bound to zone 1");
    }
}
