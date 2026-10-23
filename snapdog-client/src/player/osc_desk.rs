//! Volume control on a networked mixing desk over OSC.
//!
//! The desk becomes the volume authority: the client plays at unity gain and `SnapDog`'s volume
//! moves channel faders instead of scaling PCM. With a reverse channel the client also polls the
//! faders and reports what someone changed at the desk. [`Desk`] separates one desk family's
//! protocol (addresses, fader law) from the transport, so further desks such as the Behringer
//! Wing can sit beside the X32.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};

use super::VolumeReport;

mod osc;

use osc::{Arg, decode, encode};

// ── Desk protocols ──────────────────────────────────────────────

/// What differs between desk families. The transport and the convergence logic in
/// [`DeskMixer`] are shared.
pub trait Desk: Send + 'static {
    /// Short name for logs.
    const NAME: &'static str;
    fn fader_address(&self, channel: u8) -> String;
    fn on_address(&self, channel: u8) -> String;
    /// `SnapDog` volume (0–100) as the desk's fader value.
    fn fader_value(&self, percent: u8) -> f32;
    /// The inverse of [`fader_value`](Self::fader_value), clamped to 0–100.
    fn fader_percent(&self, value: f32) -> u8;
    /// A fader value in dB, for logs.
    fn fader_db(&self, value: f32) -> f64;
}

/// Behringer X32 and Midas M32. OSC over UDP on a fixed port.
pub struct X32;

/// The X32 listens for OSC on this port only.
pub const X32_PORT: u16 = 10023;

impl Desk for X32 {
    const NAME: &'static str = "X32";

    fn fader_address(&self, channel: u8) -> String {
        format!("/ch/{channel:02}/mix/fader")
    }

    fn on_address(&self, channel: u8) -> String {
        format!("/ch/{channel:02}/mix/on")
    }

    /// The volume is the fader position, over the whole travel: 100 % is the top (+10 dB),
    /// 75 % is 0 dB. That is what `--mixer midi` gives on the X32 too (fader = CC / 128),
    /// so both ways to the desk agree. A ceiling belongs in the client's `max_volume`.
    fn fader_value(&self, percent: u8) -> f32 {
        x32_snap(f64::from(percent.min(100)) / 100.0)
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 0..=100
    fn fader_percent(&self, value: f32) -> u8 {
        (f64::from(value) * 100.0).round().clamp(0.0, 100.0) as u8
    }

    fn fader_db(&self, value: f32) -> f64 {
        x32_fader_to_db(f64::from(value))
    }
}

/// The desk stores the fader in 1024 steps (`k / 1023`) and rounds as documented in the
/// X32 OSC reference (p. 132). Snapping the same way makes a read-back compare equal.
#[allow(clippy::cast_possible_truncation)] // a fader value in 0..=1 fits f32 exactly enough
fn x32_snap(value: f64) -> f32 {
    ((value.clamp(0.0, 1.0) * 1023.5).floor() / 1023.0) as f32
}

/// The X32 fader law: four linear pieces in dB, 0.0 is off (X32 OSC reference, p. 132).
fn x32_fader_to_db(f: f64) -> f64 {
    if f <= 0.0 {
        f64::NEG_INFINITY
    } else if f >= 0.5 {
        f.mul_add(40.0, -30.0)
    } else if f >= 0.25 {
        f.mul_add(80.0, -50.0)
    } else if f >= 0.0625 {
        f.mul_add(160.0, -70.0)
    } else {
        f.mul_add(480.0, -90.0)
    }
}

/// Where the desk is and which channels carry this client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeskTarget {
    pub addr: SocketAddr,
    pub channels: Vec<u8>,
}

/// Parse `--mixer x32:<host>:<channels>[:<port>]`, channels as `31` or `31,32`.
///
/// Both channels of a stereo pair can be named: they are then written explicitly, so the
/// result does not depend on which parameters the desk's channel link covers.
///
/// # Errors
///
/// A missing or out-of-range channel, an unparsable port, or a host that does not resolve.
pub fn parse_x32_param(param: &str) -> anyhow::Result<DeskTarget> {
    let parts: Vec<&str> = param.split(':').collect();
    let (host, channels, port) = match parts.as_slice() {
        [host, channels] => (*host, *channels, X32_PORT),
        [host, channels, port] => (
            *host,
            *channels,
            port.parse::<u16>()
                .map_err(|_| anyhow!("invalid port {port:?}"))?,
        ),
        _ => bail!("expected x32:<host>:<channel>[,<channel>][:<port>], got {param:?}"),
    };
    if host.is_empty() {
        bail!("x32 mixer needs a host");
    }
    let mut parsed = Vec::new();
    for channel in channels.split(',') {
        let channel: u8 = channel
            .trim()
            .parse()
            .map_err(|_| anyhow!("invalid channel {channel:?}"))?;
        if !(1..=32).contains(&channel) {
            bail!("channel {channel} is outside 1–32");
        }
        if !parsed.contains(&channel) {
            parsed.push(channel);
        }
    }
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow!("cannot resolve {host}: {e}"))?
        .next()
        .ok_or_else(|| anyhow!("{host} resolves to no address"))?;
    Ok(DeskTarget {
        addr,
        channels: parsed,
    })
}

