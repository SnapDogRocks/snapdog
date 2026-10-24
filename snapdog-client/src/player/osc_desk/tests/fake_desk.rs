//! Worker and reverse-channel tests against a fake desk.

use super::*;

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
