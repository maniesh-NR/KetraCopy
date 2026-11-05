//! Material Design 3 elevation tokens as egui [`Shadow`]s.
//!
//! MD3 specifies level 0–5. The spec's umbra/penumbra pairs are folded into
//! one egui shadow per level (blur + y-offset + spread), with the shadow
//! alpha adapted for light vs. dark surfaces.

use egui::{Shadow, Vec2};
use std::time::Duration;

/// One elevation level: blur radius, downward offset and spread in px.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub blur: u32,
    pub offset_y: u32,
    pub spread: u32,
}

/// MD3 elevation levels 1..=5 (index 0 → level 1).
pub const LEVELS: [Level; 5] = [
    Level { blur: 3, offset_y: 1, spread: 1 },
    Level { blur: 6, offset_y: 2, spread: 2 },
    Level { blur: 8, offset_y: 4, spread: 4 },
    Level { blur: 12, offset_y: 6, spread: 6 },
    Level { blur: 16, offset_y: 8, spread: 8 },
];

/// The egui shadow for MD3 elevation `level` (1..=5; 0 → no shadow).
///
/// `dark` switches to a stronger shadow, since dark surfaces need more
/// contrast for elevation to read (per the M3 dark-theme guidance).
pub fn shadow(level: u32, dark: bool) -> Shadow {
    if level == 0 {
        return Shadow::NONE;
    }
    let l = LEVELS[(level as usize).min(LEVELS.len()) - 1];
    let alpha = if dark { 100 } else { 46 };
    Shadow {
        offset: Vec2::new(0.0, l.offset_y as f32),
        blur: l.blur as f32,
        spread: l.spread as f32,
        color: egui::Color32::from_black_alpha(alpha),
    }
}

/// Convenience for repaint pacing used by animated (indeterminate) widgets.
pub const ANIM_INTERVAL: Duration = Duration::from_millis(33);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_is_monotonic() {
        for w in LEVELS.windows(2) {
            assert!(w[1].blur > w[0].blur);
            assert!(w[1].offset_y >= w[0].offset_y);
            assert!(w[1].spread > w[0].spread);
        }
    }

    #[test]
    fn level_zero_is_no_shadow_and_clamped() {
        assert_eq!(shadow(0, false), Shadow::NONE);
        // Out-of-range levels clamp to the last token instead of panicking.
        assert_eq!(shadow(99, true), shadow(5, true));
    }

    #[test]
    fn dark_shadows_are_stronger() {
        let l = shadow(2, false);
        let d = shadow(2, true);
        assert!(d.color.a() > l.color.a());
        assert_eq!(d.blur, l.blur);
    }
}
