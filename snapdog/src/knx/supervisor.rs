// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Supervision of the KNX transports: capped exponential backoff with jitter, quiet logging
//! and a cooperative shutdown signal, shared by the client and the device transport, plus the
//! client connection loop.

use std::hash::BuildHasher;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use knx_rs_ip::{ConnectionSpec, Multiplexer};
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use super::client::{ClientListener, ClientPublisher};
use super::device::{DeviceListener, DevicePublisher};
use super::transport::{KnxListener, KnxPublisher};
use super::{Bridge, SessionEnd, run_session};

/// First retry delay.
pub const BACKOFF_BASE: Duration = Duration::from_secs(1);
/// Longest delay between two attempts.
pub const BACKOFF_CAP: Duration = Duration::from_secs(60);
/// A connection that lasted at least this long counts as healthy again: the next loss
/// starts over at [`BACKOFF_BASE`] instead of continuing the previous backoff. A connection
/// that drops right after it was made (a gateway that accepts and then resets) keeps backing
/// off instead of hammering the gateway once a second.
pub const STABLE_AFTER: Duration = Duration::from_secs(30);

/// Capped exponential backoff with jitter.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    cap: Duration,
    failures: u32,
}

impl Backoff {
    /// Backoff growing from `base` to at most `cap`.
    #[must_use]
    pub const fn new(base: Duration, cap: Duration) -> Self {
        Self {
            base,
            cap,
            failures: 0,
        }
    }

    /// Consecutive failures since the last [`reset`](Self::reset).
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// Record a failure and return how long to wait before the next attempt.
    pub fn failure(&mut self) -> Duration {
        let delay = self.delay_for(self.failures, fastrand::f64());
        self.failures = self.failures.saturating_add(1);
        delay
    }

    /// Forget all failures.
    pub const fn reset(&mut self) {
        self.failures = 0;
    }

    /// Delay after `failures` earlier failures. `unit` in `0.0..1.0` places the delay in the
    /// upper half of the nominal value ("equal jitter"): attempts of several instances spread
    /// out, and the delay never exceeds the cap.
    #[must_use]
    pub fn delay_for(&self, failures: u32, unit: f64) -> Duration {
        let nominal = self
            .base
            .saturating_mul(1u32.checked_shl(failures.min(20)).unwrap_or(u32::MAX))
            .min(self.cap);
        nominal.mul_f64(0.5f64.mul_add(unit.clamp(0.0, 1.0), 0.5))
    }
}

/// The delay before the next attempt after a connection that lasted `lasted` has ended. A
/// connection that stayed up for [`STABLE_AFTER`] was healthy, so the backoff starts over; one
/// that dropped soon after it was made keeps backing off.
pub fn delay_after_session(backoff: &mut Backoff, lasted: Duration) -> Duration {
    if lasted >= STABLE_AFTER {
        backoff.reset();
    }
    backoff.failure()
}

/// Whether the `failures`-th consecutive failure deserves a warning: the first few, then
/// every tenth. Everything in between is logged at debug level, so a gateway that stays away
/// for a day produces a line every ten minutes at the capped delay, not one per attempt.
#[must_use]
pub const fn warn_on_failure(failures: u32) -> bool {
    failures <= 3 || failures.is_multiple_of(10)
}

/// Completes when shutdown was requested. Never completes when the sender is gone: dropping
/// the bridge handle leaves the bridge running.
pub async fn stop_requested(shutdown: &mut watch::Receiver<bool>) {
    if shutdown.wait_for(|stop| *stop).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Sleep for `delay`. Returns `false` when shutdown was requested first.
pub async fn sleep_or_shutdown(delay: Duration, shutdown: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(delay) => !*shutdown.borrow(),
        () = stop_requested(shutdown) => false,
    }
}

// ── Client connection ─────────────────────────────────────────

/// The gateway or router a client connects to, as written in `[knx].url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTarget {
    scheme: String,
    /// `host:port` as written; an IP literal needs no lookup.
    authority: String,
    literal: Option<SocketAddr>,
}

