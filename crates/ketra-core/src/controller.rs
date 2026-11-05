//! Speed controller: binds the token-bucket throttle and the AIMD adapter
//! into the object the pipeline talks to. A fixed user limit (`--limit`)
//! bypasses AIMD adaptation; without a limit the bucket refills at the
//! AIMD-chosen rate (effectively unbounded).

use crate::aimd::Aimd;
use crate::throttle::TokenBucket;

/// Combined throttle + adaptation policy for one transfer stream.
pub struct SpeedController {
    bucket: TokenBucket,
    aimd: Aimd,
    /// User-fixed limit in bytes/sec; `None` = automatic (AIMD).
    manual_limit: Option<u64>,
    /// When false, `before_chunk` is a no-op (unlimited).
    enabled: bool,
}

impl SpeedController {
    /// Create a controller. `limit` (bytes/sec) applies a hard throttle.
    pub fn new(limit: Option<u64>) -> Self {
        match limit {
            Some(l) => {
                let l = l.max(1024) as f64;
                SpeedController {
                    bucket: TokenBucket::new(l),
                    aimd: Aimd::new(l, l, l),
                    manual_limit: Some(l as u64),
                    enabled: true,
                }
            }
            None => SpeedController {
                bucket: TokenBucket::new(f64::INFINITY),
                aimd: Aimd::new(16.0 * 1024.0 * 1024.0, 1024.0, 1.0 * 1024.0 * 1024.0 * 1024.0),
                manual_limit: None,
                enabled: false,
            },
        }
    }

    /// Call before reading/writing a chunk of `n` bytes.
    pub fn before_chunk(&mut self, n: u64) {
        if self.enabled {
            self.bucket.acquire(n);
        }
    }

    /// Call after a chunk completed. `stalled` = the chunk took noticeably
    /// longer than expected (congestion signal for AIMD).
    pub fn after_chunk(&mut self, _bytes: u64, stalled: bool) {
        if self.manual_limit.is_some() {
            return; // fixed limit: no adaptation
        }
        if stalled {
            self.aimd.on_congestion();
            self.bucket.set_rate(self.aimd.rate());
        } else {
            self.aimd.on_success();
            self.bucket.set_rate(self.aimd.rate());
        }
    }

    /// Whether any throttling is active.
    pub fn is_active(&self) -> bool {
        self.enabled
    }

    /// Effective current rate (bytes/sec), for progress display.
    pub fn effective_rate(&self) -> f64 {
        if self.manual_limit.is_some() {
            self.bucket.rate()
        } else {
            self.aimd.rate()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unlimited_is_noop() {
        let mut c = SpeedController::new(None);
        assert!(!c.is_active());
        let t = std::time::Instant::now();
        c.before_chunk(1024 * 1024);
        assert!(t.elapsed() < std::time::Duration::from_millis(20));
    }

    #[test]
    fn test_manual_limit_throttles_and_disables_aimd() {
        let mut c = SpeedController::new(Some(1024 * 1024));
        assert!(c.is_active());
        assert!((c.effective_rate() - 1024.0 * 1024.0).abs() < 1.0);
        // after_chunk must not change the rate when a manual limit is set
        c.after_chunk(1024 * 1024, true);
        assert!((c.effective_rate() - 1024.0 * 1024.0).abs() < 1.0);
        let t = std::time::Instant::now();
        c.before_chunk(3 * 1024 * 1024);
        assert!(t.elapsed() >= std::time::Duration::from_millis(900));
    }

    #[test]
    fn test_aimd_adapts_bucket_rate() {
        let mut c = SpeedController::new(None);
        let r0 = c.effective_rate();
        c.after_chunk(1024, true);
        let r1 = c.effective_rate();
        assert!(r1 < r0);
        c.after_chunk(1024, false);
        assert!(c.effective_rate() > r1);
    }
}