// ── Transport ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    percent: u8,
    muted: bool,
}

#[derive(Debug, Clone, Copy)]
struct Timing {
    reply_timeout: Duration,
    query_tries: u32,
    send_gap: Duration,
    min_backoff: Duration,
    max_backoff: Duration,
    /// How often the reverse channel reads the desk.
    poll_interval: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            reply_timeout: Duration::from_millis(300),
            query_tries: 3,
            send_gap: Duration::from_millis(10),
            min_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            poll_interval: Duration::from_millis(500),
        }
    }
}

/// Read-back tolerance: half a fader step.
const FADER_TOLERANCE: f32 = 0.5 / 1023.0;

/// How often a write is repeated when the desk answers with a different value. After that
/// the desk is left alone until the next volume change, so the client does not fight
/// someone moving the fader by hand.
const MISMATCH_RETRIES: u32 = 3;

/// A desk mixer. Volume changes go to a worker thread that coalesces bursts, writes the
/// desk, reads the values back and repeats until they match.
///
/// With a reverse channel (`report`), the worker also reads the desk while idle. When
/// someone moves a fader or switches a channel, the change goes to `report` and the other
/// named channels follow. `SnapDog`'s first volume after start still wins.
pub struct DeskMixer {
    tx: Sender<Target>,
}

impl DeskMixer {
    /// Start the worker for `desk` at `target`.
    ///
    /// # Errors
    ///
    /// The local UDP socket cannot be opened or connected.
    pub fn start<D: Desk>(
        desk: D,
        target: &DeskTarget,
        report: Option<VolumeReport>,
    ) -> std::io::Result<Self> {
        Self::start_with(desk, target, report, Timing::default())
    }

    fn start_with<D: Desk>(
        desk: D,
        target: &DeskTarget,
        report: Option<VolumeReport>,
        timing: Timing,
    ) -> std::io::Result<Self> {
        let bind = if target.addr.is_ipv4() {
            SocketAddr::from(([0, 0, 0, 0], 0))
        } else {
            SocketAddr::from(([0u16; 8], 0))
        };
        let sock = UdpSocket::bind(bind)?;
        sock.connect(target.addr)?;
        sock.set_read_timeout(Some(timing.reply_timeout))?;
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            desk,
            sock,
            channels: target.channels.clone(),
            timing,
            report,
        };
        thread::Builder::new()
            .name("desk-mixer".into())
            .spawn(move || worker.run(&rx))?;
        Ok(Self { tx })
    }

    /// Queue a volume change. Returns at once; the worker applies the latest value.
    pub fn set(&self, percent: u8, muted: bool) {
        // The worker lives as long as this sender, so the send cannot fail.
        let _ = self.tx.send(Target { percent, muted });
    }
}

#[derive(Clone, Copy)]
enum Outcome {
    Confirmed,
    Unreachable,
    Mismatch,
}

struct Worker<D> {
    desk: D,
    sock: UdpSocket,
    channels: Vec<u8>,
    timing: Timing,
    /// The reverse channel, if enabled.
    report: Option<VolumeReport>,
}

/// The worker's state between volume changes.
struct Progress {
    /// What the desk shows as far as `SnapDog` knows: confirmed by a read-back, or
    /// reported back from the desk.
    confirmed: Option<Target>,
    /// A volume still to be written.
    pending: Option<Target>,
    backoff: Duration,
    mismatches: u32,
}

/// Take whatever arrived meanwhile: only the newest volume matters.
fn latest(mut target: Target, rx: &Receiver<Target>) -> Target {
    while let Ok(newer) = rx.try_recv() {
        target = newer;
    }
    target
}

