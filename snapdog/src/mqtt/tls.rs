// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The TLS client of the MQTT connection, built from the `[mqtt]` PEM files.
//!
//! Built twice on purpose: once when the configuration is loaded
//! ([`check_material`], so that `--check-config` and the start-up log say what is
//! wrong with a file), once when the bridge starts. The policy is described in
//! [`super::transport`].

use anyhow::{Context, Result};
use rumqttc::tokio_native_tls::native_tls::{
    Certificate, Identity, Protocol, TlsConnector, TlsConnectorBuilder,
};

use crate::config::{MAX_PEM_BYTES, MqttConfig, read_bounded};

/// What the policy asks of the TLS library, and nothing else: the calls that
/// matter for security are the only ones this trait has, so that a test can see
/// them, and `danger_accept_invalid_certs` and `danger_accept_invalid_hostnames`
/// are not among them.
trait TlsSetup {
    fn min_protocol_version(&mut self, protocol: Protocol);
    /// Do not trust the platform's trust store.
    fn disable_built_in_roots(&mut self);
    fn add_root_certificate(&mut self, root: Certificate);
    fn identity(&mut self, identity: Identity);
}

// The delegation to the real builder: one line per call, and not covered by a test. The
// builder cannot be inspected, and a handshake cannot tell the mutants apart: a client that
// keeps the system trust store next to `ca_file` still verifies the test broker, and the
// macOS (Secure Transport), where it was tried, does not complete a TLS 1.1 handshake
// whatever the client asks for (OpenSSL 3 is expected to behave the same, not tried), so a
// missing minimum version changes nothing observable. Keep these lines trivial.
impl TlsSetup for TlsConnectorBuilder {
    fn min_protocol_version(&mut self, protocol: Protocol) {
        Self::min_protocol_version(self, Some(protocol));
    }

    fn disable_built_in_roots(&mut self) {
        Self::disable_built_in_roots(self, true);
    }

    fn add_root_certificate(&mut self, root: Certificate) {
        Self::add_root_certificate(self, root);
    }

    fn identity(&mut self, identity: Identity) {
        Self::identity(self, identity);
    }
}

/// The parsed TLS material of a section.
struct Material {
    /// The private CA(s) of `ca_file`, if one is configured.
    roots: Option<Vec<Certificate>>,
    identity: Option<Identity>,
}

/// The policy: TLS 1.2 or newer; verification of the certificate and of its host
/// name stays on (nothing here can turn it off); a private CA replaces the
/// system roots instead of joining them, so that no public CA can vouch for a
/// broker that is supposed to be vouched for by a private one; the client identity,
/// if any.
fn apply_policy(setup: &mut impl TlsSetup, material: Material) {
    setup.min_protocol_version(Protocol::Tlsv12);
    if let Some(roots) = material.roots {
        setup.disable_built_in_roots();
        for root in roots {
            setup.add_root_certificate(root);
        }
    }
    if let Some(identity) = material.identity {
        setup.identity(identity);
    }
}

/// Read and parse the PEM files named by `config`.
fn load_material(config: &MqttConfig) -> Result<Material> {
    let roots = if let Some(path) = &config.ca_file {
        let pem = read(path, "ca_file")?;
        let roots = Certificate::stack_from_pem(&pem).with_context(|| {
            format!(
                "[mqtt] ca_file {}: not a PEM certificate bundle",
                path.display()
            )
        })?;
        anyhow::ensure!(
            !roots.is_empty(),
            "[mqtt] ca_file {} contains no certificate",
            path.display()
        );
        Some(roots)
    } else {
        None
    };

    let identity = if let (Some(cert), Some(key)) = (&config.client_cert, &config.client_key) {
        Some(
            Identity::from_pkcs8(&read(cert, "client_cert")?, &read(key, "client_key")?)
                .with_context(|| {
                    format!(
                        "[mqtt] client_cert {} and client_key {} do not make a TLS identity \
                         (does the key belong to the certificate, is it unencrypted PKCS#8?)",
                        cert.display(),
                        key.display()
                    )
                })?,
        )
    } else {
        None
    };
    Ok(Material { roots, identity })
}