impl ClientTarget {
    /// Check the URL: `udp://`, `tunnel://` or `router://`, followed by `host:port` (an IP
    /// literal, or a host name that is resolved on every connection attempt).
    ///
    /// # Errors
    ///
    /// Returns an error naming what is wrong with the URL.
    pub fn parse(url: &str) -> Result<Self> {
        let (scheme, authority) = url
            .split_once("://")
            .context("Invalid KNX URL: missing '://' (expected e.g. udp://192.168.1.50:3671)")?;
        anyhow::ensure!(
            matches!(scheme, "udp" | "tunnel" | "router"),
            "Invalid KNX URL: unsupported scheme '{scheme}' (use udp://, tunnel:// or router://)"
        );
        let literal = authority.parse::<SocketAddr>().ok();
        if literal.is_none() {
            let (host, port) = authority
                .rsplit_once(':')
                .context("Invalid KNX URL: missing port (expected host:port)")?;
            anyhow::ensure!(!host.is_empty(), "Invalid KNX URL: missing host");
            anyhow::ensure!(
                port.parse::<u16>().is_ok(),
                "Invalid KNX URL: invalid port '{port}'"
            );
        }
        Ok(Self {
            scheme: scheme.to_string(),
            authority: authority.to_string(),
            literal,
        })
    }

    /// The connection to make now. A host name is looked up again every time, so a gateway
    /// that came back under a new address is found.
    async fn resolve(&self) -> Result<ConnectionSpec> {
        let addr = if let Some(addr) = self.literal {
            addr
        } else {
            let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host(&self.authority)
                .await
                .with_context(|| format!("resolving {}", self.authority))?
                .collect();
            // KNXnet/IP gateways listen on IPv4 in practice; prefer it.
            addrs.sort_by_key(SocketAddr::is_ipv6);
            addrs
                .into_iter()
                .next()
                .with_context(|| format!("{} has no address", self.authority))?
        };
        knx_rs_ip::parse_url(&format!("{}://{addr}", self.scheme)).context("Invalid KNX URL")
    }
}

/// Log a failed attempt: a warning for the first few and every tenth, debug otherwise.
fn log_failure(what: &str, failures: u32, delay: Duration, error: &str) {
    if warn_on_failure(failures) {
        tracing::warn!(attempt = failures, retry_in = ?delay, %error, "{what}");
    } else {
        tracing::debug!(attempt = failures, retry_in = ?delay, %error, "{what}");
    }
}

/// Drop the notifications that piled up while there was no transport: the state is published
/// in full when a session starts.
fn drain_stale(notify_rx: &mut broadcast::Receiver<Arc<str>>) {
    use broadcast::error::TryRecvError;
    while let Ok(_) | Err(TryRecvError::Lagged(_)) = notify_rx.try_recv() {}
}

/// What a connected client session consists of: the two halves the bridge runs on, the signal
/// that the connection looks dead, and the task that drives the connection, which ends with
/// the session.
pub struct ClientLink<P, L> {
    pub publisher: P,
    pub listener: L,
    pub broken: Option<Arc<tokio::sync::Notify>>,
    pub driver: Option<tokio::task::JoinHandle<()>>,
}

/// The way a client connection is made: a real one, or one that a test makes (and loses) on a
/// schedule.
pub trait Dial: Send {
    type Publisher: KnxPublisher;
    type Listener: KnxListener;

    fn dial(
        &mut self,
    ) -> impl std::future::Future<Output = Result<ClientLink<Self::Publisher, Self::Listener>>> + Send;
}

/// Connects to the gateway or router a [`ClientTarget`] names.
struct Gateway<'a>(&'a ClientTarget);

impl Dial for Gateway<'_> {
    type Publisher = ClientPublisher;
    type Listener = ClientListener;

    async fn dial(&mut self) -> Result<ClientLink<ClientPublisher, ClientListener>> {
        let spec = self.0.resolve().await?;
        let conn = knx_rs_ip::connect(spec)
            .await
            .map_err(anyhow::Error::from)?;
        let mux = Multiplexer::new(conn);
        let (publisher, broken) = ClientPublisher::new(mux.handle());
        let listener = ClientListener::new(mux.handle());
        Ok(ClientLink {
            publisher,
            listener,
            broken: Some(broken),
            driver: Some(tokio::spawn(mux.run())),
        })
    }
}

/// Connect to the gateway, run the bridge, and do it again whenever the connection is lost,
/// until shutdown. Failures to connect, at startup or later, back off from one second to a
/// minute with jitter; every new connection publishes all status values again.
pub async fn run_client<S>(
    target: ClientTarget,
    bridge: Bridge<S>,
    notify_rx: broadcast::Receiver<Arc<str>>,
    shutdown: watch::Receiver<bool>,
) where
    S: BuildHasher + Send + Sync + 'static,
{
    let url = format!("{}://{}", target.scheme, target.authority);
    run_client_with(&url, Gateway(&target), bridge, notify_rx, shutdown).await;
}

