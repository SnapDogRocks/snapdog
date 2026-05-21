//! CLI argument parsing — maps command-line args to [`ClientSettings`].

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;

use snapcast_client::config::{self, Auth, ClientSettings, MixerMode, ServerSettings};

/// Snapcast client — synchronized multiroom audio player.
#[derive(Parser, Debug)]
#[command(
    version,
    about,
    after_help = "\
  With 'url' = <tcp|ws|wss>://<snapserver host or IP or mDNS service name>[:port]\n\
  For example: 'tcp://192.168.1.1:1704', or 'ws://homeserver.local'\n\
  If 'url' is not configured, snapdog-client defaults to 'tcp://_snapdog._tcp'"
)]
pub struct Cli {
    /// Snapserver URL: `<tcp|ws|wss>://<host>[:<port>]`
    pub url: Option<String>,

    /// Instance id when running multiple instances on the same host
    #[arg(short, long, default_value_t = 1)]
    pub instance: u32,

    /// Unique host id (default: MAC address)
    #[arg(long = "hostID")]
    pub host_id: Option<String>,

    /// Client certificate file (PEM format)
    #[arg(long = "cert")]
    pub certificate: Option<PathBuf>,

    /// Client private key file (PEM format)
    #[arg(long = "cert-key")]
    pub certificate_key: Option<PathBuf>,

    /// Key password (for encrypted private key)
    #[arg(long = "key-password")]
    pub key_password: Option<String>,

    /// Verify server with CA certificate (PEM format). Use without value for default certificates.
    #[arg(long = "server-cert")]
    #[allow(clippy::option_option)]
    pub server_certificate: Option<Option<PathBuf>>,

    /// List PCM devices
    #[arg(short, long)]
    pub list: bool,

    /// Play a test tone and exit (verifies audio output works)
    #[arg(long)]
    pub test_tone: bool,

    /// PCM device index or name
    #[arg(short, long, default_value = "default")]
    pub soundcard: String,

    /// Additional latency of the audio device (ms)
    #[arg(long, default_value_t = 0)]
    pub latency: i32,

    /// Resample to `<rate>:<bits>:<channels>`
    #[arg(long)]
    pub sampleformat: Option<String>,

    /// Audio player backend and optional parameters: `<name>[:<params>|?]`
    #[arg(long, default_value = "")]
    pub player: String,

    /// Mixer mode: `software|hardware|midi|none|?[:<params>]`
    ///
    /// Examples:
    ///   --mixer software (default, PCM amplitude scaling)
    ///   --mixer hardware:Master (ALSA control, Linux only)
    ///   --mixer midi:interface:ch[:cc] (MIDI CC, default CC7)
    ///   --mixer none
    #[arg(long, default_value = "software")]
    pub mixer: String,

    /// Daemonize, optional process priority [-20..19]
    #[cfg(unix)]
    #[arg(short, long)]
    #[allow(clippy::option_option)]
    pub daemon: Option<Option<i32>>,

    /// The `user[:group]` to run snapclient as when daemonized
    #[cfg(unix)]
    #[arg(long)]
    pub user: Option<String>,

    /// Log sink: null|system|stdout|stderr|file:`<path>`
    #[arg(long, default_value = "stdout")]
    pub logsink: String,

    /// Log filter: `<tag>:<level>[,<tag>:<level>]*`
    #[arg(long, default_value = "*:info")]
    pub logfilter: String,

    /// mDNS service name for server discovery
    #[arg(long, default_value = "_snapdog._tcp")]
    pub mdns_name: String,

    /// Pre-shared key for f32lz4e decryption (default: built-in key)
    #[cfg(feature = "encryption")]
    #[arg(long)]
    pub encryption_psk: Option<String>,
}

