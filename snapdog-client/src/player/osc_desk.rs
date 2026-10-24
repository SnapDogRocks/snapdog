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
mod tests;
