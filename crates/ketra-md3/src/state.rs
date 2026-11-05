//! Material Design 3 state layers.
//!
//! Interactive surfaces communicate hover / focus / pressed / dragged states
//! by painting the *on-color* of the container over the container at a fixed
//! opacity: hover 8%, pressed and focused 12%, dragged 16%.

use crate::color::blend;
use egui::Color32;

pub const HOVER: f32 = 0.08;
pub const PRESSED: f32 = 0.12;
pub const FOCUS: f32 = 0.12;
pub const DRAGGED: f32 = 0.16;

/// Content opacity for disabled containers/components: 38%.
pub const DISABLED_CONTENT: f32 = 0.38;

/// The state-layer color: `on` blended over `container` at `opacity`.
///
/// Example (light scheme, filled button): `state_layer(primary, on_primary,
/// HOVER)` is primary with an 8% white wash.
pub fn state_layer(container: Color32, on: Color32, opacity: f32) -> Color32 {
    blend(container, on, opacity)
}

/// Content (text/icons) drawn on disabled containers: the on-color at 38%
/// opacity, expressed as an rgba color so it can be painted over anything.
pub fn disabled_content(on: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        on.r(),
        on.g(),
        on.b(),
        (255.0 * DISABLED_CONTENT).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::from_hex;

    #[test]
    fn opacities_match_spec() {
        assert_eq!(HOVER, 0.08);
        assert_eq!(PRESSED, 0.12);
        assert_eq!(FOCUS, 0.12);
        assert_eq!(DRAGGED, 0.16);
        assert!(HOVER < PRESSED && PRESSED < DRAGGED);
    }

    #[test]
    fn hover_layer_on_baseline_primary() {
        let s = crate::color::light();
        let layer = state_layer(s.primary, s.on_primary, HOVER);
        // Same math as the color test: 8% white over #6750A4.
        assert_eq!((layer.r(), layer.g(), layer.b()), (115, 94, 171));
    }

    #[test]
    fn disabled_content_is_faint() {
        let s = crate::color::light();
        // 38% opacity ≈ alpha 97.
        assert_eq!(disabled_content(s.on_surface).a(), 97);
        assert_eq!(disabled_content(from_hex("#FFFFFF").unwrap()).a(), 97);
    }
}