impl Cli {
    /// Parse CLI args and build a [`ClientSettings`].
    pub fn into_settings(self) -> Result<ClientSettings> {
        let default_url = format!("tcp://{}", self.mdns_name);
        let url = self.url.as_deref().unwrap_or(&default_url);
        let mut server = parse_url(url)?;

        // TLS certificate options
        if let Some(cert) = self.certificate {
            server.certificate = Some(cert);
        }
        if let Some(key) = self.certificate_key {
            server.certificate_key = Some(key);
        }
        if let Some(pw) = self.key_password {
            server.key_password = Some(pw);
        }
        if let Some(server_cert) = self.server_certificate {
            // --server-cert without value → use default certs (empty path)
            // --server-cert=path → use specific cert
            server.server_certificate = Some(server_cert.unwrap_or_default());
        }

        // Player
        let (player_name, player_param) = if self.player.is_empty() {
            (String::new(), String::new())
        } else if let Some((name, param)) = self.player.split_once(':') {
            (name.to_string(), param.to_string())
        } else {
            (self.player, String::new())
        };

        // Sample format
        let sample_format = match self.sampleformat {
            Some(ref sf) => sf
                .parse()
                .with_context(|| format!("invalid sample format: {sf}"))?,
            None => snapcast_client::SampleFormat::default(),
        };

        // Mixer
        let (mixer_mode_str, mixer_param) = self
            .mixer
            .split_once(':')
            .map(|(m, p)| (m, p.to_string()))
            .unwrap_or((&self.mixer, String::new()));
        let mixer_mode = match mixer_mode_str {
            "software" => MixerMode::Software,
            "hardware" => MixerMode::Hardware,
            "script" => MixerMode::Script,
            "none" | "midi" => MixerMode::None, // midi handled by snapdog-client Mixer
            other => bail!("unknown mixer mode: {other}"),
        };

        Ok(ClientSettings {
            instance: self.instance,
            host_id: self.host_id.unwrap_or_default(),
            server,
            player: config::PlayerSettings {
                player_name,
                parameter: player_param,
                latency: self.latency,
                pcm_device: config::PcmDevice {
                    name: self.soundcard,
                    ..Default::default()
                },
                sample_format,
                mixer: config::MixerSettings {
                    mode: mixer_mode,
                    parameter: mixer_param,
                },
            },
            logging: config::LoggingSettings {
                sink: self.logsink,
                filter: self.logfilter,
            },
            #[cfg(unix)]
            daemon: self.daemon.map(|priority| config::DaemonSettings {
                priority: priority.or(Some(-3)),
                user: self.user,
            }),
        })
    }
}

/// Parse a snapcast URL into [`ServerSettings`].
fn parse_url(url: &str) -> Result<ServerSettings> {
    let mut settings = ServerSettings::default();

    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("missing scheme separator '://'"))?;

    match scheme {
        "tcp" | "ws" | "wss" => settings.scheme = scheme.to_string(),
        _ => bail!("unsupported scheme: {scheme} (expected tcp, ws, or wss)"),
    }

    // Extract optional user:password@
    let rest = if let Some((userinfo, host_part)) = rest.split_once('@') {
        if let Some((user, password)) = userinfo.split_once(':') {
            settings.auth = Some(Auth {
                scheme: "Basic".into(),
                param: base64_encode_credentials(user, password),
            });
        }
        host_part
    } else {
        rest
    };

    // Extract host and optional port
    if let Some((host, port_str)) = rest.rsplit_once(':') {
        settings.host = host.to_string();
        settings.port = port_str
            .parse()
            .with_context(|| format!("invalid port: {port_str}"))?;
    } else {
        settings.host = rest.to_string();
        settings.port = default_port(&settings.scheme);
    }

    Ok(settings)
}

fn default_port(scheme: &str) -> u16 {
    match scheme {
        "ws" => snapcast_proto::DEFAULT_HTTP_PORT,
        "wss" => snapcast_proto::DEFAULT_WSS_PORT,
        _ => snapcast_proto::DEFAULT_STREAM_PORT,
    }
}

