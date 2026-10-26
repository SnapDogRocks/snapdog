// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Home Assistant MQTT discovery (stage 1 of RFC HAI-0007).
//!
//! Home Assistant's MQTT integration has no `media_player` platform, so a zone cannot
//! be announced as one. What MQTT discovery can express is announced instead: volume,
//! mute, shuffle, repeat and presence as `number`, `switch` and `select`, transport as
//! `button`, what plays as `sensor`, clients as entities of their own. There is no cover
//! art: Home Assistant's `image` entity logs an error for every message on its URL topic
//! that holds no valid URL, which a zone state without a cover (idle, or a radio without
//! artwork) always is, and it fetches without an API key header (RFC HAI-0007 §5 item 2). The whole server
//! is **one device message** (`{prefix}/device/snapdog_{id}/config`, retained) that
//! lists every entity as a component; the entities read the existing state topics
//! (`zones/{n}/state`, `clients/{n}/state`) through templates and send to the existing
//! command topics, so nothing about the topic scheme changes.
//!
//! - [`device_payload`] builds the message. It is pure, so that golden tests can pin it.
//! - [`Discovery`] sends it: after every (re)connect, again after Home Assistant's birth
//!   message (a Home Assistant that restarted forgets what it discovered), and when the
//!   set of clients changes. The message always lists *all* components: Home Assistant
//!   documents that removing single components by an update is fragile.
//! - An explicit `discovery = false` deletes the message (an empty retained publish),
//!   which takes the entities out of Home Assistant.
//!
//! Entity identity follows the list positions of `[[zone]]` and `[[client]]` (RFC
//! HAI-DEC-6 is open): the `unique_id` of an entity contains the zone or client number,
//! so moving a zone in the file moves its entities onto another zone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rumqttc::QoS;
use serde_json::{Value, json};
use tracing::Instrument;

use super::publisher::Publisher;
use super::topics::Topics;
use crate::config::{DiscoveryMode, MqttConfig};
use crate::state;

/// Who the discovery message says `SnapDog` is.
const ORIGIN_NAME: &str = "SnapDog";
const SUPPORT_URL: &str = env!("CARGO_PKG_REPOSITORY");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The longest text a Home Assistant state may have.
const HA_STATE_MAX_CHARS: usize = 255;
/// A random wait before the discovery message is sent again after Home Assistant's
/// birth message, in milliseconds: it keeps a restarting installation from sending
/// everything at the same moment.
const BIRTH_DELAY_MS: std::ops::RangeInclusive<u64> = 1_000..=5_000;

/// The server `SnapDog` announces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfo {
    /// The stable server id (`state_dir/server_id`, also in the mDNS TXT record).
    pub id: String,
    /// The server's display name (`name`).
    pub name: String,
    /// `[http].base_url`, shown as the device's configuration URL.
    pub base_url: String,
}

/// What the device message lists: the zones and clients that exist right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub zones: Vec<(usize, String)>,
    pub clients: Vec<(usize, String)>,
}

impl Model {
    /// The zones and clients of `store`, by index.
    pub fn of(store: &state::Store) -> Self {
        let sorted = |names: Vec<(usize, String)>| {
            let mut names = names;
            names.sort_unstable_by_key(|(index, _)| *index);
            names
        };
        Self {
            zones: sorted(
                store
                    .zones
                    .iter()
                    .map(|(i, z)| (*i, z.name.clone()))
                    .collect(),
            ),
            clients: sorted(
                store
                    .clients
                    .iter()
                    .map(|(i, c)| (*i, c.name.clone()))
                    .collect(),
            ),
        }
    }
}

/// `{prefix}/device/snapdog_{id}/config`. The object id may only hold
/// `[a-zA-Z0-9_-]` (Home Assistant's rule); a server id is a UUID, but anything
/// else is reduced to that alphabet.
pub fn device_topic(prefix: &str, server_id: &str) -> String {
    let id: String = server_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}/device/snapdog_{id}/config")
}

/// The device message for `model`.
#[must_use]
pub fn device_payload(topics: &Topics, server: &ServerInfo, model: &Model) -> Value {
    let mut components = serde_json::Map::new();
    for (n, name) in &model.zones {
        zone_components(&mut components, topics, server, *n, name);
    }
    let zone_names: Vec<&str> = model.zones.iter().map(|(_, n)| n.as_str()).collect();
    for (n, name) in &model.clients {
        client_components(&mut components, topics, server, *n, name, &zone_names);
    }
    let status = topics.availability();
    let mut device = json!({
        "identifiers": [format!("snapdog_{}", server.id)],
        "name": server.name,
        "manufacturer": ORIGIN_NAME,
        "model": "Multi-zone audio server",
        "sw_version": VERSION,
    });
    if server.base_url.starts_with("http://") || server.base_url.starts_with("https://") {
        device["configuration_url"] = json!(server.base_url);
    }
    json!({
        "device": device,
        "origin": { "name": ORIGIN_NAME, "sw_version": VERSION, "support_url": SUPPORT_URL },
        "availability": [
            { "topic": status, "payload_available": "online", "payload_not_available": "offline" }
        ],
        "qos": 1,
        "components": components,
    })
}

