// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! A throwaway PKI for the TLS tests: a CA, a server certificate for `localhost`
//! and `127.0.0.1`, and a client certificate. Generated afresh for every test run,
//! valid for a few days, never written anywhere but a temporary directory.
//!
//! Included by the unit tests (`mqtt::test_pki`) and, through `#[path]`, by
//! `tests/mqtt_tier2.rs`.

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, PKCS_RSA_SHA256,
};
use time::{Duration, OffsetDateTime};

/// PEM strings of one CA and the certificates it signed.
pub struct Pki {
    pub ca: String,
    pub server_cert: String,
    pub server_key: String,
    pub client_cert: String,
    /// PKCS#8 (a PEM `PRIVATE KEY` block).
    pub client_key: String,
}

fn dn(common_name: &str) -> DistinguishedName {
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, common_name);
    name
}

/// A CA certificate valid from yesterday for a week.
fn ca_params(common_name: &str) -> CertificateParams {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
    params.distinguished_name = dn(common_name);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(7);
    params
}

fn leaf_params(
    common_name: &str,
    sans: &[&str],
    usage: ExtendedKeyUsagePurpose,
) -> CertificateParams {
    let mut params =
        CertificateParams::new(sans.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
            .expect("leaf params");
    params.distinguished_name = dn(common_name);
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![usage];
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(7);
    params
}

/// A new CA with a server certificate for `localhost`, `127.0.0.1` and `::1`, and a
/// client certificate.
pub fn generate() -> Pki {
    generate_for(&["localhost", "127.0.0.1", "::1"])
}

/// Like [`generate`], with a server certificate for exactly `server_names` (DNS names
/// or IP addresses): a connection to any other name must not verify.
pub fn generate_for(server_names: &[&str]) -> Pki {
    let ca_key = KeyPair::generate().expect("CA key");
    let ca_params = ca_params("SnapDog test CA");
    let ca_cert = ca_params.self_signed(&ca_key).expect("CA certificate");
    let issuer = Issuer::from_params(&ca_params, &ca_key);

    let server_key = KeyPair::generate().expect("server key");
    let server_cert = leaf_params(
        "localhost",
        server_names,
        ExtendedKeyUsagePurpose::ServerAuth,
    )
    .signed_by(&server_key, &issuer)
    .expect("server certificate");

    // RSA: macOS's Secure Transport imports no other kind of PEM key
    // (`Identity::from_pkcs8` fails with "Unknown format in import" for EC keys).
    let client_key = KeyPair::generate_for(&PKCS_RSA_SHA256).expect("client key");
    let client_cert = leaf_params(
        "snapdog-test-client",
        &[],
        ExtendedKeyUsagePurpose::ClientAuth,
    )
    .signed_by(&client_key, &issuer)
    .expect("client certificate");

    Pki {
        ca: ca_cert.pem(),
        server_cert: server_cert.pem(),
        server_key: server_key.serialize_pem(),
        client_cert: client_cert.pem(),
        client_key: client_key.serialize_pem(),
    }
}

/// The PEM of a CA that signed nothing the tests use: trusting it must not make a
/// broker of [`generate`] verifiable.
pub fn unrelated_ca_pem() -> String {
    let key = KeyPair::generate().expect("CA key");
    ca_params("Some other CA")
        .self_signed(&key)
        .expect("CA certificate")
        .pem()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_material_is_pem_in_the_expected_formats() {
        let pki = generate();
        for cert in [&pki.ca, &pki.server_cert, &pki.client_cert] {
            assert!(cert.starts_with("-----BEGIN CERTIFICATE-----"));
        }
        for key in [&pki.server_key, &pki.client_key] {
            // PKCS#8 is the plain "PRIVATE KEY" label (spelled in parts, see config/mqtt.rs).
            let header = format!("{0}BEGIN PRIVATE KEY{0}", "-----");
            assert!(key.starts_with(&header), "PKCS#8");
        }
        assert_ne!(pki.ca, unrelated_ca_pem());
    }
}