fn base64_encode_credentials(user: &str, password: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tcp_url() {
        let s = parse_url("tcp://192.168.1.1:1704").unwrap();
        assert_eq!(s.scheme, "tcp");
        assert_eq!(s.host, "192.168.1.1");
        assert_eq!(s.port, 1704);
        assert!(s.auth.is_none());
    }

    #[test]
    fn parse_ws_url_default_port() {
        let s = parse_url("ws://homeserver.local").unwrap();
        assert_eq!(s.scheme, "ws");
        assert_eq!(s.host, "homeserver.local");
        assert_eq!(s.port, 1780);
    }

    #[test]
    fn parse_wss_url() {
        let s = parse_url("wss://secure.host:1788").unwrap();
        assert_eq!(s.scheme, "wss");
        assert_eq!(s.port, 1788);
    }

    #[test]
    fn parse_url_with_credentials() {
        let s = parse_url("tcp://user:pass@myhost:1704").unwrap();
        assert_eq!(s.host, "myhost");
        let auth = s.auth.unwrap();
        assert_eq!(auth.scheme, "Basic");
        assert_eq!(auth.param, "dXNlcjpwYXNz");
    }

    #[test]
    fn parse_invalid_scheme() {
        assert!(parse_url("http://localhost").is_err());
    }

    #[test]
    fn parse_invalid_url() {
        assert!(parse_url("garbage").is_err());
    }

    #[test]
    fn parse_mdns_service_name() {
        let s = parse_url("tcp://_snapdog._tcp").unwrap();
        assert_eq!(s.scheme, "tcp");
        assert_eq!(s.host, "_snapdog._tcp");
        assert_eq!(s.port, 1704);
    }

    #[test]
    fn default_url_with_mdns() {
        let cli = Cli::parse_from(["snapdog-client"]);
        assert!(cli.url.is_none());
        let settings = cli.into_settings().unwrap();
        // With mdns feature, default host is mDNS service name
        assert!(settings.server.host == "_snapdog._tcp" || settings.server.host == "localhost");
    }

    #[test]
    fn cli_into_settings_mixer() {
        let cli = Cli::parse_from(["snapdog-client", "--mixer", "hardware:hw:0"]);
        let s = cli.into_settings().unwrap();
        assert_eq!(s.player.mixer.mode, MixerMode::Hardware);
        assert_eq!(s.player.mixer.parameter, "hw:0");
    }

    #[test]
    fn cli_into_settings_player_with_params() {
        let cli = Cli::parse_from([
            "snapdog-client",
            "--instance",
            "2",
            "--hostID",
            "my-id",
            "--player",
            "alsa:buffer_time=100",
            "--latency",
            "50",
            "--sampleformat",
            "48000:16:*",
            "--soundcard",
            "hw:1",
            "--logsink",
            "stderr",
            "--logfilter",
            "*:debug",
            "tcp://localhost:1704",
        ]);
        let s = cli.into_settings().unwrap();
        assert_eq!(s.instance, 2);
        assert_eq!(s.host_id, "my-id");
        assert_eq!(s.player.player_name, "alsa");
        assert_eq!(s.player.parameter, "buffer_time=100");
        assert_eq!(s.player.latency, 50);
        assert_eq!(s.player.sample_format.rate(), 48000);
        assert_eq!(s.player.sample_format.channels(), 0);
        assert_eq!(s.player.pcm_device.name, "hw:1");
    }

    #[test]
    fn cli_cert_options() {
        let cli = Cli::parse_from([
            "snapdog-client",
            "--cert",
            "/path/to/cert.pem",
            "--cert-key",
            "/path/to/key.pem",
            "--key-password",
            "secret",
            "wss://server:1788",
        ]);
        let s = cli.into_settings().unwrap();
        assert_eq!(
            s.server.certificate.unwrap().to_str().unwrap(),
            "/path/to/cert.pem"
        );
        assert_eq!(
            s.server.certificate_key.unwrap().to_str().unwrap(),
            "/path/to/key.pem"
        );
        assert_eq!(s.server.key_password.unwrap(), "secret");
    }

    #[test]
    fn cli_list_flag() {
        let cli = Cli::parse_from(["snapdog-client", "--list"]);
        assert!(cli.list);
    }
}
