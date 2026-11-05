//! MD3 components built directly on egui primitives: buttons, cards, list
//! items, linear progress, top app bar, snackbar and filter chips.
//!
//! Everything reads its colors from the active [`color::Scheme`] stashed by
//! [`crate::apply_theme`] (falling back to the light scheme), so components
//! stay in sync with the theme without any global mutable state.

use crate::{color, elevation, shape, state, typography};
use egui::{
    Align, Align2, Color32, Context, Frame, Id, InnerResponse, Layout, Margin, Rect, Response,
    RichText, Sense, TextStyle, Ui, Vec2,
};

/// Fetch the active scheme (falls back to the light baseline).
pub fn current_scheme(ctx: &Context) -> color::Scheme {
    ctx.data_mut(|d| d.get_temp::<color::Scheme>(crate::theme::scheme_id()))
        .unwrap_or_else(color::light)
}

// ---------------------------------------------------------------------------
// Buttons (40dp pill, state layers)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn md_button(
    ui: &mut Ui,
    text: &str,
    fill: Color32,
    text_color: Color32,
    state_on: Color32,
    outlined: bool,
) -> Response {
    let s = current_scheme(ui.ctx());
    let font = typography::label_large();

    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, text_color);
    let desired = Vec2::new(galley.size().x + 48.0, 40.0); // 24px side padding
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());

    let enabled = ui.is_enabled();
    let (fill, text_color, state_on) = if enabled {
        (fill, text_color, state_on)
    } else {
        // MD3 disabled: container = on-surface 12%, content = on-surface 38%.
        (
            state::state_layer(s.surface, s.on_surface, 0.12),
            state::disabled_content(s.on_surface),
            s.on_surface,
        )
    };

    ui.painter().rect_filled(rect, shape::rounding(shape::FULL), fill);
    if outlined && enabled {
        ui.painter()
            .rect_stroke(rect, shape::rounding(shape::FULL), stroke_of(&s));
    }

    // State layer (hover 8%, pressed 12%).
    let opacity = if !enabled {
        0.0
    } else if resp.is_pointer_button_down_on() {
        state::PRESSED
    } else if resp.hovered() {
        state::HOVER
    } else {
        0.0
    };
    if opacity > 0.0 {
        ui.painter().rect_filled(
            rect,
            shape::rounding(shape::FULL),
            state::state_layer(fill, state_on, opacity),
        );
    }

    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley, text_color);
    resp
}

fn stroke_of(s: &color::Scheme) -> egui::Stroke {
    egui::Stroke::new(1.0_f32, s.outline)
}

/// MD3 filled button (primary container, on-primary label).
pub fn filled_button(ui: &mut Ui, text: &str) -> Response {
    let s = current_scheme(ui.ctx());
    md_button(ui, text, s.primary, s.on_primary, s.on_primary, false)
}

/// Filled button with explicit colors (e.g. the destructive "Cancel" action
/// uses `error` / `on_error`).
pub fn filled_button_colored(ui: &mut Ui, text: &str, fill: Color32, on_fill: Color32) -> Response {
    md_button(ui, text, fill, on_fill, on_fill, false)
}

/// MD3 tonal button (secondary-container fill).
pub fn tonal_button(ui: &mut Ui, text: &str) -> Response {
    let s = current_scheme(ui.ctx());
    md_button(
        ui,
        text,
        s.secondary_container,
        s.on_secondary_container,
        s.on_secondary_container,
        false,
    )
}

/// MD3 outlined button (transparent fill, 1px outline).
pub fn outlined_button(ui: &mut Ui, text: &str) -> Response {
    let s = current_scheme(ui.ctx());
    md_button(
        ui,
        text,
        Color32::TRANSPARENT,
        s.primary,
        s.primary,
        true,
    )
}

/// MD3 text button (no fill, primary label).
pub fn text_button(ui: &mut Ui, text: &str) -> Response {
    let s = current_scheme(ui.ctx());
    md_button(ui, text, Color32::TRANSPARENT, s.primary, s.primary, false)
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

/// The two MD3 card variants KetraCopy uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// Surface-container-low fill plus elevation level 1 shadow.
    Elevated,
    /// Surface-container-highest fill, no shadow.
    Filled,
}

