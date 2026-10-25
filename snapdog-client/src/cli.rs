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
  With 'url' = tcp://<snapserver host or IP>[:port], optionally with user:password@ before the host\n\
  For example: 'tcp://192.168.1.1:1704'\n\
  If 'url' is not given, snapdog-client looks for a SnapDog server on the local network (mDNS, 5 s)\n\
  ws:// and wss:// are not supported by the Snapcast client library and are rejected"
)]
pub struct Cli {
    /// Snapserver URL: `tcp://<host>[:<port>]` (default: find a server via mDNS)
    pub url: Option<String>,

    /// Instance id when running multiple instances on the same host
    #[arg(short, long, default_value_t = 1)]
    pub instance: u32,

    /// Unique host id (default: MAC address)
    #[arg(long = "hostID")]
    pub host_id: Option<String>,

    /// Client certificate file (PEM format). Has no effect: TLS is not available (see --help)
    #[arg(long = "cert", hide = true)]
    pub certificate: Option<PathBuf>,

    /// Client private key file (PEM format). Has no effect: TLS is not available
    #[arg(long = "cert-key", hide = true)]
    pub certificate_key: Option<PathBuf>,

    /// Key password (for encrypted private key). Has no effect: TLS is not available
    #[arg(long = "key-password", hide = true)]
    pub key_password: Option<String>,

    /// Verify server with CA certificate (PEM format). Has no effect: TLS is not available
    #[arg(long = "server-cert", hide = true)]
    #[allow(clippy::option_option)]
    pub server_certificate: Option<Option<PathBuf>>,

    /// List the audio output devices (index and name) and exit
    #[arg(short, long)]
    pub list: bool,

    /// Play a test tone and exit (verifies audio output works)
    #[arg(long)]
    pub test_tone: bool,

    /// Audio output device: `default`, an index from --list, or part of a device name
    #[arg(short, long, default_value = "default")]
    pub soundcard: String,

    /// Additional latency of the audio device (ms)
    #[arg(long, default_value_t = 0)]
    pub latency: i32,

    /// Accepted for compatibility, has no effect: the output follows the stream's format
    #[arg(long, hide = true)]
    pub sampleformat: Option<String>,

    /// `null` discards the audio (no output). Any other value is ignored: playback
    /// always uses the system audio output, choose the device with --soundcard
    #[arg(long, default_value = "")]
    pub player: String,

    /// Mixer mode: `software|hardware|midi|x32|none[:<params>]`
    ///
    /// Examples:
    ///   --mixer software (default, PCM amplitude scaling)
    ///   --mixer hardware:Master (ALSA control, Linux only)
    ///   --mixer midi:interface:ch[:cc] (MIDI CC, default CC7)
    ///   --mixer x32:host:ch[,ch][:port] (Behringer X32 channel fader over OSC, same scale as midi)
    ///   --mixer none
    // verbatim: clap would otherwise join the examples into one line.
    #[arg(long, default_value = "software", verbatim_doc_comment)]
    pub mixer: String,

    /// Which way volume flows between the server and a mixer that can report
    /// changes made at the mixer itself (so far only x32)
    #[arg(long, value_enum, default_value_t = MixerSync::Both)]
    pub mixer_sync: MixerSync,

    /// Daemonize, optional process priority [-20..19]
    #[cfg(unix)]
    #[arg(short, long)]
    #[allow(clippy::option_option)]
    pub daemon: Option<Option<i32>>,

    /// Not implemented, so it is refused. Use the service manager's user setting
    /// (the shipped systemd unit uses `DynamicUser=yes`)
    #[cfg(unix)]
    #[arg(long, hide = true)]
    pub user: Option<String>,

    /// Log sink: null|stdout|stderr|file:`<path>`|system (`system` writes to stderr, not syslog)
    #[arg(long, default_value = "stdout")]
    pub logsink: String,

    /// Log filter: `<tag>:<level>[,<tag>:<level>]*`
    #[arg(long, default_value = "*:info")]
    pub logfilter: String,

    /// Pre-shared key for f32lz4e decryption (default: built-in key)
    #[cfg(feature = "encryption")]
    #[arg(long)]
    pub encryption_psk: Option<String>,
}

/// Value of `--mixer-sync`.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerSync {
    /// The server sets the mixer, and changes made at the mixer are reported back to it
    Both,
    /// The server sets the mixer; changes made at the mixer stay until the next server volume
    Out,
}

