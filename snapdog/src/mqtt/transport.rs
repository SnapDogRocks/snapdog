// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Connection options: address, credentials, last will and TLS.
//!
//! **TLS stack.** rumqttc's own rustls feature pulls in `rustls-webpki` 0.102,
//! which carries advisories the project deliberately keeps out of its graph (see
//! ADR-020 and `docs/dependencies.md`), and there is no newer rumqttc. The
//! `use-native-tls` feature the workspace already builds with is therefore the TLS
//! backend of this connection (OpenSSL on Linux, Secure Transport on macOS, `SChannel`
//! on Windows). The platform stack is already in the binary for librespot, so this adds
//! no TLS library.
//!
//! **Policy.** TLS 1.2 or newer, and the highest the platform offers (1.3 with
//! OpenSSL; Secure Transport has no 1.3). The server certificate and its host name
//! are always verified, against the system trust store, or against `ca_file` alone
//! when one is given. There is no switch that turns verification off. A handshake
//! that fails is a connection error that is retried: the connection is never
//! downgraded to plaintext.

use anyhow::Result;
use rumqttc::{LastWill, MqttOptions, QoS, TlsConfiguration, Transport};

use super::tls::tls_connector;
use super::topics::Topics;
use crate::config::MqttConfig;

/// MQTT keep-alive interval.
const MQTT_KEEP_ALIVE: std::time::Duration = std::time::Duration::from_mins(1);

/// Largest packet accepted from the broker, in bytes (rumqttc's own default is
/// 10 KiB). A packet over the limit is not skipped, it ends the connection, and a
/// *retained* one is replayed by the broker at every subscribe, so a limit that is
/// too low turns one oversized retained message under `{base}.../set` into an
/// endless reconnect loop that no command survives. 1 MiB lets such a message
/// arrive and be rejected by the 256-byte payload check; the broker's own
/// `message_size_limit` is the real bound (see `docs/mqtt.md`).
pub const MAX_INCOMING_PACKET_BYTES: usize = 1 << 20;
/// Largest packet sent to the broker, in bytes. Our payloads are bounded on their
/// own (see `payloads.rs`); this is the backstop, large enough for the discovery
/// message of a big installation.
pub const MAX_OUTGOING_PACKET_BYTES: usize = 1 << 20;