/// A Material card: `card(ui, CardKind::Filled, |ui| { … })`.
pub fn card<R>(ui: &mut Ui, kind: CardKind, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let s = current_scheme(ui.ctx());
    let f = match kind {
        CardKind::Elevated => Frame::none()
            .fill(s.surface_container_low)
            .rounding(shape::rounding(shape::MEDIUM))
            .shadow(elevation::shadow(1, s.dark)),
        CardKind::Filled => Frame::none()
            .fill(s.surface_container_highest)
            .rounding(shape::rounding(shape::MEDIUM)),
    };
    f.inner_margin(Margin::same(16.0)).show(ui, add_contents)
}

// ---------------------------------------------------------------------------
// List item
// ---------------------------------------------------------------------------

/// A one- or two-line MD3 list item with a trailing slot (56/72dp tall).
///
/// The returned response covers the whole row (hover only); interactive
/// trailing widgets (e.g. a Remove button) return their own responses.
pub fn list_item<R>(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    trailing: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let s = current_scheme(ui.ctx());
    let height = if subtitle.is_some() { 72.0 } else { 56.0 };
    let inner = ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width() - 120.0);
                ui.label(
                    RichText::new(title)
                        .text_style(TextStyle::Body)
                        .color(s.on_surface),
                );
                if let Some(sub) = subtitle {
                    ui.label(
                        RichText::new(sub)
                            .text_style(TextStyle::Small)
                            .color(s.on_surface_variant),
                    );
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), trailing)
        },
    );

    // Hover wash over the whole row (8% on-surface).
    let row = inner.response.rect;
    if inner.response.hovered() {
        ui.painter().rect_filled(
            row,
            0.0,
            state::state_layer(s.surface, s.on_surface, state::HOVER),
        );
    }
    // Unwrap the trailing closure's InnerResponse; expose the row response.
    InnerResponse {
        response: inner.response,
        inner: inner.inner.inner,
    }
}

// ---------------------------------------------------------------------------
// Linear progress
// ---------------------------------------------------------------------------

/// MD3 linear progress indicator with fully rounded ends.
///
/// `Some(0.0..=1.0)` renders determinate; `None` (or a negative fraction)
/// renders the indeterminate animation (and requests repaints).
/// Occupies the full remaining width at 6px tall.
pub fn linear_progress(ui: &mut Ui, fraction: Option<f32>) -> Response {
    let s = current_scheme(ui.ctx());
    let height = 6.0;
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());

    // Track.
    ui.painter()
        .rect_filled(rect, shape::rounding(height), s.surface_container_highest);

    let bar = |ui: &Ui, x0: f32, x1: f32| {
        if x1 > x0 {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    egui::pos2(rect.left() + x0, rect.top()),
                    egui::pos2(rect.left() + x1, rect.bottom()),
                ),
                shape::rounding(height),
                s.primary,
            );
        }
    };

    match fraction {
        Some(f) if f.is_finite() && f >= 0.0 => {
            let f = f.clamp(0.0, 1.0);
            // Keep the indicator at least one cap wide so the rounded ends
            // read as a pill even at 0%.
            let w = (width * f).max(height);
            bar(ui, 0.0, w);
        }
        _ => {
            // Indeterminate: a 1/3-width bar sweeping left → right.
            let t = ui.input(|i| i.time) as f32;
            let phase = (t * 0.7) % 1.4; // 0..1.4, includes a beat off-track
            let w = width / 3.0;
            let x0 = ((width + w) * (phase.min(1.0))) - w;
            bar(ui, x0, x0 + w);
            ui.ctx()
                .request_repaint_after(elevation::ANIM_INTERVAL);
        }
    }

    resp
}

// ---------------------------------------------------------------------------
// Top app bar
// ---------------------------------------------------------------------------

/// MD3 top app bar: 64dp surface bar with a title-large label and a
/// right-aligned trailing slot.
pub fn top_app_bar<R>(
    ui: &mut Ui,
    title: &str,
    trailing: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let s = current_scheme(ui.ctx());
    let inner = Frame::none()
        .fill(s.surface)
        .inner_margin(Margin {
            left: 16.0,
            right: 16.0,
            top: 0.0,
            bottom: 0.0,
        })
        .show(ui, |ui| {
            ui.set_min_size(Vec2::new(ui.available_width(), 64.0));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(
                    RichText::new(title)
                        .text_style(TextStyle::Heading)
                        .color(s.on_surface),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), trailing)
            })
        });
    // Hairline under the bar (MD3: surface-variant divider).
    let bar = inner.response.rect;
    ui.painter().hline(
        bar.x_range(),
        bar.bottom(),
        egui::Stroke::new(1.0_f32, s.outline_variant),
    );
    InnerResponse {
        response: inner.response,
        inner: inner.inner.inner.inner,
    }
}