/// Copy the keys of `extra` into `target` (both objects).
fn merge(target: &mut Value, extra: Value) {
    if let (Some(target), Value::Object(extra)) = (target.as_object_mut(), extra) {
        target.extend(extra);
    }
}

/// `presence_enabled` as `Presence enabled`, `title` as `Title`.
fn title_case(what: &str) -> String {
    let spaced = what.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

// ── Sending it ────────────────────────────────────────────────

/// Announces the entities to Home Assistant (or takes them away again).
#[derive(Clone)]
pub struct Discovery {
    inner: Arc<Inner>,
}

struct Inner {
    mode: DiscoveryMode,
    publisher: Publisher,
    store: state::SharedState,
    server: ServerInfo,
    device_topic: String,
    status_topic: String,
    /// What the broker has been told last.
    announced: Mutex<Option<Model>>,
    /// A re-announcement after a birth message is already waiting.
    birth_pending: AtomicBool,
    birth_delay_ms: std::ops::RangeInclusive<u64>,
}

impl Discovery {
    /// `None` when nothing about Home Assistant is configured.
    pub fn new(
        config: &MqttConfig,
        publisher: Publisher,
        store: state::SharedState,
        server: ServerInfo,
    ) -> Option<Self> {
        let mode = config.discovery_mode();
        if mode == DiscoveryMode::Untouched {
            return None;
        }
        let ha = config.homeassistant.clone().unwrap_or_default();
        Some(Self {
            inner: Arc::new(Inner {
                mode,
                publisher,
                store,
                device_topic: device_topic(&ha.discovery_prefix, &server.id),
                status_topic: ha.status_topic,
                server,
                announced: Mutex::new(None),
                birth_pending: AtomicBool::new(false),
                birth_delay_ms: BIRTH_DELAY_MS,
            }),
        })
    }

    /// Whether `topic` is where Home Assistant announces that it started.
    pub fn is_birth_topic(&self, topic: &str) -> bool {
        self.inner.mode == DiscoveryMode::On && topic == self.inner.status_topic
    }

    /// After a (re)connect: subscribe to the birth topic and announce, or (with
    /// `discovery = false`, once per start) delete what an earlier run announced.
    pub async fn on_connected(&self, first_connect: bool) {
        match self.inner.mode {
            DiscoveryMode::On => {
                if let Err(e) = self
                    .inner
                    .publisher
                    .client()
                    .try_subscribe(&self.inner.status_topic, QoS::AtLeastOnce)
                {
                    tracing::warn!(error = %e, "MQTT subscription to Home Assistant's birth topic could not be queued");
                }
                self.announce().await;
            }
            DiscoveryMode::Off if first_connect => {
                // An empty retained payload deletes the device (and its entities).
                match self
                    .inner
                    .publisher
                    .retained(&self.inner.device_topic, Vec::<u8>::new())
                {
                    Ok(()) => tracing::info!(
                        topic = %self.inner.device_topic,
                        "Home Assistant discovery is off: asked the broker to delete the device message of an earlier run"
                    ),
                    Err(e) => {
                        tracing::warn!(error = %e, "could not queue the deletion of the Home Assistant device message");
                    }
                }
            }
            _ => {}
        }
    }

    /// Send the device message now.
    pub async fn announce(&self) {
        let model = Model::of(&*self.inner.store.read().await);
        self.send(model);
    }

    fn send(&self, model: Model) {
        let payload = device_payload(self.inner.publisher.topics(), &self.inner.server, &model);
        match self
            .inner
            .publisher
            .retained(&self.inner.device_topic, payload.to_string())
        {
            Ok(()) => {
                tracing::info!(
                    zones = model.zones.len(),
                    clients = model.clients.len(),
                    topic = %self.inner.device_topic,
                    "Announced the SnapDog device to Home Assistant"
                );
                *self.announced() = Some(model);
            }
            Err(e) => tracing::error!(
                error = %e,
                zones = model.zones.len(),
                clients = model.clients.len(),
                "Home Assistant discovery message could not be published (too many zones and clients for one message?)"
            ),
        }
    }

    fn announced(&self) -> std::sync::MutexGuard<'_, Option<Model>> {
        self.inner
            .announced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Announce again if the zones or clients differ from what was announced (a
    /// client that Snapcast accepted at runtime has entities of its own).
    pub async fn announce_if_changed(&self) {
        if self.inner.mode != DiscoveryMode::On {
            return;
        }
        let model = Model::of(&*self.inner.store.read().await);
        if self.announced().as_ref() != Some(&model) {
            self.send(model);
        }
    }

    /// Home Assistant announced that it started: it forgot what it discovered, so
    /// send the message again, after a random delay (one wait at a time).
    pub fn on_birth(&self) {
        if self.inner.birth_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        let delay = Duration::from_millis(fastrand::u64(this.inner.birth_delay_ms.clone()));
        tokio::spawn(
            async move {
                tokio::time::sleep(delay).await;
                this.inner.birth_pending.store(false, Ordering::SeqCst);
                this.announce().await;
            }
            .in_current_span(),
        );
    }
}

/// The entities of zone `n`.
#[allow(clippy::too_many_lines)] // one table of entities, splitting it would hide the set
fn zone_components(
    components: &mut serde_json::Map<String, Value>,
    topics: &Topics,
    server: &ServerInfo,
    n: usize,
    name: &str,
) {
    let mut add = |id: String, component: Value| {
        components.insert(id, component);
    };
    let uid = |id: &str| format!("snapdog_{}_{id}", server.id);
    let n = &n;
    let state_topic = topics.zone_state(*n);
    let command = |tail: &str| topics.join(&format!("zones/{n}/{tail}/set"));
    let id = |what: &str| format!("zone_{n}_{what}");
    let label = |what: &str| format!("{name} {what}");
    // Everything of a zone reads `zones/{n}/state`.
    let base = |what: &str, platform: &str| {
        json!({
            "platform": platform,
            "unique_id": uid(&id(what)),
            "name": label(&title_case(what)),
            "state_topic": state_topic,
        })
    };

    let mut state = base("state", "sensor");
    merge(
        &mut state,
        json!({
            "device_class": "enum",
            "options": ["playing", "paused", "stopped", "idle"],
            "value_template": "{{ value_json.state }}",
        }),
    );
    add(id("state"), state);

    let mut volume = base("volume", "number");
    merge(
        &mut volume,
        json!({
            "min": 0, "max": 100, "step": 1, "mode": "slider",
            "unit_of_measurement": "%",
            "value_template": "{{ (value_json.volume_level * 100) | round(0) | int }}",
            // Zone volume takes a fraction up to 1.0 and a percentage above, so
            // `1` would mean 100 %: always send the fraction.
            "command_topic": command("volume"),
            "command_template": "{{ (value | float / 100) | round(2) }}",
        }),
    );
    add(id("volume"), volume);

    for (what, key) in [("mute", "is_volume_muted"), ("shuffle", "shuffle")] {
        let mut switch = base(what, "switch");
        merge(
            &mut switch,
            json!({
                "value_template": format!("{{{{ 'ON' if value_json.{key} else 'OFF' }}}}"),
                "state_on": "ON", "state_off": "OFF",
                "payload_on": "ON", "payload_off": "OFF",
                "command_topic": command(what),
            }),
        );
        add(id(what), switch);
    }

    let mut repeat = base("repeat", "select");
    merge(
        &mut repeat,
        json!({
            "options": ["off", "one", "all"],
            "value_template": "{{ value_json.repeat }}",
            "command_topic": command("repeat"),
        }),
    );
    add(id("repeat"), repeat);

    for what in ["play", "pause", "stop", "next", "previous"] {
        add(
            id(what),
            json!({
                "platform": "button",
                "unique_id": uid(&id(what)),
                "name": label(&title_case(what)),
                "command_topic": command("control"),
                "payload_press": what,
            }),
        );
    }

    for (what, key) in [
        ("title", "media_title"),
        ("artist", "media_artist"),
        ("album", "media_album_name"),
    ] {
        let mut sensor = base(what, "sensor");
        merge(
            &mut sensor,
            json!({
                // A state is at most 255 characters; the tag is empty while idle.
                "value_template": format!(
                    "{{{{ (value_json.{key} | default(''))[:{HA_STATE_MAX_CHARS}] }}}}"
                ),
            }),
        );
        add(id(what), sensor);
    }

    let mut duration = base("duration", "sensor");
    merge(
        &mut duration,
        json!({
            "device_class": "duration",
            "unit_of_measurement": "s",
            // `none` renders as 'None', which Home Assistant reads as unknown.
            "value_template":
                "{{ value_json.media_duration if 'media_duration' in value_json else none }}",
        }),
    );
    add(id("duration"), duration);

    let mut source = base("source", "sensor");
    merge(
        &mut source,
        json!({ "value_template": "{{ value_json.source }}" }),
    );
    add(id("source"), source);

    let mut playlist = base("playlist", "number");
    merge(
        &mut playlist,
        json!({
            "min": 0, "max": 999, "step": 1, "mode": "box",
            "value_template":
                "{{ value_json.playlist_index if value_json.playlist_index is not none else none }}",
            "command_topic": command("playlist"),
            "command_template": "{{ value | int }}",
        }),
    );
    add(id("playlist"), playlist);

    let mut presence = base("presence", "binary_sensor");
    merge(
        &mut presence,
        json!({
            "device_class": "occupancy",
            "value_template": "{{ 'ON' if value_json.presence else 'OFF' }}",
            "payload_on": "ON", "payload_off": "OFF",
        }),
    );
    add(id("presence"), presence);

    let mut presence_enabled = base("presence_enabled", "switch");
    merge(
        &mut presence_enabled,
        json!({
            "name": label("Presence control"),
            "entity_category": "config",
            "value_template": "{{ 'ON' if value_json.presence_enabled else 'OFF' }}",
            "state_on": "ON", "state_off": "OFF",
            "payload_on": "ON", "payload_off": "OFF",
            "command_topic": command("presence/enable"),
        }),
    );
    add(id("presence_enabled"), presence_enabled);

    let mut timeout = base("presence_timeout", "number");
    merge(
        &mut timeout,
        json!({
            "name": label("Presence timeout"),
            "entity_category": "config",
            "min": 0, "max": 65535, "step": 1, "mode": "box",
            "unit_of_measurement": "s",
            "value_template": "{{ value_json.auto_off_delay }}",
            "command_topic": command("presence/timeout"),
            "command_template": "{{ value | int }}",
        }),
    );
    add(id("presence_timeout"), timeout);
}

/// The entities of client `n`; `zone_names` are the options of its zone select.
#[allow(clippy::too_many_lines)] // one table of entities, splitting it would hide the set
fn client_components(
    components: &mut serde_json::Map<String, Value>,
    topics: &Topics,
    server: &ServerInfo,
    n: usize,
    name: &str,
    zone_names: &[&str],
) {
    let mut add = |id: String, component: Value| {
        components.insert(id, component);
    };
    let uid = |id: &str| format!("snapdog_{}_{id}", server.id);
    let n = &n;
    let status = topics.availability();
    let state_topic = topics.client_state(*n);
    let command = |tail: &str| topics.join(&format!("clients/{n}/{tail}/set"));
    let id = |what: &str| format!("client_{n}_{what}");
    let label = |what: &str| format!("{name} {what}");
    // A client that is not connected to Snapcast cannot be told anything: its
    // controls follow the server's availability and the client's own.
    let controls_available = json!([
        { "topic": status, "payload_available": "online", "payload_not_available": "offline" },
        {
            "topic": state_topic,
            "value_template": "{{ 'online' if value_json.connected else 'offline' }}",
            "payload_available": "online", "payload_not_available": "offline",
        },
    ]);
    let base = |what: &str, platform: &str| {
        json!({
            "platform": platform,
            "unique_id": uid(&id(what)),
            "name": label(&title_case(what)),
            "state_topic": state_topic,
        })
    };

    let mut volume = base("volume", "number");
    merge(
        &mut volume,
        json!({
            "min": 0, "max": 100, "step": 1, "mode": "slider", "unit_of_measurement": "%",
            "value_template": "{{ value_json.volume }}",
            "command_topic": command("volume"),
            "command_template": "{{ value | int }}",
            "availability": controls_available,
            "availability_mode": "all",
        }),
    );
    add(id("volume"), volume);

    let mut mute = base("mute", "switch");
    merge(
        &mut mute,
        json!({
            "value_template": "{{ 'ON' if value_json.muted else 'OFF' }}",
            "state_on": "ON", "state_off": "OFF",
            "payload_on": "ON", "payload_off": "OFF",
            "command_topic": command("mute"),
            "availability": controls_available,
            "availability_mode": "all",
        }),
    );
    add(id("mute"), mute);

    let mut latency = base("latency", "number");
    merge(
        &mut latency,
        json!({
            "entity_category": "config",
            "min": -10_000, "max": 10_000, "step": 1, "mode": "box",
            "unit_of_measurement": "ms",
            "value_template": "{{ value_json.latency }}",
            "command_topic": command("latency"),
            "command_template": "{{ value | int }}",
            "availability": controls_available,
            "availability_mode": "all",
        }),
    );
    add(id("latency"), latency);

    // The zone is a number in the state and a name for people: the templates map
    // between the two with the list of zone names (the options).
    let names = serde_json::to_string(&zone_names).expect("a list of strings");
    let mut zone = base("zone", "select");
    merge(
        &mut zone,
        json!({
            "options": zone_names,
            "value_template": format!(
                "{{% set zones = {names} %}}{{{{ zones[value_json.zone - 1] if 0 < value_json.zone <= zones | length else none }}}}"
            ),
            "command_topic": command("zone"),
            "command_template": format!("{{{{ {names}.index(value) + 1 }}}}"),
        }),
    );
    add(id("zone"), zone);

    let mut connected = base("connected", "binary_sensor");
    merge(
        &mut connected,
        json!({
            "device_class": "connectivity",
            "entity_category": "diagnostic",
            "value_template": "{{ 'ON' if value_json.connected else 'OFF' }}",
            "payload_on": "ON", "payload_off": "OFF",
        }),
    );
    add(id("connected"), connected);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::test_support::test_server;

    fn model() -> Model {
        Model {
            zones: vec![(1, "Living Room".into()), (2, "Kitchen".into())],
            clients: vec![(1, "Sofa".into()), (2, "Shelf".into())],
        }
    }

    fn payload(server: &ServerInfo) -> Value {
        device_payload(&Topics::new("snapdog"), server, &model())
    }

    fn components(payload: &Value) -> &serde_json::Map<String, Value> {
        payload["components"].as_object().expect("components")
    }

    /// The device message of a small installation, written out. A change to an entity
    /// shows up here as a diff to review: regenerate with
    /// `SNAPDOG_UPDATE_GOLDEN=1 cargo test -p snapdog --lib discovery`.
    #[test]
    fn the_device_message_matches_its_golden_file() {
        const GOLDEN: &str = "src/mqtt/golden/discovery_device.json";
        // The version moves with every release: it is not what is pinned here.
        let mut message = payload(&test_server());
        message["device"]["sw_version"] = json!("<version>");
        message["origin"]["sw_version"] = json!("<version>");
        let actual = serde_json::to_string_pretty(&message).unwrap() + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);
        if std::env::var_os("SNAPDOG_UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (SNAPDOG_UPDATE_GOLDEN=1 writes it)",
                path.display()
            )
        });
        assert_eq!(
            serde_json::from_str::<Value>(&expected).unwrap(),
            serde_json::from_str::<Value>(&actual).unwrap(),
            "the discovery message changed: review the diff, then SNAPDOG_UPDATE_GOLDEN=1"
        );
    }

    #[test]
    fn the_message_has_what_home_assistant_requires_of_a_device_message() {
        let p = payload(&test_server());
        // `device` and `origin` are mandatory at the root, the origin needs a name.
        assert!(p["device"]["identifiers"].is_array(), "{}", p["device"]);
        assert_eq!(p["device"]["name"], "SnapDog");
        assert_eq!(p["origin"]["name"], "SnapDog");
        assert_eq!(p["origin"]["sw_version"], env!("CARGO_PKG_VERSION"));
        // Every component names its platform and has a unique id and something else.
        let mut unique = std::collections::HashSet::new();
        for (key, component) in components(&p) {
            let object = component.as_object().expect(key);
            assert!(object["platform"].is_string(), "{key}");
            let id = object["unique_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{key}: unique_id"));
            assert!(unique.insert(id.to_string()), "duplicate unique_id {id}");
            assert!(id.starts_with("snapdog_0b1c2d3e-"), "{id}");
            assert!(
                object.len() > 2,
                "{key}: nothing but platform and unique_id"
            );
            assert!(object["name"].is_string(), "{key}");
            // Shared options live at the root, not in a component.
            assert!(
                !object.contains_key("device") && !object.contains_key("origin"),
                "{key}"
            );
        }
        // The availability of the server is the shared default.
        assert_eq!(p["availability"][0]["topic"], "snapdog/status");
        assert_eq!(p["availability"][0]["payload_available"], "online");
        assert_eq!(p["availability"][0]["payload_not_available"], "offline");
        assert_eq!(p["qos"], 1);
    }

    #[test]
    fn the_entity_set_follows_the_rfc_table() {
        let p = payload(&test_server());
        let c = components(&p);
        let platform = |key: &str| c[key]["platform"].as_str().unwrap().to_string();
        for (key, expected) in [
            ("zone_1_state", "sensor"),
            ("zone_1_volume", "number"),
            ("zone_1_mute", "switch"),
            ("zone_1_shuffle", "switch"),
            ("zone_1_repeat", "select"),
            ("zone_1_play", "button"),
            ("zone_1_pause", "button"),
            ("zone_1_stop", "button"),
            ("zone_1_next", "button"),
            ("zone_1_previous", "button"),
            ("zone_1_title", "sensor"),
            ("zone_1_artist", "sensor"),
            ("zone_1_album", "sensor"),
            ("zone_1_duration", "sensor"),
            ("zone_1_source", "sensor"),
            ("zone_1_playlist", "number"),
            ("zone_1_presence", "binary_sensor"),
            ("zone_1_presence_enabled", "switch"),
            ("zone_1_presence_timeout", "number"),
            ("client_1_volume", "number"),
            ("client_1_mute", "switch"),
            ("client_1_latency", "number"),
            ("client_1_zone", "select"),
            ("client_1_connected", "binary_sensor"),
            ("zone_2_volume", "number"),
            ("client_2_connected", "binary_sensor"),
        ] {
            assert_eq!(platform(key), expected, "{key}");
        }
        assert_eq!(c.len(), 2 * 19 + 2 * 5);
        // No position sensor: a per-second update would only fill the database.
        assert!(!c.keys().any(|k| k.contains("position")));
    }

    /// Home Assistant's `image` entity answers every message without a valid URL on its
    /// URL topic with an error in the log (seen with Home Assistant 2026.10.4), and a
    /// zone state has none most of the time. So no cover entity is announced.
    #[test]
    fn no_image_entity_is_announced() {
        let p = payload(&test_server());
        assert!(
            !components(&p)
                .values()
                .any(|c| c["platform"] == "image" || c.get("url_topic").is_some())
        );
    }

    /// Every command topic an entity writes to is one that `SnapDog` subscribes to, and
    /// parses as the command it is meant to be.
    #[test]
    fn every_command_topic_is_a_real_command_topic() {
        let topics = Topics::new("snapdog");
        let p = payload(&test_server());
        let mut seen = 0;
        for (key, component) in components(&p) {
            let Some(topic) = component["command_topic"].as_str() else {
                continue;
            };
            let parsed = topics
                .parse_command(topic)
                .unwrap_or_else(|e| panic!("{key}: {topic}: {e}"));
            let expected_scope = if key.starts_with("client_") {
                crate::mqtt::topics::Scope::Client
            } else {
                crate::mqtt::topics::Scope::Zone
            };
            assert_eq!(parsed.scope, expected_scope, "{key}");
            let n: usize = key.split('_').nth(1).unwrap().parse().unwrap();
            assert_eq!(parsed.index, n, "{key} addresses another zone or client");
            seen += 1;
        }
        // 2 zones x (volume, mute, shuffle, repeat, 5 buttons, playlist, enable,
        // timeout) + 2 clients x (volume, mute, latency, zone)
        assert_eq!(seen, 2 * 12 + 2 * 4);
    }

    #[test]
    fn every_state_topic_is_a_published_state_topic() {
        let topics = Topics::new("snapdog");
        let p = payload(&test_server());
        for (key, component) in components(&p) {
            let topic = component["state_topic"]
                .as_str()
                .or_else(|| component["url_topic"].as_str());
            let Some(topic) = topic else { continue };
            let n: usize = key.split('_').nth(1).unwrap().parse().unwrap();
            let expected = if key.starts_with("client_") {
                topics.client_state(n)
            } else {
                topics.zone_state(n)
            };
            assert_eq!(topic, expected, "{key}");
        }
    }

    /// The templates read keys that the state payloads have (a rename on either
    /// side would silently blank an entity).
    #[tokio::test]
    async fn the_templates_read_keys_that_the_state_payloads_publish() {
        use crate::mqtt::payloads::{client_state_payload, zone_state_payload};
        use crate::mqtt::test_support::test_state_with_client;
        let shared = test_state_with_client();
        let (zone, client) = {
            let store = shared.read().await;
            (
                zone_state_payload("http://h", &store.zones[&1]),
                client_state_payload(&store.clients[&1]),
            )
        };
        let p = payload(&test_server());
        let template_keys = |text: &str| -> Vec<String> {
            text.split("value_json.")
                .skip(1)
                .map(|rest| {
                    rest.chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect()
                })
                .collect()
        };
        // Keys that only exist while a track plays are read defensively in the
        // templates, so they may be absent from an idle zone's state.
        let optional = [
            "media_title",
            "media_artist",
            "media_album_name",
            "media_duration",
            "media_image_url",
        ];
        let mut checked = 0;
        for (key, component) in components(&p) {
            let source = if key.starts_with("client_") {
                &client
            } else {
                &zone
            };
            for field in ["value_template", "url_template"] {
                let Some(template) = component[field].as_str() else {
                    continue;
                };
                for used in template_keys(template) {
                    if optional.contains(&used.as_str()) {
                        continue;
                    }
                    assert!(source.get(&used).is_some(), "{key}: {field} reads `{used}`");
                    checked += 1;
                }
            }
        }
        assert!(checked > 20, "{checked}");
    }

    #[test]
    fn a_client_is_only_controllable_while_it_is_connected() {
        let p = payload(&test_server());
        let c = components(&p);
        for key in ["client_1_volume", "client_1_mute", "client_1_latency"] {
            assert_eq!(c[key]["availability_mode"], "all", "{key}");
            let list = c[key]["availability"].as_array().unwrap();
            assert_eq!(list[0]["topic"], "snapdog/status");
            assert_eq!(list[1]["topic"], "snapdog/clients/1/state");
            assert!(
                list[1]["value_template"]
                    .as_str()
                    .unwrap()
                    .contains("connected")
            );
        }
        // The connected sensor itself, and the zone (which works while the client is
        // away), follow the server only.
        assert!(c["client_1_connected"].get("availability").is_none());
        assert!(c["client_1_zone"].get("availability").is_none());
    }

    #[test]
    fn categories_and_classes_are_the_ones_home_assistant_allows() {
        let p = payload(&test_server());
        for (key, component) in components(&p) {
            if let Some(category) = component.get("entity_category") {
                assert!(
                    matches!(category.as_str(), Some("config" | "diagnostic")),
                    "{key}"
                );
                // Sensors may only be diagnostic.
                if matches!(
                    component["platform"].as_str(),
                    Some("sensor" | "binary_sensor")
                ) {
                    assert_eq!(category, "diagnostic", "{key}");
                }
            }
            if component["device_class"] == "enum" {
                assert!(
                    component["options"]
                        .as_array()
                        .is_some_and(|o| !o.is_empty()),
                    "{key}"
                );
                assert!(component.get("unit_of_measurement").is_none(), "{key}");
            }
        }
    }

    /// The numbers carry the ranges the commands accept, and send integers (the
    /// parsers refuse `50.0`) except for the zone volume, which sends a fraction.
    #[test]
    fn numbers_send_what_the_command_parsers_accept() {
        let p = payload(&test_server());
        let c = components(&p);
        assert_eq!(
            c["zone_1_volume"]["command_template"],
            "{{ (value | float / 100) | round(2) }}"
        );
        for key in [
            "zone_1_playlist",
            "zone_1_presence_timeout",
            "client_1_volume",
            "client_1_latency",
        ] {
            assert_eq!(c[key]["command_template"], "{{ value | int }}", "{key}");
        }
        assert_eq!(
            (
                c["client_1_latency"]["min"].as_i64(),
                c["client_1_latency"]["max"].as_i64()
            ),
            (Some(-10_000), Some(10_000))
        );
        assert_eq!(c["zone_1_presence_timeout"]["max"], 65535);
        assert_eq!(
            (
                c["zone_1_volume"]["min"].as_i64(),
                c["zone_1_volume"]["max"].as_i64()
            ),
            (Some(0), Some(100))
        );
        // The switches send ON and OFF, which the strict parser accepts in any case.
        assert_eq!(c["zone_1_mute"]["payload_on"], "ON");
        assert_eq!(c["zone_1_mute"]["payload_off"], "OFF");
        // The repeat options are payloads the parser knows.
        for option in c["zone_1_repeat"]["options"].as_array().unwrap() {
            assert!(crate::mqtt::parse::parse_repeat(option.as_str().unwrap()).is_ok());
        }
        // The buttons send transport commands the parser knows.
        for what in ["play", "pause", "stop", "next", "previous"] {
            let payload = c[&format!("zone_1_{what}")]["payload_press"]
                .as_str()
                .unwrap();
            assert!(
                crate::mqtt::parse::parse_transport(payload).is_ok(),
                "{what}"
            );
        }
    }

    /// What Home Assistant's own template engine (2026.10.4, run in its container
    /// against this message) renders from the command templates, fed to the parsers
    /// that receive it. If a template changes, re-run it there and update the strings.
    #[test]
    fn what_home_assistant_renders_for_a_command_is_accepted_by_the_parsers() {
        use crate::mqtt::parse::{
            parse_client_volume, parse_count, parse_latency_ms, parse_seconds, parse_zone_volume,
        };
        // zone_1_volume, command_template "{{ (value | float / 100) | round(2) }}"
        for (value, rendered, percent) in [
            (0, "0.0", 0),
            (1, "0.01", 1),
            (7, "0.07", 7),
            (33, "0.33", 33),
            (50, "0.5", 50),
            (100, "1.0", 100),
        ] {
            assert_eq!(
                parse_zone_volume(rendered),
                Ok(percent),
                "slider at {value}"
            );
        }
        // "{{ value | int }}" renders a float from a number entity as an integer.
        assert_eq!(parse_count("3"), Ok(3), "playlist");
        assert_eq!(parse_seconds("90"), Ok(90), "presence timeout");
        assert_eq!(parse_client_volume("40"), Ok(40), "client volume");
        assert_eq!(parse_latency_ms("-250"), Ok(-250), "latency");
        assert_eq!(parse_latency_ms("20"), Ok(20), "latency");
        // client_1_zone maps "Living Room" to "1" and "Kitchen" to "2".
        assert_eq!((parse_count("1"), parse_count("2")), (Ok(1), Ok(2)));
        // What a plain float would have been (no template): refused, which is why the
        // templates exist.
        assert!(parse_count("3.0").is_err());
        assert!(parse_seconds("90.0").is_err());
    }

    #[test]
    fn zone_names_are_embedded_safely_and_the_select_lists_them() {
        let model = Model {
            zones: vec![(1, "Dad's \"den\" {{ x }}".into()), (2, "Küche".into())],
            clients: vec![(1, "C".into())],
        };
        let p = device_payload(&Topics::new("snapdog"), &test_server(), &model);
        let select = &components(&p)["client_1_zone"];
        assert_eq!(select["options"], json!(["Dad's \"den\" {{ x }}", "Küche"]));
        let value = select["value_template"].as_str().unwrap();
        let command = select["command_template"].as_str().unwrap();
        // The names are JSON string literals inside the template (escaped quotes).
        assert!(value.contains(r#""Dad's \"den\" {{ x }}""#), "{value}");
        assert!(
            command.starts_with("{{ [") && command.ends_with(".index(value) + 1 }}"),
            "{command}"
        );
        // The message is still one valid JSON document.
        serde_json::from_str::<Value>(&p.to_string()).unwrap();
    }

    #[test]
    fn the_device_topic_follows_the_prefix_and_keeps_to_home_assistants_alphabet() {
        assert_eq!(
            device_topic("homeassistant", "0b1c2d3e-aaaa"),
            "homeassistant/device/snapdog_0b1c2d3e-aaaa/config"
        );
        assert_eq!(
            device_topic("ha", "a b/c+d#"),
            "ha/device/snapdog_a_b_c_d_/config"
        );
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(title_case("presence_enabled"), "Presence enabled");
        assert_eq!(title_case("volume"), "Volume");
        assert_eq!(title_case(""), "");
    }

    #[tokio::test]
    async fn the_model_is_read_from_the_server_state_in_index_order() {
        let store = crate::mqtt::test_support::test_state_with_client();
        let model = Model::of(&*store.read().await);
        assert_eq!(
            model.zones,
            [(1, "Zone 1".to_string()), (2, "Zone 2".to_string())]
        );
        assert_eq!(model.clients, [(1, "Test".to_string())]);
    }

    #[test]
    fn a_large_installation_still_fits_one_message() {
        // 16 zones and 64 clients is large for a home; the message has to stay well
        // below the packet limit, or the publish is refused (and logged).
        let model = Model {
            zones: (1..=16).map(|i| (i, format!("Zone number {i}"))).collect(),
            clients: (1..=64)
                .map(|i| (i, format!("Client number {i}")))
                .collect(),
        };
        let size = device_payload(&Topics::new("snapdog"), &test_server(), &model)
            .to_string()
            .len();
        assert!(
            size < crate::mqtt::transport::MAX_OUTGOING_PACKET_BYTES / 2,
            "{size}"
        );
    }

    /// One message holds every entity, so a huge installation can exceed the packet
    /// limit. That is refused with an error in the log, not sent as a message that
    /// would end the connection.
    #[test]
    fn an_absurdly_large_installation_is_refused_by_the_publisher() {
        let model = Model {
            zones: (1..=64).map(|i| (i, format!("Zone number {i}"))).collect(),
            clients: (1..=512)
                .map(|i| (i, format!("Client number {i}")))
                .collect(),
        };
        let size = device_payload(&Topics::new("snapdog"), &test_server(), &model)
            .to_string()
            .len();
        assert!(
            size > crate::mqtt::transport::MAX_OUTGOING_PACKET_BYTES,
            "{size}"
        );
    }
}
