// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The `[mqtt]` section: broker address, credentials and TLS material.
//!
//! Everything that can be checked without talking to the broker is checked when
//! the configuration is loaded, and the messages say which key is wrong and why.
//! A secret never appears in an error message or in `Debug` output.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use super::types::SecretString;

/// Port of a plaintext broker listener when the address names none.
const DEFAULT_PORT: u16 = 1883;
/// Port of a TLS broker listener when the address names none.
const DEFAULT_TLS_PORT: u16 = 8883;
/// Largest certificate or key file read (PEM, a few kilobytes in practice).
const MAX_PEM_BYTES: u64 = 1 << 20;
/// Largest password file read.
const MAX_SECRET_FILE_BYTES: u64 = 4096;
/// Longest base topic, in bytes. MQTT allows 65535; this is far beyond sensible.
const MAX_BASE_TOPIC_BYTES: usize = 256;

/// MQTT bridge configuration.
///
/// A plain `broker = "host:1883"` is a plaintext connection, as it has always
/// been. A TLS listener is selected by the scheme of `broker` (`mqtts://`,
/// `ssl://`); there is no separate switch and no way to turn certificate or host
/// name verification off.
///
/// Environment variables override the file (and win over `password_file`):
/// `SNAPDOG_MQTT_PASSWORD`, `SNAPDOG_MQTT_USERNAME`, `SNAPDOG_MQTT_CA_FILE`,
/// `SNAPDOG_MQTT_CLIENT_CERT`, `SNAPDOG_MQTT_CLIENT_KEY`. They apply only if an
/// `[mqtt]` section (or, in KNX device mode, an ETS-programmed broker) exists.
#[derive(Debug, Deserialize, Clone)]
pub struct MqttConfig {
    /// Broker address: `host:port`, `mqtt://host[:port]` or `tcp://host[:port]`
    /// (plaintext, port 1883 unless named), `mqtts://host[:port]` or
    /// `ssl://host[:port]` (TLS, port 8883 unless named). IPv6 literals in
    /// brackets (plaintext only: a TLS certificate is matched against a name).
    /// No credentials in the address: use `username` and `password`.
    pub broker: String,
    /// MQTT client ID. Defaults to "snapdog". Must be unique per broker: a second
    /// client with the same ID makes the broker drop the first.
    #[serde(default = "default_mqtt_client_id")]
    pub client_id: String,
    /// MQTT username (empty for anonymous).
    #[serde(default)]
    pub username: String,
    /// MQTT password. Needs a `username`. Prefer `password_file` or the
    /// `SNAPDOG_MQTT_PASSWORD` environment variable over writing it into the file.
    #[serde(default)]
    pub password: SecretString,
    /// File that holds the password (one line; a trailing newline is dropped), for
    /// Docker/Kubernetes secrets and systemd credentials. Exclusive with `password`.
    #[serde(default)]
    pub password_file: Option<PathBuf>,
    /// Base topic prefix. Trailing slash is added automatically if missing.
    #[serde(
        default = "default_mqtt_base_topic",
        deserialize_with = "deserialize_base_topic"
    )]
    pub base_topic: String,
    /// PEM file with the CA certificate(s) that sign the broker's certificate.
    /// Replaces the system trust store for this connection (a private CA is the
    /// only CA that may vouch for your broker). Needs a TLS broker address.
    #[serde(default)]
    pub ca_file: Option<PathBuf>,
    /// PEM file with the client certificate (and its chain) for mutual TLS. Needs
    /// `client_key`.
    #[serde(default)]
    pub client_cert: Option<PathBuf>,
    /// PEM file with the private key of `client_cert`: unencrypted PKCS#8 (a PEM
    /// `PRIVATE KEY` block, not `RSA PRIVATE KEY` or `EC PRIVATE KEY`). Convert with
    /// `openssl pkey -in old.key -out client.key`.
    #[serde(default)]
    pub client_key: Option<PathBuf>,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            broker: String::new(),
            client_id: default_mqtt_client_id(),
            username: String::new(),
            password: SecretString::default(),
            password_file: None,
            base_topic: default_mqtt_base_topic(),
            ca_file: None,
            client_cert: None,
            client_key: None,
        }
    }
}