/// Something about the command line that cannot work as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// The flag cannot be honoured and running anyway would mislead the user. Startup stops.
    Error(String),
    /// The flag is accepted for compatibility but ignored. Logged once at startup.
    Warning(String),
}

impl Cli {
    /// Check the command line for flags this client cannot honour. Nothing a user
    /// types is silently dropped: it either works, is rejected here, or is
    /// reported here as ignored.
    #[must_use]
    pub fn issues(&self) -> Vec<Issue> {
        let mut issues = Vec::new();

        if let Some((scheme, _)) = self.url.as_deref().and_then(|u| u.split_once("://"))
            && matches!(scheme, "ws" | "wss")
        {
            issues.push(Issue::Error(format!(
                "{scheme}:// is not supported: the Snapcast client library streams audio over \
                 tcp:// only. Use tcp://<host>[:1704]."
            )));
        }

        #[cfg(unix)]
        if let Some(user) = &self.user {
            issues.push(Issue::Error(format!(
                "--user {user} is not implemented, so the client would keep running with the \
                 privileges of the invoking user. Run it as that user instead (the systemd unit \
                 uses DynamicUser=yes)."
            )));
        }

        let player = self.player.split(':').next().unwrap_or_default();
        if !matches!(player, "" | "null") {
            issues.push(Issue::Warning(format!(
                "--player {} is ignored: audio always plays on the system's audio output. Use \
                 --soundcard to choose the device, or --player null to discard the audio.",
                self.player
            )));
        }
        if player == "null" && self.soundcard != "default" {
            issues.push(Issue::Warning(format!(
                "--soundcard {} is ignored with --player null: no audio device is opened.",
                self.soundcard
            )));
        }

        if let Some(format) = &self.sampleformat {
            issues.push(Issue::Warning(format!(
                "--sampleformat {format} is ignored: the output follows the stream's format."
            )));
        }

        if self.mixer.split(':').next() == Some("script") {
            issues.push(Issue::Warning(
                "--mixer script is not implemented: using the software mixer.".into(),
            ));
        }

        let tls: Vec<&str> = [
            (self.certificate.is_some(), "--cert"),
            (self.certificate_key.is_some(), "--cert-key"),
            (self.key_password.is_some(), "--key-password"),
            (self.server_certificate.is_some(), "--server-cert"),
        ]
        .into_iter()
        .filter_map(|(set, flag)| set.then_some(flag))
        .collect();
        if !tls.is_empty() {
            issues.push(Issue::Warning(format!(
                "{} {} ignored: this client connects over plain tcp://, TLS is not available.",
                tls.join(", "),
                if tls.len() == 1 { "is" } else { "are" }
            )));
        }

        issues
    }

    /// Parse CLI args and build a [`ClientSettings`].
    pub fn into_settings(self) -> Result<ClientSettings> {
        let url = self.url.as_deref().unwrap_or("tcp://");
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
            // midi and x32 are handled by snapdog-client's own Mixer.
            "none" | "midi" | "x32" => MixerMode::None,
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
        // No URL specified → empty host triggers mDNS discovery at runtime
        assert_eq!(settings.server.host, "");
    }

    #[test]
    fn cli_into_settings_mixer() {
        let cli = Cli::parse_from(["snapdog-client", "--mixer", "hardware:hw:0"]);
        let s = cli.into_settings().unwrap();
        assert_eq!(s.player.mixer.mode, MixerMode::Hardware);
        assert_eq!(s.player.mixer.parameter, "hw:0");
    }

    #[test]
    fn cli_into_settings_accepts_x32_mixer() {
        // The client drives the desk itself; snapcast must not reject the mode.
        let cli = Cli::parse_from(["snapdog-client", "--mixer", "x32:192.168.1.20:31,32"]);
        let s = cli.into_settings().unwrap();
        assert_eq!(s.player.mixer.mode, MixerMode::None);
    }

    #[test]
    fn mixer_help_keeps_one_example_per_line() {
        use clap::CommandFactory;
        let help = Cli::command().render_long_help().to_string();
        for example in ["--mixer software", "--mixer none"] {
            assert!(
                help.lines().any(|l| l.trim_start().starts_with(example)),
                "{example:?} not on its own line:\n{help}"
            );
        }
    }

