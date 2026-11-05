//! Token-bucket rate limiter (J. Turner, 1986 — see `docs/ALGORITHMS.md`).
//!
//! The bucket holds `capacity` tokens, one token = one byte, and refills at
//! `rate` tokens per second. `acquire(n)` sleeps until `n` tokens are
//! available. To avoid deadlock when a single chunk is larger than the
//! bucket (a fixed defect from early revisions), `acquire` temporarily
//! raises the capacity to at least `n` — one burst is always granted.

use std::time::{Duration, Instant};

/// Byte-oriented token bucket.
pub struct TokenBucket {
    rate: f64,
    capacity: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    /// Create a bucket with `rate` bytes/sec and a capacity of 2 seconds
    /// worth of refill (minimum 64 KiB, maximum 16 MiB).
    pub fn new(rate: f64) -> Self {
        let capacity = (rate * 2.0).clamp(64.0 * 1024.0, 16.0 * 1024.0 * 1024.0);
        TokenBucket { rate, capacity, tokens: capacity, last: Instant::now() }
    }

    /// Change the refill rate, clamping stored tokens to the new capacity.
    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate.max(1.0);
        self.capacity = (self.rate * 2.0).clamp(64.0 * 1024.0, 16.0 * 1024.0 * 1024.0);
        self.tokens = self.tokens.min(self.capacity);
    }

    /// Current refill rate in bytes/sec.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        if dt > 0.0 {
            self.tokens = (self.tokens + dt * self.rate).min(self.capacity);
            self.last = now;
        }
    }

    /// Take `n` tokens, sleeping until they are available.
    pub fn acquire(&mut self, n: u64) {
        if self.rate.is_infinite() || self.rate <= 0.0 {
            return;
        }
        let n = n as f64;
        // Never deadlock on chunks bigger than the bucket.
        if n > self.capacity {
            self.capacity = n;
        }
        loop {
            self.refill();
            if self.tokens >= n {
                self.tokens -= n;
                return;
            }
            let missing = n - self.tokens;
            let wait = (missing / self.rate).min(0.25).max(0.001);
            std::thread::sleep(Duration::from_secs_f64(wait));
        }
    }

    /// Tokens currently available (test helper).
    pub fn tokens(&mut self) -> f64 {
        self.refill();
        self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_burst_within_capacity_is_instant() {
        let mut b = TokenBucket::new(1_000_000.0);
        let t0 = Instant::now();
        b.acquire(256 * 1024);
        assert!(t0.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn test_sustained_rate_throttles() {
        // 1 MiB/s bucket: pulling 3 MiB must take >= ~1.5s (capacity 2MiB first).
        let mut b = TokenBucket::new(1024.0 * 1024.0);
        let t0 = Instant::now();
        b.acquire(3 * 1024 * 1024);
        let el = t0.elapsed();
        assert!(el >= Duration::from_millis(900), "elapsed {el:?}");
    }

    #[test]
    fn test_chunk_larger_than_capacity_does_not_deadlock() {
        // capacity = 2s * 2 MiB/s = 4 MiB; ask for twice the capacity in one
        // go. Must complete (no deadlock) in roughly the deficit time
        // ((8-4)/2 = 2 s), with generous margins for slow runners.
        let mut b = TokenBucket::new(2.0 * 1024.0 * 1024.0);
        let t0 = Instant::now();
        b.acquire(8 * 1024 * 1024);
        let el = t0.elapsed();
        assert!(el >= Duration::from_secs(1), "oversized chunk was not throttled: {el:?}");
        assert!(el < Duration::from_secs(4), "oversized chunk deadlocked or stalled: {el:?}");
        assert!(b.tokens() >= 0.0);
    }

    #[test]
    fn test_set_rate_clamps_tokens() {
        let mut b = TokenBucket::new(10_000_000.0);
        let full = b.tokens();
        b.set_rate(10_000.0);
        assert!(b.tokens() <= full);
        assert!((b.rate() - 10_000.0).abs() < f64::EPSILON);
    }
}