fn default_mqtt_client_id() -> String {
    "snapdog".into()
}

fn default_mqtt_base_topic() -> String {
    "snapdog/".into()
}

/// Normalise an MQTT base topic to end in exactly one `/`. Idempotent.
///
/// The single definition of the normalisation: the config deserializer and the
/// MQTT bridge's topic scheme both call it, so a base topic means the same thing
/// wherever it is read.
#[must_use]
pub fn normalize_base_topic(base: &str) -> String {
    let mut v = base.to_string();
    if !v.ends_with('/') {
        v.push('/');
    }
    v
}

fn deserialize_base_topic<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    Ok(normalize_base_topic(&String::deserialize(deserializer)?))
}

/// Where and how to reach the broker, from [`MqttConfig::broker`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerEndpoint {
    /// Host name or IP literal. IPv6 literals keep their brackets, which is what
    /// the socket layer wants.
    pub host: String,
    /// TCP port: the one named in the address, else 1883 (plaintext) or 8883 (TLS).
    pub port: u16,
    /// Whether the connection is TLS.
    pub tls: bool,
}

impl BrokerEndpoint {
    /// Parse a broker address (see [`MqttConfig::broker`]).
    ///
    /// # Errors
    ///
    /// Returns an error naming what is wrong. The address is never quoted, as it
    /// could contain a password that was put there by mistake.
    pub fn parse(broker: &str) -> Result<Self> {
        let broker = broker.trim();
        ensure!(!broker.is_empty(), "[mqtt] broker is empty");
        ensure!(
            !broker.contains('@'),
            "[mqtt] broker must not contain credentials (user:password@host): \
             use the username and password keys"
        );

        let (scheme, rest) = broker
            .split_once("://")
            .map_or((None, broker), |(s, r)| (Some(s.to_ascii_lowercase()), r));
        let tls = match scheme.as_deref() {
            None | Some("mqtt" | "tcp") => false,
            Some("mqtts" | "ssl" | "tls") => true,
            Some("ws" | "wss" | "http" | "https") => bail!(
                "[mqtt] broker: WebSocket and HTTP transports are not supported \
                 (use mqtt:// or mqtts://)"
            ),
            Some(_) => bail!(
                "[mqtt] broker has an unknown scheme (use mqtt://, tcp://, mqtts:// or ssl://, \
                 or just host:port)"
            ),
        };
        ensure!(
            !rest.contains(['/', '?', '#']) && !rest.contains(char::is_whitespace),
            "[mqtt] broker must be host[:port] without a path, query or spaces"
        );

        let (host, port) = split_host_port(rest)?;
        let default_port = if tls { DEFAULT_TLS_PORT } else { DEFAULT_PORT };
        Ok(Self {
            host,
            port: port.unwrap_or(default_port),
            tls,
        })
    }

    /// Whether the broker is on this machine: `localhost`, `*.localhost` or a
    /// loopback address. A name that merely resolves to one does not count.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        let host = self.host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = host.parse::<IpAddr>() {
            return ip.is_loopback();
        }
        let host = host.to_ascii_lowercase();
        host == "localhost" || host.ends_with(".localhost")
    }
}

