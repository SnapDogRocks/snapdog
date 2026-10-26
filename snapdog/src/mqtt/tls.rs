// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The TLS client of the MQTT connection, built from the `[mqtt]` PEM files.
//!
//! Built twice on purpose: once when the configuration is loaded
//! ([`check_material`], so that `--check-config` and the start-up log say what is
//! wrong with a file), once when the bridge starts. The policy is described in
//! [`super::transport`].

use anyhow::{Context, Result};
use rumqttc::tokio_native_tls::native_tls::{Certificate, Identity, Protocol, TlsConnector};

use crate::config::{MAX_PEM_BYTES, MqttConfig, read_bounded};

/// The TLS client for `config`: TLS 1.2 or newer, verification on, optional private
/// CA (replacing the system roots) and optional client certificate.
pub fn tls_connector(config: &MqttConfig) -> Result<TlsConnector> {
    let mut builder = TlsConnector::builder();
    builder.min_protocol_version(Some(Protocol::Tlsv12));
    // Deliberately never called: danger_accept_invalid_certs and
    // danger_accept_invalid_hostnames.

    if let Some(path) = &config.ca_file {
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
        // A private CA is the only CA that may vouch for this broker.
        builder.disable_built_in_roots(true);
        for root in roots {
            builder.add_root_certificate(root);
        }
    }

    if let (Some(cert), Some(key)) = (&config.client_cert, &config.client_key) {
        let identity = Identity::from_pkcs8(&read(cert, "client_cert")?, &read(key, "client_key")?)
            .with_context(|| {
                format!(
                    "[mqtt] client_cert {} and client_key {} do not make a TLS identity (does the \
                     key belong to the certificate, is it unencrypted PKCS#8?)",
                    cert.display(),
                    key.display()
                )
            })?;
        builder.identity(identity);
    }

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
