//! Material Design 3 color roles and the baseline (purple seed) schemes.
//!
//! Values follow the M3 baseline published at m3.material.io (2021 baseline
//! with the 2023 surface-container tonal roles). Both the light and the dark
//! scheme are represented by [`Scheme`]; every field is one of the standard
//! MD3 color roles.

use egui::Color32;

/// Parse a `"#RRGGBB"` (or `"RRGGBB"`) hex string into a [`Color32`].
///
/// Returns `None` when the string is not exactly 6 hex digits (after an
/// optional `#`). Used by the scheme tables and unit-tested.
pub fn from_hex(hex: &str) -> Option<Color32> {
    let body = hex.strip_prefix('#').unwrap_or(hex);
    if body.len() != 6 || !body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&body[i..i + 2], 16).ok();
    Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

/// Linear (gamma-space) blend of `overlay` over `base` at `alpha` (0.0..=1.0).
///
/// MD3 state layers are specified exactly this way: the "on" color painted
/// over the container at a fixed opacity. `alpha` is clamped; both colors are
/// treated as opaque. `(1.0, black→white)` yields `0x80` per channel, which
/// the unit tests pin down.
pub fn blend(base: Color32, overlay: Color32, alpha: f32) -> Color32 {
    let a = alpha.clamp(0.0, 1.0);
    let mix = |b: u8, o: u8| -> u8 {
        let v = b as f32 * (1.0 - a) + o as f32 * a;
        v.round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(
        mix(base.r(), overlay.r()),
        mix(base.g(), overlay.g()),
        mix(base.b(), overlay.b()),
    )
}

/// One complete Material Design 3 color scheme (light or dark).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scheme {
    pub dark: bool,

    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,

    pub secondary: Color32,
    pub on_secondary: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,

    pub tertiary: Color32,
    pub on_tertiary: Color32,
    pub tertiary_container: Color32,
    pub on_tertiary_container: Color32,

    pub error: Color32,
    pub on_error: Color32,
    pub error_container: Color32,
    pub on_error_container: Color32,

    pub surface: Color32,
    pub surface_dim: Color32,
    pub surface_bright: Color32,
    pub surface_container_lowest: Color32,
    pub surface_container_low: Color32,
    pub surface_container: Color32,
    pub surface_container_high: Color32,
    pub surface_container_highest: Color32,

    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,

    pub inverse_surface: Color32,
    pub inverse_on_surface: Color32,
    pub inverse_primary: Color32,

    pub scrim: Color32,
    pub shadow: Color32,
}

/// The baseline MD3 light scheme (purple seed `#6750A4`).
pub fn light() -> Scheme {
    Scheme {
        dark: false,

        primary: from_hex("#6750A4").expect("valid hex"),
        on_primary: from_hex("#FFFFFF").expect("valid hex"),
        primary_container: from_hex("#EADDFF").expect("valid hex"),
        on_primary_container: from_hex("#21005D").expect("valid hex"),

        secondary: from_hex("#625B71").expect("valid hex"),
        on_secondary: from_hex("#FFFFFF").expect("valid hex"),
        secondary_container: from_hex("#E8DEF8").expect("valid hex"),
        on_secondary_container: from_hex("#1D192B").expect("valid hex"),

        tertiary: from_hex("#7D5260").expect("valid hex"),
        on_tertiary: from_hex("#FFFFFF").expect("valid hex"),
        tertiary_container: from_hex("#FFD8E4").expect("valid hex"),
        on_tertiary_container: from_hex("#31111D").expect("valid hex"),

        error: from_hex("#B3261E").expect("valid hex"),
        on_error: from_hex("#FFFFFF").expect("valid hex"),
        error_container: from_hex("#F9DEDC").expect("valid hex"),
        on_error_container: from_hex("#410E0B").expect("valid hex"),

        surface: from_hex("#FFFBFE").expect("valid hex"),
        surface_dim: from_hex("#DED8E1").expect("valid hex"),
        surface_bright: from_hex("#FFFBFE").expect("valid hex"),
        surface_container_lowest: from_hex("#FFFFFF").expect("valid hex"),
        surface_container_low: from_hex("#F7F2FA").expect("valid hex"),
        surface_container: from_hex("#F3EDF7").expect("valid hex"),
        surface_container_high: from_hex("#ECE6F0").expect("valid hex"),
        surface_container_highest: from_hex("#E6E0E9").expect("valid hex"),

        on_surface: from_hex("#1C1B1F").expect("valid hex"),
        on_surface_variant: from_hex("#49454F").expect("valid hex"),
        outline: from_hex("#79747E").expect("valid hex"),
        outline_variant: from_hex("#CAC4D0").expect("valid hex"),

        inverse_surface: from_hex("#313033").expect("valid hex"),
        inverse_on_surface: from_hex("#F4EFF4").expect("valid hex"),
        inverse_primary: from_hex("#D0BCFF").expect("valid hex"),

        scrim: from_hex("#000000").expect("valid hex"),
        shadow: from_hex("#000000").expect("valid hex"),
    }
}