/// Split `host[:port]` / `[v6][:port]`.
fn split_host_port(rest: &str) -> Result<(String, Option<u16>)> {
    let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
        let (addr, after) = inner
            .split_once(']')
            .context("[mqtt] broker has an unterminated IPv6 address")?;
        addr.parse::<std::net::Ipv6Addr>()
            .ok()
            .context("[mqtt] broker has an invalid IPv6 address")?;
        let port = match after {
            "" => None,
            _ => Some(
                after
                    .strip_prefix(':')
                    .context("[mqtt] broker has text after the IPv6 address")?,
            ),
        };
        (format!("[{addr}]"), port)
    } else {
        match rest.matches(':').count() {
            0 => (rest.to_string(), None),
            1 => {
                let (h, p) = rest.split_once(':').unwrap_or((rest, ""));
                (h.to_string(), Some(p))
            }
            _ => bail!("[mqtt] broker: an IPv6 address needs brackets ([::1]:1883)"),
        }
    };

    ensure!(!host.is_empty(), "[mqtt] broker has no host");
    if !host.starts_with('[') {
        ensure!(
            host.len() <= 253
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_')),
            "[mqtt] broker host may only contain letters, digits, '-', '.' and '_'"
        );
    }
    let port = match port {
        None => None,
        Some(p) => {
            let parsed: u16 = p
                .parse()
                .ok()
                .filter(|p| *p != 0)
                .context("[mqtt] broker port must be a number from 1 to 65535")?;
            Some(parsed)
        }
    };
    Ok((host, port))
}

impl MqttConfig {
    /// The parsed broker address.
    ///
    /// # Errors
    ///
    /// See [`BrokerEndpoint::parse`].
    pub fn endpoint(&self) -> Result<BrokerEndpoint> {
        BrokerEndpoint::parse(&self.broker)
    }

    /// Check the section and load file-based secrets.
    ///
    /// Run once after the environment overrides. Reads `password_file` into
    /// `password` (and then forgets the path), and reads the certificate and key
    /// files far enough to tell a wrong file from a right one.
    ///
    /// # Errors
    ///
    /// Returns an error that names the key and what is wrong with it.
    pub fn resolve(&mut self) -> Result<()> {
        let endpoint = self.endpoint()?;

        ensure!(
            !self.client_id.is_empty()
                && self.client_id.len() <= 256
                && !self.client_id.chars().any(char::is_control),
            "[mqtt] client_id must be 1 to 256 characters without control characters"
        );
        ensure!(
            !self.username.chars().any(char::is_control),
            "[mqtt] username must not contain control characters"
        );
        validate_base_topic(&self.base_topic)?;

        // Credentials
        if let Some(path) = self.password_file.take() {
            ensure!(
                self.password.is_empty(),
                "[mqtt] password and password_file are both set: use only one"
            );
            let secret = read_secret_file(&path, "password_file")?;
            ensure!(
                !secret.is_empty(),
                "[mqtt] password_file {} is empty",
                path.display()
            );
            self.password = SecretString::from(secret);
        }
        ensure!(
            self.password.is_empty() || !self.username.is_empty(),
            "[mqtt] a password is set but username is empty: the broker would never see it. \
             Set username, or remove the password"
        );

        // TLS material
        let tls_options =
            self.ca_file.is_some() || self.client_cert.is_some() || self.client_key.is_some();
        if tls_options && !endpoint.tls {
            bail!(
                "[mqtt] ca_file, client_cert and client_key apply to TLS only, but the broker \
                 address is plaintext. Use an mqtts:// address, or remove them"
            );
        }
        ensure!(
            self.client_cert.is_some() == self.client_key.is_some(),
            "[mqtt] client_cert and client_key must be set together (mutual TLS needs both)"
        );
        if endpoint.tls {
            ensure!(
                !endpoint.host.starts_with('['),
                "[mqtt] broker: TLS needs a host name, not an IPv6 literal: the certificate \
                 is matched against the name"
            );
        }
        if let Some(path) = &self.ca_file {
            let pem = read_pem(path, "ca_file")?;
            ensure!(
                pem.contains("-----BEGIN CERTIFICATE-----"),
                "[mqtt] ca_file {} contains no PEM certificate (-----BEGIN CERTIFICATE-----)",
                path.display()
            );
        }
        if let Some(path) = &self.client_cert {
            let pem = read_pem(path, "client_cert")?;
            ensure!(
                pem.contains("-----BEGIN CERTIFICATE-----"),
                "[mqtt] client_cert {} contains no PEM certificate (-----BEGIN CERTIFICATE-----)",
                path.display()
            );
        }
        if let Some(path) = &self.client_key {
            let pem = read_pem(path, "client_key")?;
            check_client_key(&pem, path)?;
        }
        Ok(())
    }

