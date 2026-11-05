//! Material Design 3 type scale, mapped onto egui's [`TextStyle`] keys.
//!
//! MD3 defines 15 roles (display/headline/title/body/label × large/medium/
//! small) at fixed point sizes. egui only has five text-style slots, so this
//! module maps them as:
//!
//! | egui `TextStyle` | MD3 role        | pt |
//! |------------------|-----------------|----|
//! | `Heading`        | headline-small  | 24 |
//! | `Body`           | body-large      | 16 |
//! | `Button`         | label-large     | 14 |
//! | `Small`          | body-small      | 12 |
//! | `Monospace`      | body (mono)     | 13 |
//!
//! The remaining roles are exposed as [`FontId`]s for direct use by the
//! components (e.g. [`title_large`] for the top app bar).

use egui::{FontId, TextStyle};
use std::collections::BTreeMap;

// ---- display ----
pub const DISPLAY_LARGE: f32 = 57.0;
pub const DISPLAY_MEDIUM: f32 = 45.0;
pub const DISPLAY_SMALL: f32 = 36.0;
// ---- headline ----
pub const HEADLINE_LARGE: f32 = 32.0;
pub const HEADLINE_MEDIUM: f32 = 28.0;
pub const HEADLINE_SMALL: f32 = 24.0;
// ---- title ----
pub const TITLE_LARGE: f32 = 22.0;
pub const TITLE_MEDIUM: f32 = 16.0;
pub const TITLE_SMALL: f32 = 14.0;
// ---- body ----
pub const BODY_LARGE: f32 = 16.0;
pub const BODY_MEDIUM: f32 = 14.0;
pub const BODY_SMALL: f32 = 12.0;
// ---- label ----
pub const LABEL_LARGE: f32 = 14.0;
pub const LABEL_MEDIUM: f32 = 12.0;
pub const LABEL_SMALL: f32 = 11.0;

/// Font for an MD3 role, by size name (`display_large`, `label_small`, …).
pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn display_large() -> FontId {
    FontId::proportional(DISPLAY_LARGE)
}
pub fn display_medium() -> FontId {
    FontId::proportional(DISPLAY_MEDIUM)
}
pub fn display_small() -> FontId {
    FontId::proportional(DISPLAY_SMALL)
}
pub fn headline_large() -> FontId {
    FontId::proportional(HEADLINE_LARGE)
}
pub fn headline_medium() -> FontId {
    FontId::proportional(HEADLINE_MEDIUM)
}
pub fn headline_small() -> FontId {
    FontId::proportional(HEADLINE_SMALL)
}
pub fn title_large() -> FontId {
    FontId::proportional(TITLE_LARGE)
}
pub fn title_medium() -> FontId {
    FontId::proportional(TITLE_MEDIUM)
}
pub fn title_small() -> FontId {
    FontId::proportional(TITLE_SMALL)
}
pub fn body_large() -> FontId {
    FontId::proportional(BODY_LARGE)
}
pub fn body_medium() -> FontId {
    FontId::proportional(BODY_MEDIUM)
}
pub fn body_small() -> FontId {
    FontId::proportional(BODY_SMALL)
}
pub fn label_large() -> FontId {
    FontId::proportional(LABEL_LARGE)
}
pub fn label_medium() -> FontId {
    FontId::proportional(LABEL_MEDIUM)
}
pub fn label_small() -> FontId {
    FontId::proportional(LABEL_SMALL)
}

/// The egui [`Style::text_styles`] map that installs the MD3 type scale.
pub fn style_map() -> BTreeMap<TextStyle, FontId> {
    let mut m = BTreeMap::new();
    m.insert(TextStyle::Heading, headline_small());
    m.insert(TextStyle::Body, body_large());
    m.insert(TextStyle::Button, label_large());
    m.insert(TextStyle::Small, body_small());
    m.insert(TextStyle::Monospace, FontId::monospace(13.0));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md3_type_sizes() {
        assert_eq!(DISPLAY_LARGE, 57.0);
        assert_eq!(HEADLINE_MEDIUM, 28.0);
        assert_eq!(TITLE_LARGE, 22.0);
        assert_eq!(BODY_LARGE, 16.0);
        assert_eq!(BODY_MEDIUM, 14.0);
        assert_eq!(LABEL_LARGE, 14.0);
        assert_eq!(LABEL_SMALL, 11.0);
    }

    #[test]
    fn style_map_covers_all_egui_slots() {
        let m = style_map();
        for t in [
            TextStyle::Small,
            TextStyle::Body,
            TextStyle::Button,
            TextStyle::Heading,
            TextStyle::Monospace,
        ] {
            assert!(m.contains_key(&t), "missing {t:?}");
        }
        // MD3 size ramp is monotonic per role family.
        assert!(DISPLAY_LARGE > DISPLAY_MEDIUM && DISPLAY_MEDIUM > DISPLAY_SMALL);
        assert!(HEADLINE_LARGE > HEADLINE_MEDIUM && HEADLINE_MEDIUM > HEADLINE_SMALL);
        assert!(TITLE_LARGE > TITLE_MEDIUM && TITLE_MEDIUM > TITLE_SMALL);
        assert!(BODY_LARGE > BODY_MEDIUM && BODY_MEDIUM > BODY_SMALL);
        assert!(LABEL_LARGE >= LABEL_MEDIUM && LABEL_MEDIUM > LABEL_SMALL);
    }

    #[test]
    fn fonts_carry_the_right_family() {
        assert_eq!(label_large().family, egui::FontFamily::Proportional);
        assert_eq!(FontId::monospace(13.0).family, egui::FontFamily::Monospace);
        assert_eq!(body_large().size, 16.0);
    }
}
