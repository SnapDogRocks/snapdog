// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Flood protection for inbound commands, and throttled logging.
//!
//! Anyone who may publish below the base topic can send commands as fast as the
//! broker forwards them. The limiter keeps that from becoming a flood of zone
//! commands: every addressed object (a zone, a client) has its own token bucket,
//! and one bucket covers everything. Both are cheap, in-memory and deterministic
//! (the clock is a parameter).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::topics::{MAX_INDEX, Scope};

/// Commands per object that may arrive in one burst.
const OBJECT_BURST: f64 = 30.0;
/// Commands per second per object, sustained. A slider drag sends about ten.
const OBJECT_RATE: f64 = 20.0;
/// Commands in one burst over all objects.
const GLOBAL_BURST: f64 = 100.0;
/// Commands per second over all objects, sustained.
const GLOBAL_RATE: f64 = 100.0;

/// A token bucket: `capacity` tokens, refilled at `rate` per second.
#[derive(Clone, Copy, Debug)]
struct TokenBucket {
    tokens: f64,
    capacity: f64,
    rate: f64,
    last: Instant,
}

impl TokenBucket {
    const fn new(capacity: f64, rate: f64, now: Instant) -> Self {
        Self {
            tokens: capacity,
            capacity,
            rate,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = elapsed.mul_add(self.rate, self.tokens).min(self.capacity);
        self.last = now;
    }

    /// Whether a token is available (without taking it).
    fn has_token(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= 1.0
    }

    fn take(&mut self) {
        self.tokens = (self.tokens - 1.0).max(0.0);
    }
}

/// Per-object and global command rate limit.
#[derive(Debug)]
pub struct RateLimiter {
    objects: HashMap<(Scope, usize), TokenBucket>,
    global: TokenBucket,
    /// Capacity and refill rate of a new per-object bucket.
    object: (f64, f64),
}

impl RateLimiter {
    pub fn new(now: Instant) -> Self {
        Self {
            objects: HashMap::new(),
            global: TokenBucket::new(GLOBAL_BURST, GLOBAL_RATE, now),
            object: (OBJECT_BURST, OBJECT_RATE),
        }
    }

    /// A limiter that never refuses, for the tests that are about something else.
    #[cfg(test)]
    pub fn unlimited(now: Instant) -> Self {
        Self {
            objects: HashMap::new(),
            global: TokenBucket::new(1e12, 1e12, now),
            object: (1e12, 1e12),
        }
    }

    /// Whether a command for object `index` of `scope` may pass at `now`. A command
    /// that does not pass costs nothing.
    ///
    /// `index` is bounded by [`MAX_INDEX`] (the topic parser enforces it), so the
    /// bookkeeping stays finite whatever is published.
    pub fn allow(&mut self, scope: Scope, index: usize, now: Instant) -> bool {
        debug_assert!(index <= MAX_INDEX);
        let object = self
            .objects
            .entry((scope, index))
            .or_insert_with(|| TokenBucket::new(self.object.0, self.object.1, now));
        if !object.has_token(now) || !self.global.has_token(now) {
            return false;
        }
        object.take();
        self.global.take();
        true
    }
}

/// Lets one log line per category through per interval and counts what it held
/// back, so that a flood of bad messages cannot flood the log.
#[derive(Debug)]
pub struct LogThrottle {
    interval: Duration,
    categories: HashMap<&'static str, (Instant, u64)>,
}

impl LogThrottle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            categories: HashMap::new(),
        }
    }

    /// `Some(suppressed)` if a line of `category` may be logged at `now`, with the
    /// number of lines held back since the last one; `None` if it must be held back.
    pub fn check(&mut self, category: &'static str, now: Instant) -> Option<u64> {
        let Some((last, suppressed)) = self.categories.get_mut(category) else {
            self.categories.insert(category, (now, 0));
            return Some(0);
        };
        if now.saturating_duration_since(*last) >= self.interval {
            let held_back = *suppressed;
            *last = now;
            *suppressed = 0;
            Some(held_back)
        } else {
            *suppressed += 1;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn a_burst_passes_up_to_the_bucket_size_then_is_dropped() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(t0);
        let passed = (0..100)
            .filter(|_| limiter.allow(Scope::Zone, 1, t0))
            .count();
        assert_eq!(passed, OBJECT_BURST as usize);
    }

    #[test]
    fn the_sustained_rate_is_the_refill_rate() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(t0);
        // Drain, then offer 100 commands per second for 10 s.
        while limiter.allow(Scope::Zone, 1, t0) {}
        let mut passed = 0;
        for ms in (0..10_000).step_by(10) {
            if limiter.allow(Scope::Zone, 1, at(t0, ms + 10)) {
                passed += 1;
            }
        }
        // 20 per second for 10 s, give or take the edge.
        assert!((195..=205).contains(&passed), "{passed}");
    }

    #[test]
    fn slow_senders_are_never_limited() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(t0);
        for i in 0..1000 {
            assert!(limiter.allow(Scope::Zone, 1, at(t0, i * 100)), "{i}");
        }
    }

    #[test]
    fn objects_have_their_own_buckets() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(t0);
        while limiter.allow(Scope::Zone, 1, t0) {}
        assert!(!limiter.allow(Scope::Zone, 1, t0));
        assert!(
            limiter.allow(Scope::Zone, 2, t0),
            "another zone is unaffected"
        );
        assert!(
            limiter.allow(Scope::Client, 1, t0),
            "a client is not the zone with the same number"
        );
    }

    #[test]
    fn the_global_bucket_caps_the_sum_over_all_objects() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(t0);
        let passed = (0..1000)
            .filter(|i| limiter.allow(Scope::Zone, 1 + i % 50, t0))
            .count();
        assert_eq!(passed, GLOBAL_BURST as usize);
    }

    #[test]
    fn a_denied_command_does_not_spend_the_global_budget() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(t0);
        while limiter.allow(Scope::Zone, 1, t0) {}
        // Zone 1 hammering must not starve zone 2 of the global budget.
        for _ in 0..10_000 {
            assert!(!limiter.allow(Scope::Zone, 1, t0));
        }
        assert!(limiter.allow(Scope::Zone, 2, t0));
    }

    #[test]
    fn bookkeeping_is_bounded_by_the_index_range() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(t0);
        for index in 1..=MAX_INDEX {
            limiter.allow(Scope::Zone, index, t0);
            limiter.allow(Scope::Client, index, t0);
        }
        assert_eq!(limiter.objects.len(), 2 * MAX_INDEX);
    }

    #[test]
    fn log_throttle_lets_one_line_per_interval_through_and_counts_the_rest() {
        let t0 = Instant::now();
        let mut throttle = LogThrottle::new(Duration::from_secs(10));
        assert_eq!(throttle.check("bad payload", t0), Some(0));
        assert_eq!(throttle.check("bad payload", at(t0, 1)), None);
        assert_eq!(throttle.check("bad payload", at(t0, 9_000)), None);
        assert_eq!(
            throttle.check("bad payload", at(t0, 10_000)),
            Some(2),
            "reports what it held back"
        );
        assert_eq!(throttle.check("bad payload", at(t0, 10_001)), None);
        // Categories are independent.
        assert_eq!(throttle.check("rate limited", at(t0, 10_001)), Some(0));
    }
}