/// The baseline MD3 dark scheme (purple seed `#6750A4`).
pub fn dark() -> Scheme {
    Scheme {
        dark: true,

        primary: from_hex("#D0BCFF").expect("valid hex"),
        on_primary: from_hex("#381E72").expect("valid hex"),
        primary_container: from_hex("#4F378B").expect("valid hex"),
        on_primary_container: from_hex("#EADDFF").expect("valid hex"),

        secondary: from_hex("#CCC2DC").expect("valid hex"),
        on_secondary: from_hex("#332D41").expect("valid hex"),
        secondary_container: from_hex("#4A4458").expect("valid hex"),
        on_secondary_container: from_hex("#E8DEF8").expect("valid hex"),

        tertiary: from_hex("#EFB8C8").expect("valid hex"),
        on_tertiary: from_hex("#492532").expect("valid hex"),
        tertiary_container: from_hex("#633B48").expect("valid hex"),
        on_tertiary_container: from_hex("#FFD8E4").expect("valid hex"),

        error: from_hex("#F2B8B5").expect("valid hex"),
        on_error: from_hex("#601410").expect("valid hex"),
        error_container: from_hex("#8C1D18").expect("valid hex"),
        on_error_container: from_hex("#F9DEDC").expect("valid hex"),

        surface: from_hex("#1C1B1F").expect("valid hex"),
        surface_dim: from_hex("#141218").expect("valid hex"),
        surface_bright: from_hex("#3B383E").expect("valid hex"),
        surface_container_lowest: from_hex("#0F0D13").expect("valid hex"),
        surface_container_low: from_hex("#1D1B20").expect("valid hex"),
        surface_container: from_hex("#211F26").expect("valid hex"),
        surface_container_high: from_hex("#2B2930").expect("valid hex"),
        surface_container_highest: from_hex("#36343B").expect("valid hex"),

        on_surface: from_hex("#E6E1E5").expect("valid hex"),
        on_surface_variant: from_hex("#CAC4D0").expect("valid hex"),
        outline: from_hex("#938F99").expect("valid hex"),
        outline_variant: from_hex("#49454F").expect("valid hex"),

        inverse_surface: from_hex("#E6E1E5").expect("valid hex"),
        inverse_on_surface: from_hex("#313033").expect("valid hex"),
        inverse_primary: from_hex("#6750A4").expect("valid hex"),

        scrim: from_hex("#000000").expect("valid hex"),
        shadow: from_hex("#000000").expect("valid hex"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parse_round_trip() {
        let c = from_hex("#6750A4").unwrap();
        assert_eq!((c.r(), c.g(), c.b()), (0x67, 0x50, 0xA4));
        assert_eq!(from_hex("FFFBFE").unwrap(), Color32::from_rgb(0xFF, 0xFB, 0xFE));
        assert_eq!(from_hex(""), None);
        assert_eq!(from_hex("#12345"), None);
        assert_eq!(from_hex("#12345G"), None);
        assert_eq!(from_hex("#1234567"), None);
    }

    #[test]
    fn blend_math() {
        let black = Color32::from_rgb(0, 0, 0);
        let white = Color32::from_rgb(255, 255, 255);
        // 50% white over black = mid gray (0x80).
        assert_eq!(blend(black, white, 0.5), Color32::from_rgb(128, 128, 128));
        // alpha 0 → base, alpha 1 → overlay.
        let base = Color32::from_rgb(10, 20, 30);
        let over = Color32::from_rgb(200, 210, 220);
        assert_eq!(blend(base, over, 0.0), base);
        assert_eq!(blend(base, over, 1.0), over);
        // alpha is clamped.
        assert_eq!(blend(base, over, 2.0), over);
        assert_eq!(blend(base, over, -1.0), base);
        // 8% white over the baseline primary (hover state layer of a filled
        // button in light mode) — pinned so ramp changes are noticed.
        let primary = from_hex("#6750A4").unwrap();
        let hovered = blend(primary, white, 0.08);
        assert_eq!((hovered.r(), hovered.g(), hovered.b()), (115, 94, 171));
    }

    #[test]
    fn baseline_scheme_values() {
        let l = light();
        assert_eq!(l.primary, from_hex("#6750A4").unwrap());
        assert_eq!(l.on_primary, Color32::WHITE);
        assert_eq!(l.primary_container, from_hex("#EADDFF").unwrap());
        assert_eq!(l.secondary_container, from_hex("#E8DEF8").unwrap());
        assert_eq!(l.tertiary_container, from_hex("#FFD8E4").unwrap());
        assert_eq!(l.error, from_hex("#B3261E").unwrap());
        assert!(!l.dark);

        let d = dark();
        assert_eq!(d.primary, from_hex("#D0BCFF").unwrap());
        assert_eq!(d.error, from_hex("#F2B8B5").unwrap());
        assert!(d.dark);
        // Light and dark must differ on the roles that flip.
        assert_ne!(l.surface, d.surface);
        assert_ne!(l.primary, d.primary);
    }
}