/// Connection options for `config`: address, keep-alive, credentials, TLS and the
/// last will (`offline`, retained, on the availability topic).
///
/// # Errors
///
/// Returns an error if the broker address is invalid or the TLS material cannot be
/// used.
pub fn mqtt_options(config: &MqttConfig, topics: &Topics) -> Result<MqttOptions> {
    let endpoint = config.endpoint()?;
    let mut opts = MqttOptions::new(&config.client_id, &endpoint.host, endpoint.port);
    opts.set_keep_alive(MQTT_KEEP_ALIVE);
    opts.set_max_packet_size(MAX_INCOMING_PACKET_BYTES, MAX_OUTGOING_PACKET_BYTES);
    if !config.username.is_empty() {
        opts.set_credentials(&config.username, &*config.password);
    }
    opts.set_last_will(LastWill::new(
        topics.availability(),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    if endpoint.tls {
        let connector = tls_connector(config)?;
        opts.set_transport(Transport::tls_with_config(
            TlsConfiguration::NativeConnector(connector),
        ));
    }
    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::test_pki;

    /// The `[mqtt]` section from TOML, resolved the way the config loader does.
    fn config(toml: &str) -> MqttConfig {
        let mut c: MqttConfig = toml::from_str(toml).unwrap();
        c.resolve().unwrap();
        c
    }

    /// The section as written, not resolved: what a caller that skipped the config
    /// loader would hand to the bridge.
    fn unresolved(toml: &str) -> MqttConfig {
        toml::from_str(toml).unwrap()
    }

    fn options(c: &MqttConfig) -> Result<MqttOptions> {
        mqtt_options(c, &Topics::new(&c.base_topic))
    }

    #[test]
    fn a_plain_address_gives_a_plaintext_connection_as_before() {
        let c = config("broker = \"192.168.1.10:1883\"\nusername = \"u\"\npassword = \"p\"");
        let opts = options(&c).unwrap();
        assert!(matches!(opts.transport(), Transport::Tcp));
        assert_eq!(opts.broker_address(), ("192.168.1.10".to_string(), 1883));
        assert_eq!(opts.client_id(), "snapdog");
        let login = opts.credentials().expect("credentials");
        assert_eq!(
            (login.username.as_str(), login.password.as_str()),
            ("u", "p")
        );
        assert!(opts.clean_session(), "the session is deliberately clean");
        let will = opts.last_will().expect("last will");
        assert_eq!(will.topic, "snapdog/status");
        assert_eq!(&will.message[..], b"offline");
        assert!(will.retain);
    }

    #[test]
    fn the_incoming_packet_limit_is_above_rumqttcs_default_so_that_a_retained_command_arrives() {
        // rumqttc's default is 10 KiB: one retained message above it, replayed at
        // every subscribe, drops the connection every time.
        let opts = options(&config("broker = \"localhost:1883\"")).unwrap();
        assert_eq!(opts.max_packet_size(), MAX_INCOMING_PACKET_BYTES);
        assert!(opts.max_packet_size() >= 256 * 1024);
    }

    #[test]
    fn last_will_follows_a_custom_base_topic() {
        let c = config("broker = \"localhost:1883\"\nbase_topic = \"home/audio\"");
        let will = options(&c).unwrap().last_will().expect("last will");
        assert_eq!(will.topic, "home/audio/status");
    }

    #[test]
    fn no_credentials_means_none_are_sent() {
        let opts = options(&config("broker = \"localhost:1883\"")).unwrap();
        assert!(opts.credentials().is_none());
    }

    #[test]
    fn a_tls_address_gives_a_tls_transport_on_the_default_port() {
        let opts = options(&config("broker = \"mqtts://broker.example.org\"")).unwrap();
        assert!(matches!(opts.transport(), Transport::Tls(_)));
        assert_eq!(
            opts.broker_address(),
            ("broker.example.org".to_string(), 8883)
        );
    }

    #[test]
    fn a_ca_file_and_a_client_identity_make_a_tls_client() {
        let pki = test_pki::generate();
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, content: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, content).unwrap();
            p.display().to_string()
        };
        let ca = write("ca.pem", &pki.ca);
        let cert = write("client.pem", &pki.client_cert);
        let key = write("client.key", &pki.client_key);

        let c = config(&format!(
            "broker = \"mqtts://localhost:8883\"\nca_file = {ca:?}\nclient_cert = {cert:?}\nclient_key = {key:?}"
        ));
        assert!(matches!(
            options(&c).unwrap().transport(),
            Transport::Tls(_)
        ));
    }

    #[test]
    fn a_ca_bundle_with_several_certificates_is_accepted() {
        let pki = test_pki::generate();
        let bundle = format!("{}\n{}", pki.ca, test_pki::unrelated_ca_pem());
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("bundle.pem");
        std::fs::write(&ca, bundle).unwrap();
        let c = config(&format!(
            "broker = \"mqtts://localhost\"\nca_file = {:?}",
            ca.display().to_string()
        ));
        assert!(options(&c).is_ok());
    }

    #[test]
    fn unusable_tls_material_is_an_error_and_never_a_plaintext_connection() {
        let pki = test_pki::generate();
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, content: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, content).unwrap();
            p.display().to_string()
        };
        // Looks like PEM to the config check, is not.
        let bogus_ca = write(
            "bogus-ca.pem",
            "-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n",
        );
        let c = unresolved(&format!(
            "broker = \"mqtts://localhost\"\nca_file = {bogus_ca:?}"
        ));
        let err = format!("{:#}", options(&c).unwrap_err());
        assert!(err.contains("ca_file"), "{err}");

        // A key that does not belong to the certificate (a key of another pair).
        let cert = write("client.pem", &pki.client_cert);
        let other_key = write("other.key", &test_pki::generate().client_key);
        let c = unresolved(&format!(
            "broker = \"mqtts://localhost\"\nclient_cert = {cert:?}\nclient_key = {other_key:?}"
        ));
        let err = format!("{:#}", options(&c).unwrap_err());
        assert!(err.contains("client_key"), "{err}");
    }
}