impl<D: Desk> Worker<D> {
    fn run(&self, rx: &Receiver<Target>) {
        self.announce();
        let mut p = Progress {
            confirmed: None,
            pending: None,
            backoff: self.timing.min_backoff,
            mismatches: 0,
        };
        loop {
            let polling = self.report.is_some() && p.confirmed.is_some();
            let received = if p.pending.is_some() {
                rx.recv_timeout(p.backoff)
            } else if polling {
                rx.recv_timeout(self.timing.poll_interval)
            } else {
                rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match received {
                // The desk already shows it, typically the server confirming a report.
                Ok(target) if polling && p.pending.is_none() && Some(target) == p.confirmed => {
                    continue;
                }
                Ok(target) => {
                    p.pending = Some(target);
                    p.backoff = self.timing.min_backoff;
                    p.mismatches = 0;
                }
                Err(RecvTimeoutError::Timeout) if p.pending.is_none() => {
                    if let Some(shown) = p.confirmed.and_then(|c| self.follow(c)) {
                        p.confirmed = Some(shown);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if let Some(target) = p.pending.map(|t| latest(t, rx)) {
                let outcome = self.apply(target, p.confirmed);
                self.settle(&mut p, target, outcome);
            }
        }
    }

    /// Update the progress after writing `target`.
    fn settle(&self, p: &mut Progress, target: Target, outcome: Outcome) {
        match outcome {
            Outcome::Confirmed => {
                if p.confirmed.is_none() {
                    tracing::info!(
                        desk = D::NAME,
                        volume = target.percent,
                        muted = target.muted,
                        "Desk volume applied"
                    );
                }
                p.confirmed = Some(target);
                p.pending = None;
            }
            Outcome::Unreachable => {
                if p.backoff == self.timing.min_backoff {
                    tracing::warn!(desk = D::NAME, "Desk not answering, retrying");
                }
                p.backoff = (p.backoff * 2).min(self.timing.max_backoff);
                p.pending = Some(target);
            }
            Outcome::Mismatch => {
                p.mismatches += 1;
                if p.mismatches < MISMATCH_RETRIES {
                    p.pending = Some(target);
                    return;
                }
                tracing::warn!(
                    desk = D::NAME,
                    "Desk reports a different value after {MISMATCH_RETRIES} writes; {}",
                    if self.report.is_some() {
                        "following the desk"
                    } else {
                        "leaving it until the next volume change"
                    }
                );
                // With the reverse channel someone holds the fader: stop writing, and the
                // next poll reports where it stands. Without it, forget the desk's state.
                p.confirmed = self.report.as_ref().map(|_| target);
                p.pending = None;
                p.mismatches = 0;
            }
        }
    }

    /// Log what the desk is and where its faders stand before `SnapDog` touches them.
    fn announce(&self) {
        if let Some(args) = self.query("/info") {
            let info: Vec<&str> = args
                .iter()
                .filter_map(|a| match a {
                    Arg::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            tracing::info!(desk = D::NAME, info = %info.join(" "), "Desk answering");
        } else {
            tracing::warn!(
                desk = D::NAME,
                "Desk not answering yet; the volume is applied once it does"
            );
        }
        for &channel in &self.channels {
            if let Some(Arg::Float(value)) = self
                .query(&self.desk.fader_address(channel))
                .and_then(|args| args.into_iter().next())
            {
                tracing::info!(
                    desk = D::NAME,
                    channel,
                    db = format!("{:.1}", self.desk.fader_db(value)),
                    "Desk fader before the first SnapDog volume"
                );
            }
        }
    }

    /// Write `target` and read it back.
    ///
    /// The on/off switch is written only when `SnapDog`'s own mute state changes, never on a
    /// plain volume change, so a channel someone muted at the desk stays muted (and with the
    /// reverse channel, gets reported). With the reverse channel the first write also sets the
    /// switch, so both sides start out agreeing. Muting switches the channel off before moving
    /// the fader; unmuting moves the fader first.
    fn apply(&self, target: Target, confirmed: Option<Target>) -> Outcome {
        let fader = self.desk.fader_value(target.percent);
        let switch = confirmed.map_or_else(
            || target.muted || self.report.is_some(),
            |c| c.muted != target.muted,
        );
        for &channel in &self.channels {
            if switch && target.muted {
                self.send(&self.desk.on_address(channel), Arg::Int(0));
            }
            self.send(&self.desk.fader_address(channel), Arg::Float(fader));
            if switch && !target.muted {
                self.send(&self.desk.on_address(channel), Arg::Int(1));
            }
        }
        let mut outcome = Outcome::Confirmed;
        for &channel in &self.channels {
            match self.query(&self.desk.fader_address(channel)) {
                None => return Outcome::Unreachable,
                Some(args) => match args.first() {
                    Some(Arg::Float(v)) if (v - fader).abs() <= FADER_TOLERANCE => {}
                    _ => outcome = Outcome::Mismatch,
                },
            }
            if switch {
                let expected = i32::from(!target.muted);
                match self.query(&self.desk.on_address(channel)) {
                    None => return Outcome::Unreachable,
                    Some(args) if args.first() == Some(&Arg::Int(expected)) => {}
                    Some(_) => outcome = Outcome::Mismatch,
                }
            }
        }
        outcome
    }

    /// Read the desk and report what changed since `confirmed`. Returns what the desk shows
    /// now, or `None` when nothing changed or the desk did not answer.
    ///
    /// The first channel that differs counts. The other named channels are set to its exact
    /// fader value and switch, so an unlinked pair stays together.
    fn follow(&self, confirmed: Target) -> Option<Target> {
        let mut seen = Vec::with_capacity(self.channels.len());
        for &channel in &self.channels {
            let Some(&Arg::Float(fader)) = self.query(&self.desk.fader_address(channel))?.first()
            else {
                return None;
            };
            let Some(&Arg::Int(on)) = self.query(&self.desk.on_address(channel))?.first() else {
                return None;
            };
            seen.push((channel, fader, on != 0));
        }
        let shows = |fader: f32, on: bool| Target {
            percent: self.desk.fader_percent(fader),
            muted: !on,
        };
        let &(moved, fader, on) = seen
            .iter()
            .find(|&&(_, fader, on)| shows(fader, on) != confirmed)?;
        let now = shows(fader, on);
        tracing::info!(
            desk = D::NAME,
            channel = moved,
            volume = now.percent,
            muted = now.muted,
            "Desk changed, reporting"
        );
        if let Some(report) = &self.report {
            report(now.percent, now.muted);
        }
        for &(channel, other_fader, other_on) in &seen {
            if (other_fader - fader).abs() > FADER_TOLERANCE {
                self.send(&self.desk.fader_address(channel), Arg::Float(fader));
            }
            if other_on != on {
                self.send(&self.desk.on_address(channel), Arg::Int(i32::from(on)));
            }
        }
        Some(now)
    }

    fn send(&self, address: &str, arg: Arg) {
        if let Err(e) = self.sock.send(&encode(address, &[arg])) {
            tracing::debug!(desk = D::NAME, address, error = %e, "Desk send failed");
        }
        thread::sleep(self.timing.send_gap);
    }

    /// Ask for `address` and wait for the matching reply, skipping anything else.
    fn query(&self, address: &str) -> Option<Vec<Arg>> {
        let request = encode(address, &[]);
        let mut buf = [0u8; 1536];
        for _ in 0..self.timing.query_tries {
            if self.sock.send(&request).is_err() {
                thread::sleep(self.timing.send_gap);
                continue;
            }
            let deadline = Instant::now() + self.timing.reply_timeout;
            while Instant::now() < deadline {
                let Ok(n) = self.sock.recv(&mut buf) else {
                    break;
                };
                if let Some((reply, args)) = decode(&buf[..n])
                    && reply == address
                {
                    return Some(args);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // ── X32 fader law ──

    #[test]
    fn fader_law_matches_the_reference_table() {
        // p. 158
        for (f, db) in [
            (0.0625, -60.0),
            (0.1, -54.0),
            (0.23125, -33.0),
            (0.33125, -23.5),
            (0.6, -6.0),
            (0.6875, -2.5),
            (0.7125, -1.5),
            (0.86875, 4.75),
            (0.99375, 9.75),
        ] {
            assert!((x32_fader_to_db(f) - db).abs() < 1e-9, "{f} -> {db}");
        }
        assert!(x32_fader_to_db(0.0).is_infinite());
    }

    #[test]
    fn volume_is_the_fader_position() {
        let desk = X32;
        let full = desk.fader_value(100);
        assert!(
            (desk.fader_db(full) - 10.0).abs() < 1e-6,
            "100 % is the top"
        );
        let unity = desk.fader_db(desk.fader_value(75));
        assert!(unity.abs() < 0.05, "75 % = 0 dB, got {unity}");
        assert!(desk.fader_db(desk.fader_value(0)).is_infinite());
        assert!(desk.fader_value(150) <= full, "clamped");
        // Every value lies on one of the desk's 1024 steps.
        for percent in 0..=100 {
            let steps = f64::from(desk.fader_value(percent)) * 1023.0;
            assert!((steps - steps.round()).abs() < 1e-3, "{percent} %");
        }
    }

    // ── Parameters ──

    #[test]
    fn parses_parameters() {
        let t = parse_x32_param("127.0.0.1:31").unwrap();
        assert_eq!(t.addr, SocketAddr::from(([127, 0, 0, 1], X32_PORT)));
        assert_eq!(t.channels, vec![31]);
        let t = parse_x32_param("127.0.0.1:31,32,31:10024").unwrap();
        assert_eq!(t.addr.port(), 10024);
        assert_eq!(t.channels, vec![31, 32]);
    }

    #[test]
    fn rejects_bad_parameters() {
        for bad in [
            "",
            "127.0.0.1",
            ":31",
            "127.0.0.1:0",
            "127.0.0.1:33",
            "127.0.0.1:x",
            "127.0.0.1:31:port",
            "127.0.0.1:31:1:2",
        ] {
            assert!(parse_x32_param(bad).is_err(), "{bad:?} accepted");
        }
    }

    /// Read-only check against a real desk: `X32_LIVE_HOST=<addr> cargo test -p snapdog-client
    /// -- --ignored live_desk`. Queries only, never writes.
    #[test]
    #[ignore = "needs a real X32 reachable at X32_LIVE_HOST"]
    fn live_desk_answers_read_only_queries() {
        let host = std::env::var("X32_LIVE_HOST").expect("set X32_LIVE_HOST");
        let target = parse_x32_param(&format!("{host}:31,32")).unwrap();
        let sock = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();
        sock.connect(target.addr).unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let worker = Worker {
            desk: X32,
            sock,
            channels: target.channels,
            timing: Timing::default(),
            report: None,
        };
        let info = worker.query("/info").expect("desk did not answer /info");
        assert!(matches!(info.first(), Some(Arg::Str(_))), "{info:?}");
        println!("info: {info:?}");
        for channel in [31u8, 32] {
            let fader = worker
                .query(&X32.fader_address(channel))
                .expect("no fader reply");
            let Some(Arg::Float(value)) = fader.first() else {
                panic!("fader reply {fader:?}");
            };
            assert!((0.0..=1.0).contains(value));
            println!("ch{channel}: {value} = {:.1} dB", X32.fader_db(*value));
        }
    }

    /// Reverse channel against a real desk, which it writes to and restores afterwards:
    /// `X32_LIVE_HOST=<addr> X32_LIVE_WRITE_CHANNELS=29,30 cargo test -p snapdog-client --
    /// --ignored live_write --nocapture`. Name two unlinked channels with no signal: their
    /// faders go up to +6 dB and their switches off and on. A second socket plays the hand
    /// at the desk.
    #[test]
    #[ignore = "writes to a real X32 at X32_LIVE_HOST"]
    fn live_write_reverse_channel() {
        let Ok(channels) = std::env::var("X32_LIVE_WRITE_CHANNELS") else {
            println!("X32_LIVE_WRITE_CHANNELS not set, nothing written");
            return;
        };
        let host = std::env::var("X32_LIVE_HOST").expect("set X32_LIVE_HOST");
        let target = parse_x32_param(&format!("{host}:{channels}")).unwrap();
        let &[a, b] = target.channels.as_slice() else {
            panic!("name exactly two channels");
        };
        let hand = LiveHand::connect(&target);
        let saved: Vec<(String, Arg)> = [a, b]
            .iter()
            .flat_map(|&ch| [X32.fader_address(ch), X32.on_address(ch)])
            .map(|addr| {
                let value = hand.read(&addr).expect("desk did not answer");
                (addr, value)
            })
            .collect();
        println!("saved: {saved:?}");

        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            live_steps(&hand, &target, a, b);
        }));

        // The mixer is dropped, on success or panic. Let its worker finish a poll in
        // flight and stop before the desk is restored.
        thread::sleep(Duration::from_secs(1));
        for (address, value) in &saved {
            hand.set(address, value.clone());
        }
        for (address, value) in &saved {
            assert_eq!(
                hand.read(address).as_ref(),
                Some(value),
                "{address} not restored"
            );
        }
        println!("restored");
        if let Err(panic) = run {
            std::panic::resume_unwind(panic);
        }
    }

    /// The hand at a real desk: a socket of its own, as the desk's surface would be.
    struct LiveHand(Worker<X32>);

    impl LiveHand {
        fn connect(target: &DeskTarget) -> Self {
            let sock = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).unwrap();
            sock.connect(target.addr).unwrap();
            sock.set_read_timeout(Some(Duration::from_millis(300)))
                .unwrap();
            Self(Worker {
                desk: X32,
                sock,
                channels: target.channels.clone(),
                timing: Timing::default(),
                report: None,
            })
        }

        fn read(&self, address: &str) -> Option<Arg> {
            self.0.query(address).and_then(|v| v.into_iter().next())
        }

        fn set(&self, address: &str, value: Arg) {
            self.0.send(address, value);
        }

        /// Wait until `address` shows `want`, faders within half a step.
        fn wait(&self, address: &str, want: &Arg) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let got = self.read(address);
                let hit = match (&got, want) {
                    (Some(Arg::Float(g)), Arg::Float(w)) => (g - w).abs() <= FADER_TOLERANCE,
                    (got, want) => got.as_ref() == Some(want),
                };
                if hit {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{address} is {got:?}, wanted {want:?}"
                );
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    /// What `live_write_reverse_channel` does on channels `a` and `b`.
    fn live_steps(hand: &LiveHand, target: &DeskTarget, a: u8, b: u8) {
        let reports = Reports::default();
        let sink = Arc::clone(&reports.0);
        let report: VolumeReport = Box::new(move |percent, muted| {
            println!("reported: {percent} % muted={muted}");
            sink.lock().unwrap().push((percent, muted));
        });
        let mixer = DeskMixer::start(X32, target, Some(report)).unwrap();

        // SnapDog's first volume sets faders and switches.
        mixer.set(40, false);
        for ch in [a, b] {
            hand.wait(&X32.fader_address(ch), &Arg::Float(X32.fader_value(40)));
            hand.wait(&X32.on_address(ch), &Arg::Int(1));
        }
        thread::sleep(Duration::from_secs(1));

        // A hand moves the second fader off the grid; the first follows exactly.
        hand.set(&X32.fader_address(b), Arg::Float(0.33));
        reports.wait_for((33, false));
        hand.wait(&X32.fader_address(a), &Arg::Float(x32_snap(0.33)));

        // The hand switches the second channel off.
        hand.set(&X32.on_address(b), Arg::Int(0));
        reports.wait_for((33, true));
        hand.wait(&X32.on_address(a), &Arg::Int(0));

        // The server answers with another volume and unmutes.
        mixer.set(30, false);
        for ch in [a, b] {
            hand.wait(&X32.fader_address(ch), &Arg::Float(X32.fader_value(30)));
            hand.wait(&X32.on_address(ch), &Arg::Int(1));
        }
        thread::sleep(Duration::from_secs(1));

        // High up the travel, above 0 dB: reported as it is, the other follows.
        hand.set(&X32.fader_address(a), Arg::Float(0.9));
        reports.wait_for((90, false));
        hand.wait(&X32.fader_address(b), &Arg::Float(x32_snap(0.9)));

        assert_eq!(reports.all(), vec![(33, false), (33, true), (90, false)]);
    }

    // ── Worker against a fake desk ──

    #[derive(Default)]
    struct FakeState {
        values: HashMap<String, Arg>,
        writes: Vec<(String, Arg)>,
        drop_fader_writes: u32,
        silent: bool,
        /// Answered queries, to see the reverse channel polling.
        queries: usize,
    }

    struct FakeDesk {
        target: DeskTarget,
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeDesk {
        fn start(channels: &[u8]) -> Self {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            let addr = sock.local_addr().unwrap();
            let mut state = FakeState::default();
            for &ch in channels {
                state
                    .values
                    .insert(X32.fader_address(ch), Arg::Float(0.392));
                state.values.insert(X32.on_address(ch), Arg::Int(1));
            }
            let state = Arc::new(Mutex::new(state));
            let shared = Arc::clone(&state);
            thread::spawn(move || {
                let mut buf = [0u8; 1536];
                while let Ok((n, from)) = sock.recv_from(&mut buf) {
                    if let Some((address, args)) = decode(&buf[..n])
                        && let Some(reply) = Self::handle(&shared, address, args)
                    {
                        let _ = sock.send_to(&reply, from);
                    }
                }
            });
            Self {
                target: DeskTarget {
                    addr,
                    channels: channels.to_vec(),
                },
                state,
            }
        }

        /// Apply a write or answer a query. Offline (`silent`) means neither happens.
        fn handle(state: &Mutex<FakeState>, address: String, args: Vec<Arg>) -> Option<Vec<u8>> {
            let mut s = state.lock().unwrap();
            if s.silent {
                return None;
            }
            if let Some(value) = args.into_iter().next() {
                s.writes.push((address.clone(), value.clone()));
                if address.ends_with("/fader") && s.drop_fader_writes > 0 {
                    s.drop_fader_writes -= 1;
                } else {
                    s.values.insert(address, value);
                }
                return None;
            }
            let answer: Vec<Arg> = s.values.get(&address).cloned().into_iter().collect();
            s.queries += 1;
            drop(s);
            if address == "/info" {
                return Some(encode(
                    "/info",
                    &["V2.07", "osc-server", "X32RACK", "4.13"].map(|s| Arg::Str(s.into())),
                ));
            }
            Some(encode(&address, &answer))
        }

        const TIMING: Timing = Timing {
            reply_timeout: Duration::from_millis(50),
            query_tries: 2,
            send_gap: Duration::from_millis(1),
            min_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(100),
            poll_interval: Duration::from_millis(20),
        };

        /// Without the reverse channel (`--mixer-sync out`).
        fn mixer(&self) -> DeskMixer {
            DeskMixer::start_with(X32, &self.target, None, Self::TIMING).unwrap()
        }

        /// With the reverse channel; the reports land in the returned list.
        fn reporting_mixer(&self) -> (DeskMixer, Reports) {
            let reports = Reports::default();
            let sink = Arc::clone(&reports.0);
            let report: VolumeReport = Box::new(move |percent, muted| {
                sink.lock().unwrap().push((percent, muted));
            });
            let mixer = DeskMixer::start_with(X32, &self.target, Some(report), Self::TIMING);
            (mixer.unwrap(), reports)
        }

        fn value(&self, address: &str) -> Option<Arg> {
            self.state.lock().unwrap().values.get(address).cloned()
        }

        /// Change a value as a hand at the desk would: no write goes through the client.
        fn touch(&self, address: &str, value: Arg) {
            self.state
                .lock()
                .unwrap()
                .values
                .insert(address.to_owned(), value);
        }

        fn wait_for(&self, address: &str, want: &Arg) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if self.value(address).as_ref() == Some(want) {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
            panic!("{address} is {:?}, wanted {want:?}", self.value(address));
        }

        fn writes_to(&self, suffix: &str) -> Vec<Arg> {
            self.state
                .lock()
                .unwrap()
                .writes
                .iter()
                .filter(|(a, _)| a.ends_with(suffix))
                .map(|(_, v)| v.clone())
                .collect()
        }
    }

    #[derive(Default)]
    struct Reports(Arc<Mutex<Vec<(u8, bool)>>>);

    impl Reports {
        fn all(&self) -> Vec<(u8, bool)> {
            self.0.lock().unwrap().clone()
        }

        fn wait_for(&self, want: (u8, bool)) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if self.all().last() == Some(&want) {
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
            panic!("reports {:?}, wanted {want:?} last", self.all());
        }
    }

    #[test]
    fn fader_percent_inverts_fader_value() {
        for percent in 0..=100 {
            assert_eq!(X32.fader_percent(X32.fader_value(percent)), percent);
        }
        assert_eq!(X32.fader_percent(1.5), 100, "clamped");
        assert_eq!(X32.fader_percent(0.0), 0);
    }

    #[test]
    fn volume_moves_every_named_channel() {
        let desk = FakeDesk::start(&[31, 32]);
        let mixer = desk.mixer();
        mixer.set(70, false);
        let want = Arg::Float(X32.fader_value(70));
        desk.wait_for("/ch/31/mix/fader", &want);
        desk.wait_for("/ch/32/mix/fader", &want);
    }

    #[test]
    fn plain_volume_change_never_touches_the_on_switch() {
        let desk = FakeDesk::start(&[31]);
        // Someone muted the channel at the desk.
        desk.state
            .lock()
            .unwrap()
            .values
            .insert(X32.on_address(31), Arg::Int(0));
        let mixer = desk.mixer();
        mixer.set(40, false);
        mixer.set(60, false);
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(X32.fader_value(60)));
        assert!(
            desk.writes_to("/mix/on").is_empty(),
            "the client unmuted the desk"
        );
        assert_eq!(desk.value("/ch/31/mix/on"), Some(Arg::Int(0)));
    }

    #[test]
    fn mute_and_unmute_switch_the_channel() {
        let desk = FakeDesk::start(&[31]);
        let mixer = desk.mixer();
        mixer.set(50, true);
        desk.wait_for("/ch/31/mix/on", &Arg::Int(0));
        mixer.set(50, false);
        desk.wait_for("/ch/31/mix/on", &Arg::Int(1));
    }

    #[test]
    fn a_burst_lands_on_its_last_value() {
        let desk = FakeDesk::start(&[31]);
        let mixer = desk.mixer();
        for percent in 0..=100 {
            mixer.set(percent, false);
        }
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(X32.fader_value(100)));
    }

    #[test]
    fn lost_writes_are_repeated() {
        let desk = FakeDesk::start(&[31]);
        desk.state.lock().unwrap().drop_fader_writes = 2;
        let mixer = desk.mixer();
        mixer.set(80, false);
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(X32.fader_value(80)));
        assert!(desk.writes_to("/mix/fader").len() >= 3);
    }

    #[test]
    fn a_desk_that_answers_later_still_gets_the_volume() {
        let desk = FakeDesk::start(&[31]);
        desk.state.lock().unwrap().silent = true;
        let mixer = desk.mixer();
        mixer.set(30, false);
        thread::sleep(Duration::from_millis(250));
        desk.state.lock().unwrap().silent = false;
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(X32.fader_value(30)));
    }

    #[test]
    fn stops_fighting_a_fader_held_by_hand() {
        let desk = FakeDesk::start(&[31]);
        desk.state.lock().unwrap().drop_fader_writes = u32::MAX;
        let mixer = desk.mixer();
        mixer.set(90, false);
        thread::sleep(Duration::from_millis(600));
        let writes = desk.writes_to("/mix/fader").len();
        thread::sleep(Duration::from_millis(400));
        assert_eq!(
            desk.writes_to("/mix/fader").len(),
            writes,
            "still writing after giving up"
        );
        assert!(writes <= usize::try_from(MISMATCH_RETRIES).unwrap());
    }

    // ── Reverse channel ──

    /// A reporting mixer on `channels`, settled at 50 %.
    fn settled(channels: &[u8]) -> (FakeDesk, DeskMixer, Reports) {
        let desk = FakeDesk::start(channels);
        let (mixer, reports) = desk.reporting_mixer();
        mixer.set(50, false);
        for &ch in channels {
            desk.wait_for(&X32.fader_address(ch), &Arg::Float(X32.fader_value(50)));
        }
        // The first write also sets every switch; after that the worker reads back and
        // starts polling. A hand move before then would race the write.
        let deadline = Instant::now() + Duration::from_secs(3);
        while desk.writes_to("/mix/on").len() < channels.len() {
            assert!(Instant::now() < deadline, "switches never written");
            thread::sleep(Duration::from_millis(5));
        }
        let start = desk.state.lock().unwrap().queries;
        // Read-back plus two full polls: fader and switch per channel each.
        while desk.state.lock().unwrap().queries < start + 6 * channels.len() {
            assert!(Instant::now() < deadline, "the worker never polled");
            thread::sleep(Duration::from_millis(5));
        }
        (desk, mixer, reports)
    }

    #[test]
    fn the_first_volume_wins_over_the_desk() {
        let desk = FakeDesk::start(&[31]);
        // Muted at the desk before the client started.
        desk.touch("/ch/31/mix/on", Arg::Int(0));
        let (mixer, reports) = desk.reporting_mixer();
        mixer.set(50, false);
        desk.wait_for("/ch/31/mix/on", &Arg::Int(1));
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(X32.fader_value(50)));
        thread::sleep(Duration::from_millis(150));
        assert!(reports.all().is_empty(), "{:?}", reports.all());
    }

    #[test]
    fn a_fader_moved_by_hand_is_reported() {
        let (desk, _mixer, reports) = settled(&[31]);
        desk.touch("/ch/31/mix/fader", Arg::Float(X32.fader_value(70)));
        reports.wait_for((70, false));
    }

    #[test]
    fn a_channel_switched_off_at_the_desk_is_reported_as_mute() {
        let (desk, _mixer, reports) = settled(&[31]);
        desk.touch("/ch/31/mix/on", Arg::Int(0));
        reports.wait_for((50, true));
        desk.touch("/ch/31/mix/on", Arg::Int(1));
        reports.wait_for((50, false));
    }

    #[test]
    fn the_other_channel_follows_the_moved_one_exactly() {
        let (desk, _mixer, reports) = settled(&[31, 32]);
        // Off the percent grid, as a hand leaves it.
        desk.touch("/ch/32/mix/fader", Arg::Float(0.537));
        desk.touch("/ch/32/mix/on", Arg::Int(0));
        reports.wait_for((54, true));
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(0.537));
        desk.wait_for("/ch/31/mix/on", &Arg::Int(0));
    }

    #[test]
    fn the_servers_confirmation_leaves_the_hand_position() {
        let (desk, mixer, reports) = settled(&[31]);
        desk.touch("/ch/31/mix/fader", Arg::Float(0.537));
        reports.wait_for((54, false));
        let writes = desk.writes_to("/mix/fader").len();
        // The server answers with the volume the client just reported.
        mixer.set(54, false);
        thread::sleep(Duration::from_millis(150));
        assert_eq!(desk.writes_to("/mix/fader").len(), writes);
        assert_eq!(desk.value("/ch/31/mix/fader"), Some(Arg::Float(0.537)));
    }

    #[test]
    fn a_capped_volume_from_the_server_moves_the_fader_back() {
        let (desk, mixer, reports) = settled(&[31]);
        desk.touch("/ch/31/mix/fader", Arg::Float(X32.fader_value(90)));
        reports.wait_for((90, false));
        // The client's max_volume is 60.
        mixer.set(60, false);
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(X32.fader_value(60)));
        thread::sleep(Duration::from_millis(150));
        assert_eq!(
            reports.all(),
            vec![(90, false)],
            "the push-back is no report"
        );
    }

    #[test]
    fn the_whole_fader_travel_is_reported_and_left_alone() {
        let (desk, _mixer, reports) = settled(&[31]);
        desk.touch("/ch/31/mix/fader", Arg::Float(1.0)); // +10 dB
        reports.wait_for((100, false));
        let writes = desk.writes_to("/mix/fader").len();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(desk.writes_to("/mix/fader").len(), writes);
        assert_eq!(desk.value("/ch/31/mix/fader"), Some(Arg::Float(1.0)));
    }

    #[test]
    fn a_held_fader_is_followed_not_fought() {
        let desk = FakeDesk::start(&[31]);
        desk.state.lock().unwrap().drop_fader_writes = u32::MAX;
        let (mixer, reports) = desk.reporting_mixer();
        mixer.set(90, false);
        // The fader stays at 0.392, about 39 %.
        reports.wait_for((39, false));
        let writes = desk.writes_to("/mix/fader").len();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(desk.writes_to("/mix/fader").len(), writes, "still writing");
    }

    #[test]
    fn without_the_reverse_channel_a_hand_move_stays_unreported_and_untouched() {
        let desk = FakeDesk::start(&[31]);
        let mixer = desk.mixer();
        mixer.set(50, false);
        desk.wait_for("/ch/31/mix/fader", &Arg::Float(X32.fader_value(50)));
        let writes = desk.state.lock().unwrap().writes.len();
        desk.touch("/ch/31/mix/fader", Arg::Float(0.9));
        thread::sleep(Duration::from_millis(150));
        assert_eq!(desk.state.lock().unwrap().writes.len(), writes);
        assert_eq!(desk.value("/ch/31/mix/fader"), Some(Arg::Float(0.9)));
    }
}