/// The TLS client for `config` (see [`apply_policy`]).
pub fn tls_connector(config: &MqttConfig) -> Result<TlsConnector> {
    let mut builder = TlsConnector::builder();
    apply_policy(&mut builder, load_material(config)?);
    builder
        .build()
        .context("[mqtt] cannot set up the TLS client")
}

fn read(path: &std::path::Path, key: &str) -> Result<Vec<u8>> {
    read_bounded(path, key, MAX_PEM_BYTES)
}

/// Check that the TLS material of `config` is usable: the CA file holds a
/// certificate this platform's TLS library accepts, and the client key belongs to
/// the client certificate. Nothing is dialled.
///
/// # Errors
///
/// Returns an error that names the key and what is wrong with it.
pub fn check_material(config: &MqttConfig) -> Result<()> {
    tls_connector(config).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt::test_pki;

    /// Records what the policy asks of the TLS library.
    #[derive(Default)]
    struct Recorder(Vec<String>);

    impl TlsSetup for Recorder {
        fn min_protocol_version(&mut self, protocol: Protocol) {
            self.0.push(format!("min_protocol_version={protocol:?}"));
        }

        fn disable_built_in_roots(&mut self) {
            self.0.push("disable_built_in_roots".into());
        }

        fn add_root_certificate(&mut self, _root: Certificate) {
            self.0.push("add_root_certificate".into());
        }

        fn identity(&mut self, _identity: Identity) {
            self.0.push("identity".into());
        }
    }

    fn config(toml: &str) -> MqttConfig {
        toml::from_str(toml).unwrap()
    }

    fn calls(config: &MqttConfig) -> Vec<String> {
        let mut recorder = Recorder::default();
        apply_policy(&mut recorder, load_material(config).unwrap());
        recorder.0
    }

    /// Review mutation M8: dropping the minimum version went unnoticed.
    #[test]
    fn tls_1_2_is_the_minimum_with_or_without_private_material() {
        let calls = calls(&config("broker = \"mqtts://b\""));
        assert_eq!(calls, ["min_protocol_version=Tlsv12"]);
    }

    /// Review mutation M7: keeping the system trust store next to `ca_file` went
    /// unnoticed, although it lets any public CA vouch for a broker that was meant to
    /// be vouched for by the private one.
    #[test]
    fn a_private_ca_replaces_the_system_trust_store() {
        let pki = test_pki::generate();
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("ca.pem");
        std::fs::write(&ca, format!("{}\n{}", pki.ca, test_pki::unrelated_ca_pem())).unwrap();
        let calls = calls(&config(&format!(
            "broker = \"mqtts://b\"\nca_file = {:?}",
            ca.display().to_string()
        )));
        assert_eq!(
            calls,
            [
                "min_protocol_version=Tlsv12",
                "disable_built_in_roots",
                "add_root_certificate",
                "add_root_certificate",
            ]
        );
    }

    #[test]
    fn without_a_ca_file_the_system_trust_store_stays() {
        let calls = calls(&config("broker = \"mqtts://b\""));
        assert!(!calls.contains(&"disable_built_in_roots".to_string()));
    }

    #[test]
    fn a_client_identity_is_presented_only_when_one_is_configured() {
        let pki = test_pki::generate();
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = (dir.path().join("c.pem"), dir.path().join("c.key"));
        std::fs::write(&cert, &pki.client_cert).unwrap();
        std::fs::write(&key, &pki.client_key).unwrap();
        let with = calls(&config(&format!(
            "broker = \"mqtts://b\"\nclient_cert = {:?}\nclient_key = {:?}",
            cert.display().to_string(),
            key.display().to_string()
        )));
        assert!(with.contains(&"identity".to_string()), "{with:?}");
        assert!(!calls(&config("broker = \"mqtts://b\"")).contains(&"identity".to_string()));
    }
}