    /// A warning if credentials would be sent unencrypted to another machine, which
    /// anybody on the path can read (and replay). `None` otherwise.
    #[must_use]
    pub fn plaintext_credentials_warning(&self) -> Option<String> {
        let endpoint = self.endpoint().ok()?;
        let has_credentials = !self.username.is_empty() || !self.password.is_empty();
        (has_credentials && !endpoint.tls && !endpoint.is_loopback()).then(|| {
            format!(
                "MQTT credentials are sent unencrypted to {}:{}: anyone on the network path can \
                 read them. Use an mqtts:// broker address",
                endpoint.host, endpoint.port
            )
        })
    }
}

/// MQTT topic rules for a prefix that is both published to and subscribed under.
fn validate_base_topic(base: &str) -> Result<()> {
    ensure!(
        base.len() <= MAX_BASE_TOPIC_BYTES,
        "[mqtt] base_topic is longer than {MAX_BASE_TOPIC_BYTES} bytes"
    );
    ensure!(
        !base.contains(['+', '#', '\0']),
        "[mqtt] base_topic must not contain the wildcards '+' and '#' or NUL"
    );
    ensure!(
        !base.starts_with('$'),
        "[mqtt] base_topic must not start with '$' (reserved for broker topics such as $SYS)"
    );
    ensure!(
        !base.chars().any(char::is_control),
        "[mqtt] base_topic must not contain control characters"
    );
    Ok(())
}

/// Read a small text file, with a size limit and an error that names the key.
fn read_limited(path: &Path, key: &str, limit: u64) -> Result<String> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("[mqtt] {key}: cannot read {}", path.display()))?;
    ensure!(
        meta.is_file(),
        "[mqtt] {key}: {} is not a regular file",
        path.display()
    );
    ensure!(
        meta.len() <= limit,
        "[mqtt] {key}: {} is larger than {limit} bytes",
        path.display()
    );
    std::fs::read_to_string(path)
        .with_context(|| format!("[mqtt] {key}: cannot read {} as UTF-8 text", path.display()))
}

fn read_pem(path: &Path, key: &str) -> Result<String> {
    read_limited(path, key, MAX_PEM_BYTES)
}

fn read_secret_file(path: &Path, key: &str) -> Result<String> {
    let mut text = read_limited(path, key, MAX_SECRET_FILE_BYTES)?;
    // Secret files end in a newline more often than not; a password does not.
    while text.ends_with(['\n', '\r']) {
        text.pop();
    }
    Ok(text)
}

/// The `-----BEGIN <label>-----` line of a PEM block.
///
/// Built from parts on purpose: a private-key header spelled out in the source
/// trips every secret scanner (gitleaks runs in this repository's hooks).
fn pem_begin(label: &str) -> String {
    const DASHES: &str = "-----";
    format!("{DASHES}BEGIN {label}{DASHES}")
}

