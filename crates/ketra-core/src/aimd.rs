//! AIMD (Additive-Increase / Multiplicative-Decrease) congestion-style
//! rate adaptation, after Chiu & Jain, "Analysis of the Increase and
//! Decrease Algorithms for Congestion Avoidance in Computer Networks",
//! Computer Networks and ISDN Systems 17(1), 1989.
//!
//! The copy engine treats a stalled chunk read/write (slow device, busy
//! network) as a congestion signal: the rate is cut multiplicatively.
//! Smooth chunks additively increase the rate again, probing for the
//! available bandwidth.

/// AIMD rate controller.
pub struct Aimd {
    rate: f64,
    min_rate: f64,
    max_rate: f64,
    add: f64,
    decrease: f64,
}

impl Aimd {
    /// Create a controller starting at `start` bytes/sec.
    pub fn new(start: f64, min_rate: f64, max_rate: f64) -> Self {
        let start = start.clamp(min_rate, max_rate);
        Aimd { rate: start, min_rate, max_rate, add: (max_rate - min_rate) / 100.0, decrease: 0.5 }
    }

    /// Current target rate in bytes/sec.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// A chunk completed smoothly: additive increase.
    pub fn on_success(&mut self) {
        self.rate = (self.rate + self.add).min(self.max_rate);
    }

    /// A chunk stalled (congestion signal): multiplicative decrease.
    pub fn on_congestion(&mut self) {
        self.rate = (self.rate * self.decrease).max(self.min_rate);
    }

    /// Hard drop back to the minimum rate (e.g. an I/O error occurred).
    pub fn on_error(&mut self) {
        self.rate = self.min_rate;
    }

    /// Override the target rate (used when the user sets a fixed limit).
    pub fn set_rate(&mut self, r: f64) {
        self.rate = r.clamp(self.min_rate, self.max_rate);
    }

    /// Clamp bounds.
    pub fn bounds(&self) -> (f64, f64) {
        (self.min_rate, self.max_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_additive_increase_is_linear() {
        // Two successes add exactly twice as much as one success.
        let mut one = Aimd::new(1_000_000.0, 64.0 * 1024.0, 10_000_000.0);
        let base = one.rate();
        one.on_success();
        let d1 = one.rate() - base;

        let mut two = Aimd::new(1_000_000.0, 64.0 * 1024.0, 10_000_000.0);
        two.on_success();
        two.on_success();
        assert!((two.rate() - base - 2.0 * d1).abs() < 1e-6);
    }

    #[test]
    fn test_multiplicative_decrease_halves() {
        let mut a = Aimd::new(2_000_000.0, 64.0 * 1024.0, 10_000_000.0);
        a.on_congestion();
        assert!((a.rate() - 1_000_000.0).abs() < 1.0);
        a.on_error();
        assert_eq!(a.rate(), 64.0 * 1024.0);
    }

    #[test]
    fn test_bounds_are_respected() {
        let mut a = Aimd::new(9_900_000.0, 64.0 * 1024.0, 10_000_000.0);
        for _ in 0..50 {
            a.on_success();
        }
        assert_eq!(a.rate(), 10_000_000.0);
        for _ in 0..50 {
            a.on_congestion();
        }
        assert_eq!(a.rate(), 64.0 * 1024.0);
        a.set_rate(1_000_000_000.0);
        assert_eq!(a.rate(), 10_000_000.0);
    }
}
