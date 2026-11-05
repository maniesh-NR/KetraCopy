//! Install an MD3 scheme into egui's [`Style`] / [`Visuals`].
//!
//! [`apply_theme`] is idempotent and cheap; call it once at startup and
//! again whenever the light/dark preference changes. It also stashes the
//! active [`Scheme`] in egui's data map so the components can read exact
//! MD3 roles without a global.

use crate::{color, elevation, shape, typography};
use egui::{Color32, Context, Id, Stroke, Style, Vec2, Visuals};

/// Where the active scheme is stashed for [`crate::current_scheme`].
/// (Not a `const`: `Id::new` is not `const fn` in egui 0.28.)
pub fn scheme_id() -> Id {
    Id::new("ketra-md3::scheme")
}

/// Apply the MD3 light or dark baseline scheme to the context.
pub fn apply_theme(ctx: &Context, dark: bool) {
    let scheme = if dark { color::dark() } else { color::light() };

    // Stash for the components.
    ctx.data_mut(|d| d.insert_temp(scheme_id(), scheme));

    let mut style = Style::clone(&ctx.style());
    style.visuals = visuals_from(&scheme);
    style.text_styles = typography::style_map();

    // Spacing: MD3 relies on generous, consistent gaps.
    let sp = &mut style.spacing;
    sp.item_spacing = Vec2::new(8.0, 8.0);
    sp.button_padding = Vec2::new(16.0, 8.0); // label-large in a 40px pill
    sp.menu_margin = egui::Margin::same(8.0);
    sp.window_margin = egui::Margin::same(16.0);
    sp.interact_size = Vec2::new(40.0, 32.0);
    sp.slider_width = 160.0;
    sp.slider_rail_height = 8.0;
    sp.combo_width = 200.0;
    sp.indent = 20.0;

    style.animation_time = 0.12;

    ctx.set_style(style);
}

/// Build egui [`Visuals`] from an MD3 [`color::Scheme`].
///
/// Public so apps that build their own [`Style`] can embed it.
pub fn visuals_from(s: &color::Scheme) -> Visuals {
    let mut v = if s.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    v.dark_mode = s.dark;
    v.override_text_color = Some(s.on_surface);

    v.panel_fill = s.surface;
    v.window_fill = s.surface;
    v.extreme_bg_color = s.surface_container_lowest; // text edits, scroll bars
    v.faint_bg_color = s.surface_container; // striped rows
    v.code_bg_color = s.surface_container_high;
    v.warn_fg_color = s.tertiary;
    v.error_fg_color = s.error;
    v.hyperlink_color = s.primary;

    v.selection.bg_fill = s.primary;
    v.selection.stroke = Stroke::new(1.0_f32, s.on_primary);

    // Windows / popups / menus.
    v.window_rounding = shape::rounding(shape::LARGE);
    v.window_shadow = elevation::shadow(3, s.dark);
    v.window_fill = s.surface;
    v.window_stroke = Stroke::new(1.0_f32, s.outline_variant);
    v.popup_shadow = elevation::shadow(2, s.dark);
    v.menu_rounding = shape::rounding(shape::EXTRA_SMALL);
    v.panel_fill = s.surface;

    // Widgets.
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = s.surface_container_high;
    w.noninteractive.weak_bg_fill = s.surface_container_high;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, s.outline_variant);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, s.on_surface); // normal text
    w.noninteractive.rounding = shape::rounding(shape::EXTRA_SMALL);

    w.inactive.bg_fill = s.secondary_container;
    w.inactive.weak_bg_fill = s.secondary_container;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, s.on_secondary_container);
    w.inactive.rounding = shape::rounding(shape::SMALL);

    // Hover/active/open use pre-blended state layers over the tonal
    // container (hover 8%, pressed 12%).
    w.hovered.bg_fill = crate::state::state_layer(
        s.secondary_container,
        s.on_secondary_container,
        crate::state::HOVER,
    );
    w.hovered.weak_bg_fill = w.hovered.bg_fill;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, s.outline);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, s.on_secondary_container);
    w.hovered.rounding = shape::rounding(shape::SMALL);

    w.active.bg_fill = crate::state::state_layer(
        s.secondary_container,
        s.on_secondary_container,
        crate::state::PRESSED,
    );
    w.active.weak_bg_fill = w.active.bg_fill;
    w.active.bg_stroke = Stroke::new(1.0_f32, s.primary);
    w.active.fg_stroke = Stroke::new(1.0_f32, s.on_secondary_container);
    w.active.rounding = shape::rounding(shape::SMALL);

    w.open.bg_fill = w.hovered.bg_fill;
    w.open.weak_bg_fill = w.hovered.weak_bg_fill;
    w.open.bg_stroke = Stroke::new(1.0_f32, s.outline);
    w.open.fg_stroke = w.hovered.fg_stroke;
    w.open.rounding = shape::rounding(shape::EXTRA_SMALL);

    // Text cursor: 2px primary.
    v.text_cursor.stroke = Stroke::new(2.0_f32, s.primary);

    v
}

/// Convenience: egui's own "faint" fill, useful for separators.
pub fn hairline(s: &color::Scheme) -> Color32 {
    s.outline_variant
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color;

    /// Building visuals from both schemes must not panic and must agree
    /// with the scheme's own light/dark flag.
    #[test]
    fn visuals_from_schemes() {
        let lv = visuals_from(&color::light());
        let dv = visuals_from(&color::dark());
        assert!(!lv.dark_mode);
        assert!(dv.dark_mode);
        assert_eq!(lv.panel_fill, color::light().surface);
        assert_eq!(dv.panel_fill, color::dark().surface);
        // Selection follows the primary role.
        assert_eq!(lv.selection.bg_fill, color::light().primary);
        assert_eq!(dv.selection.bg_fill, color::dark().primary);
        // Hovered widgets carry a pre-blended state layer (8% over container).
        assert_ne!(lv.widgets.hovered.bg_fill, color::light().secondary_container);
    }

    #[test]
    fn hairline_is_outline_variant() {
        assert_eq!(hairline(&color::light()), color::light().outline_variant);
    }
}