/// The key must be an unencrypted PKCS#8 key: the one format every TLS backend
/// `SnapDog` runs on reads.
fn check_client_key(pem: &str, path: &Path) -> Result<()> {
    let pkcs8 = pem_begin("PRIVATE KEY");
    if pem.contains(&pkcs8) {
        return Ok(());
    }
    let convert = "convert it with: openssl pkey -in <old key> -out <new key>";
    if pem.contains(&pem_begin("ENCRYPTED PRIVATE KEY")) {
        bail!(
            "[mqtt] client_key {} is encrypted, and SnapDog cannot ask for a passphrase; {convert}",
            path.display()
        );
    }
    if pem.contains(&pem_begin("RSA PRIVATE KEY")) || pem.contains(&pem_begin("EC PRIVATE KEY")) {
        bail!(
            "[mqtt] client_key {} is in the traditional (PKCS#1/SEC1) format, but PKCS#8 is \
             required ({pkcs8}); {convert}",
            path.display()
        );
    }
    bail!(
        "[mqtt] client_key {} contains no PEM private key ({pkcs8})",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(broker: &str) -> Result<BrokerEndpoint> {
        BrokerEndpoint::parse(broker)
    }

    fn ep(host: &str, port: u16, tls: bool) -> BrokerEndpoint {
        BrokerEndpoint {
            host: host.into(),
            port,
            tls,
        }
    }

    #[test]
    fn a_plain_host_port_stays_a_plaintext_connection() {
        // What every existing config has: nothing about it may change.
        assert_eq!(
            endpoint("192.168.1.10:1883").unwrap(),
            ep("192.168.1.10", 1883, false)
        );
        assert_eq!(endpoint("mqtt:1883").unwrap(), ep("mqtt", 1883, false));
        assert_eq!(
            endpoint("localhost:1884").unwrap(),
            ep("localhost", 1884, false)
        );
        assert_eq!(
            endpoint("  broker.lan:1883 ").unwrap(),
            ep("broker.lan", 1883, false)
        );
    }

    #[test]
    fn schemes_select_plaintext_or_tls_with_their_default_ports() {
        assert_eq!(
            endpoint("mqtt://broker").unwrap(),
            ep("broker", 1883, false)
        );
        assert_eq!(
            endpoint("tcp://broker:1999").unwrap(),
            ep("broker", 1999, false)
        );
        assert_eq!(
            endpoint("mqtts://broker").unwrap(),
            ep("broker", 8883, true)
        );
        assert_eq!(
            endpoint("ssl://broker.example.org:8884").unwrap(),
            ep("broker.example.org", 8884, true)
        );
        assert_eq!(
            endpoint("TLS://Broker:8883").unwrap(),
            ep("Broker", 8883, true)
        );
        assert_eq!(
            endpoint("MQTTS://broker").unwrap(),
            ep("broker", 8883, true)
        );
        // A bare host (no port) is plaintext on the default port.
        assert_eq!(endpoint("broker").unwrap(), ep("broker", 1883, false));
    }

    #[test]
    fn ipv6_literals_need_brackets_and_keep_them() {
        assert_eq!(endpoint("[::1]:1883").unwrap(), ep("[::1]", 1883, false));
        assert_eq!(
            endpoint("mqtt://[fe80::1]").unwrap(),
            ep("[fe80::1]", 1883, false)
        );
        assert!(endpoint("::1:1883").is_err());
        assert!(endpoint("[::1").is_err());
        assert!(endpoint("[nonsense]:1883").is_err());
        assert!(endpoint("[::1]x").is_err());
    }

    #[test]
    fn credentials_in_the_address_are_refused_without_echoing_them() {
        for broker in [
            "user:hunter2@broker:1883",
            "mqtts://user:hunter2@broker",
            "hunter2@broker",
        ] {
            let err = endpoint(broker).unwrap_err().to_string();
            assert!(err.contains("credentials"), "{err}");
            assert!(!err.contains("hunter2"), "{err}");
        }
    }

    #[test]
    fn unsupported_or_malformed_addresses_are_refused_precisely() {
        for (broker, needle) in [
            ("", "empty"),
            ("   ", "empty"),
            ("ws://broker:9001", "WebSocket"),
            ("wss://broker", "WebSocket"),
            ("https://broker", "WebSocket"),
            ("ftp://broker", "unknown scheme"),
            ("broker:0", "port"),
            ("broker:65536", "port"),
            ("broker:abc", "port"),
            ("broker:", "port"),
            ("mqtt://broker/path", "path"),
            ("broker:1883?x=y", "path"),
            ("bro ker:1883", "spaces"),
            ("mqtt://:1883", "no host"),
            ("br\u{f6}ker:1883", "host may only"),
            ("a:b:c", "IPv6"),
        ] {
            let err = endpoint(broker).unwrap_err().to_string();
            assert!(err.contains(needle), "{broker:?}: {err}");
            assert!(err.starts_with("[mqtt] broker"), "{err}");
        }
    }

    #[test]
    fn loopback_detection() {
        for host in [
            "localhost:1883",
            "LOCALHOST",
            "127.0.0.1:1883",
            "[::1]:1883",
            "broker.localhost",
            "127.5.5.5",
        ] {
            assert!(endpoint(host).unwrap().is_loopback(), "{host}");
        }
        for host in [
            "192.168.1.10:1883",
            "broker.lan",
            "10.0.0.1",
            "[fe80::1]",
            "localhost.example.org",
            "mqtt",
        ] {
            assert!(!endpoint(host).unwrap().is_loopback(), "{host}");
        }
    }

    // ── MqttConfig ────────────────────────────────────────────

    fn parse(toml: &str) -> MqttConfig {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn minimal_and_historic_sections_parse_with_their_old_defaults() {
        let c = parse(r#"broker = "192.168.1.10:1883""#);
        assert_eq!(c.client_id, "snapdog");
        assert_eq!(c.base_topic, "snapdog/");
        assert!(c.username.is_empty() && c.password.is_empty());
        assert!(c.ca_file.is_none() && c.client_cert.is_none() && c.client_key.is_none());
        assert!(c.password_file.is_none());
        let mut c = c;
        c.resolve().unwrap();

        // The example config of the previous release, verbatim.
        let mut c = parse(
            "broker = \"192.168.1.10:1883\"\nbase_topic = \"snapdog\"\nusername = \"snapdog\"\npassword = \"snapdog\"",
        );
        assert_eq!(c.base_topic, "snapdog/");
        c.resolve().unwrap();
    }

    #[test]
    fn debug_output_never_shows_the_password() {
        let c = parse("broker = \"b:1883\"\nusername = \"u\"\npassword = \"hunter2-secret\"");
        let shown = format!("{c:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("********"));
    }

    #[test]
    fn a_password_without_a_username_is_an_error_not_silently_ignored() {
        let mut c = parse("broker = \"b:1883\"\npassword = \"hunter2\"");
        let err = c.resolve().unwrap_err().to_string();
        assert!(
            err.contains("password") && err.contains("username"),
            "{err}"
        );
        assert!(!err.contains("hunter2"));
        // A username alone is fine (token-style auth, brokers without passwords).
        let mut c = parse("broker = \"b:1883\"\nusername = \"u\"");
        c.resolve().unwrap();
    }

    #[test]
    fn a_password_file_is_read_trimmed_and_replaces_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pw");
        for (content, expected) in [
            ("s3cret\n", "s3cret"),
            ("s3cret\r\n", "s3cret"),
            ("s3cret", "s3cret"),
            ("with space \n", "with space "),
            ("a\nb\n", "a\nb"),
        ] {
            std::fs::write(&file, content).unwrap();
            let mut c = parse(&format!(
                "broker = \"b:1883\"\nusername = \"u\"\npassword_file = {:?}",
                file.display().to_string()
            ));
            c.resolve().unwrap();
            assert_eq!(c.password.as_str(), expected, "{content:?}");
            assert!(c.password_file.is_none(), "resolved once, then forgotten");
        }
    }

    #[test]
    fn password_file_errors_name_the_file_and_never_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let mut c = parse(&format!(
            "broker = \"b:1883\"\nusername = \"u\"\npassword_file = {:?}",
            missing.display().to_string()
        ));
        let err = format!("{:#}", c.resolve().unwrap_err());
        assert!(
            err.contains("password_file") && err.contains("nope"),
            "{err}"
        );

        let empty = dir.path().join("empty");
        std::fs::write(&empty, "\n").unwrap();
        let mut c = parse(&format!(
            "broker = \"b:1883\"\nusername = \"u\"\npassword_file = {:?}",
            empty.display().to_string()
        ));
        assert!(c.resolve().unwrap_err().to_string().contains("empty"));

        let big = dir.path().join("big");
        std::fs::write(&big, "x".repeat(5000)).unwrap();
        let mut c = parse(&format!(
            "broker = \"b:1883\"\nusername = \"u\"\npassword_file = {:?}",
            big.display().to_string()
        ));
        assert!(c.resolve().unwrap_err().to_string().contains("larger"));

        let mut c = parse(&format!(
            "broker = \"b:1883\"\nusername = \"u\"\npassword_file = {:?}",
            dir.path().display().to_string()
        ));
        assert!(
            c.resolve()
                .unwrap_err()
                .to_string()
                .contains("not a regular file")
        );
    }

    #[test]
    fn password_and_password_file_together_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pw");
        std::fs::write(&file, "from-file").unwrap();
        let mut c = parse(&format!(
            "broker = \"b:1883\"\nusername = \"u\"\npassword = \"inline\"\npassword_file = {:?}",
            file.display().to_string()
        ));
        let err = c.resolve().unwrap_err().to_string();
        assert!(err.contains("only one"), "{err}");
    }

    #[test]
    fn tls_options_need_a_tls_address() {
        let dir = tempfile::tempdir().unwrap();
        let pem = dir.path().join("ca.pem");
        std::fs::write(
            &pem,
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let ca = pem.display().to_string();
        for line in [
            format!("ca_file = {ca:?}"),
            format!("client_cert = {ca:?}"),
            format!("client_key = {ca:?}"),
        ] {
            let mut c = parse(&format!("broker = \"b:1883\"\n{line}"));
            let err = c.resolve().unwrap_err().to_string();
            assert!(
                err.contains("plaintext") && err.contains("mqtts://"),
                "{line}: {err}"
            );
        }
        let mut c = parse(&format!("broker = \"mqtts://b\"\nca_file = {ca:?}"));
        c.resolve().unwrap();
    }

    #[test]
    fn client_certificate_and_key_come_as_a_pair() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("c.pem");
        std::fs::write(
            &cert,
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let mut c = parse(&format!(
            "broker = \"mqtts://b\"\nclient_cert = {:?}",
            cert.display().to_string()
        ));
        assert!(c.resolve().unwrap_err().to_string().contains("together"));
        let mut c = parse(&format!(
            "broker = \"mqtts://b\"\nclient_key = {:?}",
            cert.display().to_string()
        ));
        assert!(c.resolve().unwrap_err().to_string().contains("together"));
    }

    #[test]
    fn certificate_and_key_files_are_checked_for_what_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, content: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, content).unwrap();
            p.display().to_string()
        };
        let cert = write(
            "cert.pem",
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
        );
        // Samples are built, not spelled out (see `pem_begin`).
        let sample = |label: &str| {
            format!(
                "{}\nAAAA\n{}\n",
                pem_begin(label),
                pem_begin(label).replace("BEGIN", "END")
            )
        };
        let key = write("key.pem", &sample("PRIVATE KEY"));
        let rsa = write("rsa.pem", &sample("RSA PRIVATE KEY"));
        let ec = write("ec.pem", &sample("EC PRIVATE KEY"));
        let enc = write("enc.pem", &sample("ENCRYPTED PRIVATE KEY"));
        let junk = write("junk.pem", "this is not PEM");

        let try_section = |lines: &str| {
            let mut c = parse(&format!("broker = \"mqtts://b\"\n{lines}"));
            c.resolve().map_err(|e| e.to_string())
        };
        try_section(&format!("ca_file = {cert:?}")).unwrap();
        try_section(&format!("client_cert = {cert:?}\nclient_key = {key:?}")).unwrap();

        let e = try_section(&format!("ca_file = {junk:?}")).unwrap_err();
        assert!(
            e.contains("ca_file") && e.contains("no PEM certificate"),
            "{e}"
        );
        let e = try_section(&format!("ca_file = {key:?}")).unwrap_err();
        assert!(e.contains("no PEM certificate"), "a key is not a CA: {e}");
        let e = try_section(&format!("client_cert = {junk:?}\nclient_key = {key:?}")).unwrap_err();
        assert!(e.contains("client_cert"), "{e}");
        for (bad_key, needle) in [
            (&rsa, "PKCS#1/SEC1"),
            (&ec, "PKCS#1/SEC1"),
            (&enc, "encrypted"),
            (&junk, "no PEM private key"),
            (&cert, "no PEM private key"),
        ] {
            let e = try_section(&format!("client_cert = {cert:?}\nclient_key = {bad_key:?}"))
                .unwrap_err();
            assert!(
                e.contains("client_key") && e.contains(needle),
                "{needle}: {e}"
            );
        }
        let e = try_section(&format!("client_cert = {cert:?}\nclient_key = {rsa:?}")).unwrap_err();
        assert!(e.contains("openssl pkey"), "tells how to convert: {e}");

        let e = try_section("ca_file = \"/nonexistent/ca.pem\"").unwrap_err();
        assert!(
            e.contains("ca_file") && e.contains("/nonexistent/ca.pem"),
            "{e}"
        );
    }

    #[test]
    fn tls_to_an_ipv6_literal_is_refused() {
        let mut c = parse("broker = \"mqtts://[::1]:8883\"");
        assert!(c.resolve().unwrap_err().to_string().contains("host name"));
    }

    #[test]
    fn identity_and_topic_rules() {
        for (line, needle) in [
            ("client_id = \"\"", "client_id"),
            ("client_id = \"a\\u0000b\"", "client_id"),
            ("username = \"a\\nb\"", "username"),
            ("base_topic = \"a/+/b\"", "wildcards"),
            ("base_topic = \"a/#\"", "wildcards"),
            ("base_topic = \"$SYS/snapdog\"", "'$'"),
        ] {
            let mut c = parse(&format!("broker = \"b:1883\"\n{line}"));
            let err = c.resolve().unwrap_err().to_string();
            assert!(err.contains(needle), "{line}: {err}");
        }
        let mut c = parse(&format!(
            "broker = \"b:1883\"\nbase_topic = {:?}",
            "x".repeat(300)
        ));
        assert!(c.resolve().unwrap_err().to_string().contains("longer"));
        // What worked before keeps working.
        for base in ["", "snapdog", "home/audio/", "a b", "über"] {
            let mut c = parse(&format!("broker = \"b:1883\"\nbase_topic = {base:?}"));
            c.resolve().unwrap_or_else(|e| panic!("{base:?}: {e}"));
        }
    }

    #[test]
    fn plaintext_credentials_to_another_machine_are_flagged() {
        let warn = |toml: &str| parse(toml).plaintext_credentials_warning();
        // Flagged.
        let w = warn("broker = \"192.168.1.10:1883\"\nusername = \"u\"\npassword = \"p\"").unwrap();
        assert!(
            w.contains("192.168.1.10:1883") && w.contains("mqtts://"),
            "{w}"
        );
        assert!(!w.contains("\"p\""));
        assert!(warn("broker = \"broker.lan:1883\"\nusername = \"u\"").is_some());
        // Not flagged: TLS, loopback, or nothing to protect.
        assert!(
            warn("broker = \"mqtts://broker.lan\"\nusername = \"u\"\npassword = \"p\"").is_none()
        );
        assert!(warn("broker = \"localhost:1883\"\nusername = \"u\"\npassword = \"p\"").is_none());
        assert!(warn("broker = \"127.0.0.1:1883\"\nusername = \"u\"\npassword = \"p\"").is_none());
        assert!(warn("broker = \"[::1]:1883\"\nusername = \"u\"\npassword = \"p\"").is_none());
        assert!(warn("broker = \"192.168.1.10:1883\"").is_none());
    }
}