    #[test]
    fn mixer_sync_defaults_to_both_and_can_be_turned_off() {
        let cli = Cli::parse_from(["snapdog-client"]);
        assert_eq!(cli.mixer_sync, MixerSync::Both);
        let cli = Cli::parse_from(["snapdog-client", "--mixer-sync", "out"]);
        assert_eq!(cli.mixer_sync, MixerSync::Out);
        assert!(Cli::try_parse_from(["snapdog-client", "--mixer-sync", "in"]).is_err());
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
            "tcp://server:1704",
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

    fn issues(args: &[&str]) -> Vec<Issue> {
        let command_line: Vec<&str> = std::iter::once("snapdog-client")
            .chain(args.iter().copied())
            .collect();
        Cli::parse_from(command_line).issues()
    }

    fn assert_clean(args: &[&str]) {
        assert_eq!(issues(args), Vec::<Issue>::new(), "{args:?}");
    }

    fn only(issue: &[Issue]) -> &str {
        assert_eq!(issue.len(), 1, "{issue:?}");
        match &issue[0] {
            Issue::Error(m) | Issue::Warning(m) => m,
        }
    }

    #[test]
    fn plain_invocations_have_no_issues() {
        assert_clean(&[]);
        assert_clean(&["tcp://192.168.1.1:1704"]);
        assert_clean(&["--player", "null"]);
        assert_clean(&["--soundcard", "hw:1", "--mixer", "hardware:Master"]);
        assert_clean(&["--mixer", "x32:192.168.1.20:1", "--latency", "30"]);
    }

    #[test]
    fn websocket_urls_are_rejected_before_connecting() {
        for url in ["ws://homeserver.local", "wss://secure.host:1788"] {
            let found = issues(&[url]);
            assert!(
                matches!(found.as_slice(), [Issue::Error(m)] if m.contains("tcp://")),
                "{found:?}"
            );
        }
        // The URL parser still understands them, so the message can name the scheme.
        assert_eq!(parse_url("wss://h").unwrap().scheme, "wss");
    }

    #[test]
    #[cfg(unix)]
    fn user_is_refused_because_it_is_not_implemented() {
        let found = issues(&["--daemon", "--user", "nobody"]);
        assert!(matches!(found.as_slice(), [Issue::Error(m)] if m.contains("--user nobody")));
        assert!(matches!(
            issues(&["--user", "nobody"]).as_slice(),
            [Issue::Error(_)]
        ));
    }

    #[test]
    fn unknown_player_is_reported_as_ignored() {
        let found = issues(&["--player", "alsa:buffer_time=100"]);
        assert!(only(&found).contains("--player alsa:buffer_time=100 is ignored"));
        assert!(matches!(found[0], Issue::Warning(_)));
    }

    #[test]
    fn soundcard_with_null_player_is_reported() {
        let found = issues(&["--player", "null", "--soundcard", "hw:1"]);
        assert!(only(&found).contains("--soundcard hw:1 is ignored"));
    }

    #[test]
    fn sampleformat_is_validated_and_reported() {
        let found = issues(&["--sampleformat", "48000:16:*"]);
        assert!(only(&found).contains("--sampleformat 48000:16:* is ignored"));
        let cli = Cli::parse_from(["snapdog-client", "--sampleformat", "garbage"]);
        assert!(cli.into_settings().is_err());
    }

    #[test]
    fn mixer_script_is_reported() {
        let found = issues(&["--mixer", "script:/usr/bin/vol.sh"]);
        assert!(only(&found).contains("--mixer script is not implemented"));
    }

    #[test]
    fn tls_flags_are_reported_once_together() {
        let found = issues(&["--cert", "c.pem", "--cert-key", "k.pem", "--server-cert"]);
        let message = only(&found);
        assert!(
            message.contains("--cert, --cert-key, --server-cert are ignored"),
            "{message}"
        );
        let found = issues(&["--key-password", "x"]);
        assert!(only(&found).contains("--key-password is ignored"));
    }

    #[test]
    fn stub_flags_are_hidden_from_help() {
        use clap::CommandFactory;
        let help = Cli::command().render_long_help().to_string();
        for hidden in [
            "--cert",
            "--key-password",
            "--server-cert",
            "--sampleformat",
            "--user ",
        ] {
            assert!(
                !help.contains(hidden),
                "{hidden} should not be advertised:\n{help}"
            );
        }
        for shown in [
            "--player",
            "--soundcard",
            "--list",
            "--mixer-sync",
            "--mixer",
        ] {
            assert!(help.contains(shown), "{shown} missing from help");
        }
        assert!(
            !help.contains("<tcp|ws|wss>"),
            "ws/wss must not be advertised"
        );
    }
}
