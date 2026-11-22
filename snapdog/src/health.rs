//! Process-wide fault state behind the KNX "System Fault" object.
//!
//! Components report a fault when they fail and clear it when they recover. Subscribers (the
//! KNX publisher) are woken through a `watch` channel on every change, so a fault reaches the
//! bus within moments instead of at the next heartbeat.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use tokio::sync::watch;

/// A component whose failure counts as a system fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaultSource {
    /// The Snapcast server: the embedded server task ended with an error, or the connection to
    /// an external Snapcast server is down.
    Snapcast,
    /// The Subsonic server cannot be reached.
    Subsonic,
    /// The `AirPlay` receiver of a zone failed to start.
    AirPlay(usize),
    /// The Spotify Connect receiver of a zone failed to start or stopped with an error.
    Spotify(usize),
}

static FAULTS: LazyLock<watch::Sender<BTreeSet<FaultSource>>> =
    LazyLock::new(|| watch::channel(BTreeSet::new()).0);

/// Record that `source` failed (`ok == false`) or works (`ok == true`). Subscribers are woken
/// only when the set of active faults changes.
pub fn report(source: FaultSource, ok: bool) {
    let changed = FAULTS.send_if_modified(|faults| {
        if ok {
            faults.remove(&source)
        } else {
            faults.insert(source)
        }
    });
    if changed {
        if ok {
            tracing::info!(?source, "Fault cleared");
        } else {
            tracing::warn!(?source, "Fault raised");
        }
    }
}

/// Whether no fault is active.
#[must_use]
pub fn is_healthy() -> bool {
    FAULTS.borrow().is_empty()
}

/// The active faults, for display and diagnostics.
#[must_use]
pub fn active_faults() -> Vec<FaultSource> {
    FAULTS.borrow().iter().copied().collect()
}

/// A receiver that is woken when the set of active faults changes.
#[must_use]
pub fn subscribe() -> watch::Receiver<BTreeSet<FaultSource>> {
    FAULTS.subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The state is process-wide: each test uses its own zone index so parallel tests in one
    // process do not see each other's faults through `is_healthy`.

    #[tokio::test]
    async fn a_fault_wakes_subscribers_and_recovery_clears_it() {
        let mut rx = subscribe();
        rx.borrow_and_update();
        report(FaultSource::Spotify(901), false);
        rx.changed().await.unwrap();
        assert!(rx.borrow_and_update().contains(&FaultSource::Spotify(901)));
        assert!(!is_healthy());

        // Reporting the same state again changes nothing and wakes nobody.
        report(FaultSource::Spotify(901), false);
        assert!(!rx.has_changed().unwrap());

        report(FaultSource::Spotify(901), true);
        rx.changed().await.unwrap();
        assert!(!rx.borrow_and_update().contains(&FaultSource::Spotify(901)));
    }
}
