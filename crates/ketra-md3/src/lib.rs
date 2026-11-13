//! ketra-md3 — a small Material Design 3 design system for egui.
//!
//! Deliberately minimal: depends only on the `egui` crate (which `eframe`
//! re-exports), so the design system stays reusable by any egui-based UI
//! and remains easy to test headlessly.
//!
//! Layers:
//!
//! | Module | Role |
//! |---|---|
//! | [`color`] | MD3 light/dark color scheme tokens |
//! | [`typography`] | MD3 type scale (`FontId`s for egui text styles) |
//! | [`elevation`] | MD3 elevation levels + shadow/overlay rules |
//! | [`shape`] | corner rounding scale |
//! | [`state`] | state-layer opacities (hover/press/disabled) |
//! | [`theme`] | apply the scheme to egui `Context` / `Visuals` |
//! | [`components`] | filled/tonal/outlined/text buttons, cards, list items, progress, top app bar, snackbar, chips |

pub mod color;
pub mod components;
pub mod elevation;
pub mod shape;
pub mod state;
pub mod theme;
pub mod typography;

/// Crate version (workspace).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schemes_are_complete() {
        // Both schemes must produce distinct, sane containers.
        let light = color::light();
        let dark = color::dark();
        assert_ne!(light.surface, dark.surface);
        assert_ne!(light.primary, dark.primary);
    }

    #[test]
    fn test_typography_covers_egui_styles() {
        let map = typography::style_map();
        for style in [
            egui::TextStyle::Heading,
            egui::TextStyle::Body,
            egui::TextStyle::Monospace,
            egui::TextStyle::Button,
            egui::TextStyle::Small,
        ] {
            assert!(map.contains_key(&style), "missing {style:?}");
        }
    }

    #[test]
    fn test_elevation_monotonic() {
        // Higher elevation = stronger shadow, never lighter than level 0.
        let l0 = elevation::shadow(0, false);
        let l2 = elevation::shadow(2, false);
        assert!(l2.spread >= l0.spread);
    }

    #[test]
    fn test_rounding_scale() {
        assert_eq!(shape::rounding(0.0).nw, 0.0);
        assert!(shape::rounding(12.0).nw > 0.0);
    }

    #[test]
    fn test_state_layers() {
        let on = egui::Color32::BLACK;
        // state layer = on-color at opacity over container: differs from container
        assert_ne!(state::state_layer(egui::Color32::WHITE, on, 0.08), egui::Color32::WHITE);
        // disabled content = on-color at 38% opacity (alpha ≈ 97)
        let d = state::disabled_content(egui::Color32::BLACK);
        assert_eq!(d.a(), 97);
    }

    #[test]
    fn test_version_present() {
        assert!(!VERSION.is_empty());
    }
}