/// [`run_client`] with the way a connection is made left open, so that the supervision can be
/// run against connections that are made (and lost) on a schedule.
pub async fn run_client_with<S, D: Dial>(
    url: &str,
    mut dialer: D,
    bridge: Bridge<S>,
    mut notify_rx: broadcast::Receiver<Arc<str>>,
    mut shutdown: watch::Receiver<bool>,
) where
    S: BuildHasher + Send + Sync + 'static,
{
    let mut backoff = Backoff::new(BACKOFF_BASE, BACKOFF_CAP);

    loop {
        if *shutdown.borrow() {
            break;
        }

        let connected = tokio::select! {
            r = dialer.dial() => r,
            () = stop_requested(&mut shutdown) => break,
        };
        let link = match connected {
            Ok(link) => link,
            Err(e) => {
                let delay = backoff.failure();
                log_failure(
                    "KNX gateway not reachable",
                    backoff.failures(),
                    delay,
                    &format!("{e:#}"),
                );
                if !sleep_or_shutdown(delay, &mut shutdown).await {
                    break;
                }
                continue;
            }
        };

        if backoff.failures() > 0 {
            tracing::info!(%url, attempts = backoff.failures(), "KNX gateway reachable again — connected");
        } else {
            tracing::info!(%url, "KNX client connected");
        }
        let connected_at = Instant::now();

        let ClientLink {
            publisher,
            listener,
            broken,
            driver,
        } = link;
        drain_stale(&mut notify_rx);

        let end = tokio::select! {
            end = run_session(&publisher, listener, broken.as_deref(), &bridge, &mut notify_rx) => Some(end),
            () = stop_requested(&mut shutdown) => None,
        };
        // Dropping the multiplexer drops the tunnel, which tells the gateway goodbye.
        if let Some(driver) = driver {
            driver.abort();
        }
        drop(publisher);

        match end {
            None | Some(SessionEnd::NotificationsClosed) => break,
            Some(end) => {
                let delay = delay_after_session(&mut backoff, connected_at.elapsed());
                tracing::warn!(
                    reason = ?end,
                    retry_in = ?delay,
                    "KNX connection lost — reconnecting"
                );
                if !sleep_or_shutdown(delay, &mut shutdown).await {
                    break;
                }
            }
        }
    }
    tracing::info!("KNX client stopped");
}