// ---------------------------------------------------------------------------
// Snackbar-style status line
// ---------------------------------------------------------------------------

/// MD3 snackbar: an inverse-surface bar with rounded extra-small corners,
/// used here as a persistent status/warning line.
pub fn snackbar(ui: &mut Ui, message: &str) -> Response {
    let s = current_scheme(ui.ctx());
    let (rect, resp) = ui
        .allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::hover())
        ;
    ui.painter()
        .rect_filled(rect, shape::rounding(shape::EXTRA_SMALL), s.inverse_surface);
    let galley = ui.painter().layout(
        message.to_owned(),
        typography::body_medium(),
        s.inverse_on_surface,
        rect.width() - 32.0,
    );
    let pos = egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, s.inverse_on_surface);
    resp
}

// ---------------------------------------------------------------------------
// Filter chip
// ---------------------------------------------------------------------------

/// MD3 filter chip: 32dp, small corner radius; `selected` fills it with the
/// secondary container and prefixes a check mark.
pub fn chip(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let s = current_scheme(ui.ctx());
    let text = if selected {
        format!("✓ {label}")
    } else {
        label.to_owned()
    };
    let font = typography::label_large();
    let galley = ui.painter().layout_no_wrap(
        text.clone(),
        font,
        if selected {
            s.on_secondary_container
        } else {
            s.on_surface_variant
        },
    );
    let desired = Vec2::new(galley.size().x + 32.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());

    let (fill, text_color, on) = if selected {
        (s.secondary_container, s.on_secondary_container, s.on_secondary_container)
    } else {
        (s.surface, s.on_surface_variant, s.on_surface)
    };
    ui.painter()
        .rect_filled(rect, shape::rounding(shape::SMALL), fill);
    if !selected {
        ui.painter().rect_stroke(
            rect,
            shape::rounding(shape::SMALL),
            egui::Stroke::new(1.0_f32, s.outline_variant),
        );
    }
    let opacity = if resp.is_pointer_button_down_on() {
        state::PRESSED
    } else if resp.hovered() {
        state::HOVER
    } else {
        0.0
    };
    if opacity > 0.0 {
        ui.painter().rect_filled(
            rect,
            shape::rounding(shape::SMALL),
            state::state_layer(fill, on, opacity),
        );
    }
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley, text_color);
    resp
}

/// A modal scrim + centered card, used for the start-transfer confirmation.
///
/// Paints a 40% scrim over the whole screen and centers `add_contents` in a
/// filled card with elevation 3. Returns the card's inner response.
pub fn modal<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let s = current_scheme(ui.ctx());
    let screen = ui.ctx().screen_rect();
    egui::Area::new(Id::new("ketra-md3::modal-scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.left_top())
        .show(ui.ctx(), |ui| {
            ui.set_max_size(screen.size());
            ui.allocate_response(screen.size(), Sense::click());
            ui.painter().rect_filled(
                screen,
                0.0,
                Color32::from_black_alpha((255.0 * SCRIM_OPACITY).round() as u8),
            );
        });

    egui::Area::new(Id::new("ketra-md3::modal"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            Frame::none()
                .fill(s.surface_container_high)
                .rounding(shape::rounding(shape::EXTRA_LARGE))
                .shadow(elevation::shadow(3, s.dark))
                .inner_margin(Margin::same(24.0))
                .show(ui, add_contents)
        })
        .inner
}

/// Backdrop opacity of the [`modal`] scrim (40% black, per M3 dialogs).
pub const SCRIM_OPACITY: f32 = 0.40;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::from_hex;

    #[test]
    fn scrim_alpha_is_40_percent() {
        let a = (255.0 * SCRIM_OPACITY).round() as u8;
        assert_eq!(a, 102);
    }

    #[test]
    fn components_exist_and_scheme_fallback_is_light() {
        // Headless context without apply_theme: current_scheme must fall
        // back to the light baseline.
        let ctx = Context::default();
        let s = current_scheme(&ctx);
        assert_eq!(s.primary, from_hex("#6750A4").unwrap());
    }
}
