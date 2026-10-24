use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod fake_desk;

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

/// Volume reports the mixer hands back, shared by the live and the fake-desk tests.
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