/// Run the bridge on the device transport. The device server restarts itself (see
/// `device.rs`), so one session lasts until shutdown.
pub async fn run_device<S>(
    publisher: DevicePublisher,
    listener: DeviceListener,
    bridge: Bridge<S>,
    mut notify_rx: broadcast::Receiver<Arc<str>>,
    mut shutdown: watch::Receiver<bool>,
) where
    S: BuildHasher + Send + Sync + 'static,
{
    tokio::select! {
        end = run_session(&publisher, listener, None, &bridge, &mut notify_rx) => {
            tracing::debug!(?end, "KNX device bridge ended");
        }
        () = stop_requested(&mut shutdown) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_grows_exponentially_and_stops_at_the_cap() {
        let b = Backoff::new(BACKOFF_BASE, BACKOFF_CAP);
        // unit = 1.0 gives the nominal value.
        let nominal: Vec<u64> = (0..9).map(|n| b.delay_for(n, 1.0).as_secs()).collect();
        assert_eq!(nominal, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn jitter_stays_between_half_and_all_of_the_nominal_delay() {
        let b = Backoff::new(BACKOFF_BASE, BACKOFF_CAP);
        for n in 0..12 {
            let nominal = b.delay_for(n, 1.0);
            assert_eq!(b.delay_for(n, 0.0), nominal / 2);
            for _ in 0..20 {
                let d = b.delay_for(n, fastrand::f64());
                assert!(d >= nominal / 2 && d <= nominal, "{d:?} vs {nominal:?}");
            }
        }
    }

    #[test]
    fn huge_failure_counts_do_not_overflow() {
        let b = Backoff::new(BACKOFF_BASE, BACKOFF_CAP);
        assert_eq!(b.delay_for(u32::MAX, 1.0), BACKOFF_CAP);
    }

    #[test]
    fn failure_counts_and_reset() {
        let mut b = Backoff::new(BACKOFF_BASE, BACKOFF_CAP);
        let first = b.failure();
        assert!(first <= BACKOFF_BASE && first >= BACKOFF_BASE / 2);
        b.failure();
        assert_eq!(b.failures(), 2);
        b.reset();
        assert_eq!(b.failures(), 0);
    }

    #[test]
    fn warnings_are_rate_limited() {
        let warned: Vec<u32> = (1..=40).filter(|n| warn_on_failure(*n)).collect();
        assert_eq!(warned, vec![1, 2, 3, 10, 20, 30, 40]);
    }

    #[test]
    fn a_stable_connection_resets_the_backoff_and_a_short_one_does_not() {
        let mut b = Backoff::new(BACKOFF_BASE, BACKOFF_CAP);
        for _ in 0..5 {
            b.failure();
        }
        // Dropped soon after it was made: keep backing off (6th failure, nominal 32 s).
        let short = delay_after_session(&mut b, Duration::from_secs(2));
        assert!(short >= Duration::from_secs(16), "{short:?}");
        assert_eq!(b.failures(), 6);
        // Stayed up for a while: start over (first failure, nominal 1 s).
        let long = delay_after_session(&mut b, STABLE_AFTER);
        assert!(long <= BACKOFF_BASE, "{long:?}");
        assert_eq!(b.failures(), 1);
    }

    #[test]
    fn client_urls() {
        for ok in [
            "udp://192.168.1.50:3671",
            "tunnel://192.168.1.50:3671",
            "router://224.0.23.12:3671",
            "tunnel://[::1]:3671",
            "udp://gateway.local:3671",
            "tunnel://localhost:13671",
        ] {
            assert!(ClientTarget::parse(ok).is_ok(), "{ok}");
        }
        for (bad, why) in [
            ("192.168.1.50:3671", "missing '://'"),
            ("tcp://192.168.1.50:3671", "unsupported scheme"),
            ("udp://192.168.1.50", "missing port"),
            ("udp://gateway.local", "missing port"),
            ("udp://gateway.local:abc", "invalid port"),
            ("udp://:3671", "missing host"),
        ] {
            let err = ClientTarget::parse(bad).unwrap_err().to_string();
            assert!(err.contains(why), "{bad}: {err}");
        }
    }

    #[tokio::test]
    async fn host_names_resolve_to_a_connection() {
        let t = ClientTarget::parse("tunnel://localhost:13671").unwrap();
        let spec = t.resolve().await.unwrap();
        assert!(
            matches!(spec, ConnectionSpec::Tunnel(a) if a.port() == 13671 && a.ip().is_loopback())
        );
        let t = ClientTarget::parse("udp://224.0.23.12:3671").unwrap();
        assert!(matches!(
            t.resolve().await.unwrap(),
            ConnectionSpec::Router(_)
        ));
    }

    #[tokio::test]
    async fn sleep_ends_early_on_shutdown() {
        let (tx, mut rx) = watch::channel(false);
        let sleeper =
            tokio::spawn(
                async move { sleep_or_shutdown(Duration::from_secs(3600), &mut rx).await },
            );
        tx.send(true).unwrap();
        assert!(!sleeper.await.unwrap());
    }

    #[tokio::test]
    async fn dropping_the_shutdown_sender_does_not_stop_anything() {
        let (tx, mut rx) = watch::channel(false);
        drop(tx);
        // A bridge whose handle was dropped keeps running: the stop signal never fires.
        let stopped =
            tokio::time::timeout(Duration::from_millis(50), stop_requested(&mut rx)).await;
        assert!(stopped.is_err());
        assert!(sleep_or_shutdown(Duration::from_millis(10), &mut rx).await);
    }
    // ── The client loop, run against connections that follow a script ──────────

    /// A publisher that takes everything.
    struct Takes;

    impl KnxPublisher for Takes {
        const ADDRESSED: bool = true;

        fn write(
            &self,
            _target: super::super::transport::Target,
            _dpt: knx_rs_core::dpt::Dpt,
            _value: &knx_rs_core::dpt::DptValue,
        ) -> impl std::future::Future<Output = bool> + Send {
            std::future::ready(true)
        }
    }

    /// A listener whose connection ends after a while.
    struct EndsAfter(Duration);

    impl KnxListener for EndsAfter {
        fn recv(
            &mut self,
        ) -> impl std::future::Future<Output = Option<super::super::transport::Incoming>> + Send
        {
            let lasts = self.0;
            async move {
                tokio::time::sleep(lasts).await;
                None
            }
        }
    }

    /// Connections on a script: `None` is a failed attempt, `Some(d)` a connection that is lost
    /// after `d`. When the script is used up the next attempt asks for shutdown.
    struct Scripted {
        script: std::vec::IntoIter<Option<Duration>>,
        dialed: Arc<std::sync::Mutex<Vec<Instant>>>,
        stop: watch::Sender<bool>,
    }

    impl Dial for Scripted {
        type Publisher = Takes;
        type Listener = EndsAfter;

        fn dial(
            &mut self,
        ) -> impl std::future::Future<Output = Result<ClientLink<Takes, EndsAfter>>> + Send
        {
            self.dialed.lock().unwrap().push(Instant::now());
            std::future::ready(match self.script.next() {
                Some(Some(lasts)) => Ok(ClientLink {
                    publisher: Takes,
                    listener: EndsAfter(lasts),
                    broken: None,
                    driver: None,
                }),
                Some(None) => Err(anyhow::anyhow!("refused")),
                None => {
                    let _ = self.stop.send(true);
                    Err(anyhow::anyhow!("done"))
                }
            })
        }
    }

    /// When each attempt was made, for a script of failures and connections.
    async fn dial_times(script: Vec<Option<Duration>>) -> Vec<Instant> {
        let cfg = crate::config::load_raw(
            toml::from_str(
                r#"
[knx]
role = "client"
url = "udp://192.168.1.50:3671"

[[zone]]
name = "Z1"
"#,
            )
            .unwrap(),
        )
        .unwrap();
        let store = crate::state::init(&cfg, None).unwrap();
        let (snap_tx, _snap_rx) = tokio::sync::mpsc::channel(4);
        let bridge = Bridge::new(
            &cfg,
            store,
            std::collections::HashMap::<usize, crate::player::ZoneCommandSender>::new(),
            snap_tx,
        );
        let (_notify_tx, notify_rx) = broadcast::channel(8);
        let (stop, shutdown) = watch::channel(false);
        let dialed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let scripted = Scripted {
            script: script.into_iter(),
            dialed: Arc::clone(&dialed),
            stop,
        };
        run_client_with("udp://test", scripted, bridge, notify_rx, shutdown).await;
        dialed.lock().unwrap().clone()
    }

    /// Time between the end of a connection that began at `begin` and lasted `lasted`, and `next`.
    fn pause_after(begin: Instant, lasted: Duration, next: Instant) -> Duration {
        next.duration_since(begin + lasted)
    }

    /// A connection that stayed up for 30 s or more was healthy: after its loss the client tries
    /// again after the first delay (0.5 to 1 s), not after the delay that three failures before
    /// it had grown to (4 to 8 s). (The mutation that replaces the call in the client loop with
    /// a plain `backoff.failure()` is killed by this.)
    #[tokio::test(start_paused = true)]
    async fn a_client_connection_that_lasted_starts_the_backoff_over() {
        let lasted = STABLE_AFTER + Duration::from_secs(1);
        let t = dial_times(vec![None, None, None, Some(lasted)]).await;
        assert_eq!(t.len(), 5);
        let pause = pause_after(t[3], lasted, t[4]);
        assert!(
            pause <= BACKOFF_BASE + Duration::from_millis(50),
            "waited {pause:?} after a healthy connection"
        );
    }

    /// A connection that is lost after a few seconds is no sign of health: the delays keep
    /// growing, as they do for failed attempts.
    #[tokio::test(start_paused = true)]
    async fn a_client_connection_that_drops_at_once_keeps_backing_off() {
        let lasted = Duration::from_secs(2);
        let t = dial_times(vec![Some(lasted), Some(lasted), Some(lasted)]).await;
        assert_eq!(t.len(), 4);
        // 1st loss: 0.5 to 1 s, 2nd: 1 to 2 s, 3rd: 2 to 4 s.
        let pauses: Vec<Duration> = (0..3)
            .map(|i| pause_after(t[i], lasted, t[i + 1]))
            .collect();
        assert!(
            pauses[0] <= Duration::from_secs(1) + Duration::from_millis(50),
            "{pauses:?}"
        );
        assert!(pauses[1] >= Duration::from_secs(1), "{pauses:?}");
        assert!(pauses[2] >= Duration::from_secs(2), "{pauses:?}");
    }
}
