//! Material Design 3 shape scale: corner radii as egui [`Rounding`].
//!
//! Tokens (dp ≈ px at 1× scale): none 0, extra-small 4, small 8, medium 12,
//! large 16, extra-large 28. "Full" (pill / circle) is represented by a
//! radius much larger than any widget half-height — egui clamps each corner
//! radius to the rect, so a big value renders as a perfect pill.

use egui::Rounding;

pub const NONE: f32 = 0.0;
pub const EXTRA_SMALL: f32 = 4.0;
pub const SMALL: f32 = 8.0;
pub const MEDIUM: f32 = 12.0;
pub const LARGE: f32 = 16.0;
pub const EXTRA_LARGE: f32 = 28.0;

/// Large enough that egui clamps it into a pill/circle for any widget size.
pub const FULL: f32 = 1000.0;

/// Uniform corner rounding of `px` pixels (the whole shape scale is uniform,
/// so a single constructor covers all tokens).
pub fn rounding(px: f32) -> Rounding {
    Rounding::same(px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_values() {
        assert_eq!(NONE, 0.0);
        assert_eq!(EXTRA_SMALL, 4.0);
        assert_eq!(SMALL, 8.0);
        assert_eq!(MEDIUM, 12.0);
        assert_eq!(LARGE, 16.0);
        assert_eq!(EXTRA_LARGE, 28.0);
    }

    #[test]
    fn rounding_is_uniform_and_monotonic() {
        let r = rounding(MEDIUM);
        assert_eq!((r.nw, r.ne, r.sw, r.se), (12.0, 12.0, 12.0, 12.0));
        assert!(FULL > EXTRA_LARGE);
    }
}
